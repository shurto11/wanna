//! Google Tasks → wannad の一度きりの移行ツール。
//!
//! calendar-tui の token.json を再利用して全件取得し、imp/clau を2値に畳んで wannad に POST する。
//! ID は Google Tasks の ID から UUIDv5 で決めるので、何度実行しても重複しない。
//!
//! ```text
//! wanna-migrate [--dry-run] [--threshold 6] [--server URL] [--token TOKEN] [--credentials PATH]
//! ```

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::PathBuf;
use wanna_core::{pos, Quadrant, Want};

/// notes 末尾に埋め込んでいた旧メタデータ行のマーカー
const META_MARKER: &str = "[todo-meta]";

struct Args {
    dry_run: bool,
    threshold: u8,
    server: Option<String>,
    token: Option<String>,
    credentials: Option<PathBuf>,
}

fn parse_args() -> Result<Args> {
    let mut a = Args { dry_run: false, threshold: 6, server: None, token: None, credentials: None };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        let mut val = || it.next().with_context(|| format!("{arg} の値がありません"));
        match arg.as_str() {
            "--dry-run" | "-n" => a.dry_run = true,
            "--threshold" => a.threshold = val()?.parse()?,
            "--server" => a.server = Some(val()?),
            "--token" => a.token = Some(val()?),
            "--credentials" => a.credentials = Some(PathBuf::from(val()?)),
            "-h" | "--help" => {
                println!(
                    "usage: wanna-migrate [--dry-run] [--threshold 6] [--server URL] [--token TOKEN] [--credentials PATH]\n\
                     server/token を省略すると ~/.config/wanna/config.toml を使う"
                );
                std::process::exit(0);
            }
            _ => bail!("不明な引数: {arg}"),
        }
    }
    Ok(a)
}

fn config_dir(app: &str) -> PathBuf {
    dirs::config_dir().unwrap_or_else(|| PathBuf::from(".")).join(app)
}

// ───────── Google 認証 (calendar-tui の token.json を再利用) ─────────

#[derive(Deserialize)]
struct Token {
    access_token: String,
    refresh_token: Option<String>,
    expires_at: Option<i64>,
}

#[derive(Deserialize)]
struct Credentials {
    installed: Installed,
}

#[derive(Deserialize)]
struct Installed {
    client_id: String,
    client_secret: String,
}

fn access_token(http: &reqwest::blocking::Client, creds_path: Option<PathBuf>) -> Result<String> {
    let path = config_dir("calendar-tui").join("token.json");
    let token: Token = serde_json::from_str(
        &std::fs::read_to_string(&path)
            .with_context(|| format!("{} を読めません。先に calendar-tui で認証してください", path.display()))?,
    )?;
    let fresh = token.expires_at.is_none_or(|e| chrono::Utc::now().timestamp() < e - 60);
    if fresh {
        return Ok(token.access_token);
    }
    let refresh = token.refresh_token.context("token.json に refresh_token がありません")?;
    let creds_path = creds_path.unwrap_or_else(|| config_dir("calendar-tui").join("credentials.json"));
    let creds: Credentials = serde_json::from_str(
        &std::fs::read_to_string(&creds_path)
            .with_context(|| format!("{} を読めません (--credentials で指定可)", creds_path.display()))?,
    )?;
    #[derive(Deserialize)]
    struct Refreshed {
        access_token: String,
    }
    let res = http
        .post("https://oauth2.googleapis.com/token")
        .form(&[
            ("client_id", creds.installed.client_id.as_str()),
            ("client_secret", creds.installed.client_secret.as_str()),
            ("refresh_token", refresh.as_str()),
            ("grant_type", "refresh_token"),
        ])
        .send()?;
    if !res.status().is_success() {
        bail!("トークンのリフレッシュに失敗: {} {}", res.status(), res.text().unwrap_or_default());
    }
    Ok(res.json::<Refreshed>()?.access_token)
}

// ───────── Google Tasks ─────────

#[derive(Deserialize)]
struct Page<T> {
    items: Option<Vec<T>>,
    #[serde(rename = "nextPageToken")]
    next_page_token: Option<String>,
}

#[derive(Deserialize)]
struct GList {
    id: String,
    title: Option<String>,
}

#[derive(Deserialize, Clone)]
struct GTask {
    id: String,
    title: Option<String>,
    notes: Option<String>,
    status: Option<String>,
    completed: Option<String>,
    position: Option<String>,
    parent: Option<String>,
    updated: Option<String>,
    #[serde(default)]
    deleted: bool,
}

fn fetch_all<T: for<'de> Deserialize<'de>>(
    http: &reqwest::blocking::Client,
    token: &str,
    url: &str,
    query: &[(&str, &str)],
) -> Result<Vec<T>> {
    let mut out = Vec::new();
    let mut page: Option<String> = None;
    loop {
        let mut req = http.get(url).bearer_auth(token).query(query).query(&[("maxResults", "100")]);
        if let Some(p) = &page {
            req = req.query(&[("pageToken", p)]);
        }
        let res = req.send()?;
        if !res.status().is_success() {
            bail!("Tasks API エラー {}: {}", res.status(), res.text().unwrap_or_default());
        }
        let p: Page<T> = res.json()?;
        out.extend(p.items.unwrap_or_default());
        page = p.next_page_token;
        if page.is_none() {
            return Ok(out);
        }
    }
}

// ───────── メタ (imp / clau) ─────────

#[derive(Deserialize, Clone, Copy)]
struct Meta {
    #[serde(default = "five")]
    imp: u8,
    #[serde(default = "five")]
    clau: u8,
}

