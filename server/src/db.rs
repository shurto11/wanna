use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension, Row};
use std::path::Path;
use wanna_core::{now_rfc3339, pos, Quadrant, Want, WantPatch};

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS rev_counter (
  id  INTEGER PRIMARY KEY CHECK (id = 0),
  rev INTEGER NOT NULL
);
INSERT OR IGNORE INTO rev_counter (id, rev) VALUES (0, 0);

CREATE TABLE IF NOT EXISTS wants (
  id         TEXT    PRIMARY KEY,
  title      TEXT    NOT NULL,
  notes      TEXT    NOT NULL DEFAULT '',
  energy     INTEGER NOT NULL CHECK (energy IN (0, 1)),
  clau       INTEGER NOT NULL CHECK (clau   IN (0, 1)),
  pos        TEXT    NOT NULL,
  done_at    TEXT,
  deleted    INTEGER NOT NULL DEFAULT 0,
  rev        INTEGER NOT NULL,
  created_at TEXT    NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_wants_rev      ON wants(rev);
CREATE INDEX IF NOT EXISTS idx_wants_quadrant ON wants(energy, clau, pos);
CREATE INDEX IF NOT EXISTS idx_wants_done     ON wants(done_at);
"#;

const COLUMNS: &str = "id, title, notes, energy, clau, pos, done_at, deleted, rev, created_at";

pub fn open(path: &Path) -> Result<Connection> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let conn = Connection::open(path)?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.execute_batch(SCHEMA)?;
    Ok(conn)
}

fn row_to_want(r: &Row) -> rusqlite::Result<Want> {
    Ok(Want {
        id: r.get(0)?,
        title: r.get(1)?,
        notes: r.get(2)?,
        energy: r.get(3)?,
        clau: r.get(4)?,
        pos: r.get(5)?,
        done_at: r.get(6)?,
        deleted: r.get(7)?,
        rev: r.get(8)?,
        created_at: r.get(9)?,
    })
}

pub fn current_rev(conn: &Connection) -> Result<i64> {
    Ok(conn.query_row("SELECT rev FROM rev_counter WHERE id = 0", [], |r| r.get(0))?)
}

fn next_rev(conn: &Connection) -> Result<i64> {
    Ok(conn.query_row(
        "UPDATE rev_counter SET rev = rev + 1 WHERE id = 0 RETURNING rev",
        [],
        |r| r.get(0),
    )?)
}

/// `since` より新しい変更（tombstone 含む）。None なら全件。
pub fn changes_since(conn: &Connection, since: Option<i64>) -> Result<(i64, Vec<Want>)> {
    let rev = current_rev(conn)?;
    let mut stmt =
        conn.prepare(&format!("SELECT {COLUMNS} FROM wants WHERE rev > ?1 ORDER BY rev"))?;
    let wants = stmt
        .query_map([since.unwrap_or(0)], row_to_want)?
        .collect::<rusqlite::Result<_>>()?;
    Ok((rev, wants))
}

pub fn get(conn: &Connection, id: &str) -> Result<Option<Want>> {
    Ok(conn
        .query_row(&format!("SELECT {COLUMNS} FROM wants WHERE id = ?1"), [id], row_to_want)
        .optional()?)
}

/// 区分の末尾に付ける pos（`exclude` 自身は数えない）
fn tail_pos(conn: &Connection, q: Quadrant, exclude: &str) -> Result<String> {
    let last: Option<String> = conn.query_row(
        "SELECT max(pos) FROM wants
         WHERE energy = ?1 AND clau = ?2 AND deleted = 0 AND done_at IS NULL AND id != ?3",
        params![q.energy, q.clau, exclude],
        |r| r.get(0),
    )?;
    Ok(pos::between(last.as_deref(), None))
}

fn write(conn: &Connection, w: &Want) -> Result<()> {
    conn.execute(
        &format!(
            "INSERT INTO wants ({COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT(id) DO UPDATE SET
               title = excluded.title, notes = excluded.notes,
               energy = excluded.energy, clau = excluded.clau, pos = excluded.pos,
               done_at = excluded.done_at, deleted = excluded.deleted, rev = excluded.rev"
        ),
        params![
            w.id, w.title, w.notes, w.energy, w.clau, w.pos, w.done_at, w.deleted, w.rev,
            w.created_at
        ],
    )?;
    Ok(())
}

