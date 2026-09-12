//! Resolves Apple Mail mailbox URLs against the Envelope Index's `mailboxes` table.
//!
//! Mailbox identity is `mailboxes.url` (`UNIQUE`) — never a filesystem path (imdinu #105). A URL
//! round-trips through percent-encoding and Unicode normalization differently depending on where
//! it was typed or copied from, so lookups normalize percent-decode -> NFC -> casefold before
//! comparing (umbrella §4.1.2): NFC alone is not enough — this store's real Vietnamese IMAP
//! folder names (e.g. `[Gmail]/Tất cả thư`) are stored NFD-encoded on disk (imdinu #120).

use std::collections::HashMap;

use amx_core::AmxError;
use percent_encoding::percent_decode_str;
use unicode_normalization::UnicodeNormalization;

use crate::conn::RoConnection;

/// The counters Phase 2's sync planner reads instead of `change_identifier`, which is NULL on
/// every mailbox in the live store.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MailboxCounts {
    pub total: i64,
    pub unread: i64,
    pub deleted: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mailbox {
    pub rowid: i64,
    pub url: String,
    pub counts: MailboxCounts,
}

pub struct MailboxRegistry {
    by_normalized_url: HashMap<String, Mailbox>,
}

impl MailboxRegistry {
    /// Loads every row of `mailboxes` from `conn` and indexes it by normalized URL.
    pub fn load(conn: &RoConnection) -> Result<Self, AmxError> {
        let mut stmt = conn.as_connection().prepare(
            "SELECT ROWID, url, total_count, unread_count, deleted_count FROM mailboxes",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(Mailbox {
                rowid: row.get(0)?,
                url: row.get(1)?,
                counts: MailboxCounts {
                    total: row.get(2)?,
                    unread: row.get(3)?,
                    deleted: row.get(4)?,
                },
            })
        })?;

        let mut by_normalized_url = HashMap::new();
        for row in rows {
            let mailbox = row?;
            by_normalized_url.insert(normalize(&mailbox.url), mailbox);
        }
        Ok(Self { by_normalized_url })
    }

    /// Resolves `url` against the registry — a caller may pass a percent-encoded, NFD- or
    /// NFC-normalized, or differently-cased URL and still match the same mailbox.
    pub fn resolve(&self, url: &str) -> Option<&Mailbox> {
        self.by_normalized_url.get(&normalize(url))
    }

    pub fn len(&self) -> usize {
        self.by_normalized_url.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_normalized_url.is_empty()
    }
}

/// percent-decode -> NFC -> casefold, in that order — the order the umbrella spec requires so
/// NFC normalization runs on decoded text, not on percent-escape sequences.
fn normalize(url: &str) -> String {
    let decoded = percent_decode_str(url).decode_utf8_lossy();
    decoded.nfc().collect::<String>().to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::{Connection, params};
    use serde::Deserialize;

    #[derive(Deserialize)]
    struct FixtureRow {
        url: String,
        total_count: i64,
        unread_count: i64,
        deleted_count: i64,
    }

    const URLS_ENCODED_NFD: &str = include_str!("../tests/fixtures/mailbox/urls_encoded_nfd.json");

    fn seed_store(path: &std::path::Path) {
        let conn = Connection::open(path).unwrap();
        crate::fixtures::build_schema(&conn).unwrap();
        let fixture: Vec<FixtureRow> = serde_json::from_str(URLS_ENCODED_NFD).unwrap();
        for row in fixture {
            conn.execute(
                "INSERT INTO mailboxes (url, total_count, unread_count, deleted_count) \
                 VALUES (?1, ?2, ?3, ?4)",
                params![
                    row.url,
                    row.total_count,
                    row.unread_count,
                    row.deleted_count
                ],
            )
            .unwrap();
        }
    }

    #[test]
    fn resolves_nfc_and_nfd_query_to_the_same_nfd_stored_mailbox() {
        let file = tempfile::NamedTempFile::new().unwrap();
        seed_store(file.path());

        let conn = RoConnection::open(file.path()).unwrap();
        let registry = MailboxRegistry::load(&conn).unwrap();
        assert_eq!(registry.len(), 3);

        let nfc_query = "imap://AB4FC904-21FE-4AC0-A089-716246CE5C46/[Gmail]/Tất cả thư";
        let found_nfc = registry
            .resolve(nfc_query)
            .expect("NFC-form query should resolve");
        assert_eq!(
            found_nfc.counts,
            MailboxCounts {
                total: 22036,
                unread: 6202,
                deleted: 0
            }
        );

        let nfd_query: String = nfc_query.nfd().collect();
        let found_nfd = registry
            .resolve(&nfd_query)
            .expect("NFD-form query should resolve");
        assert_eq!(found_nfd.rowid, found_nfc.rowid);
    }

    #[test]
    fn resolve_is_case_insensitive() {
        let file = tempfile::NamedTempFile::new().unwrap();
        seed_store(file.path());

        let conn = RoConnection::open(file.path()).unwrap();
        let registry = MailboxRegistry::load(&conn).unwrap();

        let upper_query = "IMAP://AB4FC904-21FE-4AC0-A089-716246CE5C46/[GMAIL]/QUAN TRỌNG";
        assert!(registry.resolve(upper_query).is_some());
    }

    #[test]
    fn unresolved_url_returns_none() {
        let file = tempfile::NamedTempFile::new().unwrap();
        seed_store(file.path());

        let conn = RoConnection::open(file.path()).unwrap();
        let registry = MailboxRegistry::load(&conn).unwrap();
        assert_eq!(registry.resolve("imap://nonexistent/INBOX"), None);
    }
}
