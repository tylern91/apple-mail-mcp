//! Startup probe + re-probe of Full Disk Access to the mail store (Phase 3 task 7, umbrella
//! §4.4). Every MCP response stamps `health.store_access` from here while degraded — not just
//! tools that touch the store — so an agent never has to guess whether an empty result means
//! "no matches" or "we lost access mid-session".
//!
//! The `#[cfg(target_os)]` probe pair mirrors `amx-cli`'s `doctor::probe_access`: `amx-mcp` does
//! not depend on `amx-cli`, so the pattern is repeated here rather than shared.

use std::path::{Path, PathBuf};
use std::sync::RwLock;

use amx_core::traits::AccessState;
use amx_index::meta;
use rusqlite::Connection;

#[cfg(target_os = "macos")]
fn probe(store_path: &Path) -> AccessState {
    amx_store::tcc::TccProbe::probe(store_path)
}

/// TCC (and Full Disk Access with it) is macOS-only — there is nothing to deny on any other
/// platform this crate might be built and tested on.
#[cfg(not(target_os = "macos"))]
fn probe(_store_path: &Path) -> AccessState {
    AccessState::Granted
}

fn state_kind(state: &AccessState) -> &'static str {
    match state {
        AccessState::Granted => "granted",
        AccessState::Denied { .. } => "denied",
        AccessState::PendingRestart { .. } => "pending_restart",
    }
}

fn responsible_process(state: &AccessState) -> Option<&str> {
    match state {
        AccessState::Granted => None,
        AccessState::Denied {
            responsible_process,
        }
        | AccessState::PendingRestart {
            responsible_process,
        } => Some(responsible_process.as_str()),
    }
}

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Best-effort: a failure to persist a transition should not crash the health probe itself —
/// `health.store_access` on the next response is still correct even if `doctor`'s
/// `degraded_since` falls back to "unknown".
fn record_transition(meta_conn: &Connection, state: &AccessState) {
    let _ = meta::record_health_transition(
        meta_conn,
        state_kind(state),
        responsible_process(state),
        now_unix(),
    );
}

/// Tracks the mail store's Full Disk Access state across the server's lifetime.
pub struct HealthMonitor {
    store_path: PathBuf,
    state: RwLock<AccessState>,
    prober: Box<dyn Fn(&Path) -> AccessState + Send + Sync>,
}

impl HealthMonitor {
    /// Probes `store_path` once at startup and records the initial transition.
    pub fn start(store_path: PathBuf, meta_conn: &Connection) -> Self {
        Self::start_with_prober(store_path, meta_conn, probe)
    }

    fn start_with_prober(
        store_path: PathBuf,
        meta_conn: &Connection,
        prober: impl Fn(&Path) -> AccessState + Send + Sync + 'static,
    ) -> Self {
        let state = prober(&store_path);
        record_transition(meta_conn, &state);
        Self {
            store_path,
            state: RwLock::new(state),
            prober: Box::new(prober),
        }
    }

    /// The last-known access state, without re-probing.
    pub fn current(&self) -> AccessState {
        self.state
            .read()
            .expect("health monitor lock poisoned")
            .clone()
    }

    /// Re-probes access, recording a transition only when the state actually changed.
    ///
    /// A probe result that flips from anything else to `Granted` is confirmed with one
    /// immediate second probe before being finalized — `TccProbe` reads the filesystem live, so
    /// a single success is already enforced, but a second confirming read guards a re-probe that
    /// raced a grant applying mid-syscall from momentarily reporting `Granted` and then flapping
    /// back on the very next tick.
    pub fn reprobe(&self, meta_conn: &Connection) -> AccessState {
        let previous = self.current();
        let mut next = (self.prober)(&self.store_path);
        if next == AccessState::Granted && previous != AccessState::Granted {
            next = (self.prober)(&self.store_path);
        }

        if next != previous {
            record_transition(meta_conn, &next);
            *self.state.write().expect("health monitor lock poisoned") = next.clone();
        }
        next
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    fn denied(process: &str) -> AccessState {
        AccessState::Denied {
            responsible_process: process.to_string(),
        }
    }

    /// Returns `states[0]` on the first call, `states[1]` on the second, and so on, repeating
    /// the last entry once the sequence is exhausted.
    fn sequence_prober(states: Vec<AccessState>) -> impl Fn(&Path) -> AccessState + Send + Sync {
        let index = Mutex::new(0usize);
        move |_path| {
            let mut i = index.lock().unwrap();
            let state = states
                .get(*i)
                .cloned()
                .unwrap_or_else(|| states.last().unwrap().clone());
            *i += 1;
            state
        }
    }

    #[test]
    fn start_records_the_initial_transition() {
        let meta_file = tempfile::NamedTempFile::new().unwrap();
        let meta_conn = meta::open(meta_file.path()).unwrap();

        let monitor = HealthMonitor::start_with_prober(
            PathBuf::from("/store"),
            &meta_conn,
            sequence_prober(vec![denied("amxcli")]),
        );

        assert_eq!(monitor.current(), denied("amxcli"));
        let (state, responsible_process, _since) =
            meta::last_health_transition(&meta_conn).unwrap().unwrap();
        assert_eq!(state, "denied");
        assert_eq!(responsible_process.as_deref(), Some("amxcli"));
    }

    #[test]
    fn reprobe_with_unchanged_state_does_not_record_a_new_transition() {
        let meta_file = tempfile::NamedTempFile::new().unwrap();
        let meta_conn = meta::open(meta_file.path()).unwrap();

        let monitor = HealthMonitor::start_with_prober(
            PathBuf::from("/store"),
            &meta_conn,
            sequence_prober(vec![AccessState::Granted]),
        );
        monitor.reprobe(&meta_conn);
        monitor.reprobe(&meta_conn);

        let count: i64 = meta_conn
            .query_row("SELECT COUNT(*) FROM health_transitions", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn reprobe_does_not_finalize_a_grant_that_flickers_on_confirmation() {
        let meta_file = tempfile::NamedTempFile::new().unwrap();
        let meta_conn = meta::open(meta_file.path()).unwrap();

        // Sequence: Denied at startup, then a reprobe whose candidate read claims Granted but
        // whose immediate confirming read reverts to Denied.
        let monitor = HealthMonitor::start_with_prober(
            PathBuf::from("/store"),
            &meta_conn,
            sequence_prober(vec![
                denied("amxcli"),
                AccessState::Granted,
                denied("amxcli"),
            ]),
        );

        let result = monitor.reprobe(&meta_conn);

        assert_eq!(result, denied("amxcli"));
        assert_eq!(monitor.current(), denied("amxcli"));
    }

    #[test]
    fn reprobe_confirms_a_genuine_grant() {
        let meta_file = tempfile::NamedTempFile::new().unwrap();
        let meta_conn = meta::open(meta_file.path()).unwrap();

        // Sequence: Denied at startup, then both the candidate and confirming reprobe reads
        // agree the store is now Granted.
        let monitor = HealthMonitor::start_with_prober(
            PathBuf::from("/store"),
            &meta_conn,
            sequence_prober(vec![
                denied("amxcli"),
                AccessState::Granted,
                AccessState::Granted,
            ]),
        );

        let result = monitor.reprobe(&meta_conn);

        assert_eq!(result, AccessState::Granted);
        assert_eq!(monitor.current(), AccessState::Granted);
        assert_eq!(
            meta::last_health_transition(&meta_conn).unwrap().unwrap().0,
            "granted"
        );
    }
}
