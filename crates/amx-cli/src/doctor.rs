//! `amxcli doctor` — a read-only readiness screen (umbrella §4.4's `HealthMonitor`, surfaced as
//! a CLI report rather than a background monitor). No Tantivy: this phase ends at the read path,
//! before an index exists to report on.

use std::fmt;
use std::path::Path;

use amx_core::AmxError;
use amx_core::traits::AccessState;
use amx_store::readiness::{count_account_directories, probe_access};
use amx_store::{MailboxRegistry, RoConnection};

/// Raw filesystem coverage — counted by walking `.emlx`/`.partial.emlx` files directly, not by
/// consulting the Envelope Index. This is deliberately cruder than task 12's per-message
/// completeness oracle: `doctor` answers "how much of the store is even parseable", not "is this
/// one message's body indexed".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CoverageCounts {
    pub clean_parse: usize,
    pub quarantine: usize,
    pub partial: usize,
}

#[derive(Debug)]
pub struct DoctorReport {
    pub access_state: AccessState,
    pub store_version: String,
    pub account_count: usize,
    pub mailbox_count: usize,
    pub coverage: CoverageCounts,
}

impl fmt::Display for DoctorReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let fda = match &self.access_state {
            AccessState::Granted => "granted".to_string(),
            AccessState::Denied {
                responsible_process,
            } => format!("denied ({responsible_process})"),
            AccessState::PendingRestart {
                responsible_process,
            } => format!("pending restart ({responsible_process})"),
        };
        writeln!(f, "Full Disk Access: {fda}")?;
        writeln!(f, "Store version:    {}", self.store_version)?;
        writeln!(f, "Accounts:         {}", self.account_count)?;
        writeln!(f, "Mailboxes:        {}", self.mailbox_count)?;
        writeln!(
            f,
            "Coverage:         {} clean-parse, {} quarantined, {} partial",
            self.coverage.clean_parse, self.coverage.quarantine, self.coverage.partial
        )
    }
}

/// Gathers the doctor screen's data from `store_path` (a `~/Library/Mail/V10`-shaped directory)
/// without writing anything.
pub fn gather(store_path: &Path) -> Result<DoctorReport, AmxError> {
    let access_state = probe_access(store_path);

    let store_version = store_path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "unknown".to_string());

    let account_count = count_account_directories(store_path);

    let envelope_index_path = store_path.join("MailData").join("Envelope Index");
    let mailbox_count = if envelope_index_path.exists() {
        let conn = RoConnection::open(&envelope_index_path)?;
        MailboxRegistry::load(&conn)?.len()
    } else {
        0
    };

    let coverage = count_coverage(store_path);

    Ok(DoctorReport {
        access_state,
        store_version,
        account_count,
        mailbox_count,
        coverage,
    })
}

fn count_coverage(store_path: &Path) -> CoverageCounts {
    let mut counts = CoverageCounts::default();
    walk_emlx_files(store_path, &mut counts);
    counts
}

fn walk_emlx_files(dir: &Path, counts: &mut CoverageCounts) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_dir() {
            walk_emlx_files(&path, counts);
            continue;
        }

        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if name.ends_with(".partial.emlx") {
            counts.partial += 1;
        } else if name.ends_with(".emlx") {
            match std::fs::read(&path) {
                Ok(bytes) => match amx_parse::parse_emlx(&path, &bytes) {
                    Ok(_) => counts.clean_parse += 1,
                    Err(_) => counts.quarantine += 1,
                },
                Err(_) => counts.quarantine += 1,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    const WELL_FORMED_EMLX: &[u8] =
        include_bytes!("../../amx-parse/tests/fixtures/emlx/well_formed.emlx");

    #[test]
    fn counts_account_directories_excluding_mail_data() {
        let store = TempDir::new().unwrap();
        fs::create_dir(store.path().join("MailData")).unwrap();
        fs::create_dir(store.path().join("ACCOUNT-ONE")).unwrap();
        fs::create_dir(store.path().join("ACCOUNT-TWO")).unwrap();

        assert_eq!(count_account_directories(store.path()), 2);
    }

    #[test]
    fn missing_store_path_reports_zero_accounts() {
        assert_eq!(count_account_directories(Path::new("/nonexistent")), 0);
    }

    #[test]
    fn coverage_counts_partial_files_without_parsing_them() {
        let store = TempDir::new().unwrap();
        let messages_dir = store.path().join("ACCOUNT/INBOX.mbox/GEN/Data/Messages");
        fs::create_dir_all(&messages_dir).unwrap();
        fs::write(messages_dir.join("1.partial.emlx"), b"not real emlx bytes").unwrap();

        let counts = count_coverage(store.path());
        assert_eq!(
            counts,
            CoverageCounts {
                clean_parse: 0,
                quarantine: 0,
                partial: 1,
            }
        );
    }

    #[test]
    fn coverage_distinguishes_clean_parse_from_quarantine() {
        let store = TempDir::new().unwrap();
        let messages_dir = store.path().join("ACCOUNT/INBOX.mbox/GEN/Data/Messages");
        fs::create_dir_all(&messages_dir).unwrap();

        fs::write(messages_dir.join("2.emlx"), WELL_FORMED_EMLX).unwrap();
        fs::write(messages_dir.join("3.emlx"), b"not a real emlx file at all").unwrap();

        let counts = count_coverage(store.path());
        assert_eq!(
            counts,
            CoverageCounts {
                clean_parse: 1,
                quarantine: 1,
                partial: 0,
            }
        );
    }

    #[test]
    fn gather_reports_zero_mailboxes_when_envelope_index_is_absent() {
        let store = TempDir::new().unwrap();
        let report = gather(store.path()).unwrap();
        assert_eq!(report.mailbox_count, 0);
        assert_eq!(report.account_count, 0);
        assert_eq!(
            report.store_version,
            store.path().file_name().unwrap().to_string_lossy()
        );
    }
}
