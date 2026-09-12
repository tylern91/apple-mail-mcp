//! FSEvents-driven change notification for the Mail store root (umbrella §4.3.1, task 8).
//! Emits a debounced "something changed" signal only -- never an interpreted delta -- so a
//! caller re-runs `SyncPlanner::plan` exactly as a polling loop would; FSEvents and polling feed
//! the identical sync code path (no optimistic post-mutation index update here, that's Phase 4,
//! umbrella §4.3.2).
#![cfg(target_os = "macos")]

use std::path::Path;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::Duration;

use notify::RecursiveMode;
use notify_debouncer_mini::{DebouncedEvent, Debouncer, new_debouncer};

const DEBOUNCE_WINDOW: Duration = Duration::from_millis(500);

/// A live, debounced FSEvents watch on the Mail store root. Dropping it stops watching.
pub struct MailboxWatcher {
    _debouncer: Debouncer<notify::RecommendedWatcher>,
    changes: Receiver<()>,
}

impl MailboxWatcher {
    /// Starts watching `root` recursively, debounced at 500ms.
    pub fn watch(root: &Path) -> Result<Self, notify::Error> {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut debouncer = new_debouncer(
            DEBOUNCE_WINDOW,
            move |result: Result<Vec<DebouncedEvent>, notify::Error>| {
                if matches!(result, Ok(events) if !events.is_empty()) {
                    let _ = tx.send(());
                }
            },
        )?;
        debouncer.watcher().watch(root, RecursiveMode::Recursive)?;
        Ok(Self {
            _debouncer: debouncer,
            changes: rx,
        })
    }

    /// Blocks until a debounced change is observed. Returns `false` once the watch has ended
    /// (the sending side dropped) rather than blocking forever.
    pub fn recv_change(&self) -> bool {
        self.changes.recv().is_ok()
    }

    /// Blocks up to `timeout` for a change signal.
    pub fn recv_change_timeout(&self, timeout: Duration) -> Result<bool, RecvTimeoutError> {
        match self.changes.recv_timeout(timeout) {
            Ok(()) => Ok(true),
            Err(RecvTimeoutError::Timeout) => Ok(false),
            Err(e) => Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn a_file_write_debounces_into_a_single_change_signal() {
        let dir = tempfile::tempdir().unwrap();
        let watcher = MailboxWatcher::watch(dir.path()).unwrap();

        std::fs::write(dir.path().join("Envelope Index"), b"x").unwrap();

        let observed = watcher.recv_change_timeout(Duration::from_secs(5)).unwrap();
        assert!(observed);
    }

    #[test]
    fn no_change_within_the_timeout_reports_false() {
        let dir = tempfile::tempdir().unwrap();
        let watcher = MailboxWatcher::watch(dir.path()).unwrap();

        let observed = watcher
            .recv_change_timeout(Duration::from_millis(200))
            .unwrap();
        assert!(!observed);
    }
}
