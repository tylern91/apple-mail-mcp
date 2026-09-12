//! Resolves the on-disk `.emlx` path for a message, given the mailbox directory it lives under.
//!
//! Layout, proved against the live store rather than assumed (umbrella §4.1.1): `Data/` plus one
//! directory per digit of `floor(ROWID / 1000)`, read in reverse digit order, then
//! `Messages/<ROWID>[.partial].emlx`. `floor == 0` means no shard directories at all — the file
//! sits directly under `Data/Messages/`.

use std::path::{Path, PathBuf};

pub struct EmlxPathResolver;

impl EmlxPathResolver {
    /// `mailbox_dir` is the directory a mailbox's `Data/` lives directly under — e.g.
    /// `<AccountUUID>/INBOX.mbox/<GenerationUUID>` on a real store, or a bare temp dir in tests.
    /// `partial` selects the `.partial.emlx` suffix Mail uses for a message whose attachment
    /// bytes were never downloaded.
    pub fn resolve(mailbox_dir: &Path, rowid: i64, partial: bool) -> PathBuf {
        let shard = rowid / 1000;
        let mut path = mailbox_dir.join("Data");
        if shard > 0 {
            for digit in shard.to_string().chars().rev() {
                path.push(digit.to_string());
            }
        }
        path.push("Messages");
        let suffix = if partial { ".partial.emlx" } else { ".emlx" };
        path.push(format!("{rowid}{suffix}"));
        path
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn parse_matrix(text: &str) -> Vec<(i64, bool, String)> {
        text.lines()
            .filter(|line| !line.trim().is_empty() && !line.starts_with('#'))
            .map(|line| {
                let mut parts = line.split('|');
                let rowid: i64 = parts.next().unwrap().parse().unwrap();
                let partial: bool = parts.next().unwrap().parse().unwrap();
                let expected = parts.next().unwrap().to_string();
                (rowid, partial, expected)
            })
            .collect()
    }

    #[test]
    fn shard_matrix_resolves_exactly() {
        let matrix = include_str!("../tests/fixtures/paths/shard_matrix.txt");
        let mailbox_dir = Path::new("/store");
        for (rowid, partial, expected) in parse_matrix(matrix) {
            let resolved = EmlxPathResolver::resolve(mailbox_dir, rowid, partial);
            let expected_path = mailbox_dir.join(&expected);
            assert_eq!(resolved, expected_path, "rowid {rowid} partial {partial}");
        }
    }

    #[test]
    fn nested_inbox_path_composes_correctly() {
        let fixture = include_str!("../tests/fixtures/paths/nested_inbox.txt");
        let mut lines = fixture
            .lines()
            .filter(|line| !line.trim().is_empty() && !line.starts_with('#'));

        let mailbox_dir_line = lines.next().unwrap();
        let mailbox_dir = Path::new(mailbox_dir_line.split('|').nth(1).unwrap());

        for line in lines {
            let mut parts = line.split('|');
            let rowid: i64 = parts.next().unwrap().parse().unwrap();
            let partial: bool = parts.next().unwrap().parse().unwrap();
            let expected = parts.next().unwrap();

            let resolved = EmlxPathResolver::resolve(mailbox_dir, rowid, partial);
            assert_eq!(resolved, mailbox_dir.join(expected));
        }
    }

    #[test]
    fn zero_shard_has_no_shard_directories() {
        let resolved = EmlxPathResolver::resolve(Path::new("/store"), 172, false);
        assert_eq!(resolved, Path::new("/store/Data/Messages/172.emlx"));
    }
}
