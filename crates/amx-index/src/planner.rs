//! Computes incremental sync work from three signals, none of them `mailboxes.change_identifier`
//! (NULL on every mailbox in the live store — umbrella §0 correction C1). Paths are never walked
//! (imdinu #84); everything here is a targeted SQL query against the read-only Envelope Index.

use std::collections::HashMap;

use amx_core::AmxError;
use amx_store::conn::RoConnection;
use amx_store::registry::MailboxCounts;

/// What the planner needs remembered between runs — the watermark plus each mailbox's last-seen
/// count tuple. Callers persist this (e.g. in `meta.sqlite`, task 3) and pass the prior value back
/// in on the next call; [`WorkSet::next_state`] is the value to persist afterward.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyncState {
    pub watermark: i64,
    pub mailbox_counts: HashMap<i64, MailboxCounts>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkKind {
    Added,
    Modified,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkItem {
    pub rowid: i64,
    pub mailbox: i64,
    pub kind: WorkKind,
}

/// The planner's output. Structurally incapable of carrying a deletion (parasxos #2) — there is
/// no variant and no field a deletion could be encoded into. The `Reconciler` (task 7) is the only
/// code path ever allowed to emit one, from a full-snapshot scan.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorkSet {
    pub items: Vec<WorkItem>,
    pub next_state: SyncState,
}

pub struct SyncPlanner;

impl SyncPlanner {
    /// Computes the work required to catch the index up to `conn`'s current state, given the
    /// last-persisted `state`.
    ///
    /// Soft-deleted rows (`deleted = 1`) are excluded from both `Added` and `Modified` — a message
    /// that arrived and was deleted before ever being indexed needs no work, and a message the
    /// index already holds that becomes deleted still surfaces as `Modified` (via its mailbox's
    /// `deleted_count` change) so the write path can drop it from Tantivy; the incremental planner
    /// itself never emits a delete signal.
    pub fn plan(conn: &RoConnection, state: &SyncState) -> Result<WorkSet, AmxError> {
        let sql = conn.as_connection();

        let new_watermark: i64 = sql.query_row(
            "SELECT COALESCE(MAX(ROWID), ?1) FROM messages",
            [state.watermark],
            |row| row.get(0),
        )?;

        let mut items = Vec::new();
        Self::collect_added(sql, state.watermark, &mut items)?;

        let current_counts = Self::load_mailbox_counts(sql)?;
        Self::collect_modified(sql, state, &current_counts, &mut items)?;

        Ok(WorkSet {
            items,
            next_state: SyncState {
                watermark: new_watermark,
                mailbox_counts: current_counts,
            },
        })
    }

    fn collect_added(
        sql: &rusqlite::Connection,
        watermark: i64,
        items: &mut Vec<WorkItem>,
    ) -> Result<(), AmxError> {
        let mut stmt =
            sql.prepare("SELECT ROWID, mailbox FROM messages WHERE ROWID > ?1 AND deleted = 0")?;
        let rows = stmt.query_map([watermark], |row| Ok((row.get(0)?, row.get(1)?)))?;
        for row in rows {
            let (rowid, mailbox): (i64, i64) = row?;
            items.push(WorkItem {
                rowid,
                mailbox,
                kind: WorkKind::Added,
            });
        }
        Ok(())
    }

    fn load_mailbox_counts(
        sql: &rusqlite::Connection,
    ) -> Result<HashMap<i64, MailboxCounts>, AmxError> {
        let mut stmt =
            sql.prepare("SELECT ROWID, total_count, unread_count, deleted_count FROM mailboxes")?;
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
        let mut counts = HashMap::new();
        for row in rows {
            let (mailbox, mailbox_counts) = row?;
            counts.insert(mailbox, mailbox_counts);
        }
        Ok(counts)
    }

    /// Rows already caught by [`Self::collect_added`] (`ROWID > watermark`) are excluded here so a
    /// mailbox flagged as changed doesn't emit the same rowid twice under two different kinds.
    fn collect_modified(
        sql: &rusqlite::Connection,
        state: &SyncState,
        current_counts: &HashMap<i64, MailboxCounts>,
        items: &mut Vec<WorkItem>,
    ) -> Result<(), AmxError> {
        let mut stmt = sql.prepare(
            "SELECT ROWID FROM messages WHERE mailbox = ?1 AND ROWID <= ?2 AND deleted = 0",
        )?;
        for (&mailbox, counts) in current_counts {
            if state.mailbox_counts.get(&mailbox) == Some(counts) {
                continue;
            }
            let rows = stmt.query_map([mailbox, state.watermark], |row| row.get::<_, i64>(0))?;
            for row in rows {
                items.push(WorkItem {
                    rowid: row?,
                    mailbox,
                    kind: WorkKind::Modified,
                });
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use amx_store::fixtures::{build_schema, load_fixture};
    use rusqlite::Connection;

    const NULL_CHANGE_IDENTIFIER_SQL: &str =
        include_str!("../../amx-store/tests/fixtures/store/null_change_identifier.sql");

    fn seed(path: &std::path::Path) {
        let conn = Connection::open(path).unwrap();
        build_schema(&conn).unwrap();
        load_fixture(&conn, NULL_CHANGE_IDENTIFIER_SQL).unwrap();
        conn.execute(
            "INSERT INTO messages (ROWID, subject, mailbox, deleted) VALUES (1, 1, 1, 0), (2, 1, 1, 0)",
            [],
        )
        .unwrap();
    }

    #[test]
    fn plans_against_null_change_identifier_without_reading_it() {
        let file = tempfile::NamedTempFile::new().unwrap();
        seed(file.path());

        let conn = RoConnection::open(file.path()).unwrap();
        let state = SyncState::default();
        let work = SyncPlanner::plan(&conn, &state).unwrap();

        assert_eq!(work.next_state.watermark, 2);
        assert_eq!(work.items.len(), 2);
        assert!(work.items.iter().all(|item| item.kind == WorkKind::Added));
        assert_eq!(
            work.next_state.mailbox_counts.get(&1),
            Some(&MailboxCounts {
                total: 120,
                unread: 4,
                deleted: 0
            })
        );
    }

    #[test]
    fn unchanged_mailbox_tuple_yields_no_modified_work_on_second_pass() {
        let file = tempfile::NamedTempFile::new().unwrap();
        seed(file.path());

        let conn = RoConnection::open(file.path()).unwrap();
        let first = SyncPlanner::plan(&conn, &SyncState::default()).unwrap();
        let second = SyncPlanner::plan(&conn, &first.next_state).unwrap();

        assert!(second.items.is_empty());
        assert_eq!(second.next_state.watermark, first.next_state.watermark);
    }

    #[test]
    fn mailbox_count_drift_resurfaces_prior_rows_as_modified_not_added() {
        let file = tempfile::NamedTempFile::new().unwrap();
        seed(file.path());

        let conn = RoConnection::open(file.path()).unwrap();
        let first = SyncPlanner::plan(&conn, &SyncState::default()).unwrap();

        Connection::open(file.path())
            .unwrap()
            .execute("UPDATE mailboxes SET unread_count = 3 WHERE ROWID = 1", [])
            .unwrap();

        let second = SyncPlanner::plan(&conn, &first.next_state).unwrap();
        assert_eq!(second.items.len(), 2);
        assert!(
            second
                .items
                .iter()
                .all(|item| item.kind == WorkKind::Modified)
        );
    }
}
