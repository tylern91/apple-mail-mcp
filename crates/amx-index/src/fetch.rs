//! Reads a message's `.emlx` bytes from a computed path only (imdinu #84) — never a directory
//! scan. Full and partial paths are tried in that order; which one hit becomes part of the
//! result so `classify.rs` never needs to re-derive it.

use std::fs;
use std::path::Path;

use amx_store::paths::EmlxPathResolver;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchedBody {
    Full(Vec<u8>),
    Partial(Vec<u8>),
    Absent,
}

pub struct BodyFetcher;

impl BodyFetcher {
    /// `mailbox_dir` is the directory a mailbox's `Data/` lives directly under (see
    /// [`EmlxPathResolver::resolve`]). Falls back to the `.partial.emlx` path only when the full
    /// file is missing.
    pub fn fetch(mailbox_dir: &Path, rowid: i64) -> FetchedBody {
        let full_path = EmlxPathResolver::resolve(mailbox_dir, rowid, false);
        if let Ok(bytes) = fs::read(&full_path) {
            return FetchedBody::Full(bytes);
        }

        let partial_path = EmlxPathResolver::resolve(mailbox_dir, rowid, true);
        match fs::read(&partial_path) {
            Ok(bytes) => FetchedBody::Partial(bytes),
            Err(_) => FetchedBody::Absent,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn reads_the_full_emlx_when_present() {
        let dir = tempfile::tempdir().unwrap();
        let path = EmlxPathResolver::resolve(dir.path(), 42, false);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, b"full bytes").unwrap();

        assert_eq!(
            BodyFetcher::fetch(dir.path(), 42),
            FetchedBody::Full(b"full bytes".to_vec())
        );
    }

    #[test]
    fn falls_back_to_the_partial_emlx_when_the_full_file_is_absent() {
        let dir = tempfile::tempdir().unwrap();
        let path = EmlxPathResolver::resolve(dir.path(), 42, true);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, b"partial bytes").unwrap();

        assert_eq!(
            BodyFetcher::fetch(dir.path(), 42),
            FetchedBody::Partial(b"partial bytes".to_vec())
        );
    }

    #[test]
    fn absent_when_neither_full_nor_partial_exists() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(BodyFetcher::fetch(dir.path(), 42), FetchedBody::Absent);
    }
}
