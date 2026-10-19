//! Transient writer handle for a single mutate-tool call (umbrella §4.3.1's D2/D3 decisions):
//! acquire the advisory writer lock, upsert or remove exactly the documents one JXA mutation
//! touched, commit once, and release — never a long-lived writer held across tool calls, since
//! `amxcli serve` otherwise only holds an [`crate::reader::IndexReaderPool`] and re-introducing a
//! standing writer would re-break imdinu #122.

use std::path::Path;
use std::thread::sleep;
use std::time::{Duration, Instant};

use amx_core::AmxError;
use tantivy::directory::MmapDirectory;
use tantivy::{Index, Term};

use crate::classify::Classified;
use crate::document::build_document;
use crate::schema::{Fields, build_schema, register_tokenizers};
use crate::writer::{IndexWriter as WriterLock, IndexWriterGuard};

/// Total time [`MutationWriter::open`] spends retrying a busy writer lock before giving up (D2).
const ACQUIRE_BUDGET: Duration = Duration::from_secs(2);
const INITIAL_BACKOFF: Duration = Duration::from_millis(50);

pub struct MutationWriter {
    _lock: IndexWriterGuard,
    index: Index,
    writer: tantivy::IndexWriter,
    fields: Fields,
}

impl MutationWriter {
    /// Acquires the advisory writer lock (retrying with exponential backoff for up to
    /// [`ACQUIRE_BUDGET`] on contention) and opens the Tantivy writer. On sustained contention,
    /// returns [`AmxError::IndexWriterBusy`] naming the holding pid — the caller must not perform
    /// its JXA mutation in that case (D2: never a window where Mail.app changed but the index
    /// could not).
    pub fn open(index_dir: &Path, meta_path: &Path) -> Result<Self, AmxError> {
        let lock = Self::acquire_with_retry(meta_path)?;

        let (schema, fields) = build_schema();
        let directory = MmapDirectory::open(index_dir).map_err(tantivy::TantivyError::from)?;
        let index = Index::open_or_create(directory, schema)?;
        register_tokenizers(&index);
        let writer = index.writer(15_000_000)?;

        Ok(Self {
            _lock: lock,
            index,
            writer,
            fields,
        })
    }

    fn acquire_with_retry(meta_path: &Path) -> Result<IndexWriterGuard, AmxError> {
        let deadline = Instant::now() + ACQUIRE_BUDGET;
        let mut backoff = INITIAL_BACKOFF;
        loop {
            match WriterLock::acquire(meta_path) {
                Ok(guard) => return Ok(guard),
                Err(err @ AmxError::IndexWriterBusy { .. }) => {
                    if Instant::now() >= deadline {
                        return Err(err);
                    }
                    sleep(backoff.min(deadline.saturating_duration_since(Instant::now())));
                    backoff *= 2;
                }
                Err(other) => return Err(other),
            }
        }
    }

    /// Replaces (or inserts) the document for `rowid`, deleting any prior version first so the
    /// write is idempotent regardless of whether `rowid` was already indexed.
    pub fn upsert(
        &mut self,
        rowid: i64,
        account_identifier: &str,
        mailbox_url: &str,
        classified: &Classified,
    ) -> Result<(), AmxError> {
        self.writer
            .delete_term(Term::from_field_i64(self.fields.rowid, rowid));
        let doc = build_document(
            rowid,
            account_identifier,
            mailbox_url,
            classified,
            &self.fields,
        );
        self.writer.add_document(doc)?;
        Ok(())
    }

    /// Deletes the document for `rowid`, if any.
    pub fn remove(&mut self, rowid: i64) {
        self.writer
            .delete_term(Term::from_field_i64(self.fields.rowid, rowid));
    }

    /// Commits every `upsert`/`remove` since `open` and reloads the shared index's readers so a
    /// `search_messages` call issued immediately after this returns sees the change (imdinu #66).
    pub fn commit(mut self) -> Result<(), AmxError> {
        self.writer.commit()?;
        let reader = self.index.reader()?;
        reader.reload()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use amx_core::{AttachmentState, BodyState};
    use tantivy::collector::Count;
    use tantivy::query::TermQuery;
    use tantivy::schema::IndexRecordOption;

    fn empty_classified() -> Classified {
        Classified {
            body: BodyState::Indexed,
            attachments: AttachmentState::None,
            parsed: None,
        }
    }

    fn count_rowid(index_dir: &Path, rowid: i64) -> usize {
        let (schema, fields) = build_schema();
        let directory = MmapDirectory::open(index_dir).unwrap();
        let index = Index::open_or_create(directory, schema).unwrap();
        register_tokenizers(&index);
        let reader = index.reader().unwrap();
        let searcher = reader.searcher();
        let query = TermQuery::new(
            Term::from_field_i64(fields.rowid, rowid),
            IndexRecordOption::Basic,
        );
        searcher.search(&query, &Count).unwrap()
    }

    #[test]
    fn upsert_then_commit_makes_the_document_immediately_searchable() {
        let dir = tempfile::tempdir().unwrap();
        let index_dir = dir.path().join("index");
        std::fs::create_dir(&index_dir).unwrap();
        let meta_path = dir.path().join("meta.sqlite");

        let mut writer = MutationWriter::open(&index_dir, &meta_path).unwrap();
        writer
            .upsert(42, "acct-1", "imap://acct-1/INBOX", &empty_classified())
            .unwrap();
        writer.commit().unwrap();

        assert_eq!(count_rowid(&index_dir, 42), 1);
    }

    #[test]
    fn remove_then_commit_deletes_the_document() {
        let dir = tempfile::tempdir().unwrap();
        let index_dir = dir.path().join("index");
        std::fs::create_dir(&index_dir).unwrap();
        let meta_path = dir.path().join("meta.sqlite");

        let mut writer = MutationWriter::open(&index_dir, &meta_path).unwrap();
        writer
            .upsert(7, "acct-1", "imap://acct-1/INBOX", &empty_classified())
            .unwrap();
        writer.commit().unwrap();

        let mut writer = MutationWriter::open(&index_dir, &meta_path).unwrap();
        writer.remove(7);
        writer.commit().unwrap();

        assert_eq!(count_rowid(&index_dir, 7), 0);
    }

    #[test]
    fn a_lock_held_by_a_live_pid_gives_up_after_the_retry_budget() {
        let dir = tempfile::tempdir().unwrap();
        let index_dir = dir.path().join("index");
        std::fs::create_dir(&index_dir).unwrap();
        let meta_path = dir.path().join("meta.sqlite");

        // Seed the lock with our own (unimpeachably live) pid, simulating another writer.
        crate::meta::open(&meta_path)
            .unwrap()
            .execute(
                "INSERT INTO writer_lock (id, pid, process, since) VALUES (1, ?1, 'other', 0)",
                rusqlite::params![std::process::id()],
            )
            .unwrap();

        let started = Instant::now();
        let result = MutationWriter::open(&index_dir, &meta_path);
        assert!(matches!(result, Err(AmxError::IndexWriterBusy { .. })));
        assert!(started.elapsed() >= ACQUIRE_BUDGET);
    }
}
