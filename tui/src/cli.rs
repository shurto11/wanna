//! 非対話のサブコマンド。Claude Code などから1件を読み書きするためのもの。
//!
//! 常にサーバーへ直接読み書きする（TUI のキャッシュと未送信キューは使わない）。
//! ID の代わりに `@` と書くと、このディレクトリに紐付けた1件を指す (`wanna link`)。

use crate::config::Config;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use wanna_core::client::{Client, Conflict};
use wanna_core::{notes, Kind, Quadrant, Want, WantPatch};

const USAGE: &str = "\
使い方: wanna <コマンド> ...   (引数なしで TUI)

  ls [--want|--must|--memo] [--done|--archived] [--json]
                                         一覧 (既定は両方のやってない・しまってないもの。メモは --memo)
  show [ID] [--json]                     1件とメモを出す (ID 省略で @)
  new <タイトル> [--want] [--lo] [--no-clau] [--link]
                                         テンプレート入りで作る (既定は Must・重要度高・clau高)
  log [-c] <ID> <テキスト...>            進捗ログに日付付きで1行足す (-c で [c] を付ける)
  note <ID> <見出し> [テキスト...|-]     見出しの本文を置き換える (- で標準入力。省略で表示)
  done <ID>                              やったことにする
  archive <ID>                           保管庫にしまう (やってないがリストに置くほどでもないもの)
  link [ID]                              このディレクトリに1件を紐付ける (省略で今の紐付けを表示)
  unlink                                 紐付けを外す

ID は全体か末尾の一部 (ls の左端)。@ はこのディレクトリに紐付けた1件。";

/// 紐付けを置くファイル (リポジトリ直下からの相対)
const LINK_FILE: &str = ".claude/wanna.local.toml";

#[derive(Serialize, Deserialize)]
struct Link {
    id: String,
    title: String,
}

pub fn run(args: &[String]) -> Result<()> {
    let (cmd, rest) = args.split_first().context(USAGE)?;
    let (flags, pos): (Vec<&str>, Vec<&str>) = rest
        .iter()
        .map(String::as_str)
        .partition(|a| a.starts_with("--") || *a == "-c");
    let has = |f: &str| flags.contains(&f);
    if let Some(f) = flags.iter().find(|f| !KNOWN_FLAGS.contains(f)) {
        bail!("知らないオプション: {f}\n\n{USAGE}");
    }
    match cmd.as_str() {
        "ls" => {
            let shelf = match (has("--done"), has("--archived")) {
                (true, true) => bail!("--done と --archived は一緒に使えません"),
                (true, false) => Shelf::Done,
                (false, true) => Shelf::Archived,
                _ => Shelf::Active,
            };
            if has("--memo") {
                return ls_memos(&client()?, has("--json"));
            }
            ls(&client()?, has("--want"), has("--must"), shelf, has("--json"))
        }
        "show" => show(&client()?, pos.first().copied().unwrap_or("@"), has("--json")),
        "new" => {
            let title = pos.join(" ");
            if title.trim().is_empty() {
                bail!("タイトルが必要です\n\n{USAGE}");
            }
            let kind = if has("--want") { Kind::Want } else { Kind::Task };
            new(&client()?, title.trim(), kind, !has("--lo"), !has("--no-clau"), has("--link"))
        }
        "log" => {
            let [id, text @ ..] = pos.as_slice() else { bail!(USAGE) };
            let text = text.join(" ");
            if text.trim().is_empty() {
                bail!("テキストが必要です");
            }
            let date = chrono::Local::now().format("%Y-%m-%d").to_string();
            let line = notes::log_line(&date, &text, has("-c"));
            let c = client()?;
            let w = rewrite(&c, id, |n| notes::append(n, notes::LOG, &line))?;
            println!("{}: {line}", w.title);
            Ok(())
        }
        "note" => {
            let [id, name, text @ ..] = pos.as_slice() else { bail!(USAGE) };
            let c = client()?;
            if text.is_empty() {
                let w = find(&c, id)?;
                println!("{}", notes::section(&w.notes, name).unwrap_or_default());
                return Ok(());
            }
            let body = if text == ["-"] {
                let mut s = String::new();
                std::io::stdin().read_to_string(&mut s)?;
                s
            } else {
                text.join(" ")
            };
            let w = rewrite(&c, id, |n| notes::set_section(n, name, &body))?;
            println!("{}: 「{name}」を書き換えました", w.title);
            Ok(())
        }
        "done" => {
            let [id] = pos.as_slice() else { bail!(USAGE) };
            let c = client()?;
            let w = find(&c, id)?;
            let p = WantPatch { done_at: Some(Some(wanna_core::now_rfc3339())), ..Default::default() };
            c.patch(&w.id, &p)?;
            println!("やりました: {}", w.title);
            Ok(())
        }
        "archive" => {
            let [id] = pos.as_slice() else { bail!(USAGE) };
            let c = client()?;
            let w = find(&c, id)?;
            let p = WantPatch { archived_at: Some(Some(wanna_core::now_rfc3339())), ..Default::default() };
            c.patch(&w.id, &p)?;
            println!("保管庫にしまいました: {}", w.title);
            Ok(())
        }
        "link" => match pos.first() {
            Some(id) => link(&find(&client()?, id)?),
            None => {
                let (path, l) = read_link()?.context("このディレクトリには何も紐付いていません")?;
                println!("{}  {}\n({})", short(&l.id), l.title, path.display());
                Ok(())
            }
        },
        "unlink" => {
            let (path, l) = read_link()?.context("このディレクトリには何も紐付いていません")?;
            std::fs::remove_file(&path)?;
            println!("紐付けを外しました: {}", l.title);
            Ok(())
        }
        "help" | "-h" | "--help" => {
            println!("{USAGE}");
            Ok(())
        }
        _ => bail!("知らないコマンド: {cmd}\n\n{USAGE}"),
    }
}

