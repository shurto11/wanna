//! wannad — wanna のバックエンド。API と web の静的ファイルを配信する。

mod db;

use anyhow::{Context, Result};
use axum::{
    extract::{Path, Query, Request, State},
    http::{header, HeaderMap, StatusCode, Uri},
    middleware::{self, Next},
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse, Response,
    },
    routing::{get, patch, post},
    Json, Router,
};
use futures_util::{Stream, StreamExt};
use include_dir::{include_dir, Dir};
use rusqlite::Connection;
use serde::Deserialize;
use std::{
    collections::HashMap,
    convert::Infallible,
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tokio::sync::watch;
use tokio_stream::wrappers::WatchStream;
use wanna_core::{Want, WantPatch};

static WEB: Dir = include_dir!("$CARGO_MANIFEST_DIR/../web/dist");

#[derive(Deserialize)]
struct Config {
    bind: String,
    token: String,
    db: PathBuf,
}

struct AppState {
    db: Mutex<Connection>,
    token: String,
    /// 最新の rev。SSE はこれを購読する
    rev: watch::Sender<i64>,
}

type Shared = Arc<AppState>;

fn load_config() -> Result<Config> {
    let args: Vec<String> = std::env::args().collect();
    let path = match args.iter().position(|a| a == "--config" || a == "-c") {
        Some(i) => PathBuf::from(args.get(i + 1).context("--config の後にパスが必要です")?),
        None => dirs_config().join("server.toml"),
    };
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("設定ファイルを読めません: {}", path.display()))?;
    let mut cfg: Config = toml::from_str(&text).context("設定ファイルのパースに失敗")?;
    if cfg.db.is_relative() {
        cfg.db = path.parent().unwrap_or(std::path::Path::new(".")).join(&cfg.db);
    }
    Ok(cfg)
}

fn dirs_config() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default()
        .join(".config/wanna")
}

#[tokio::main]
async fn main() -> Result<()> {
    let cfg = load_config()?;
    let conn = db::open(&cfg.db)?;
    let (rev, _) = watch::channel(db::current_rev(&conn)?);
    let state = Arc::new(AppState { db: Mutex::new(conn), token: cfg.token, rev });

    let api = Router::new()
        .route("/sync", get(sync))
        .route("/wants", post(create))
        .route("/wants/{id}", patch(update).delete(remove))
        .route("/events", get(events))
        .route_layer(middleware::from_fn_with_state(state.clone(), auth));

    let app = Router::new()
        .nest("/api", api)
        .fallback(get(static_file))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(&cfg.bind)
        .await
        .with_context(|| format!("{} に bind できません", cfg.bind))?;
    eprintln!("wannad: http://{} (db: {})", cfg.bind, cfg.db.display());
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}

/// `Authorization: Bearer <token>`。EventSource はヘッダを付けられないので `?token=` も受ける。
async fn auth(
    State(st): State<Shared>,
    Query(q): Query<HashMap<String, String>>,
    headers: HeaderMap,
    req: Request,
    next: Next,
) -> Response {
    let bearer = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));
    if bearer.or(q.get("token").map(String::as_str)) == Some(st.token.as_str()) {
        next.run(req).await
    } else {
        (StatusCode::UNAUTHORIZED, "unauthorized").into_response()
    }
}

struct AppError(anyhow::Error);

impl<E: Into<anyhow::Error>> From<E> for AppError {
    fn from(e: E) -> Self {
        Self(e.into())
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        eprintln!("error: {:#}", self.0);
        (StatusCode::INTERNAL_SERVER_ERROR, format!("{:#}", self.0)).into_response()
    }
}

type ApiResult<T> = Result<T, AppError>;

fn bad_request(msg: &str) -> Response {
    (StatusCode::BAD_REQUEST, msg.to_string()).into_response()
}

fn not_found() -> Response {
    (StatusCode::NOT_FOUND, "not found").into_response()
}

fn valid_pos(p: &str) -> bool {
    !p.is_empty() && !p.ends_with('0') && p.bytes().all(|c| c.is_ascii_alphanumeric())
}

