//! `doctor` / `status` (Phase 3 task 8) — the MCP-native readiness screen. `account_count` and
//! FDA probing reuse `amx_store::readiness` (lifted there so `amxcli doctor` doesn't duplicate
//! it); `coverage` reuses `amx_index::coverage::CoverageCollector` over the whole corpus rather
//! than re-walking `.emlx` files on disk, since the Tantivy index already carries a
//! per-message `body_state`/`attachment_state` classification that `amxcli doctor`'s cruder
//! file-walk predates. `status` is `doctor`'s health-only subset, for a caller that just wants a
//! cheap access check without paying for account/mailbox/coverage counting.

use std::path::Path;

use amx_core::AmxError;
use amx_index::meta;
use amx_index::quarantine;
use amx_index::schema::Fields;
use amx_index::writer;
use amx_store::readiness::count_account_directories;
use amx_store::registry::MailboxRegistry;
use rusqlite::Connection;
use tantivy::Searcher;
use tantivy::query::AllQuery;

use crate::health_monitor::HealthMonitor;
use crate::schema::coverage::Coverage;
use crate::schema::health::HealthStatus;
use crate::schema::tools::{DoctorResponse, StatusResponse};

fn degraded_since(meta_conn: &Connection) -> Result<Option<String>, AmxError> {
    Ok(meta::last_health_transition(meta_conn)?.map(|(_, _, since)| since.to_string()))
}

pub fn run_status(
    health: &HealthMonitor,
    meta_conn: &Connection,
) -> Result<StatusResponse, AmxError> {
    let store_access = HealthStatus::from(&health.current());
    let degraded_since = degraded_since(meta_conn)?;
    Ok(StatusResponse {
        store_access,
        degraded_since,
    })
}

#[allow(clippy::too_many_arguments)]
pub fn run_doctor(
    health: &HealthMonitor,
    meta_conn: &Connection,
    store_path: &Path,
    mailbox_registry: &MailboxRegistry,
    searcher: &Searcher,
    fields: &Fields,
) -> Result<DoctorResponse, AmxError> {
    let store_access = HealthStatus::from(&health.current());
    let degraded_since = degraded_since(meta_conn)?;

    let account_count = count_account_directories(store_path);
    let mailbox_count = mailbox_registry.len();

    let coverage_envelope = searcher.search(
        &AllQuery,
        &amx_index::coverage::CoverageCollector::new(*fields),
    )?;
    let coverage = Coverage::from(coverage_envelope);

    let index_writer_busy = writer::busy_status(meta_conn)?;
    let quarantined_count = quarantine::count(meta_conn)?;

    Ok(DoctorResponse {
        store_access,
        degraded_since,
        account_count,
        mailbox_count,
        coverage,
        index_writer_busy,
        quarantined_count,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use amx_index::schema::{build_schema, register_tokenizers};
    use tantivy::directory::RamDirectory;
    use tantivy::doc;

    /// `HealthMonitor::start` uses the real `TccProbe` on macOS, so the path must actually be
    /// readable — a fake path like `/store` would probe `Denied` and defeat these tests, which
    /// are about `doctor`/`status`'s wiring, not the probe itself (already covered by
    /// `health_monitor.rs`'s own tests).
    fn health_monitor(meta_conn: &Connection) -> HealthMonitor {
        HealthMonitor::start(std::env::temp_dir(), meta_conn)
    }

    fn empty_mailbox_registry() -> MailboxRegistry {
        let file = tempfile::NamedTempFile::new().unwrap();
        let conn = Connection::open(file.path()).unwrap();
        amx_store::fixtures::build_schema(&conn).unwrap();
        let ro = amx_store::RoConnection::open(file.path()).unwrap();
        MailboxRegistry::load(&ro).unwrap()
    }

    fn empty_index() -> (tantivy::Index, Fields) {
        let (schema, fields) = build_schema();
        let index = tantivy::Index::create(RamDirectory::create(), schema, Default::default())
            .expect("in-memory index");
        register_tokenizers(&index);
        let mut writer = index.writer(15_000_000).expect("writer");
        writer
            .add_document(doc!(
                fields.rowid => 1i64,
                fields.body_state => 0u64,
                fields.attachment_state => 0u64,
            ))
            .unwrap();
        writer.commit().unwrap();
        (index, fields)
    }

    #[test]
    fn status_reports_the_monitors_current_state_and_degraded_since() {
        let meta_file = tempfile::NamedTempFile::new().unwrap();
        let meta_conn = meta::open(meta_file.path()).unwrap();
        let health = health_monitor(&meta_conn);

        let response = run_status(&health, &meta_conn).unwrap();
        assert_eq!(response.store_access, HealthStatus::Granted);
        assert!(response.degraded_since.is_some());
    }

    #[test]
    fn doctor_reports_accounts_mailboxes_coverage_and_quarantine() {
        let meta_file = tempfile::NamedTempFile::new().unwrap();
        let meta_conn = meta::open(meta_file.path()).unwrap();
        let health = health_monitor(&meta_conn);

        let store = tempfile::tempdir().unwrap();
        std::fs::create_dir(store.path().join("MailData")).unwrap();
        std::fs::create_dir(store.path().join("ACCOUNT-ONE")).unwrap();

        let (index, fields) = empty_index();
        let searcher = index.reader().unwrap().searcher();
        let registry = empty_mailbox_registry();

        let response = run_doctor(
            &health,
            &meta_conn,
            store.path(),
            &registry,
            &searcher,
            &fields,
        )
        .unwrap();

        assert_eq!(response.account_count, 1);
        assert_eq!(response.mailbox_count, 0);
        assert_eq!(response.coverage.corpus_messages, 1);
        assert_eq!(response.quarantined_count, 0);
        assert_eq!(response.index_writer_busy, None);
    }

    #[test]
    fn doctor_surfaces_a_live_writer_lock() {
        let meta_file = tempfile::NamedTempFile::new().unwrap();
        let meta_conn = meta::open(meta_file.path()).unwrap();
        meta_conn
            .execute(
                "INSERT INTO writer_lock (id, pid, process, since) VALUES (1, ?1, 'amxcli', 0)",
                rusqlite::params![std::process::id()],
            )
            .unwrap();
        let health = health_monitor(&meta_conn);

        let store = tempfile::tempdir().unwrap();
        let (index, fields) = empty_index();
        let searcher = index.reader().unwrap().searcher();
        let registry = empty_mailbox_registry();

        let response = run_doctor(
            &health,
            &meta_conn,
            store.path(),
            &registry,
            &searcher,
            &fields,
        )
        .unwrap();

        assert_eq!(
            response.index_writer_busy,
            Some(format!("amxcli ({})", std::process::id()))
        );
    }
}
