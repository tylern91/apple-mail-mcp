//! Ties `SyncPlanner` → classification → a Tantivy write → `Reconciler` into one sync pass
//! (umbrella §4.2.2). Every call to [`SyncEngine::run`] is "process start" for decision E7 — a
//! full reconcile always runs alongside whatever incremental work the planner finds, since a
//! one-shot CLI invocation has no long-lived timer to fall back on.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use amx_core::{AccountKind, AmxError, BodyState};
use amx_store::account::AccountResolver;
use amx_store::conn::RoConnection;
use amx_store::mailbox_path::MailboxPathResolver;
use amx_store::registry::{Mailbox, MailboxRegistry};
use rusqlite::Connection as MetaConnection;
use tantivy::directory::MmapDirectory;
use tantivy::{Index, Term};

use crate::classify::AvailabilityClassifier;
use crate::document::build_document;
use crate::fetch::{BodyFetcher, resolved_emlx_path};
use crate::meta;
use crate::planner::{SyncPlanner, WorkItem, WorkKind};
use crate::quarantine;
use crate::reconcile::Reconciler;
use crate::schema::{Fields, build_schema, register_tokenizers};
use crate::writer::IndexWriter as WriterLock;

/// What one [`SyncEngine::run`] pass did — surfaced by `amxcli index`/`amxcli sync` for plain
/// reporting and, via `--profile`, task 11's Q3 measurement.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
pub struct SyncReport {
    pub added: usize,
    pub modified: usize,
    pub quarantined: usize,
    pub reconciled_deletes: usize,
}

pub struct SyncEngine;

impl SyncEngine {
    /// Runs one full sync pass: acquires the writer lock, plans work against the Envelope Index
    /// at `store_conn`, classifies and indexes every work item, reconciles deletions against a
    /// full snapshot, and persists the resulting `SyncState`.
    ///
    /// `store_root` is the `~/Library/Mail/V10`-shaped directory mailbox paths resolve against;
    /// `accounts_db` is the `Accounts4.sqlite` sibling ([`AccountResolver`]); `index_dir`/
    /// `meta_path` are `amx-index`'s own on-disk state.
    pub fn run(
        store_conn: &RoConnection,
        store_root: &Path,
        accounts_db: &Path,
        index_dir: &Path,
        meta_path: &Path,
    ) -> Result<SyncReport, AmxError> {
        let _lock = WriterLock::acquire(meta_path)?;
        let meta_conn = meta::open(meta_path)?;

        let registry = MailboxRegistry::load(store_conn)?;
        let mailboxes_by_rowid: HashMap<i64, &Mailbox> = registry
            .iter()
            .map(|mailbox| (mailbox.rowid, mailbox))
            .collect();
        let account_resolver = AccountResolver::open(accounts_db)?;

        let state = meta::load_sync_state(&meta_conn)?;
        let work = SyncPlanner::plan(store_conn, &state)?;

        let (schema, fields) = build_schema();
        let directory = MmapDirectory::open(index_dir).map_err(tantivy::TantivyError::from)?;
        let index = Index::open_or_create(directory, schema)?;
        register_tokenizers(&index);
        let mut writer = index.writer(15_000_000)?;

        let mut report = SyncReport::default();
        let mut mailbox_dirs: HashMap<i64, PathBuf> = HashMap::new();
        let mut account_kinds: HashMap<String, Option<AccountKind>> = HashMap::new();

        for item in &work.items {
            Self::index_one(
                item,
                &mailboxes_by_rowid,
                store_root,
                &account_resolver,
                &mut mailbox_dirs,
                &mut account_kinds,
                &meta_conn,
                &mut writer,
                &fields,
                &mut report,
            )?;
        }
        writer.commit()?;

        let reader = index.reader()?;
        reader.reload()?;
        let searcher = reader.searcher();
        report.reconciled_deletes =
            Reconciler::reconcile(store_conn, &searcher, &mut writer, &fields)?;

        meta::save_sync_state(&meta_conn, &work.next_state)?;

        Ok(report)
    }

    /// Indexes a single work item, or quarantines it and returns without indexing when its
    /// mailbox directory or owning account cannot be resolved — a resolution failure for one
    /// item is not grounds to abort the whole pass (imdinu #125's per-file isolation, applied one
    /// layer up from parsing).
    #[allow(clippy::too_many_arguments)]
    fn index_one(
        item: &WorkItem,
        mailboxes_by_rowid: &HashMap<i64, &Mailbox>,
        store_root: &Path,
        account_resolver: &AccountResolver,
        mailbox_dirs: &mut HashMap<i64, PathBuf>,
        account_kinds: &mut HashMap<String, Option<AccountKind>>,
        meta_conn: &MetaConnection,
        writer: &mut tantivy::IndexWriter,
        fields: &Fields,
        report: &mut SyncReport,
    ) -> Result<(), AmxError> {
        let Some(mailbox) = mailboxes_by_rowid.get(&item.mailbox) else {
            // The mailbox vanished between planning and indexing; the next reconcile pass drops
            // any stale document for it, so there is nothing more to do here.
            return Ok(());
        };

        let mailbox_dir = match mailbox_dirs.get(&item.mailbox) {
            Some(dir) => dir.clone(),
            None => match MailboxPathResolver::resolve(store_root, &mailbox.url) {
                Ok(dir) => {
                    mailbox_dirs.insert(item.mailbox, dir.clone());
                    dir
                }
                Err(_) => {
                    Self::quarantine_unresolvable(meta_conn, item.rowid, &mailbox.url, report)?;
                    return Ok(());
                }
            },
        };

        let account_identifier = match MailboxPathResolver::account_identifier(&mailbox.url) {
            Ok(identifier) => identifier,
            Err(_) => {
                Self::quarantine_unresolvable(meta_conn, item.rowid, &mailbox.url, report)?;
                return Ok(());
            }
        };
        let account_kind = match account_kinds.get(&account_identifier) {
            Some(kind) => *kind,
            None => {
                let kind = account_resolver
                    .resolve(&account_identifier)?
                    .and_then(|account| account.kind);
                account_kinds.insert(account_identifier.clone(), kind);
                kind
            }
        };

        let fetched = BodyFetcher::fetch(&mailbox_dir, item.rowid);
        let emlx_path = resolved_emlx_path(&mailbox_dir, item.rowid, &fetched);
        let classified = AvailabilityClassifier::classify(&emlx_path, fetched, account_kind);

        if let BodyState::Quarantined { error } = classified.body {
            quarantine::record(meta_conn, item.rowid, &emlx_path, error)?;
            report.quarantined += 1;
        }

        // A `Modified` item may already hold a document from a prior pass; deleting first makes
        // every write idempotent regardless of `item.kind`.
        writer.delete_term(Term::from_field_i64(fields.rowid, item.rowid));
        let doc = build_document(
            item.rowid,
            &account_identifier,
            &mailbox.url,
            &classified,
            fields,
        );
        writer.add_document(doc)?;

        match item.kind {
            WorkKind::Added => report.added += 1,
            WorkKind::Modified => report.modified += 1,
        }
        Ok(())
    }

    fn quarantine_unresolvable(
        meta_conn: &MetaConnection,
        rowid: i64,
        mailbox_url: &str,
        report: &mut SyncReport,
    ) -> Result<(), AmxError> {
        quarantine::record(
            meta_conn,
            rowid,
            Path::new(mailbox_url),
            amx_core::ParseErrorKind::Io,
        )?;
        report.quarantined += 1;
        Ok(())
    }
}
