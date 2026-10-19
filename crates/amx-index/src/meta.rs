//! `meta.sqlite` — `amx-index`'s own bookkeeping database (writer lock here; the quarantine
//! table, task 5, joins it later; `sync_watermark`/`mailbox_counts`, task 10, persist
//! [`crate::planner::SyncState`] between sync passes). Distinct from Apple Mail's read-only
//! Envelope Index: this database is created and owned by `amx-index`, and is read-write.

use std::collections::HashMap;
use std::path::Path;

use amx_core::AmxError;
use amx_store::registry::MailboxCounts;
use rusqlite::{Connection, OptionalExtension, params};

use crate::planner::SyncState;

const SCHEMA_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS writer_lock (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    pid INTEGER NOT NULL,
    process TEXT NOT NULL,
    since INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS quarantine (
    message_rowid INTEGER PRIMARY KEY,
    path TEXT NOT NULL,
    error_kind TEXT NOT NULL,
    first_seen INTEGER NOT NULL,
    attempts INTEGER NOT NULL DEFAULT 1
);

CREATE TABLE IF NOT EXISTS sync_watermark (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    watermark INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS mailbox_counts (
    mailbox_id INTEGER PRIMARY KEY,
    total INTEGER NOT NULL,
    unread INTEGER NOT NULL,
    deleted INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS health_transitions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    state TEXT NOT NULL,
    responsible_process TEXT,
    since INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS triage_plans (
    hash TEXT PRIMARY KEY,
    rowids_json TEXT NOT NULL,
    operation_json TEXT NOT NULL,
    created_at INTEGER NOT NULL
);
"#;

/// Opens (creating if absent) the meta database at `path` and ensures its schema exists.
pub fn open(path: &Path) -> Result<Connection, AmxError> {
    let conn = Connection::open(path)?;
    conn.execute_batch(SCHEMA_SQL)?;
    Ok(conn)
}

/// Loads the last-persisted [`SyncState`], or `SyncState::default()` (watermark 0, no mailbox
/// counts) on a fresh `meta.sqlite` — which makes [`crate::planner::SyncPlanner`] treat every
/// existing message as `Added`, so a first sync pass is a full build with no separate code path.
pub fn load_sync_state(conn: &Connection) -> Result<SyncState, AmxError> {
    let watermark: i64 = conn
        .query_row(
            "SELECT watermark FROM sync_watermark WHERE id = 1",
            [],
            |row| row.get(0),
        )
        .optional()?
        .unwrap_or(0);

    let mut stmt = conn.prepare("SELECT mailbox_id, total, unread, deleted FROM mailbox_counts")?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            MailboxCounts {
                total: row.get(1)?,
                unread: row.get(2)?,
                deleted: row.get(3)?,
            },
        ))
    })?;
    let mut mailbox_counts = HashMap::new();
    for row in rows {
        let (mailbox, counts) = row?;
        mailbox_counts.insert(mailbox, counts);
    }
    Ok(SyncState {
        watermark,
        mailbox_counts,
    })
}

/// Persists `state` so the next sync pass resumes from it. Replaces every `mailbox_counts` row
/// wholesale rather than diffing — `state.mailbox_counts` is already the complete current set
/// ([`crate::planner::SyncPlanner::plan`] loads all of `mailboxes` every pass), so a stale row
/// left behind would misrepresent a mailbox that still exists.
pub fn save_sync_state(conn: &Connection, state: &SyncState) -> Result<(), AmxError> {
    conn.execute(
        "INSERT INTO sync_watermark (id, watermark) VALUES (1, ?1)
         ON CONFLICT(id) DO UPDATE SET watermark = excluded.watermark",
        params![state.watermark],
    )?;
    conn.execute("DELETE FROM mailbox_counts", [])?;
    for (&mailbox, counts) in &state.mailbox_counts {
        conn.execute(
            "INSERT INTO mailbox_counts (mailbox_id, total, unread, deleted)
             VALUES (?1, ?2, ?3, ?4)",
            params![mailbox, counts.total, counts.unread, counts.deleted],
        )?;
    }
    Ok(())
}

/// Appends a `health_transitions` row for `amx-mcp`'s `HealthMonitor` (Phase 3 task 7). Each row
/// is a full transition record rather than a single "current state" row so `doctor` can report
/// how long the store has been in its current state ("denied since 08-07 (4 days)") without a
/// separate started-at column that could drift out of sync with the row it describes.
pub fn record_health_transition(
    conn: &Connection,
    state: &str,
    responsible_process: Option<&str>,
    since: i64,
) -> Result<(), AmxError> {
    conn.execute(
        "INSERT INTO health_transitions (state, responsible_process, since) VALUES (?1, ?2, ?3)",
        params![state, responsible_process, since],
    )?;
    Ok(())
}

