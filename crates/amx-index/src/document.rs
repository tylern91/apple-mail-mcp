//! Builds the Tantivy document for one Envelope Index message. Shared by [`crate::sync`]'s full
//! sync pass and [`crate::mutate`]'s single-message re-index after a JXA mutation — both need the
//! identical document shape, since `body`/`attachment_text` are indexed but never stored
//! (`schema.rs`), so there is no way to partially patch a document already in the index.

use amx_parse::{EmlxAddress, ParsedMessage, extract_by_content_type, html_to_text};
use amx_store::registry::MailboxRegistry;
use tantivy::{DateTime, TantivyDocument};

use crate::classify::Classified;
use crate::schema::Fields;

pub fn build_document(
    rowid: i64,
    account_identifier: &str,
    mailbox_url: &str,
    classified: &Classified,
    fields: &Fields,
) -> TantivyDocument {
    let mut doc = TantivyDocument::new();
    doc.add_i64(fields.rowid, rowid);
    doc.add_text(fields.account_id, account_identifier);
    doc.add_text(
        fields.mailbox_key,
        MailboxRegistry::normalize_url(mailbox_url),
    );
    doc.add_u64(fields.body_state, classified.body.discriminant());
    doc.add_u64(
        fields.attachment_state,
        classified.attachments.discriminant(),
    );

    let Some(parsed) = &classified.parsed else {
        return doc;
    };

    if let Some(subject) = &parsed.subject {
        doc.add_text(fields.subject, subject);
    }
    let sender = format_addresses(&parsed.from);
    if !sender.is_empty() {
        doc.add_text(fields.sender, sender);
    }
    let recipients = format_addresses(&parsed.to);
    if !recipients.is_empty() {
        doc.add_text(fields.recipients, recipients);
    }
    if let Some(body) = body_text(parsed) {
        doc.add_text(fields.body, body);
    }
    let attachments = attachment_text(parsed);
    if !attachments.is_empty() {
        doc.add_text(fields.attachment_text, attachments);
    }
    if let Some(date) = parsed.date {
        doc.add_date(fields.date_sent, DateTime::from_timestamp_secs(date));
    }
    if let Some(date_received) = parsed.footer.date_received {
        doc.add_date(
            fields.date_received,
            DateTime::from_timestamp_secs(date_received),
        );
    }
    if let Some(flags) = parsed.footer.flags {
        doc.add_u64(fields.flags, flags as u64);
    }
    if let Some(thread_id) = parsed.footer.conversation_id {
        doc.add_i64(fields.thread_id, thread_id);
    }
    doc
}

fn format_addresses(addresses: &[EmlxAddress]) -> String {
    addresses
        .iter()
        .map(|address| match &address.name {
            Some(name) if !name.is_empty() => format!("{name} <{}>", address.email),
            _ => address.email.clone(),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Prefers `mail-parser`'s already-decoded plain-text body; falls back to converting the HTML
/// body when a message carries only that (umbrella §5.1's `body` field is plain text either way).
fn body_text(parsed: &ParsedMessage) -> Option<String> {
    parsed
        .body_text
        .clone()
        .or_else(|| parsed.body_html.as_deref().map(html_to_text))
}

/// Text extracted from every attachment part whose MIME type `extract_by_content_type`
/// recognizes and can read successfully. A part with an unrecognized type, or one that fails to
/// extract, simply contributes nothing here — it does not block indexing the rest of the
/// message.
fn attachment_text(parsed: &ParsedMessage) -> String {
    parsed
        .attachment_parts
        .iter()
        .filter_map(|part| {
            extract_by_content_type(
                part.content_type.as_deref().unwrap_or(""),
                part.content_subtype.as_deref(),
                &part.bytes,
            )
            .and_then(Result::ok)
        })
        .collect::<Vec<_>>()
        .join("\n")
}
