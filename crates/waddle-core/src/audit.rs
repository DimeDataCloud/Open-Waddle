//! Append-only, hash-chained action log in SQLite. Each row's hash covers the
//! previous row's hash, so editing or deleting history breaks the chain.
//! Triggers reject UPDATE and DELETE through normal SQL.

use anyhow::Context;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::path::Path;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Default)]
pub struct AuditEntry {
    pub task_id: String,
    pub kind: String,
    pub tool: Option<String>,
    pub args: Option<String>,
    pub tier: Option<u8>,
    pub decision: Option<String>,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct AuditRecord {
    pub id: i64,
    pub ts_ms: i64,
    pub task_id: String,
    pub kind: String,
    pub tool: Option<String>,
    pub args: Option<String>,
    pub tier: Option<u8>,
    pub decision: Option<String>,
    pub detail: Option<String>,
    pub hash: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct VerifyReport {
    pub ok: bool,
    pub entries: usize,
    pub first_bad_id: Option<i64>,
}

const MAX_FIELD: usize = 4000;
const GENESIS: &str = "waddle-audit-genesis";

pub struct AuditLog {
    conn: Mutex<Connection>,
}

fn clip(s: Option<String>) -> Option<String> {
    s.map(|s| if s.len() > MAX_FIELD { format!("{}…", &s[..s.floor_char_boundary(MAX_FIELD)]) } else { s })
}

#[allow(clippy::too_many_arguments)]
fn chain_hash(
    prev: &str,
    ts: i64,
    task: &str,
    kind: &str,
    tool: &Option<String>,
    args: &Option<String>,
    tier: Option<u8>,
    decision: &Option<String>,
    detail: &Option<String>,
) -> String {
    let mut h = Sha256::new();
    let field = |h: &mut Sha256, s: &str| {
        h.update((s.len() as u64).to_le_bytes());
        h.update(s.as_bytes());
    };
    field(&mut h, prev);
    field(&mut h, &ts.to_string());
    field(&mut h, task);
    field(&mut h, kind);
    field(&mut h, tool.as_deref().unwrap_or("\u{0}"));
    field(&mut h, args.as_deref().unwrap_or("\u{0}"));
    field(&mut h, &tier.map(|t| t.to_string()).unwrap_or_default());
    field(&mut h, decision.as_deref().unwrap_or("\u{0}"));
    field(&mut h, detail.as_deref().unwrap_or("\u{0}"));
    hex::encode(h.finalize())
}

impl AuditLog {
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).ok();
        }
        let conn = Connection::open(path).with_context(|| format!("opening audit log {}", path.display()))?;
        Self::init(conn)
    }

    pub fn open_in_memory() -> anyhow::Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> anyhow::Result<Self> {
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             CREATE TABLE IF NOT EXISTS audit (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                ts_ms INTEGER NOT NULL,
                task_id TEXT NOT NULL,
                kind TEXT NOT NULL,
                tool TEXT, args TEXT, tier INTEGER, decision TEXT, detail TEXT,
                prev_hash TEXT NOT NULL,
                hash TEXT NOT NULL
             );
             CREATE TRIGGER IF NOT EXISTS audit_no_update BEFORE UPDATE ON audit
               BEGIN SELECT RAISE(ABORT, 'audit log is append-only'); END;
             CREATE TRIGGER IF NOT EXISTS audit_no_delete BEFORE DELETE ON audit
               BEGIN SELECT RAISE(ABORT, 'audit log is append-only'); END;",
        )?;
        Ok(Self { conn: Mutex::new(conn) })
    }

    pub fn append(&self, e: AuditEntry) -> anyhow::Result<i64> {
        let conn = self.conn.lock().unwrap();
        let prev: String = conn
            .query_row("SELECT hash FROM audit ORDER BY id DESC LIMIT 1", [], |r| r.get(0))
            .optional()?
            .unwrap_or_else(|| GENESIS.to_string());
        let ts = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0);
        let (args, detail) = (clip(e.args), clip(e.detail));
        let hash = chain_hash(&prev, ts, &e.task_id, &e.kind, &e.tool, &args, e.tier, &e.decision, &detail);
        conn.execute(
            "INSERT INTO audit (ts_ms, task_id, kind, tool, args, tier, decision, detail, prev_hash, hash)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![ts, e.task_id, e.kind, e.tool, args, e.tier, e.decision, detail, prev, hash],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn recent(&self, limit: usize) -> anyhow::Result<Vec<AuditRecord>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, ts_ms, task_id, kind, tool, args, tier, decision, detail, hash
             FROM audit ORDER BY id DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map([limit as i64], |r| {
            Ok(AuditRecord {
                id: r.get(0)?,
                ts_ms: r.get(1)?,
                task_id: r.get(2)?,
                kind: r.get(3)?,
                tool: r.get(4)?,
                args: r.get(5)?,
                tier: r.get(6)?,
                decision: r.get(7)?,
                detail: r.get(8)?,
                hash: r.get(9)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Recomputes the whole chain.
    pub fn verify(&self) -> anyhow::Result<VerifyReport> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, ts_ms, task_id, kind, tool, args, tier, decision, detail, prev_hash, hash
             FROM audit ORDER BY id ASC",
        )?;
        let mut rows = stmt.query([])?;
        let mut prev = GENESIS.to_string();
        let mut entries = 0;
        while let Some(r) = rows.next()? {
            entries += 1;
            let id: i64 = r.get(0)?;
            let (tool, args, decision, detail): (Option<String>, Option<String>, Option<String>, Option<String>) =
                (r.get(4)?, r.get(5)?, r.get(7)?, r.get(8)?);
            let stored_prev: String = r.get(9)?;
            let stored: String = r.get(10)?;
            let expect = chain_hash(&prev, r.get(1)?, &r.get::<_, String>(2)?, &r.get::<_, String>(3)?, &tool, &args, r.get(6)?, &decision, &detail);
            if stored_prev != prev || stored != expect {
                return Ok(VerifyReport { ok: false, entries, first_bad_id: Some(id) });
            }
            prev = stored;
        }
        Ok(VerifyReport { ok: true, entries, first_bad_id: None })
    }

    #[cfg(test)]
    fn raw(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(kind: &str) -> AuditEntry {
        AuditEntry { task_id: "t1".into(), kind: kind.into(), tool: Some("run_command".into()), tier: Some(3), ..Default::default() }
    }

    #[test]
    fn chain_verifies_and_lists_newest_first() {
        let log = AuditLog::open_in_memory().unwrap();
        for k in ["task_start", "tool", "task_end"] {
            log.append(entry(k)).unwrap();
        }
        assert_eq!(log.verify().unwrap(), VerifyReport { ok: true, entries: 3, first_bad_id: None });
        let recent = log.recent(2).unwrap();
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].kind, "task_end");
    }

    #[test]
    fn update_and_delete_are_rejected() {
        let log = AuditLog::open_in_memory().unwrap();
        log.append(entry("tool")).unwrap();
        let conn = log.raw();
        assert!(conn.execute("UPDATE audit SET decision = 'approved'", []).is_err());
        assert!(conn.execute("DELETE FROM audit", []).is_err());
    }

    #[test]
    fn tampering_breaks_the_chain() {
        let log = AuditLog::open_in_memory().unwrap();
        for k in ["a", "b", "c"] {
            log.append(entry(k)).unwrap();
        }
        {
            let conn = log.raw();
            conn.execute_batch("DROP TRIGGER audit_no_update; UPDATE audit SET decision = 'approved' WHERE id = 2;").unwrap();
        }
        let report = log.verify().unwrap();
        assert!(!report.ok);
        assert_eq!(report.first_bad_id, Some(2));
    }

    #[test]
    fn long_fields_are_clipped() {
        let log = AuditLog::open_in_memory().unwrap();
        log.append(AuditEntry { detail: Some("é".repeat(5000)), ..entry("tool") }).unwrap();
        let rec = &log.recent(1).unwrap()[0];
        assert!(rec.detail.as_ref().unwrap().len() <= MAX_FIELD + 4);
        assert!(log.verify().unwrap().ok);
    }
}

