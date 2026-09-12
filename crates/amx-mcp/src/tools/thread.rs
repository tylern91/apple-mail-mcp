//! `get_thread` / `get_message_links` (Phase 3 task 6).
//!
//! `thread_id` lives only in the Tantivy index — populated from the parsed `.emlx` footer's
//! conversation id in `sync.rs`'s `build_document` — so `get_thread` is the one Task 6 tool that
//! reads the index rather than the envelope index or `Accounts4.sqlite`. It has no notion of a
//! "resolved" thread id (unlike a mailbox/account filter, an arbitrary `i64` can't be fuzzy-
//! matched), so an unknown id legitimately returns an empty `messages` list, not an error.

use amx_core::{AmxError, BodyState};
use amx_index::schema::Fields;
use amx_index::search::Search;
use tantivy::Searcher;

use super::pipeline::ResolvedMessage;
use crate::schema::tools::{GetMessageLinksResponse, GetThreadResponse, ThreadMessageSummary};

pub fn run_get_thread(
    searcher: &Searcher,
    fields: &Fields,
    thread_id: i64,
) -> Result<GetThreadResponse, AmxError> {
    let hits = Search::by_thread(searcher, fields, thread_id)?;

    let messages = hits
        .into_iter()
        .map(|hit| ThreadMessageSummary {
            rowid: hit.rowid,
            subject: Some(hit.subject).filter(|s| !s.is_empty()),
            sender: Some(hit.sender).filter(|s| !s.is_empty()),
            date_sent: hit.date_sent,
        })
        .collect();

    Ok(GetThreadResponse {
        thread_id,
        messages,
    })
}

/// `Message-ID`/`In-Reply-To`/`References` come from the parsed `.emlx`, exactly the same
/// availability rule as `get_message`: unavailable is an error, quarantined/pending render as
/// empty rather than erroring on every read.
pub fn run_get_message_links(
    resolved: &ResolvedMessage,
) -> Result<GetMessageLinksResponse, AmxError> {
    match &resolved.classified.body {
        BodyState::Unavailable(reason) => Err(AmxError::BodyUnavailable {
            rowid: resolved.row.rowid,
            reason: reason.clone(),
        }),
        BodyState::Quarantined { .. } | BodyState::Pending { .. } => Ok(GetMessageLinksResponse {
            message_id: None,
            in_reply_to: None,
            references: Vec::new(),
        }),
        BodyState::Indexed => {
            let parsed = &resolved.classified.parsed;
            Ok(GetMessageLinksResponse {
                message_id: parsed.as_ref().and_then(|p| p.message_id.clone()),
                in_reply_to: parsed.as_ref().and_then(|p| p.in_reply_to.clone()),
                references: parsed
                    .as_ref()
                    .map(|p| p.references.clone())
                    .unwrap_or_default(),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use amx_core::{AttachmentState, ParseErrorKind, UnavailableReason};
    use amx_index::classify::Classified;
    use amx_index::schema::{build_schema, register_tokenizers};
    use amx_parse::ParsedMessage;
    use amx_store::query::MessageRow;
    use tantivy::directory::RamDirectory;
    use tantivy::{DateTime, doc};

    fn row() -> MessageRow {
        MessageRow {
            rowid: 1,
            mailbox: 1,
            date_sent: Some(100),
            date_received: Some(101),
            read: true,
            flagged: false,
            deleted: false,
        }
    }

    fn resolved_with(classified: Classified) -> ResolvedMessage {
        ResolvedMessage {
            row: row(),
            mailbox_url: "imap://ACCOUNT/INBOX".to_string(),
            account_id: "ACCOUNT".to_string(),
            classified,
        }
    }

    fn indexed_index() -> (tantivy::Index, Fields) {
        let (schema, fields) = build_schema();
        let index = tantivy::Index::create(RamDirectory::create(), schema, Default::default())
            .expect("in-memory index");
        register_tokenizers(&index);

        let mut writer = index.writer(15_000_000).expect("writer");
        writer
            .add_document(doc!(
                fields.rowid => 1i64,
                fields.account_id => "acct",
                fields.mailbox_key => "inbox",
                fields.subject => "original",
                fields.sender => "Alice <alice@example.com>",
                fields.body => "start",
                fields.date_sent => DateTime::from_timestamp_secs(1_700_000_000),
                fields.thread_id => 7i64,
                fields.body_state => 0u64,
                fields.attachment_state => 0u64,
            ))
            .unwrap();
        writer.commit().unwrap();
        (index, fields)
    }

    #[test]
    fn get_thread_returns_the_thread_id_alongside_its_messages() {
        let (index, fields) = indexed_index();
        let searcher = index.reader().unwrap().searcher();

        let response = run_get_thread(&searcher, &fields, 7).unwrap();
        assert_eq!(response.thread_id, 7);
        assert_eq!(response.messages.len(), 1);
        assert_eq!(response.messages[0].rowid, 1);
    }

    #[test]
    fn get_thread_with_no_matches_is_empty_not_an_error() {
        let (index, fields) = indexed_index();
        let searcher = index.reader().unwrap().searcher();

        let response = run_get_thread(&searcher, &fields, 999).unwrap();
        assert!(response.messages.is_empty());
    }

    #[test]
    fn get_message_links_unavailable_body_is_an_error() {
        let msg = resolved_with(Classified {
            body: BodyState::Unavailable(UnavailableReason::NotCachedLocally {
                account_kind: amx_core::AccountKind::Exchange,
            }),
            attachments: AttachmentState::None,
            parsed: None,
        });

        let err = run_get_message_links(&msg).unwrap_err();
        assert!(matches!(err, AmxError::BodyUnavailable { rowid: 1, .. }));
    }

    #[test]
    fn get_message_links_quarantined_renders_empty() {
        let msg = resolved_with(Classified {
            body: BodyState::Quarantined {
                error: ParseErrorKind::Io,
            },
            attachments: AttachmentState::None,
            parsed: None,
        });

        let response = run_get_message_links(&msg).unwrap();
        assert!(response.message_id.is_none());
        assert!(response.references.is_empty());
    }

    #[test]
    fn get_message_links_indexed_carries_the_reply_chain() {
        let parsed = ParsedMessage {
            subject: None,
            from: Vec::new(),
            to: Vec::new(),
            date: None,
            message_id: Some("<msg-2@example.com>".to_string()),
            in_reply_to: Some("<msg-1@example.com>".to_string()),
            references: vec![
                "<msg-0@example.com>".to_string(),
                "<msg-1@example.com>".to_string(),
            ],
            body_text: None,
            body_html: None,
            footer: Default::default(),
            attachments: AttachmentState::None,
            attachment_parts: Vec::new(),
        };
        let msg = resolved_with(Classified {
            body: BodyState::Indexed,
            attachments: AttachmentState::None,
            parsed: Some(parsed),
        });

        let response = run_get_message_links(&msg).unwrap();
        assert_eq!(response.message_id.as_deref(), Some("<msg-2@example.com>"));
        assert_eq!(response.in_reply_to.as_deref(), Some("<msg-1@example.com>"));
        assert_eq!(response.references.len(), 2);
    }
}
