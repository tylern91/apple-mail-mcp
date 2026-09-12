//! Opens (or creates) the on-disk Tantivy index once per process and hands out `Searcher`
//! handles. Tantivy's own `IndexReader` is already lock-free and its `searcher()` call cheap —
//! this module's job is just making sure every caller shares one reader, opened with tokenizers
//! registered, instead of re-deriving that setup per call. This is the `Searcher` surface Phase 3
//! wraps; deliberately minimal, no query building here (sub-plan §3).

use std::path::Path;

use amx_core::AmxError;
use tantivy::directory::MmapDirectory;
use tantivy::{Index, IndexReader, ReloadPolicy, Searcher};

use crate::schema::{build_schema, register_tokenizers};

pub struct IndexReaderPool {
    reader: IndexReader,
}

impl IndexReaderPool {
    /// Opens the index at `dir`, creating it with a fresh schema if the directory holds none yet.
    pub fn open_or_create(dir: &Path) -> Result<Self, AmxError> {
        let (schema, _fields) = build_schema();
        let directory = MmapDirectory::open(dir).map_err(tantivy::TantivyError::from)?;
        let index = Index::open_or_create(directory, schema)?;
        register_tokenizers(&index);

        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::OnCommitWithDelay)
            .try_into()?;

        Ok(Self { reader })
    }

    /// A fresh `Searcher` handle over the reader's current view of the index.
    pub fn searcher(&self) -> Searcher {
        self.reader.searcher()
    }

    /// Blocks until the reader has picked up every commit made so far. Tests use this to avoid
    /// depending on `OnCommitWithDelay`'s background timing.
    pub fn reload(&self) -> Result<(), AmxError> {
        self.reader.reload()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tantivy::doc;

    #[test]
    fn opening_twice_shares_the_same_on_disk_index() {
        let dir = tempfile::tempdir().unwrap();

        let pool = IndexReaderPool::open_or_create(dir.path()).unwrap();
        assert_eq!(pool.searcher().num_docs(), 0);
        drop(pool);

        let pool = IndexReaderPool::open_or_create(dir.path()).unwrap();
        assert_eq!(pool.searcher().num_docs(), 0);
    }

    #[test]
    fn a_committed_document_becomes_visible_after_reload() {
        let dir = tempfile::tempdir().unwrap();
        let (schema, fields) = build_schema();

        {
            let directory = MmapDirectory::open(dir.path()).unwrap();
            let index = Index::open_or_create(directory, schema).unwrap();
            register_tokenizers(&index);
            let mut writer = index.writer(15_000_000).unwrap();
            writer.add_document(doc!(fields.rowid => 1i64)).unwrap();
            writer.commit().unwrap();
        }

        let pool = IndexReaderPool::open_or_create(dir.path()).unwrap();
        pool.reload().unwrap();
        assert_eq!(pool.searcher().num_docs(), 1);
    }
}
