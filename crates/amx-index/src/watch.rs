//! Wires `amx_store`'s FSEvents watch (task 8) into [`SyncPlanner`] -- the same `plan` call a
//! polling loop uses, so FSEvents and polling share exactly one sync code path rather than a
//! second, independently-maintained one. No optimistic post-mutation index update happens here;
//! that's Phase 4 (umbrella §4.3.2).
#![cfg(target_os = "macos")]

use std::path::Path;

use amx_core::AmxError;
use amx_store::conn::RoConnection;
use amx_store::watch::MailboxWatcher;

use crate::planner::{SyncPlanner, SyncState, WorkSet};

pub struct FsEventsSync {
    watcher: MailboxWatcher,
}

impl FsEventsSync {
    /// Starts watching `store_root` for changes.
    pub fn watch(store_root: &Path) -> Result<Self, notify::Error> {
        Ok(Self {
            watcher: MailboxWatcher::watch(store_root)?,
        })
    }

    /// Blocks until FSEvents reports a debounced change, then computes the resulting `WorkSet`
    /// via the identical `SyncPlanner::plan` polling uses. Returns `None` once the watch ends.
    pub fn next_work_set(
        &self,
        conn: &RoConnection,
        state: &SyncState,
    ) -> Option<Result<WorkSet, AmxError>> {
        self.watcher
            .recv_change()
            .then(|| SyncPlanner::plan(conn, state))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use amx_store::fixtures::{build_schema, load_fixture};
    use rusqlite::Connection as RawConnection;

    const NULL_CHANGE_IDENTIFIER_SQL: &str =
        include_str!("../../amx-store/tests/fixtures/store/null_change_identifier.sql");

    #[test]
    fn a_store_write_yields_a_work_set_via_the_same_planner_call() {
        let store_file = tempfile::NamedTempFile::new().unwrap();
        let store_conn = RawConnection::open(store_file.path()).unwrap();
        build_schema(&store_conn).unwrap();
        load_fixture(&store_conn, NULL_CHANGE_IDENTIFIER_SQL).unwrap();
        store_conn
            .execute(
                "INSERT INTO messages (ROWID, subject, mailbox, deleted) VALUES (1, 1, 1, 0)",
                [],
            )
            .unwrap();
        drop(store_conn);

        let watch_dir = tempfile::tempdir().unwrap();
        let sync = FsEventsSync::watch(watch_dir.path()).unwrap();

        std::fs::write(watch_dir.path().join("touch"), b"x").unwrap();

        let conn = RoConnection::open(store_file.path()).unwrap();
        let state = SyncState::default();
        let work_set = sync
            .next_work_set(&conn, &state)
            .expect("a change was observed")
            .unwrap();

        assert!(!work_set.items.is_empty());
    }
}
