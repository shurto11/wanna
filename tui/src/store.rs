//! ローカルキャッシュ (`~/.config/wanna/cache.db`) と未送信キュー (outbox)。

use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::Path;
use wanna_core::{Want, WantPatch};

/// サーバーへ送る操作。outbox に JSON で積む。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Op {
    Create { want: Want },
    Patch { id: String, patch: WantPatch },
    Delete { id: String },
}

impl Op {
    pub fn id(&self) -> &str {
        match self {
            Op::Create { want } => &want.id,
            Op::Patch { id, .. } | Op::Delete { id } => id,
        }
    }
}

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS wants (
  id         TEXT PRIMARY KEY,
  json       TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS meta (
  key   TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS outbox (
  seq INTEGER PRIMARY KEY AUTOINCREMENT,
  op  TEXT NOT NULL
);
"#;

pub struct Store {
    conn: Connection,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.execute_batch(SCHEMA)?;
        Ok(Self { conn })
    }

    pub fn load_all(&self) -> Result<Vec<Want>> {
        let mut stmt = self.conn.prepare("SELECT json FROM wants")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        let mut out = Vec::new();
        for json in rows {
            if let Ok(w) = serde_json::from_str(&json?) {
                out.push(w);
            }
        }
        Ok(out)
    }

    pub fn put(&self, w: &Want) -> Result<()> {
        self.conn.execute(
            "INSERT INTO wants (id, json) VALUES (?1, ?2)
             ON CONFLICT(id) DO UPDATE SET json = excluded.json",
            params![w.id, serde_json::to_string(w)?],
        )?;
        Ok(())
    }

    /// 最後に同期したサーバーの rev (未同期なら 0)
    pub fn rev(&self) -> Result<i64> {
        let v: Option<String> = self
            .conn
            .query_row("SELECT value FROM meta WHERE key = 'rev'", [], |r| r.get(0))
            .optional()?;
        Ok(v.and_then(|v| v.parse().ok()).unwrap_or(0))
    }

    pub fn set_rev(&self, rev: i64) -> Result<()> {
        self.conn.execute(
            "INSERT INTO meta (key, value) VALUES ('rev', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [rev.to_string()],
        )?;
        Ok(())
    }

    pub fn push_op(&self, op: &Op) -> Result<()> {
        self.conn
            .execute("INSERT INTO outbox (op) VALUES (?1)", [serde_json::to_string(op)?])?;
        Ok(())
    }

    pub fn ops(&self) -> Result<Vec<(i64, Op)>> {
        let mut stmt = self.conn.prepare("SELECT seq, op FROM outbox ORDER BY seq")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?;
        let mut out = Vec::new();
        for row in rows {
            let (seq, json) = row?;
            match serde_json::from_str(&json) {
                Ok(op) => out.push((seq, op)),
                // 読めない操作は送りようがないので捨てる
                Err(_) => self.remove_op(seq)?,
            }
        }
        Ok(out)
    }

    pub fn remove_op(&self, seq: i64) -> Result<()> {
        self.conn.execute("DELETE FROM outbox WHERE seq = ?1", [seq])?;
        Ok(())
    }

    /// outbox に残っている操作の対象 ID
    pub fn pending_ids(&self) -> Result<HashSet<String>> {
        Ok(self.ops()?.into_iter().map(|(_, op)| op.id().to_string()).collect())
    }

    pub fn outbox_len(&self) -> Result<usize> {
        Ok(self.conn.query_row("SELECT count(*) FROM outbox", [], |r| r.get::<_, i64>(0))? as usize)
    }
}
