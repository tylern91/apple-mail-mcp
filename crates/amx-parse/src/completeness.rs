//! Completeness oracle (§4.2.4): does the on-disk `.emlx` actually carry its attachment bytes?
//!
//! `X-Apple-Content-Length` (present only on the minority of messages Mail.app has pruned
//! attachment bytes from) declares the full RFC 822 message size *as originally downloaded*. When
//! the on-disk message is shorter than that, the attachment bytes were never fetched. This oracle
//! only reads the header from the message's *top-level* headers (`Message::header()`) — a
//! `X-Apple-Content-Length` string found elsewhere, e.g. inside a nested MIME part's own headers,
//! does not count and must not be matched by tooling that inspects `.emlx` files by other means
//! (grep over the whole file will produce false positives from nested parts). Verified live via a
//! synthetic candidate (Phase 4 Task 10 check #3): a message with a genuine top-level
//! `X-Apple-Content-Length` larger than its on-disk byte count classifies as `NotDownloaded`, and
//! reverting the header reclassifies it correctly once re-parsed.

use amx_core::coverage::AttachmentState;
use mail_parser::Message;

/// Compares `X-Apple-Content-Length` (if present) against `on_disk_message_bytes` — the length of
/// the RFC 822 message actually stored in the `.emlx`, not the file's total size (the plist footer
/// is metadata, not message content, so it is excluded from the comparison).
pub fn attachment_state(message: &Message<'_>, on_disk_message_bytes: u64) -> AttachmentState {
    if let Some(declared_bytes) = declared_content_length(message)
        && on_disk_message_bytes < declared_bytes
    {
        return AttachmentState::NotDownloaded {
            declared_bytes,
            on_disk_bytes: on_disk_message_bytes,
        };
    }

    match message.attachment_count() {
        0 => AttachmentState::None,
        count => AttachmentState::Extracted { count },
    }
}

fn declared_content_length(message: &Message<'_>) -> Option<u64> {
    message
        .header("X-Apple-Content-Length")?
        .as_text()?
        .trim()
        .parse()
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use mail_parser::MessageParser;

    fn parse(raw: &str) -> Message<'_> {
        MessageParser::default().parse(raw.as_bytes()).unwrap()
    }

    #[test]
    fn declared_length_past_on_disk_size_is_not_downloaded() {
        let raw = "X-Apple-Content-Length: 999999\r\nSubject: hi\r\n\r\nbody";
        let message = parse(raw);

        let state = attachment_state(&message, raw.len() as u64);

        assert_eq!(
            state,
            AttachmentState::NotDownloaded {
                declared_bytes: 999_999,
                on_disk_bytes: raw.len() as u64,
            }
        );
    }

    #[test]
    fn no_declared_length_and_no_attachment_is_none() {
        let raw = "Subject: hi\r\n\r\nbody";
        let message = parse(raw);

        assert_eq!(
            attachment_state(&message, raw.len() as u64),
            AttachmentState::None
        );
    }

    #[test]
    fn fully_downloaded_message_with_attachment_is_extracted() {
        let raw = concat!(
            "Content-Type: multipart/mixed; boundary=b\r\n",
            "\r\n",
            "--b\r\n",
            "Content-Type: text/plain\r\n",
            "\r\n",
            "body\r\n",
            "--b\r\n",
            "Content-Type: application/pdf\r\n",
            "Content-Disposition: ATTACHMENT; filename=r.pdf\r\n",
            "\r\n",
            "%PDF-1.4 fixture bytes\r\n",
            "--b--\r\n",
        );
        let message = parse(raw);

        assert_eq!(
            attachment_state(&message, raw.len() as u64),
            AttachmentState::Extracted { count: 1 }
        );
    }

    /// A declared length that is satisfied by the on-disk bytes means the message was fully
    /// downloaded — the oracle must not flag it as missing just because the header is present.
    #[test]
    fn satisfied_declared_length_with_attachment_is_extracted() {
        let raw = concat!(
            "X-Apple-Content-Length: 1\r\n",
            "Content-Type: multipart/mixed; boundary=b\r\n",
            "\r\n",
            "--b\r\n",
            "Content-Type: text/plain\r\n",
            "\r\n",
            "body\r\n",
            "--b\r\n",
            "Content-Type: application/pdf\r\n",
            "Content-Disposition: ATTACHMENT; filename=r.pdf\r\n",
            "\r\n",
            "%PDF-1.4 fixture bytes\r\n",
            "--b--\r\n",
        );
        let message = parse(raw);

        assert_eq!(
            attachment_state(&message, raw.len() as u64),
            AttachmentState::Extracted { count: 1 }
        );
    }
}
