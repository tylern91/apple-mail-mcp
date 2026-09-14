//! MIME construction via `mail-builder` — the single place in this codebase that touches
//! RFC 2047/5322 header encoding. No hand-rolled encoding exists anywhere else, which is the
//! direct answer to UseJunior #198 (a non-ASCII `Subject:` triple-encoded into mojibake by
//! hand-rolled header logic elsewhere in that codebase).

use amx_core::AmxError;
use mail_builder::MessageBuilder;
use mail_builder::headers::address::Address;
use mail_builder::mime::make_boundary;

use crate::draft::{Draft, EmailAddress};

/// A fully-encoded RFC 5322 message, ready for SMTP submission or IMAP `APPEND`.
#[derive(Debug, Clone)]
pub struct ComposedMessage {
    /// The `Message-ID` this message was composed with — generated up front (rather than left to
    /// `mail-builder`'s own auto-generation on serialize) so callers can thread a reply/forward
    /// or file an `APPEND` copy without re-parsing the raw bytes.
    pub message_id: String,
    /// The raw RFC 5322 bytes, CRLF-terminated headers, ready to hand to `lettre`/`imap`.
    pub raw: Vec<u8>,
}

fn to_mail_builder_address(address: &EmailAddress) -> Address<'_> {
    Address::new_address(address.name.as_deref(), address.email.as_str())
}

fn address_list(addresses: &[EmailAddress]) -> Address<'_> {
    Address::new_list(addresses.iter().map(to_mail_builder_address).collect())
}

/// Generates a `Message-ID` value (without angle brackets — `mail-builder`'s `MessageId` header
/// adds those) using the same boundary-uniqueness generator `mail-builder` uses internally for
/// its own auto-generated `Message-ID`s, so this crate needs no additional randomness dependency.
fn generate_message_id() -> String {
    format!("{}@{}", make_boundary("."), local_hostname())
}

fn local_hostname() -> String {
    gethostname::gethostname()
        .into_string()
        .unwrap_or_else(|_| "localhost".to_string())
}

/// Builds the RFC 5322 message for `draft`.
///
/// Returns [`AmxError::ComposeInvalid`] if neither `text_body` nor `html_body` is set, or if
/// `from`/`to` is empty — a caller error, not a MIME-construction failure, so it is checked
/// before handing anything to `mail-builder`.
pub fn compose(draft: &Draft) -> Result<ComposedMessage, AmxError> {
    let from = draft
        .from
        .as_ref()
        .ok_or_else(|| AmxError::ComposeInvalid {
            reason: "no From address set".to_string(),
        })?;
    if draft.to.is_empty() {
        return Err(AmxError::ComposeInvalid {
            reason: "no To recipients set".to_string(),
        });
    }
    if draft.text_body.is_none() && draft.html_body.is_none() {
        return Err(AmxError::ComposeInvalid {
            reason: "neither text_body nor html_body is set".to_string(),
        });
    }

    let message_id = generate_message_id();

    let mut builder = MessageBuilder::new()
        .message_id(message_id.as_str())
        .from(to_mail_builder_address(from))
        .to(address_list(&draft.to))
        .subject(draft.subject.as_str());

    if !draft.cc.is_empty() {
        builder = builder.cc(address_list(&draft.cc));
    }
    if !draft.bcc.is_empty() {
        builder = builder.bcc(address_list(&draft.bcc));
    }
    if let Some(text_body) = &draft.text_body {
        builder = builder.text_body(text_body.as_str());
    }
    if let Some(html_body) = &draft.html_body {
        builder = builder.html_body(html_body.as_str());
    }
    if let Some(in_reply_to) = &draft.in_reply_to {
        builder = builder.in_reply_to(in_reply_to.as_str());
    }
    if !draft.references.is_empty() {
        let references: Vec<&str> = draft.references.iter().map(String::as_str).collect();
        builder = builder.references(references.as_slice());
    }
    for attachment in &draft.attachments {
        builder = builder.attachment(
            attachment.content_type.as_str(),
            attachment.filename.as_str(),
            attachment.bytes.as_slice(),
        );
    }

    let mut raw = Vec::new();
    builder.serialize(&mut raw);

    Ok(ComposedMessage { message_id, raw })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::draft::Draft;

    fn base_draft() -> Draft {
        Draft {
            from: Some(EmailAddress::with_name("Alice", "alice@example.com")),
            to: vec![EmailAddress::new("bob@example.com")],
            subject: "Hello".to_string(),
            text_body: Some("Hi Bob".to_string()),
            ..Draft::default()
        }
    }

    #[test]
    fn composes_plain_text_message() {
        let draft = base_draft();
        let composed = compose(&draft).expect("compose should succeed");
        let raw = String::from_utf8(composed.raw).expect("raw bytes should be valid utf-8");
        assert!(raw.contains("Subject: Hello"));
        assert!(raw.contains("Hi Bob"));
        assert!(composed.message_id.contains('@'));
    }

    #[test]
    fn composes_multipart_alternative_when_both_bodies_set() {
        let mut draft = base_draft();
        draft.html_body = Some("<p>Hi Bob</p>".to_string());
        let composed = compose(&draft).expect("compose should succeed");
        let raw = String::from_utf8(composed.raw).expect("raw bytes should be valid utf-8");
        assert!(raw.to_ascii_lowercase().contains("multipart/alternative"));
        assert!(raw.contains("Hi Bob"));
        assert!(raw.contains("<p>Hi Bob</p>"));
    }

    #[test]
    fn rejects_missing_from() {
        let mut draft = base_draft();
        draft.from = None;
        let err = compose(&draft).expect_err("compose should reject a missing From");
        assert!(matches!(err, AmxError::ComposeInvalid { .. }));
    }

    #[test]
    fn rejects_empty_to() {
        let mut draft = base_draft();
        draft.to.clear();
        let err = compose(&draft).expect_err("compose should reject empty To");
        assert!(matches!(err, AmxError::ComposeInvalid { .. }));
    }

    #[test]
    fn rejects_missing_body() {
        let mut draft = base_draft();
        draft.text_body = None;
        let err = compose(&draft).expect_err("compose should reject a body-less draft");
        assert!(matches!(err, AmxError::ComposeInvalid { .. }));
    }
}