const KNOWN_FLAGS: &[&str] =
    &["--want", "--must", "--memo", "--done", "--archived", "--json", "--lo", "--no-clau", "--link", "-c"];

/// ls でどこを出すか
enum Shelf {
    Active,
    Done,
    Archived,
}

fn client() -> Result<Client> {
    let cfg = Config::load();
    let (server, token) = cfg
        .remote()
        .context("サーバー未設定: ~/.config/wanna/config.toml に server と token を書いてください")?;
    Client::new(server, token)
}

fn all(c: &Client) -> Result<Vec<Want>> {
    Ok(c.sync(None)?.wants.into_iter().filter(|w| !w.deleted).collect())
}

/// 表示用の短い ID。UUIDv7 の先頭は時刻で揃いやすいので末尾を使う
fn short(id: &str) -> &str {
    &id[id.len().saturating_sub(8)..]
}

/// ID (全体 / 末尾の一部 / `@`) から1件を引く
fn find(c: &Client, q: &str) -> Result<Want> {
    let q = if q == "@" {
        read_link()?.context("このディレクトリには何も紐付いていません (wanna link <ID>)")?.1.id
    } else {
        q.to_string()
    };
    let hits: Vec<Want> = all(c)?.into_iter().filter(|w| w.id == q || w.id.ends_with(&q)).collect();
    match hits.len() {
        1 => Ok(hits.into_iter().next().unwrap()),
        0 => bail!("見つかりません: {q}"),
        _ => bail!(
            "{q} に当てはまるものが複数あります:\n{}",
            hits.iter().map(|w| format!("  {}  {}", w.id, w.title)).collect::<Vec<_>>().join("\n")
        ),
    }
}

/// メモを読んで書き換えて戻す。間に他で書き換えられていたら読み直してやり直す
fn rewrite(c: &Client, id: &str, f: impl Fn(&str) -> String) -> Result<Want> {
    let mut w = find(c, id)?;
    for _ in 0..3 {
        let p = WantPatch {
            notes: Some(f(&w.notes)),
            expect_rev: Some(w.rev),
            ..Default::default()
        };
        match c.patch(&w.id, &p) {
            Ok(w) => return Ok(w),
            Err(e) => match e.downcast::<Conflict>() {
                Ok(Conflict(cur)) => w = *cur,
                Err(e) => return Err(e),
            },
        }
    }
    bail!("何度やっても他の書き込みとぶつかりました。少し待ってやり直してください")
}

fn ls(c: &Client, want: bool, must: bool, shelf: Shelf, json: bool) -> Result<()> {
    let mut ws: Vec<Want> = all(c)?
        .into_iter()
        .filter(|w| match shelf {
            Shelf::Active => w.is_active(),
            Shelf::Done => w.done_at.is_some(),
            Shelf::Archived => w.is_archived(),
        })
        .filter(|w| w.kind.is_list())
        .filter(|w| match (want, must) {
            (true, false) => w.kind == Kind::Want,
            (false, true) => w.kind == Kind::Task,
            _ => true,
        })
        .collect();
    // リスト → 区分 (画面の並び) → 区分内の並び
    let q_index = |w: &Want| Quadrant::ALL.iter().position(|q| *q == w.quadrant());
    ws.sort_by(|a, b| {
        (a.kind.index(), q_index(a), &a.pos, &a.id).cmp(&(b.kind.index(), q_index(b), &b.pos, &b.id))
    });
    if json {
        println!("{}", serde_json::to_string_pretty(&ws)?);
        return Ok(());
    }
    // `| head` で途中で閉じられても panic しないよう、書けなくなったら黙って止める
    let mut out = std::io::stdout().lock();
    for w in &ws {
        let due = w.due().unwrap_or("");
        let line = format!("{}  {}  {}  {}  {}", short(&w.id), w.kind.label(), w.quadrant_label(), due, w.title);
        if writeln!(out, "{line}").is_err() {
            break;
        }
    }
    Ok(())
}

