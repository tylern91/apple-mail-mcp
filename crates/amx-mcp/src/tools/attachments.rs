//! `list_attachments` / `get_attachment` / `extract_attachment_text` (Phase 3 task 5).
//!
//! `AttachmentState` is a message-level classification, not a per-attachment one — a
//! `.partial.emlx` message has *no* attachment bytes on disk for *any* of its parts, so
//! `NotDownloaded`'s byte counts apply uniformly, and `get_attachment`/`extract_attachment_text`
//! reject the whole message the moment that state is seen, before even looking for `name`
//! (umbrella §10's ROWID 42191 adversarial case: two PDFs, neither downloaded).

use amx_core::{AmxError, AttachmentState};
use amx_parse::{AttachmentPart, extract_by_content_type};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;

use super::pipeline::ResolvedMessage;
use crate::schema::tools::{
    AttachmentSummary, ExtractAttachmentTextResponse, GetAttachmentResponse,
    ListAttachmentsResponse,
};

fn attachment_parts(resolved: &ResolvedMessage) -> &[AttachmentPart] {
    resolved
        .classified
        .parsed
        .as_ref()
        .map(|parsed| parsed.attachment_parts.as_slice())
        .unwrap_or_default()
}

pub fn run_list_attachments(
    resolved: &ResolvedMessage,
) -> Result<ListAttachmentsResponse, AmxError> {
    let (declared, on_disk) = match resolved.classified.attachments {
        AttachmentState::NotDownloaded {
            declared_bytes,
            on_disk_bytes,
        } => (declared_bytes, on_disk_bytes),
        _ => (0, 0),
    };

    let attachments = attachment_parts(resolved)
        .iter()
        .map(|part| AttachmentSummary {
            name: part.name.clone().unwrap_or_default(),
            content_type: part.content_type.clone(),
            declared_bytes: if matches!(
                resolved.classified.attachments,
                AttachmentState::NotDownloaded { .. }
            ) {
                declared
            } else {
                part.bytes.len() as u64
            },
            on_disk_bytes: if matches!(
                resolved.classified.attachments,
                AttachmentState::NotDownloaded { .. }
            ) {
                on_disk
            } else {
                part.bytes.len() as u64
            },
        })
        .collect();

    Ok(ListAttachmentsResponse { attachments })
}

/// Finds `name` among `resolved`'s attachment parts, rejecting the whole message first if its
/// attachment state is `NotDownloaded` — that state means *no* part's bytes reached disk, so a
/// per-part search would only ever produce a misleading `AttachmentNotFound`.
fn find_downloaded_part<'a>(
    resolved: &'a ResolvedMessage,
    rowid: i64,
    name: &str,
) -> Result<&'a AttachmentPart, AmxError> {
    if let AttachmentState::NotDownloaded {
        declared_bytes,
        on_disk_bytes,
    } = resolved.classified.attachments
    {
        return Err(AmxError::AttachmentNotDownloaded {
            rowid,
            name: name.to_string(),
            on_disk: on_disk_bytes,
            declared: declared_bytes,
        });
    }

    attachment_parts(resolved)
        .iter()
        .find(|part| part.name.as_deref() == Some(name))
        .ok_or_else(|| AmxError::AttachmentNotFound {
            rowid,
            name: name.to_string(),
        })
}

pub fn run_get_attachment(
    resolved: &ResolvedMessage,
    name: &str,
) -> Result<GetAttachmentResponse, AmxError> {
    let part = find_downloaded_part(resolved, resolved.row.rowid, name)?;
    Ok(GetAttachmentResponse {
        name: name.to_string(),
        content_type: part.content_type.clone(),
        bytes_base64: BASE64.encode(&part.bytes),
    })
}

