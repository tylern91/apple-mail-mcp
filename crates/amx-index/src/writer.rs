//! Single-writer discipline over the Tantivy index, backed by an advisory lock row in
//! `meta.sqlite` (umbrella §4.2.2). A second writer fails fast with
//! [`AmxError::IndexWriterBusy`] instead of blocking — imdinu #122's defect was the opposite
//! failure mode, a lock that could never be broken short of quitting the MCP client entirely.
//! A lock whose owning pid is no longer running is reclaimed rather than treated as forever
//! held, so a crashed writer cannot brick the index for every future run.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use amx_core::AmxError;
use rusqlite::{Connection, OptionalExtension, params};

use crate::meta;

struct LockRow {
    pid: u32,
    process: String,
    since_unix: i64,
}

/// Holds the advisory writer lock for as long as it lives. Dropping it releases the row it
/// inserted — but only if that row still names this guard's own pid, since a lock reclaimed by
/// another process in the interim is no longer this guard's to release.
#[derive(Debug)]
pub struct IndexWriterGuard {
    conn: Connection,
    pid: u32,
}

#[derive(Debug)]
pub struct IndexWriter;

impl IndexWriter {
    /// Acquires the lock in the `meta.sqlite` at `meta_path`, or returns
    /// [`AmxError::IndexWriterBusy`] naming the pid, process, and timestamp of the writer
    /// currently holding it (unless that pid is no longer alive, in which case the lock is
    /// reclaimed instead of reported as busy).
    pub fn acquire(meta_path: &Path) -> Result<IndexWriterGuard, AmxError> {
        let conn = meta::open(meta_path)?;
        let pid = std::process::id();

        if let Some(existing) = read_lock(&conn)?
            && is_alive(existing.pid)
        {
            return Err(AmxError::IndexWriterBusy {
                pid: existing.pid,
                process: existing.process,
                since: unix_secs_to_system_time(existing.since_unix),
            });
        }

        conn.execute(
            "INSERT INTO writer_lock (id, pid, process, since) VALUES (1, ?1, ?2, ?3)
             ON CONFLICT(id) DO UPDATE SET pid = excluded.pid, process = excluded.process, \
             since = excluded.since",
            params![pid, current_process_name(), unix_now()],
        )?;

        Ok(IndexWriterGuard { conn, pid })
    }
}

impl Drop for IndexWriterGuard {
    fn drop(&mut self) {
        let _ = self.conn.execute(
            "DELETE FROM writer_lock WHERE id = 1 AND pid = ?1",
            params![self.pid],
        );
    }
}

fn read_lock(conn: &Connection) -> Result<Option<LockRow>, AmxError> {
    Ok(conn
        .query_row(
            "SELECT pid, process, since FROM writer_lock WHERE id = 1",
            [],
            |row| {
                Ok(LockRow {
                    pid: row.get(0)?,
                    process: row.get(1)?,
                    since_unix: row.get(2)?,
                })
            },
        )
        .optional()?)
}

/// Whether `pid` still names a live process, checked by shelling out to `kill -0` rather than
/// adding a process-management dependency — `kill` is present identically on the macOS runtime
/// and the Linux CI portable-subset job.
fn is_alive(pid: u32) -> bool {
    std::process::Command::new("kill")
        .arg("-0")
        .arg(pid.to_string())
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn current_process_name() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|path| {
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| "amx-index".to_string())
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn unix_secs_to_system_time(secs: i64) -> SystemTime {
    UNIX_EPOCH + std::time::Duration::from_secs(secs.max(0) as u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection as RawConnection;

    #[test]
    fn acquire_and_release_round_trips_the_lock_row() {
        let dir = tempfile::tempdir().unwrap();
        let meta_path = dir.path().join("meta.sqlite");

        let guard = IndexWriter::acquire(&meta_path).unwrap();
        let held_pid: u32 = RawConnection::open(&meta_path)
            .unwrap()
            .query_row("SELECT pid FROM writer_lock WHERE id = 1", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(held_pid, std::process::id());

        drop(guard);
        let remaining: i64 = RawConnection::open(&meta_path)
            .unwrap()
            .query_row("SELECT COUNT(*) FROM writer_lock", [], |row| row.get(0))
            .unwrap();
        assert_eq!(remaining, 0);
    }

    #[test]
    fn a_lock_held_by_a_live_pid_fails_fast() {
        let dir = tempfile::tempdir().unwrap();
        let meta_path = dir.path().join("meta.sqlite");

        // Our own pid is unimpeachably alive, so seeding the lock with it simulates "another
        // writer is running" without needing to spawn a real second process.
        meta::open(&meta_path)
            .unwrap()
            .execute(
                "INSERT INTO writer_lock (id, pid, process, since) VALUES (1, ?1, 'other', 0)",
                params![std::process::id()],
            )
            .unwrap();

        let err = IndexWriter::acquire(&meta_path).unwrap_err();
        assert!(matches!(err, AmxError::IndexWriterBusy { pid, .. } if pid == std::process::id()));
    }

    #[test]
    fn a_lock_held_by_a_dead_pid_is_reclaimed() {
        let dir = tempfile::tempdir().unwrap();
        let meta_path = dir.path().join("meta.sqlite");

        let long_dead_pid: u32 = 999_999_999;
        meta::open(&meta_path)
            .unwrap()
            .execute(
                "INSERT INTO writer_lock (id, pid, process, since) VALUES (1, ?1, 'ghost', 0)",
                params![long_dead_pid],
            )
            .unwrap();

        let guard = IndexWriter::acquire(&meta_path).unwrap();
        assert_eq!(guard.pid, std::process::id());
    }
}