#[derive(Deserialize)]
struct SyncQuery {
    since: Option<i64>,
}

async fn sync(State(st): State<Shared>, Query(q): Query<SyncQuery>) -> ApiResult<Response> {
    let (rev, wants) = db::changes_since(&st.db.lock().unwrap(), q.since)?;
    Ok(Json(wanna_core::SyncResponse { rev, wants }).into_response())
}

#[derive(Deserialize)]
struct CreateBody {
    id: String,
    title: String,
    #[serde(default)]
    notes: String,
    energy: bool,
    clau: bool,
    #[serde(default)]
    pos: Option<String>,
    #[serde(default)]
    done_at: Option<String>,
    #[serde(default)]
    created_at: Option<String>,
}

async fn create(State(st): State<Shared>, Json(b): Json<CreateBody>) -> ApiResult<Response> {
    let pos = b.pos.filter(|p| !p.is_empty());
    if pos.as_deref().is_some_and(|p| !valid_pos(p)) {
        return Ok(bad_request("invalid pos"));
    }
    if b.id.is_empty() || b.id.len() > 64 {
        return Ok(bad_request("invalid id"));
    }
    let w = db::upsert(
        &mut st.db.lock().unwrap(),
        db::NewWant {
            id: b.id,
            title: b.title,
            notes: b.notes,
            energy: b.energy,
            clau: b.clau,
            pos,
            done_at: b.done_at,
            created_at: b.created_at,
        },
    )?;
    notify(&st, w.rev);
    Ok((StatusCode::CREATED, Json(w)).into_response())
}

async fn update(
    State(st): State<Shared>,
    Path(id): Path<String>,
    Json(p): Json<WantPatch>,
) -> ApiResult<Response> {
    if p.pos.as_deref().is_some_and(|p| !valid_pos(p)) {
        return Ok(bad_request("invalid pos"));
    }
    let w: Option<Want> = db::patch(&mut st.db.lock().unwrap(), &id, &p)?;
    Ok(match w {
        Some(w) => {
            notify(&st, w.rev);
            Json(w).into_response()
        }
        None => not_found(),
    })
}

async fn remove(State(st): State<Shared>, Path(id): Path<String>) -> ApiResult<Response> {
    let rev = db::delete(&mut st.db.lock().unwrap(), &id)?;
    Ok(match rev {
        Some(rev) => {
            notify(&st, rev);
            StatusCode::NO_CONTENT.into_response()
        }
        None => not_found(),
    })
}

fn notify(st: &AppState, rev: i64) {
    st.rev.send_if_modified(|cur| {
        let changed = rev > *cur;
        *cur = (*cur).max(rev);
        changed
    });
}

/// SSE。差分の中身は流さず「rev が N になった」とだけ送る。接続直後にも現在値を1回送る。
async fn events(State(st): State<Shared>) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let stream = WatchStream::new(st.rev.subscribe())
        .map(|rev| Ok(Event::default().event("rev").data(rev.to_string())));
    Sse::new(stream).keep_alive(KeepAlive::default())
}

async fn static_file(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    if path.starts_with("api/") {
        return not_found();
    }
    let path = if path.is_empty() { "index.html" } else { path };
    let (path, file) = match WEB.get_file(path) {
        Some(f) => (path, f),
        // SPA なので未知のパスは index.html を返す
        None => match WEB.get_file("index.html") {
            Some(f) => ("index.html", f),
            None => {
                return (StatusCode::NOT_FOUND, "web が埋め込まれていません (web をビルドしてから wannad をビルドしてください)")
                    .into_response()
            }
        },
    };
    let mime = mime_guess::from_path(path).first_or_octet_stream();
    // ハッシュ付きの assets は長くキャッシュ、それ以外は毎回確認させる
    let cache = if path.starts_with("assets/") {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    };
    (
        [(header::CONTENT_TYPE, mime.as_ref().to_string()), (header::CACHE_CONTROL, cache.to_string())],
        file.contents(),
    )
        .into_response()
}
