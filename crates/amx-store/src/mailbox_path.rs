//! Resolves a mailbox's on-disk `.mbox/<GenerationUUID>` directory from its Envelope Index URL.
//!
//! Mailbox identity ends at `mailboxes.url` (`registry.rs`, `paths.rs`) — nothing else in the
//! store maps that identifier to a filesystem location, and the GenerationUUID segment is not
//! derivable from the URL or ROWID: Apple Mail assigns it once per mailbox and it must be
//! discovered by listing the mailbox's `.mbox` directory. That listing runs once per mailbox
//! (callers should cache the result), never per message — imdinu #84's ban on `readdir` targets
//! the hot per-message path, not a one-time per-mailbox lookup.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use amx_core::AmxError;
use percent_encoding::percent_decode_str;
use unicode_normalization::UnicodeNormalization;

pub struct MailboxPathResolver;

impl MailboxPathResolver {
    /// `store_root` is a `~/Library/Mail/V10`-shaped directory. `mailbox_url` is a
    /// `mailboxes.url` value, e.g. `imap://<AccountUUID>/[Gmail]/Tất cả thư`.
    pub fn resolve(store_root: &Path, mailbox_url: &str) -> Result<PathBuf, AmxError> {
        let (account, segments) = split_url(mailbox_url)?;

        let mut mbox_dir = store_root.join(account);
        for segment in segments {
            mbox_dir.push(format!("{segment}.mbox"));
        }

        let generation = find_generation_dir(&mbox_dir, mailbox_url)?;
        Ok(mbox_dir.join(generation))
    }
}

/// Splits `scheme://account/seg1/seg2` into `(account, [seg1, seg2])`, percent-decoding and
/// NFD-normalizing each path segment to match the disk encoding (imdinu #120). This is distinct
/// from `registry.rs::normalize`'s NFC form, which exists only for identity comparison, never
/// for a filesystem path.
fn split_url(url: &str) -> Result<(String, Vec<String>), AmxError> {
    let after_scheme = url.split_once("://").map(|(_, rest)| rest).ok_or_else(|| {
        AmxError::MailboxUrlMalformed {
            url: url.to_string(),
        }
    })?;

    let mut parts = after_scheme.split('/');
    let account = parts
        .next()
        .filter(|segment| !segment.is_empty())
        .ok_or_else(|| AmxError::MailboxUrlMalformed {
            url: url.to_string(),
        })?
        .to_string();

    let segments = parts
        .filter(|segment| !segment.is_empty())
        .map(decode_segment)
        .collect();
    Ok((account, segments))
}

fn decode_segment(segment: &str) -> String {
    percent_decode_str(segment)
        .decode_utf8_lossy()
        .nfd()
        .collect()
}

/// Lists `mbox_dir` once for its single GenerationUUID subdirectory. A `.mbox` folder with no
/// such subdirectory (observed live: an `INBOX.mbox` holding only `Info.plist`, a mailbox never
/// synced past account setup) or more than one is a store shape this resolver does not
/// understand well enough to guess at — surfaced as an error rather than picking one silently.
fn find_generation_dir(mbox_dir: &Path, url: &str) -> Result<OsString, AmxError> {
    let unresolvable = |reason: &str| AmxError::MailboxPathUnresolvable {
        url: url.to_string(),
        path: mbox_dir.to_path_buf(),
        reason: reason.to_string(),
    };

    let entries =
        fs::read_dir(mbox_dir).map_err(|_| unresolvable("mailbox directory not found"))?;
    let mut generations = entries
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false))
        .map(|entry| entry.file_name());

    let first = generations
        .next()
        .ok_or_else(|| unresolvable("no generation subdirectory present"))?;
    if generations.next().is_some() {
        return Err(unresolvable(
            "more than one generation subdirectory present",
        ));
    }
    Ok(first)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_generation_dir(
        store_root: &Path,
        account: &str,
        mbox_chain: &[&str],
        generation: &str,
    ) -> PathBuf {
        let mut dir = store_root.join(account);
        for segment in mbox_chain {
            dir.push(format!("{segment}.mbox"));
        }
        dir.push(generation);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn resolves_a_single_level_mailbox() {
        let store_root = tempfile::tempdir().unwrap();
        let expected = make_generation_dir(
            store_root.path(),
            "AB4FC904-21FE-4AC0-A089-716246CE5C46",
            &["INBOX"],
            "F645E2FB-05D2-4A20-86B7-33FB1C8BFD18",
        );

        let resolved = MailboxPathResolver::resolve(
            store_root.path(),
            "imap://AB4FC904-21FE-4AC0-A089-716246CE5C46/INBOX",
        )
        .unwrap();
        assert_eq!(resolved, expected);
    }

    #[test]
    fn resolves_a_nested_mailbox_with_nfd_normalized_percent_decoded_segments() {
        let store_root = tempfile::tempdir().unwrap();
        // Disk stores Vietnamese folder names NFD-encoded (imdinu #120).
        let nfd_segment: String = "[Gmail]/Tất cả thư".nfd().collect::<String>();
        let (bracket_seg, vietnamese_seg) = nfd_segment.split_once('/').unwrap();
        let expected = make_generation_dir(
            store_root.path(),
            "AB4FC904-21FE-4AC0-A089-716246CE5C46",
            &[bracket_seg, vietnamese_seg],
            "F645E2FB-05D2-4A20-86B7-33FB1C8BFD18",
        );

        let nfc_url = "imap://AB4FC904-21FE-4AC0-A089-716246CE5C46/%5BGmail%5D/T\u{1EA5}t c\u{1EA3} th\u{01B0}";
        let resolved = MailboxPathResolver::resolve(store_root.path(), nfc_url).unwrap();
        assert_eq!(resolved, expected);
    }

    #[test]
    fn a_malformed_url_is_rejected() {
        let store_root = tempfile::tempdir().unwrap();
        let err = MailboxPathResolver::resolve(store_root.path(), "not-a-url").unwrap_err();
        assert!(matches!(err, AmxError::MailboxUrlMalformed { .. }));
    }

    #[test]
    fn a_mbox_directory_with_no_generation_subdirectory_is_unresolvable() {
        let store_root = tempfile::tempdir().unwrap();
        let mbox_dir = store_root.path().join("ACCT").join("INBOX.mbox");
        fs::create_dir_all(&mbox_dir).unwrap();
        fs::write(mbox_dir.join("Info.plist"), b"x").unwrap();

        let err = MailboxPathResolver::resolve(store_root.path(), "imap://ACCT/INBOX").unwrap_err();
        assert!(matches!(err, AmxError::MailboxPathUnresolvable { .. }));
    }

    #[test]
    fn a_mbox_directory_with_two_generation_subdirectories_is_ambiguous() {
        let store_root = tempfile::tempdir().unwrap();
        let mbox_dir = store_root.path().join("ACCT").join("INBOX.mbox");
        fs::create_dir_all(mbox_dir.join("GEN-A")).unwrap();
        fs::create_dir_all(mbox_dir.join("GEN-B")).unwrap();

        let err = MailboxPathResolver::resolve(store_root.path(), "imap://ACCT/INBOX").unwrap_err();
        assert!(matches!(err, AmxError::MailboxPathUnresolvable { .. }));
    }

    #[test]
    fn a_missing_mbox_directory_is_unresolvable() {
        let store_root = tempfile::tempdir().unwrap();
        let err = MailboxPathResolver::resolve(store_root.path(), "imap://ACCT/NoSuchMailbox")
            .unwrap_err();
        assert!(matches!(err, AmxError::MailboxPathUnresolvable { .. }));
    }
}