/// The most recently recorded transition, or `None` on a fresh `meta.sqlite` that has never
/// run a health probe.
pub fn last_health_transition(
    conn: &Connection,
) -> Result<Option<(String, Option<String>, i64)>, AmxError> {
    conn.query_row(
        "SELECT state, responsible_process, since FROM health_transitions \
         ORDER BY id DESC LIMIT 1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )
    .optional()
    .map_err(AmxError::from)
}

/// Persists a `triage_plan` (Phase 4 task 6) so `triage_apply` (task 7) can later look it up by
/// hash on a separate connection. `hash` is content-addressed by the caller, so a re-`INSERT` of
/// the same hash is always the same content — `DO NOTHING` keeps this idempotent.
pub fn save_triage_plan(
    conn: &Connection,
    hash: &str,
    rowids_json: &str,
    operation_json: &str,
    created_at: i64,
) -> Result<(), AmxError> {
    conn.execute(
        "INSERT INTO triage_plans (hash, rowids_json, operation_json, created_at) \
         VALUES (?1, ?2, ?3, ?4) ON CONFLICT(hash) DO NOTHING",
        params![hash, rowids_json, operation_json, created_at],
    )?;
    Ok(())
}

/// Loads a previously frozen plan's `(rowids_json, operation_json)` by its content hash, or
/// `None` if `hash` is unknown (including a hash mutated after the fact — it simply won't match
/// any row).
pub fn load_triage_plan(
    conn: &Connection,
    hash: &str,
) -> Result<Option<(String, String)>, AmxError> {
    conn.query_row(
        "SELECT rowids_json, operation_json FROM triage_plans WHERE hash = ?1",
        params![hash],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .optional()
    .map_err(AmxError::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opening_twice_is_idempotent() {
        let file = tempfile::NamedTempFile::new().unwrap();
        open(file.path()).unwrap();
        open(file.path()).unwrap();
    }

    #[test]
    fn a_fresh_database_loads_the_default_sync_state() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let conn = open(file.path()).unwrap();
        assert_eq!(load_sync_state(&conn).unwrap(), SyncState::default());
    }

    #[test]
    fn sync_state_round_trips_through_save_and_load() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let conn = open(file.path()).unwrap();

        let mut mailbox_counts = HashMap::new();
        mailbox_counts.insert(
            1,
            MailboxCounts {
                total: 120,
                unread: 4,
                deleted: 0,
            },
        );
        let state = SyncState {
            watermark: 42,
            mailbox_counts,
        };

        save_sync_state(&conn, &state).unwrap();
        assert_eq!(load_sync_state(&conn).unwrap(), state);
    }

    #[test]
    fn saving_again_replaces_stale_mailbox_counts_wholesale() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let conn = open(file.path()).unwrap();

        let mut first_counts = HashMap::new();
        first_counts.insert(
            1,
            MailboxCounts {
                total: 1,
                unread: 1,
                deleted: 0,
            },
        );
        save_sync_state(
            &conn,
            &SyncState {
                watermark: 1,
                mailbox_counts: first_counts,
            },
        )
        .unwrap();

        let mut second_counts = HashMap::new();
        second_counts.insert(
            2,
            MailboxCounts {
                total: 2,
                unread: 0,
                deleted: 0,
            },
        );
        let second_state = SyncState {
            watermark: 2,
            mailbox_counts: second_counts,
        };
        save_sync_state(&conn, &second_state).unwrap();

        assert_eq!(load_sync_state(&conn).unwrap(), second_state);
    }

    #[test]
    fn a_fresh_database_has_no_health_transition() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let conn = open(file.path()).unwrap();
        assert_eq!(last_health_transition(&conn).unwrap(), None);
    }

    #[test]
    fn health_transitions_are_recorded_newest_first() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let conn = open(file.path()).unwrap();

        record_health_transition(&conn, "denied", Some("amxcli"), 100).unwrap();
        record_health_transition(&conn, "granted", None, 200).unwrap();

        assert_eq!(
            last_health_transition(&conn).unwrap(),
            Some(("granted".to_string(), None, 200))
        );
    }

    #[test]
    fn a_fresh_database_has_no_triage_plan() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let conn = open(file.path()).unwrap();
        assert_eq!(load_triage_plan(&conn, "deadbeef").unwrap(), None);
    }

    #[test]
    fn a_triage_plan_round_trips_through_save_and_load() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let conn = open(file.path()).unwrap();

        save_triage_plan(&conn, "abc123", "[1,2,3]", "{\"kind\":\"trash\"}", 100).unwrap();

        assert_eq!(
            load_triage_plan(&conn, "abc123").unwrap(),
            Some(("[1,2,3]".to_string(), "{\"kind\":\"trash\"}".to_string()))
        );
    }

    #[test]
    fn saving_the_same_hash_twice_is_idempotent() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let conn = open(file.path()).unwrap();

        save_triage_plan(&conn, "abc123", "[1]", "{\"kind\":\"trash\"}", 100).unwrap();
        save_triage_plan(&conn, "abc123", "[1]", "{\"kind\":\"trash\"}", 200).unwrap();

        assert_eq!(
            load_triage_plan(&conn, "abc123").unwrap(),
            Some(("[1]".to_string(), "{\"kind\":\"trash\"}".to_string()))
        );
    }
}
