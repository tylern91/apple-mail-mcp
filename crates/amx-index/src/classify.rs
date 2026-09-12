//! Classifies a message's body availability once its `.emlx` bytes have been fetched
//! (`fetch.rs`). `AttachmentState` comes straight from `amx-parse`'s completeness oracle —
//! this module only ever decides [`BodyState`], the one classification `amx-parse` cannot make
//! on its own (it has no notion of the owning account's remote protocol).

use std::path::Path;

use amx_core::{AccountKind, AttachmentState, BodyState, ParseErrorKind, UnavailableReason};
use amx_parse::{ParsedMessage, parse_emlx};

use crate::fetch::FetchedBody;

#[derive(Debug, Clone, PartialEq)]
pub struct Classified {
    pub body: BodyState,
    pub attachments: AttachmentState,
    /// `Some` only when the `.emlx` bytes were present and parsed successfully — every other
    /// path has nothing further to index.
    pub parsed: Option<ParsedMessage>,
}

pub struct AvailabilityClassifier;

impl AvailabilityClassifier {
    /// `account_kind` is the owning account's [`AccountKind`] — `None` for a local "On My Mac"
    /// account, which [`UnavailableReason::NotCachedLocally`] never applies to (see
    /// [`amx_store::account::ResolvedAccount::kind`]'s doc comment). A missing `.emlx` under a
    /// local account is an anomaly, not an expected pruning outcome, so it is quarantined for
    /// investigation rather than mapped to a state that misrepresents the cause.
    pub fn classify(
        path: &Path,
        fetched: FetchedBody,
        account_kind: Option<AccountKind>,
    ) -> Classified {
        let bytes = match fetched {
            FetchedBody::Full(bytes) | FetchedBody::Partial(bytes) => bytes,
            FetchedBody::Absent => {
                let body = match account_kind {
                    Some(account_kind) => {
                        BodyState::Unavailable(UnavailableReason::NotCachedLocally { account_kind })
                    }
                    None => BodyState::Quarantined {
                        error: ParseErrorKind::Io,
                    },
                };
                return Classified {
                    body,
                    attachments: AttachmentState::None,
                    parsed: None,
                };
            }
        };

        match parse_emlx(path, &bytes) {
            Ok(parsed) => Classified {
                body: BodyState::Indexed,
                attachments: parsed.attachments.clone(),
                parsed: Some(parsed),
            },
            Err(err) => Classified {
                body: BodyState::Quarantined {
                    error: ParseErrorKind::from(&err),
                },
                attachments: AttachmentState::None,
                parsed: None,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> Vec<u8> {
        let path = format!(
            "{}/../amx-parse/tests/fixtures/emlx/{name}",
            env!("CARGO_MANIFEST_DIR")
        );
        std::fs::read(path).unwrap()
    }

    #[test]
    fn a_partial_download_is_indexed_with_attachments_not_downloaded() {
        let bytes = fixture("partial_no_attachment.emlx");
        let classified = AvailabilityClassifier::classify(
            Path::new("partial_no_attachment.emlx"),
            FetchedBody::Partial(bytes),
            Some(AccountKind::Imap),
        );

        assert_eq!(classified.body, BodyState::Indexed);
        assert!(matches!(
            classified.attachments,
            AttachmentState::NotDownloaded { .. }
        ));
    }

    #[test]
    fn an_absent_emlx_under_a_tracked_account_is_unavailable_and_not_retryable() {
        let classified = AvailabilityClassifier::classify(
            Path::new("missing.emlx"),
            FetchedBody::Absent,
            Some(AccountKind::Exchange),
        );

        assert_eq!(
            classified.body,
            BodyState::Unavailable(UnavailableReason::NotCachedLocally {
                account_kind: AccountKind::Exchange
            })
        );
        assert!(!classified.body.is_retryable());
    }

    /// Reproduces task 4's `exchange_pruned.sql` scenario end to end: a message row that
    /// survives in the Envelope Index after Exchange prunes its `.emlx` from disk classifies as
    /// terminal `Unavailable`, never as a retryable state.
    #[test]
    fn a_pruned_exchange_message_is_unavailable_and_not_retryable() {
        use amx_store::conn::RoConnection;
        use amx_store::fixtures::build_schema;
        use amx_store::registry::MailboxRegistry;
        use rusqlite::Connection;

        const EXCHANGE_PRUNED_SQL: &str =
            include_str!("../../amx-store/tests/fixtures/store/exchange_pruned.sql");

        let file = tempfile::NamedTempFile::new().unwrap();
        let seed_conn = Connection::open(file.path()).unwrap();
        build_schema(&seed_conn).unwrap();
        seed_conn.execute_batch(EXCHANGE_PRUNED_SQL).unwrap();
        drop(seed_conn);

        let conn = RoConnection::open(file.path()).unwrap();
        let registry = MailboxRegistry::load(&conn).unwrap();
        let mailbox = registry
            .resolve("exchange://work@example.com/INBOX")
            .unwrap();
        assert_eq!(mailbox.counts.total, 50);

        let dir = tempfile::tempdir().unwrap();
        let fetched = crate::fetch::BodyFetcher::fetch(dir.path(), 1);
        assert_eq!(fetched, FetchedBody::Absent);

        let classified = AvailabilityClassifier::classify(
            Path::new("pruned.emlx"),
            fetched,
            Some(AccountKind::Exchange),
        );

        assert_eq!(
            classified.body,
            BodyState::Unavailable(UnavailableReason::NotCachedLocally {
                account_kind: AccountKind::Exchange
            })
        );
        assert!(!classified.body.is_retryable());
    }

    #[test]
    fn a_well_formed_message_is_indexed() {
        let bytes = fixture("well_formed.emlx");
        let classified = AvailabilityClassifier::classify(
            Path::new("well_formed.emlx"),
            FetchedBody::Full(bytes),
            Some(AccountKind::Imap),
        );

        assert_eq!(classified.body, BodyState::Indexed);
        assert!(classified.parsed.is_some());
    }
}