/// メモの一覧。作った日の降順
fn ls_memos(c: &Client, json: bool) -> Result<()> {
    let mut ws: Vec<Want> = all(c)?.into_iter().filter(|w| w.kind == Kind::Memo).collect();
    ws.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    if json {
        println!("{}", serde_json::to_string_pretty(&ws)?);
        return Ok(());
    }
    let mut out = std::io::stdout().lock();
    for w in &ws {
        let date = w.created_at.get(..10).unwrap_or(&w.created_at);
        if writeln!(out, "{}  {date}  {}", short(&w.id), w.title).is_err() {
            break;
        }
    }
    Ok(())
}

fn show(c: &Client, id: &str, json: bool) -> Result<()> {
    let w = find(c, id)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&w)?);
        return Ok(());
    }
    println!("# {}", w.title);
    println!("id: {}", w.id);
    if w.kind.is_list() {
        println!("{} / {}", w.kind.label(), w.quadrant_label());
    } else {
        println!("{} / {}", w.kind.label(), w.created_at);
    }
    if let Some(d) = w.due() {
        println!("日時: {d}");
    }
    if let Some(d) = &w.done_at {
        println!("やった: {d}");
    } else if let Some(d) = &w.archived_at {
        println!("しまった: {d}");
    }
    println!();
    println!("{}", if w.notes.is_empty() { notes::template() } else { w.notes.clone() });
    Ok(())
}

fn new(c: &Client, title: &str, kind: Kind, axis_hi: bool, clau: bool, and_link: bool) -> Result<()> {
    // pos を空で送るとサーバーが区分の末尾に付ける
    let mut w = Want::new(title, kind, axis_hi, clau, String::new());
    w.notes = notes::template();
    let w = c.create(&w)?;
    println!("{}  {}  {}  {}", short(&w.id), w.kind.label(), w.quadrant_label(), w.title);
    if and_link {
        link(&w)?;
    }
    Ok(())
}

/// リポジトリの直下 (git でなければ今のディレクトリ)
fn project_root() -> Result<PathBuf> {
    let out = Command::new("git").args(["rev-parse", "--show-toplevel"]).output();
    match out {
        Ok(o) if o.status.success() => Ok(PathBuf::from(String::from_utf8(o.stdout)?.trim())),
        _ => Ok(std::env::current_dir()?),
    }
}

/// 今のディレクトリから上へたどって紐付けを探す
fn read_link() -> Result<Option<(PathBuf, Link)>> {
    let cwd = std::env::current_dir()?;
    for dir in cwd.ancestors() {
        let path = dir.join(LINK_FILE);
        if path.is_file() {
            let l: Link = toml::from_str(&std::fs::read_to_string(&path)?)
                .with_context(|| format!("{} を読めません", path.display()))?;
            return Ok(Some((path, l)));
        }
    }
    Ok(None)
}

fn link(w: &Want) -> Result<()> {
    let root = project_root()?;
    let path = root.join(LINK_FILE);
    std::fs::create_dir_all(path.parent().unwrap())?;
    let l = Link { id: w.id.clone(), title: w.title.clone() };
    std::fs::write(&path, toml::to_string(&l)?)?;
    exclude_from_git(&root, &path);
    println!("紐付けました: {}  {}\n({})", short(&w.id), w.title, path.display());
    Ok(())
}

/// 個人の状態なので、ignore されていなければ .git/info/exclude に足す (.gitignore は触らない)
fn exclude_from_git(root: &Path, path: &Path) {
    let ignored = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["check-ignore", "-q"])
        .arg(path)
        .status();
    let Ok(st) = ignored else { return };
    if st.code() != Some(1) {
        return; // ignore 済み (0) か git でない (128)
    }
    let Ok(o) = Command::new("git").arg("-C").arg(root).args(["rev-parse", "--git-path", "info/exclude"]).output()
    else {
        return;
    };
    let p = root.join(String::from_utf8_lossy(&o.stdout).trim());
    let cur = std::fs::read_to_string(&p).unwrap_or_default();
    let line = format!("/{LINK_FILE}");
    if !cur.lines().any(|l| l == line) {
        let _ = std::fs::create_dir_all(p.parent().unwrap_or(root));
        let sep = if cur.is_empty() || cur.ends_with('\n') { "" } else { "\n" };
        let _ = std::fs::write(&p, format!("{cur}{sep}{line}\n"));
    }
}
