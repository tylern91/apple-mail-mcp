//! `get_message` (Phase 3 task 5): renders a [`ResolvedMessage`] into the wire response.
//!
//! `BodyState::Unavailable` becomes [`AmxError::BodyUnavailable`] — there is nothing to render.
//! `BodyState::Quarantined` deliberately renders as an empty-but-present message (subject/sender/
//! body all absent) rather than an error: quarantine visibility is `doctor`/`status`'s job
//! (tasks 7-8), not a per-message error an agent has to special-case on every read.

use amx_core::{AmxError, BodyState};
use amx_parse::{EmlxAddress, ParsedMessage, html_to_text};

use super::pipeline::ResolvedMessage;
use crate::schema::tools::GetMessageResponse;

pub fn run_get_message(resolved: &ResolvedMessage) -> Result<GetMessageResponse, AmxError> {
    let row = &resolved.row;
    let base = GetMessageResponse {
        rowid: row.rowid,
        subject: None,
        sender: None,
        recipients: Vec::new(),
        date_sent: row.date_sent,
        date_received: row.date_received,
        mailbox: resolved.mailbox_url.clone(),
        account: resolved.account_id.clone(),
        read: row.read,
        flagged: row.flagged,
        body: None,
    };

    match &resolved.classified.body {
        BodyState::Unavailable(reason) => Err(AmxError::BodyUnavailable {
            rowid: row.rowid,
            reason: reason.clone(),
        }),
        BodyState::Quarantined { .. } | BodyState::Pending { .. } => Ok(base),
        BodyState::Indexed => Ok(render_parsed(base, &resolved.classified.parsed)),
    }
}

fn render_parsed(base: GetMessageResponse, parsed: &Option<ParsedMessage>) -> GetMessageResponse {
    let Some(parsed) = parsed else {
        return base;
    };

    GetMessageResponse {
        subject: parsed.subject.clone(),
        sender: parsed.from.first().map(format_address),
        recipients: parsed.to.iter().map(format_address).collect(),
        body: parsed
            .body_text
            .clone()
            .or_else(|| parsed.body_html.as_deref().map(html_to_text)),
        ..base
    }
}

fn format_address(address: &EmlxAddress) -> String {
    match &address.name {
        Some(name) if !name.is_empty() => format!("{name} <{}>", address.email),
        _ => address.email.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use amx_core::{AccountKind, UnavailableReason};
    use amx_index::classify::Classified;
    use amx_store::query::MessageRow;

    fn row() -> MessageRow {
        MessageRow {
            rowid: 42,
            mailbox: 1,
            date_sent: Some(100),
            date_received: Some(101),
            read: true,
            flagged: false,
            deleted: false,
        }
    }

    fn resolved(classified: Classified) -> ResolvedMessage {
        ResolvedMessage {
            row: row(),
            mailbox_url: "imap://ACCOUNT/INBOX".to_string(),
            account_id: "ACCOUNT".to_string(),
            classified,
        }
    }

    #[test]
    fn unavailable_body_is_an_error_not_an_empty_message() {
        let msg = resolved(Classified {
            body: BodyState::Unavailable(UnavailableReason::NotCachedLocally {
                account_kind: AccountKind::Exchange,
            }),
            attachments: amx_core::AttachmentState::None,
            parsed: None,
        });

        let err = run_get_message(&msg).unwrap_err();
        assert!(matches!(err, AmxError::BodyUnavailable { rowid: 42, .. }));
    }

    #[test]
    fn quarantined_body_renders_as_an_empty_but_present_message() {
        let msg = resolved(Classified {
            body: BodyState::Quarantined {
                error: amx_core::ParseErrorKind::Io,
            },
            attachments: amx_core::AttachmentState::None,
            parsed: None,
        });

        let response = run_get_message(&msg).unwrap();
        assert_eq!(response.rowid, 42);
        assert!(response.subject.is_none());
        assert!(response.body.is_none());
    }
}
