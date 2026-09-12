//! Parses the `.emlx` container format itself: a 10-byte space-padded ASCII byte count, a `\n`,
//! that many bytes of RFC 822 message, then an XML property-list footer — verified against the
//! live store rather than the umbrella spec's "binary plist" description, which this store's real
//! `.emlx` files contradict (every footer sampled is `<?xml version="1.0"...?>`, not `bplist00`).
//!
//! Every failure is a per-file [`ParseError`] — never a panic — so one malformed message can be
//! quarantined without aborting the rest of a sync pass (§4.2.3).

use std::path::Path;

use amx_core::ParseError;
use amx_core::coverage::AttachmentState;
use mail_parser::{Address, MessageParser};
use serde::Deserialize;

use crate::completeness;

/// The count line is always exactly 10 bytes (verified: every sampled `.emlx` in the live store
/// has `nl == 10`), left-justified and space-padded, followed by `\n`.
const COUNT_LINE_LEN: usize = 10;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmlxAddress {
    pub name: Option<String>,
    pub email: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EmlxFooter {
    pub flags: Option<i64>,
    pub date_received: Option<i64>,
    pub date_last_viewed: Option<i64>,
    pub conversation_id: Option<i64>,
    pub remote_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
struct FooterPlist {
    flags: Option<i64>,
    #[serde(rename = "date-received")]
    date_received: Option<i64>,
    #[serde(rename = "date-last-viewed")]
    date_last_viewed: Option<i64>,
    #[serde(rename = "conversation-id")]
    conversation_id: Option<i64>,
    #[serde(rename = "remote-id")]
    remote_id: Option<String>,
}

impl From<FooterPlist> for EmlxFooter {
    fn from(plist: FooterPlist) -> Self {
        Self {
            flags: plist.flags,
            date_received: plist.date_received,
            date_last_viewed: plist.date_last_viewed,
            conversation_id: plist.conversation_id,
            remote_id: plist.remote_id,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedMessage {
    pub subject: Option<String>,
    pub from: Vec<EmlxAddress>,
    pub to: Vec<EmlxAddress>,
    pub date: Option<i64>,
    pub message_id: Option<String>,
    pub body_text: Option<String>,
    pub body_html: Option<String>,
    pub footer: EmlxFooter,
    pub attachments: AttachmentState,
}

/// Parses the raw bytes of a `.emlx` file (already read from `path`, which is carried only for
/// error messages) into a [`ParsedMessage`].
pub fn parse_emlx(path: &Path, bytes: &[u8]) -> Result<ParsedMessage, ParseError> {
    let (message_bytes, footer_bytes) = split_envelope(path, bytes)?;

    let message = MessageParser::default()
        .parse(message_bytes)
        .ok_or_else(|| ParseError::InvalidMessage {
            path: path.to_path_buf(),
            reason: "mail-parser could not parse the RFC 822 message".to_string(),
        })?;

    let footer = parse_footer(path, footer_bytes)?;
    let attachments = completeness::attachment_state(&message, message_bytes.len() as u64);

    Ok(ParsedMessage {
        subject: message.subject().map(str::to_string),
        from: collect_addresses(message.from()),
        to: collect_addresses(message.to()),
        date: message.date().map(|dt| dt.to_timestamp()),
        message_id: message.message_id().map(str::to_string),
        body_text: message.body_text(0).map(|s| s.into_owned()),
        body_html: message.body_html(0).map(|s| s.into_owned()),
        footer,
        attachments,
    })
}

fn collect_addresses(address: Option<&Address<'_>>) -> Vec<EmlxAddress> {
    let Some(address) = address else {
        return Vec::new();
    };
    address
        .iter()
        .filter_map(|addr| {
            addr.address().map(|email| EmlxAddress {
                name: addr.name().map(str::to_string),
                email: email.to_string(),
            })
        })
        .collect()
}

/// Splits `bytes` into the RFC 822 message and the plist footer, per the 10-byte count-line
/// format. Never panics on truncated or non-numeric input — both are [`ParseError::MalformedEnvelope`].
fn split_envelope<'a>(path: &Path, bytes: &'a [u8]) -> Result<(&'a [u8], &'a [u8]), ParseError> {
    if bytes.len() < COUNT_LINE_LEN + 1 || bytes[COUNT_LINE_LEN] != b'\n' {
        return Err(ParseError::MalformedEnvelope {
            path: path.to_path_buf(),
            reason: "missing 10-byte count line".to_string(),
        });
    }

    let count_str = std::str::from_utf8(&bytes[..COUNT_LINE_LEN])
        .map_err(|_| ParseError::MalformedEnvelope {
            path: path.to_path_buf(),
            reason: "count line is not valid UTF-8".to_string(),
        })?
        .trim();
    let count: usize = count_str
        .parse()
        .map_err(|_| ParseError::MalformedEnvelope {
            path: path.to_path_buf(),
            reason: format!("count line {count_str:?} is not a valid byte count"),
        })?;

    let message_start = COUNT_LINE_LEN + 1;
    let message_end = message_start
        .checked_add(count)
        .filter(|&end| end <= bytes.len())
        .ok_or_else(|| ParseError::MalformedEnvelope {
            path: path.to_path_buf(),
            reason: format!("declared count {count} exceeds the file's remaining bytes"),
        })?;

    Ok((&bytes[message_start..message_end], &bytes[message_end..]))
}

fn parse_footer(path: &Path, footer_bytes: &[u8]) -> Result<EmlxFooter, ParseError> {
    let plist: FooterPlist =
        plist::from_bytes(footer_bytes).map_err(|err| ParseError::MalformedFooter {
            path: path.to_path_buf(),
            reason: err.to_string(),
        })?;
    Ok(plist.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> Vec<u8> {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/emlx")
            .join(name);
        std::fs::read(&path).unwrap_or_else(|err| panic!("reading {path:?}: {err}"))
    }

    #[test]
    fn parses_a_well_formed_message() {
        let bytes = fixture("well_formed.emlx");
        let parsed = parse_emlx(Path::new("well_formed.emlx"), &bytes).unwrap();

        assert_eq!(parsed.subject.as_deref(), Some("Fixture subject line"));
        assert_eq!(parsed.from[0].email, "sender@example.com");
        assert_eq!(parsed.to[0].email, "recipient@example.com");
        assert!(
            parsed
                .body_text
                .unwrap()
                .contains("This is the fixture body.")
        );
        assert_eq!(parsed.footer.conversation_id, Some(176));
        assert_eq!(parsed.footer.date_received, Some(1_576_065_593));
        assert_eq!(parsed.footer.remote_id.as_deref(), Some("13"));
        assert_eq!(
            parsed.attachments,
            amx_core::coverage::AttachmentState::None
        );
    }

    /// End-to-end proof of the completeness oracle (§4.2.4), modeled on the live store's ROWID
    /// 42191: `X-Apple-Content-Length` declares more bytes than the `.emlx` actually carries.
    #[test]
    fn declared_content_length_past_on_disk_size_yields_not_downloaded() {
        let bytes = fixture("partial_no_attachment.emlx");
        let parsed = parse_emlx(Path::new("partial_no_attachment.emlx"), &bytes).unwrap();

        assert!(matches!(
            parsed.attachments,
            amx_core::coverage::AttachmentState::NotDownloaded { .. }
        ));
    }

    /// A raw, non-UTF-8 byte inside the `Date:` header must never panic mail-parser or this
    /// function — it must come back as either a successfully parsed message (mail-parser
    /// lossily recovers) or an `InvalidMessage`/`MalformedEnvelope` error, never a crash.
    #[test]
    fn raw_byte_in_date_header_does_not_panic() {
        let bytes = fixture("date_header_raw_byte.emlx");
        let result = parse_emlx(Path::new("date_header_raw_byte.emlx"), &bytes);
        assert!(result.is_ok() || result.is_err());
    }

    #[test]
    fn truncated_count_line_is_malformed_envelope() {
        let bytes = b"not-a-count\nrest".to_vec();
        let err = parse_emlx(Path::new("bad.emlx"), &bytes).unwrap_err();
        assert!(matches!(err, ParseError::MalformedEnvelope { .. }));
    }

    #[test]
    fn declared_count_past_eof_is_malformed_envelope() {
        let bytes = b"999999999\nshort".to_vec();
        let err = parse_emlx(Path::new("bad.emlx"), &bytes).unwrap_err();
        assert!(matches!(err, ParseError::MalformedEnvelope { .. }));
    }

    #[test]
    fn malformed_footer_is_reported_distinctly() {
        let message = b"Subject: hi\n\nbody";
        let mut bytes = format!("{:<10}\n", message.len()).into_bytes();
        bytes.extend_from_slice(message);
        bytes.extend_from_slice(b"not a plist");

        let err = parse_emlx(Path::new("bad-footer.emlx"), &bytes).unwrap_err();
        assert!(matches!(err, ParseError::MalformedFooter { .. }));
    }
}