pub struct NewWant {
    pub id: String,
    pub title: String,
    pub notes: String,
    pub energy: bool,
    pub clau: bool,
    pub pos: Option<String>,
    pub done_at: Option<String>,
    pub created_at: Option<String>,
}

/// 作成（冪等 upsert）。同じ ID を再送しても1件のまま。tombstone は復活させない。
pub fn upsert(conn: &mut Connection, n: NewWant) -> Result<Want> {
    let tx = conn.transaction()?;
    let existing = get(&tx, &n.id)?;
    let pos = match n.pos {
        Some(p) => p,
        None => tail_pos(&tx, Quadrant { energy: n.energy, clau: n.clau }, &n.id)?,
    };
    let w = Want {
        rev: next_rev(&tx)?,
        deleted: existing.as_ref().is_some_and(|e| e.deleted),
        created_at: existing
            .map(|e| e.created_at)
            .or(n.created_at)
            .unwrap_or_else(now_rfc3339),
        id: n.id,
        title: n.title,
        notes: n.notes,
        energy: n.energy,
        clau: n.clau,
        pos,
        done_at: n.done_at,
    };
    write(&tx, &w)?;
    tx.commit()?;
    Ok(w)
}

/// 部分更新。区分が変わった / やったを取り消したのに pos が無ければ、行き先の末尾に付ける。
pub fn patch(conn: &mut Connection, id: &str, p: &WantPatch) -> Result<Option<Want>> {
    let tx = conn.transaction()?;
    let Some(mut w) = get(&tx, id)? else {
        return Ok(None);
    };
    let before_q = w.quadrant();
    let was_done = w.done_at.is_some();
    w.apply(p);
    let moved = w.quadrant() != before_q || (was_done && w.done_at.is_none());
    if moved && p.pos.is_none() {
        w.pos = tail_pos(&tx, w.quadrant(), &w.id)?;
    }
    w.rev = next_rev(&tx)?;
    write(&tx, &w)?;
    tx.commit()?;
    Ok(Some(w))
}

/// tombstone 化
pub fn delete(conn: &mut Connection, id: &str) -> Result<Option<i64>> {
    let tx = conn.transaction()?;
    if get(&tx, id)?.is_none() {
        return Ok(None);
    }
    let rev = next_rev(&tx)?;
    tx.execute("UPDATE wants SET deleted = 1, rev = ?1 WHERE id = ?2", params![rev, id])?;
    tx.commit()?;
    Ok(Some(rev))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mem() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(SCHEMA).unwrap();
        c
    }

    fn new(id: &str, energy: bool, clau: bool) -> NewWant {
        NewWant {
            id: id.into(),
            title: id.into(),
            notes: String::new(),
            energy,
            clau,
            pos: None,
            done_at: None,
            created_at: None,
        }
    }

    #[test]
    fn crud_and_sync() {
        let mut c = mem();
        let a = upsert(&mut c, new("a", true, false)).unwrap();
        let b = upsert(&mut c, new("b", true, false)).unwrap();
        assert!(a.pos < b.pos);
        assert_eq!((a.rev, b.rev), (1, 2));

        // 再送しても重複しない
        upsert(&mut c, new("a", true, false)).unwrap();
        let (rev, all) = changes_since(&c, None).unwrap();
        assert_eq!((rev, all.len()), (3, 2));

        // 区分移動で末尾に付く
        let p = WantPatch { clau: Some(true), ..Default::default() };
        let b2 = patch(&mut c, "b", &p).unwrap().unwrap();
        assert_eq!(b2.pos, pos::between(None, None));

        delete(&mut c, "a").unwrap();
        let (rev, ch) = changes_since(&c, Some(3)).unwrap();
        assert_eq!(rev, 5);
        assert_eq!(ch.iter().map(|w| w.id.as_str()).collect::<Vec<_>>(), ["b", "a"]);
        assert!(ch[1].deleted);

        // tombstone は再送で復活しない
        let a = upsert(&mut c, new("a", true, false)).unwrap();
        assert!(a.deleted);
        assert!(patch(&mut c, "zzz", &p).unwrap().is_none());
    }
}
