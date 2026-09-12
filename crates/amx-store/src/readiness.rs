//! Store-readiness probes shared by `amxcli doctor` and `amx-mcp`'s `doctor`/`status` tools
//! (Phase 3 task 8) — lifted here rather than duplicated in each caller, since both already
//! depend on `amx-store` and neither depends on the other.

use std::path::Path;

use amx_core::traits::AccessState;

#[cfg(target_os = "macos")]
pub fn probe_access(store_path: &Path) -> AccessState {
    crate::tcc::TccProbe::probe(store_path)
}

/// TCC (and Full Disk Access with it) is macOS-only — there is nothing to deny on any other
/// platform this crate might be built and tested on.
#[cfg(not(target_os = "macos"))]
pub fn probe_access(_store_path: &Path) -> AccessState {
    AccessState::Granted
}

/// Every account UUID directory sits directly under the store root, as a sibling of the
/// non-account `MailData` directory that holds the Envelope Index.
pub fn count_account_directories(store_path: &Path) -> usize {
    let Ok(entries) = std::fs::read_dir(store_path) else {
        return 0;
    };
    entries
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|ft| ft.is_dir()))
        .filter(|entry| entry.file_name() != "MailData")
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn counts_account_directories_excluding_mail_data() {
        let store = TempDir::new().unwrap();
        std::fs::create_dir(store.path().join("MailData")).unwrap();
        std::fs::create_dir(store.path().join("ACCOUNT-ONE")).unwrap();
        std::fs::create_dir(store.path().join("ACCOUNT-TWO")).unwrap();

        assert_eq!(count_account_directories(store.path()), 2);
    }

    #[test]
    fn missing_store_path_reports_zero_accounts() {
        assert_eq!(count_account_directories(Path::new("/nonexistent")), 0);
    }
}
