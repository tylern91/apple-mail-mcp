//! Q6 (umbrella §11): proves Tantivy's cross-process reader reload — an `IndexReader` opened in
//! one process observes a commit made by an `IndexWriter` opened in a *different* process against
//! the same on-disk index. imdinu #122's single-writer-lock fix depends on this property, but
//! until now only the Tantivy docs vouched for it, not a test.
//!
//! The child is this same test binary, re-executed via `current_exe()` with an env marker rather
//! than a new `[[bin]]` — the marker routes the (sole) test function into the writer role instead
//! of the parent/assertion role.

use std::env;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use amx_index::reader::IndexReaderPool;
use amx_index::schema::{build_schema, register_tokenizers};
use tantivy::directory::MmapDirectory;
use tantivy::{Index, doc};

const CHILD_PATH_ENV: &str = "AMX_INDEX_TEST_CHILD_PATH";

#[test]
fn a_reader_opened_in_one_process_sees_a_commit_made_by_a_writer_in_another() {
    if let Ok(index_dir) = env::var(CHILD_PATH_ENV) {
        child_write_one_document(Path::new(&index_dir));
        return;
    }

    let dir = tempfile::tempdir().unwrap();

    let pool = IndexReaderPool::open_or_create(dir.path()).unwrap();
    assert_eq!(pool.searcher().num_docs(), 0);

    let status = Command::new(env::current_exe().unwrap())
        .env(CHILD_PATH_ENV, dir.path())
        .status()
        .expect("failed to spawn child process");
    assert!(status.success(), "child process failed: {status}");

    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        pool.reload().unwrap();
        if pool.searcher().num_docs() == 1 {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "parent reader never observed the child process's commit"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn child_write_one_document(index_dir: &Path) {
    let (schema, fields) = build_schema();
    let directory = MmapDirectory::open(index_dir).unwrap();
    let index = Index::open_or_create(directory, schema).unwrap();
    register_tokenizers(&index);
    let mut writer = index.writer(15_000_000).unwrap();
    writer.add_document(doc!(fields.rowid => 1i64)).unwrap();
    writer.commit().unwrap();
}
