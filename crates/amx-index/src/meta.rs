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
}