pub fn run_extract_attachment_text(
    resolved: &ResolvedMessage,
    name: &str,
) -> Result<ExtractAttachmentTextResponse, AmxError> {
    let rowid = resolved.row.rowid;
    let part = find_downloaded_part(resolved, rowid, name)?;

    let extracted = extract_by_content_type(
        part.content_type.as_deref().unwrap_or(""),
        part.content_subtype.as_deref(),
        &part.bytes,
    );

    let text = match extracted {
        Some(Ok(text)) => text,
        Some(Err(err)) => {
            return Err(AmxError::AttachmentUnextractable {
                rowid,
                name: name.to_string(),
                reason: err.to_string(),
            });
        }
        None => {
            return Err(AmxError::AttachmentUnextractable {
                rowid,
                name: name.to_string(),
                reason: "no extractor for this attachment's MIME type".to_string(),
            });
        }
    };

    Ok(ExtractAttachmentTextResponse {
        name: name.to_string(),
        text,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use amx_core::BodyState;
    use amx_index::classify::Classified;
    use amx_parse::ParsedMessage;
    use amx_store::query::MessageRow;

    fn row() -> MessageRow {
        MessageRow {
            rowid: 42191,
            mailbox: 1,
            date_sent: None,
            date_received: None,
            read: false,
            flagged: false,
            deleted: false,
        }
    }

    fn resolved_with(
        attachments: AttachmentState,
        parsed: Option<ParsedMessage>,
    ) -> ResolvedMessage {
        ResolvedMessage {
            row: row(),
            mailbox_url: "imap://ACCOUNT/INBOX".to_string(),
            account_id: "ACCOUNT".to_string(),
            classified: Classified {
                body: BodyState::Indexed,
                attachments,
                parsed,
            },
        }
    }

    fn parsed_with_parts(parts: Vec<AttachmentPart>) -> ParsedMessage {
        ParsedMessage {
            subject: None,
            from: Vec::new(),
            to: Vec::new(),
            date: None,
            message_id: None,
            body_text: None,
            body_html: None,
            footer: Default::default(),
            attachments: AttachmentState::Extracted { count: parts.len() },
            attachment_parts: parts,
        }
    }

    /// Mirrors umbrella §10's ROWID 42191 adversarial case: a partial download must produce
    /// `AttachmentNotDownloaded` with both byte counts, never `AttachmentNotFound`.
    #[test]
    fn not_downloaded_state_rejects_before_searching_by_name() {
        let parsed = parsed_with_parts(vec![AttachmentPart {
            name: Some("report.pdf".to_string()),
            content_type: Some("application".to_string()),
            content_subtype: Some("pdf".to_string()),
            bytes: Vec::new(),
        }]);
        let resolved = resolved_with(
            AttachmentState::NotDownloaded {
                declared_bytes: 436_056,
                on_disk_bytes: 221_547,
            },
            Some(parsed),
        );

        let err = run_get_attachment(&resolved, "report.pdf").unwrap_err();
        match err {
            AmxError::AttachmentNotDownloaded {
                rowid,
                on_disk,
                declared,
                ..
            } => {
                assert_eq!(rowid, 42191);
                assert_eq!(on_disk, 221_547);
                assert_eq!(declared, 436_056);
            }
            other => panic!("expected AttachmentNotDownloaded, got {other:?}"),
        }
    }

    #[test]
    fn unknown_name_is_attachment_not_found() {
        let parsed = parsed_with_parts(vec![AttachmentPart {
            name: Some("report.pdf".to_string()),
            content_type: Some("application".to_string()),
            content_subtype: Some("pdf".to_string()),
            bytes: b"pdf bytes".to_vec(),
        }]);
        let resolved = resolved_with(AttachmentState::Extracted { count: 1 }, Some(parsed));

        let err = run_get_attachment(&resolved, "missing.pdf").unwrap_err();
        assert!(matches!(err, AmxError::AttachmentNotFound { .. }));
    }

    #[test]
    fn extracted_attachment_round_trips_through_base64() {
        let parsed = parsed_with_parts(vec![AttachmentPart {
            name: Some("note.txt".to_string()),
            content_type: Some("text".to_string()),
            content_subtype: Some("plain".to_string()),
            bytes: b"hello".to_vec(),
        }]);
        let resolved = resolved_with(AttachmentState::Extracted { count: 1 }, Some(parsed));

        let response = run_get_attachment(&resolved, "note.txt").unwrap();
        assert_eq!(
            BASE64.decode(response.bytes_base64).unwrap(),
            b"hello".to_vec()
        );
    }
}
