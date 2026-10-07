use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension, Row};
use std::path::Path;
use wanna_core::{now_rfc3339, pos, Kind, Quadrant, Want, WantPatch};

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS rev_counter (
  id  INTEGER PRIMARY KEY CHECK (id = 0),
  rev INTEGER NOT NULL
);
INSERT OR IGNORE INTO rev_counter (id, rev) VALUES (0, 0);
"#;

/// `{name}` を置き換えて使う (作り直すときに別名で作るため)
const WANTS_TABLE: &str = r#"
CREATE TABLE IF NOT EXISTS {name} (
  id         TEXT    PRIMARY KEY,
  title      TEXT    NOT NULL,
  notes      TEXT    NOT NULL DEFAULT '',
  kind       TEXT    NOT NULL DEFAULT 'want' CHECK (kind IN ('want', 'task', 'memo')),
  axis_hi    INTEGER NOT NULL CHECK (axis_hi IN (0, 1)),
  clau       INTEGER NOT NULL CHECK (clau    IN (0, 1)),
  pos        TEXT    NOT NULL,
  due_at     TEXT,
  done_at    TEXT,
  archived_at TEXT,
  deleted    INTEGER NOT NULL DEFAULT 0,
  rev        INTEGER NOT NULL,
  created_at TEXT    NOT NULL
);
"#;

const INDEXES: &str = r#"
CREATE INDEX IF NOT EXISTS idx_wants_rev  ON wants(rev);
CREATE INDEX IF NOT EXISTS idx_wants_list ON wants(kind, axis_hi, clau, pos);
CREATE INDEX IF NOT EXISTS idx_wants_done ON wants(done_at);
"#;

const COLUMNS: &str =
    "id, title, notes, kind, axis_hi, clau, pos, due_at, done_at, deleted, rev, created_at, archived_at";

pub fn open(path: &Path) -> Result<Connection> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let conn = Connection::open(path)?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    init(&conn)?;
    Ok(conn)
}

fn init(conn: &Connection) -> Result<()> {
    migrate(conn)?;
    conn.execute_batch(SCHEMA)?;
    conn.execute_batch(&WANTS_TABLE.replace("{name}", "wants"))?;
    conn.execute_batch(INDEXES)?;
    Ok(())
}

/// 「次にやること」を入れる前の DB を今の形にする。新規の DB では何もしない。
fn migrate(conn: &Connection) -> Result<()> {
    let cols: Vec<String> = conn
        .prepare("PRAGMA table_info(wants)")?
        .query_map([], |r| r.get::<_, String>(1))?
        .collect::<rusqlite::Result<_>>()?;
    if cols.is_empty() {
        return Ok(()); // テーブルがまだ無い
    }
    // 縦軸はリストによって意味が変わるので、energy から名前を付け直す
    if cols.iter().any(|c| c == "energy") {
        conn.execute_batch(
            "DROP INDEX IF EXISTS idx_wants_quadrant;
             ALTER TABLE wants RENAME COLUMN energy TO axis_hi;",
        )?;
    }
    if !cols.iter().any(|c| c == "kind") {
        conn.execute_batch("ALTER TABLE wants ADD COLUMN kind TEXT NOT NULL DEFAULT 'want';")?;
    }
    if !cols.iter().any(|c| c == "due_at") {
        conn.execute_batch("ALTER TABLE wants ADD COLUMN due_at TEXT;")?;
    }
    if !cols.iter().any(|c| c == "archived_at") {
        conn.execute_batch("ALTER TABLE wants ADD COLUMN archived_at TEXT;")?;
    }
    // kind の CHECK に memo を足す。SQLite は CHECK を変えられないので作り直す
    let sql: String = conn.query_row(
        "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'wants'",
        [],
        |r| r.get(0),
    )?;
    if !sql.contains("'memo'") {
        conn.execute_batch(&format!(
            "BEGIN;
             {}
             INSERT INTO wants_new ({COLUMNS}) SELECT {COLUMNS} FROM wants;
             DROP TABLE wants;
             ALTER TABLE wants_new RENAME TO wants;
             COMMIT;",
            WANTS_TABLE.replace("{name}", "wants_new")
        ))?;
    }
    Ok(())
}

