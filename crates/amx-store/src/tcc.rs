//! Classifies Full Disk Access to the Mail store, per umbrella §4.4's `HealthMonitor`.
#![cfg(target_os = "macos")]

use std::path::Path;

use amx_core::traits::AccessState;

/// Probes access to `path` (typically the Mail store root) by attempting to read it.
///
/// lean: never returns [`AccessState::PendingRestart`] — telling that apart from `Denied`
/// requires reading TCC.db to compare the recorded grant against what the kernel is actually
/// enforcing, and reading TCC.db is itself gated by Full Disk Access (umbrella §11 open
/// question). Upgrade when `amxcli doctor` needs to distinguish a stale grant from an outright
/// denial.
pub struct TccProbe;

impl TccProbe {
    pub fn probe(path: impl AsRef<Path>) -> AccessState {
        match std::fs::read_dir(path.as_ref()) {
            Ok(_) => AccessState::Granted,
            Err(_) => AccessState::Denied {
                responsible_process: responsible_process_name(),
            },
        }
    }
}

/// The identity macOS keys the FDA grant to — never a hardcoded "Terminal" (imdinu-adjacent:
/// the responsible process rotates between `amxcli`, a wrapping shell, and an MCP host).
fn responsible_process_name() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "amxcli".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn granted_for_readable_dir() {
        assert_eq!(TccProbe::probe(std::env::temp_dir()), AccessState::Granted);
    }

    #[test]
    fn denied_for_missing_path() {
        let state = TccProbe::probe("/nonexistent/amx-store-tcc-probe-test");
        assert!(matches!(state, AccessState::Denied { .. }));
    }
}