fn five() -> u8 {
    5
}

/// notes から旧 `[todo-meta]` 行を取り除き、(本文, メタ) に分ける
fn split_notes(notes: &str) -> (String, Option<Meta>) {
    let mut meta = None;
    let mut body = Vec::new();
    for line in notes.lines() {
        if let Some(json) = line.trim().strip_prefix(META_MARKER) {
            if let Ok(m) = serde_json::from_str::<Meta>(json.trim()) {
                meta = Some(m);
                continue;
            }
        }
        body.push(line);
    }
    (body.join("\n").trim().to_string(), meta)
}

fn main() -> Result<()> {
    let args = parse_args()?;
    let http = reqwest::blocking::Client::new();
    let token = access_token(&http, args.credentials.clone())?;

    let local_meta: HashMap<String, Meta> =
        std::fs::read_to_string(config_dir("calendar-tui").join("task_meta.json"))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();

    let lists: Vec<GList> =
        fetch_all(&http, &token, "https://tasks.googleapis.com/tasks/v1/users/@me/lists", &[])?;

    // (並び順キー, タスク) を集める。並びは リスト順 → 親の position → 子の position
    let mut tasks: Vec<((usize, String, String), GTask)> = Vec::new();
    for (li, list) in lists.iter().enumerate() {
        let items: Vec<GTask> = fetch_all(
            &http,
            &token,
            &format!("https://tasks.googleapis.com/tasks/v1/lists/{}/tasks", list.id),
            &[("showCompleted", "true"), ("showHidden", "true")],
        )?;
        eprintln!("リスト「{}」: {} 件", list.title.as_deref().unwrap_or("無題"), items.len());
        let position: HashMap<String, String> = items
            .iter()
            .map(|t| (t.id.clone(), t.position.clone().unwrap_or_default()))
            .collect();
        for t in items {
            let own = t.position.clone().unwrap_or_default();
            let key = match &t.parent {
                Some(p) => (li, position.get(p).cloned().unwrap_or_default(), own),
                None => (li, own, String::new()),
            };
            tasks.push((key, t));
        }
    }
    tasks.sort_by(|a, b| a.0.cmp(&b.0));

    // 区分ごとに分けて、元の並びのまま pos を振る
    let mut by_q: HashMap<Quadrant, Vec<Want>> = HashMap::new();
    let mut skipped = 0;
    for (_, t) in tasks {
        let title = t.title.as_deref().unwrap_or("").trim().to_string();
        if t.deleted || title.is_empty() {
            skipped += 1;
            continue;
        }
        let (notes, inline_meta) = split_notes(t.notes.as_deref().unwrap_or(""));
        let meta = local_meta.get(&t.id).copied().or(inline_meta);
        let (energy, clau) = match meta {
            Some(m) => (m.imp >= args.threshold, m.clau >= args.threshold),
            None => (false, false),
        };
        let done_at = (t.status.as_deref() == Some("completed"))
            .then(|| t.completed.clone().unwrap_or_else(wanna_core::now_rfc3339));
        let id = uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_URL, format!("gtasks:{}", t.id).as_bytes());
        by_q.entry(Quadrant { energy, clau }).or_default().push(Want {
            id: id.to_string(),
            title,
            notes,
            energy,
            clau,
            pos: String::new(),
            done_at,
            deleted: false,
            rev: 0,
            created_at: t.updated.clone().unwrap_or_else(wanna_core::now_rfc3339),
        });
    }

    let mut all = Vec::new();
    for q in Quadrant::ALL {
        let mut ws = by_q.remove(&q).unwrap_or_default();
        let keys = pos::n_between(None, None, ws.len());
        for (w, p) in ws.iter_mut().zip(keys) {
            w.pos = p;
        }
        let done = ws.iter().filter(|w| w.done_at.is_some()).count();
        println!("{:<14} やりたいこと {:>3} / やったこと {:>3}", q.label(), ws.len() - done, done);
        all.extend(ws);
    }
    println!("スキップ (削除済み・無題): {skipped}");

    if args.dry_run {
        for w in &all {
            let q = w.quadrant().label();
            let mark = if w.done_at.is_some() { "✓" } else { " " };
            println!("  {mark} [{q}] {}", w.title);
        }
        println!("--dry-run のため送信しません");
        return Ok(());
    }

    #[derive(Deserialize, Default)]
    struct WannaConfig {
        server: Option<String>,
        token: Option<String>,
    }
    let cfg: WannaConfig = std::fs::read_to_string(config_dir("wanna").join("config.toml"))
        .ok()
        .and_then(|s| toml::from_str(&s).ok())
        .unwrap_or_default();
    let server = args.server.or(cfg.server).context("--server か ~/.config/wanna/config.toml が必要です")?;
    let wtoken = args.token.or(cfg.token).context("--token か ~/.config/wanna/config.toml が必要です")?;
    let client = wanna_core::client::Client::new(&server, &wtoken)?;
    for (i, w) in all.iter().enumerate() {
        client.create(w).with_context(|| format!("送信失敗: {}", w.title))?;
        eprint!("\r送信 {}/{}", i + 1, all.len());
    }
    eprintln!("\n完了");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_meta_line() {
        let (body, meta) = split_notes("メモ\n\n[todo-meta] {\"imp\":7,\"clau\":2}");
        assert_eq!(body, "メモ");
        let m = meta.unwrap();
        assert_eq!((m.imp, m.clau), (7, 2));
    }
}