fn row_to_want(r: &Row) -> rusqlite::Result<Want> {
    Ok(Want {
        id: r.get(0)?,
        title: r.get(1)?,
        notes: r.get(2)?,
        kind: Kind::from_str(&r.get::<_, String>(3)?),
        axis_hi: r.get(4)?,
        clau: r.get(5)?,
        pos: r.get(6)?,
        due_at: r.get(7)?,
        done_at: r.get(8)?,
        deleted: r.get(9)?,
        rev: r.get(10)?,
        created_at: r.get(11)?,
        archived_at: r.get(12)?,
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

/// リスト・区分の末尾に付ける pos（`exclude` 自身は数えない）
fn tail_pos(conn: &Connection, kind: Kind, q: Quadrant, exclude: &str) -> Result<String> {
    let last: Option<String> = conn.query_row(
        "SELECT max(pos) FROM wants
         WHERE kind = ?1 AND axis_hi = ?2 AND clau = ?3
           AND deleted = 0 AND done_at IS NULL AND archived_at IS NULL AND id != ?4",
        params![kind.as_str(), q.axis_hi, q.clau, exclude],
        |r| r.get(0),
    )?;
    Ok(pos::between(last.as_deref(), None))
}

fn write(conn: &Connection, w: &Want) -> Result<()> {
    conn.execute(
        &format!(
            "INSERT INTO wants ({COLUMNS})
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
             ON CONFLICT(id) DO UPDATE SET
               title = excluded.title, notes = excluded.notes, kind = excluded.kind,
               axis_hi = excluded.axis_hi, clau = excluded.clau, pos = excluded.pos,
               due_at = excluded.due_at, done_at = excluded.done_at,
               archived_at = excluded.archived_at, deleted = excluded.deleted, rev = excluded.rev"
        ),
        params![
            w.id,
            w.title,
            w.notes,
            w.kind.as_str(),
            w.axis_hi,
            w.clau,
            w.pos,
            w.due_at,
            w.done_at,
            w.deleted,
            w.rev,
            w.created_at,
            w.archived_at
        ],
    )?;
    Ok(())
}

pub struct NewWant {
    pub id: String,
    pub title: String,
    pub notes: String,
    pub kind: Kind,
    pub axis_hi: bool,
    pub clau: bool,
    pub pos: Option<String>,
    pub due_at: Option<String>,
    pub done_at: Option<String>,
    pub archived_at: Option<String>,
    pub created_at: Option<String>,
}

/// 作成（冪等 upsert）。同じ ID を再送しても1件のまま。tombstone は復活させない。
pub fn upsert(conn: &mut Connection, n: NewWant) -> Result<Want> {
    let tx = conn.transaction()?;
    let existing = get(&tx, &n.id)?;
    let q = Quadrant { axis_hi: n.axis_hi, clau: n.clau };
    let pos = match n.pos {
        Some(p) => p,
        None => tail_pos(&tx, n.kind, q, &n.id)?,
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
        kind: n.kind,
        axis_hi: n.axis_hi,
        clau: n.clau,
        pos,
        // 日時を持てるのは「次にやること」だけ
        due_at: n.due_at.filter(|_| n.kind.has_due()),
        done_at: n.done_at,
        archived_at: n.archived_at,
    };
    write(&tx, &w)?;
    tx.commit()?;
    Ok(w)
}

/// 部分更新。行き先が変わった / やった・しまったを取り消したのに pos が無ければ、行き先の末尾に付ける。
pub fn patch(conn: &mut Connection, id: &str, p: &WantPatch) -> Result<Option<Want>> {
    let tx = conn.transaction()?;
    let Some(mut w) = get(&tx, id)? else {
        return Ok(None);
    };
    let before = (w.kind, w.quadrant());
    let was_active = w.is_active();
    w.apply(p);
    if !w.kind.has_due() {
        w.due_at = None;
    }
    let moved = (w.kind, w.quadrant()) != before || (!was_active && w.is_active());
    if moved && p.pos.is_none() {
        w.pos = tail_pos(&tx, w.kind, w.quadrant(), &w.id)?;
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
        init(&c).unwrap();
        c
    }

    fn new(id: &str, kind: Kind, axis_hi: bool, clau: bool) -> NewWant {
        NewWant {
            id: id.into(),
            title: id.into(),
            notes: String::new(),
            kind,
            axis_hi,
            clau,
            pos: None,
            due_at: None,
            done_at: None,
            archived_at: None,
            created_at: None,
        }
    }

    #[test]
    fn crud_and_sync() {
        let mut c = mem();
        let a = upsert(&mut c, new("a", Kind::Want, true, false)).unwrap();
        let b = upsert(&mut c, new("b", Kind::Want, true, false)).unwrap();
        assert!(a.pos < b.pos);
        assert_eq!((a.rev, b.rev), (1, 2));

        // 再送しても重複しない
        upsert(&mut c, new("a", Kind::Want, true, false)).unwrap();
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
        let a = upsert(&mut c, new("a", Kind::Want, true, false)).unwrap();
        assert!(a.deleted);
        assert!(patch(&mut c, "zzz", &p).unwrap().is_none());
    }

    #[test]
    fn lists_have_separate_pos_spaces() {
        let mut c = mem();
        let w = upsert(&mut c, new("w", Kind::Want, true, false)).unwrap();
        let t = upsert(&mut c, new("t", Kind::Task, true, false)).unwrap();
        // 同じ区分でもリストが違えば先頭から振り直す
        assert_eq!(w.pos, t.pos);

        // リストを移すと移動先の末尾に付く
        upsert(&mut c, new("t2", Kind::Task, true, false)).unwrap();
        let moved = patch(&mut c, "w", &WantPatch { kind: Some(Kind::Task), ..Default::default() })
            .unwrap()
            .unwrap();
        assert_eq!(moved.kind, Kind::Task);
        assert!(moved.pos > t.pos);
    }

    #[test]
    fn due_is_dropped_outside_tasks() {
        let mut c = mem();
        let mut n = new("a", Kind::Want, true, false);
        n.due_at = Some("2026-09-25".into());
        // やりたいことは日時を持たない
        assert_eq!(upsert(&mut c, n).unwrap().due_at, None);

        let t = patch(
            &mut c,
            "a",
            &WantPatch {
                kind: Some(Kind::Task),
                due_at: Some(Some("2026-09-25".into())),
                ..Default::default()
            },
        )
        .unwrap()
        .unwrap();
        assert_eq!(t.due_at.as_deref(), Some("2026-09-25"));

        // やりたいことに戻せば日時も落ちる
        let w = patch(&mut c, "a", &WantPatch { kind: Some(Kind::Want), ..Default::default() })
            .unwrap()
            .unwrap();
        assert_eq!(w.due_at, None);
    }

    #[test]
    fn archived_leaves_the_list() {
        let mut c = mem();
        let a = upsert(&mut c, new("a", Kind::Want, true, false)).unwrap();
        let p = WantPatch { archived_at: Some(Some(now_rfc3339())), ..Default::default() };
        patch(&mut c, "a", &p).unwrap().unwrap();
        // しまったものは末尾の計算に数えない
        let b = upsert(&mut c, new("b", Kind::Want, true, false)).unwrap();
        assert_eq!(b.pos, a.pos);
        // 取り出すと末尾に付く
        let p = WantPatch { archived_at: Some(None), ..Default::default() };
        let a = patch(&mut c, "a", &p).unwrap().unwrap();
        assert!(a.is_active());
        assert!(a.pos > b.pos);
    }

    /// 「次にやること」を入れる前の DB がそのまま開けること
    #[test]
    fn migrates_an_old_database() {
        let dir = std::env::temp_dir().join(format!("wanna-migrate-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("wanna.db");
        Connection::open(&path)
            .unwrap()
            .execute_batch(
                "CREATE TABLE rev_counter (id INTEGER PRIMARY KEY CHECK (id = 0), rev INTEGER NOT NULL);
                 INSERT INTO rev_counter VALUES (0, 1);
                 CREATE TABLE wants (
                   id TEXT PRIMARY KEY, title TEXT NOT NULL, notes TEXT NOT NULL DEFAULT '',
                   energy INTEGER NOT NULL CHECK (energy IN (0, 1)),
                   clau INTEGER NOT NULL CHECK (clau IN (0, 1)),
                   pos TEXT NOT NULL, done_at TEXT, deleted INTEGER NOT NULL DEFAULT 0,
                   rev INTEGER NOT NULL, created_at TEXT NOT NULL);
                 CREATE INDEX idx_wants_quadrant ON wants(energy, clau, pos);
                 INSERT INTO wants VALUES ('a', '古い1件', '', 1, 0, 'V', NULL, 0, 1, '2026-09-18T00:00:00Z');",
            )
            .unwrap();

        let mut c = open(&path).unwrap();
        let w = get(&c, "a").unwrap().unwrap();
        assert_eq!((w.kind, w.axis_hi, w.due_at), (Kind::Want, true, None));
        // 開いたあとは新しい列も普通に使える
        let t = upsert(&mut c, new("b", Kind::Task, false, true)).unwrap();
        assert_eq!(t.kind, Kind::Task);
        let m = upsert(&mut c, new("m", Kind::Memo, true, false)).unwrap();
        assert_eq!(m.kind, Kind::Memo);
        assert_eq!(get(&c, "a").unwrap().unwrap().title, "古い1件");
        drop(c);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
