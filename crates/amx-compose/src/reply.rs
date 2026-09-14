//! Reply/forward derivation — the parasxos #3 guard.
//!
//! parasxos' `reply_email` silently degraded to a plain `send_email` when the source-message read
//! failed, losing the thread and dropping 2 of 3 recipients with no error surfaced anywhere. Every
//! source-lookup failure here is instead a hard [`amx_core::AmxError::ReplyDerivationFailed`] — there
//! is no fallback path to a plain send anywhere in this module.

use amx_core::AmxError;
use amx_parse::emlx::{EmlxAddress, ParsedMessage};

use crate::draft::{Draft, EmailAddress};

/// Whether a reply addresses only the original sender, or every original recipient.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplyMode {
    Sender,
    All,
}

fn to_email_address(address: &EmlxAddress) -> EmailAddress {
    match &address.name {
        Some(name) => EmailAddress::with_name(name.clone(), address.email.clone()),
        None => EmailAddress::new(address.email.clone()),
    }
}

fn contains_address(addresses: &[EmailAddress], email: &str) -> bool {
    addresses
        .iter()
        .any(|addr| addr.email.eq_ignore_ascii_case(email))
}

fn dedup_excluding(addresses: Vec<EmailAddress>, exclude: &[String]) -> Vec<EmailAddress> {
    let mut seen: Vec<EmailAddress> = Vec::new();
    for address in addresses {
        let is_ours = exclude
            .iter()
            .any(|ours| ours.eq_ignore_ascii_case(&address.email));
        if is_ours || contains_address(&seen, &address.email) {
            continue;
        }
        seen.push(address);
    }
    seen
}

fn quoted_history(source: &ParsedMessage) -> String {
    let from = source
        .from
        .first()
        .map(|addr| addr.name.clone().unwrap_or_else(|| addr.email.clone()))
        .unwrap_or_else(|| "unknown sender".to_string());
    let subject = source.subject.clone().unwrap_or_default();
    let body = source.body_text.clone().unwrap_or_default();
    let quoted: String = body
        .lines()
        .map(|line| format!("> {line}"))
        .collect::<Vec<_>>()
        .join("\n");
    format!("On {from}'s message \"{subject}\":\n\n{quoted}")
}

/// Derives a reply [`Draft`] from `source`. Recipients are `Reply-To` if present, else `From`;
/// [`ReplyMode::All`] also folds in the original `To` and `Cc`, minus `our_addresses`, deduped
/// case-insensitively on the address part.
///
/// Returns [`AmxError::ReplyDerivationFailed`] — never a degraded plain send — if `source` has no
/// `Message-ID` or no derivable recipient.
pub fn derive_reply(
    rowid: i64,
    source: &ParsedMessage,
    mode: ReplyMode,
    our_addresses: &[String],
) -> Result<Draft, AmxError> {
    let source_message_id =
        source
            .message_id
            .clone()
            .ok_or_else(|| AmxError::ReplyDerivationFailed {
                rowid,
                mode: "reply",
                reason: "source message has no Message-ID".to_string(),
            })?;

    let primary = if !source.reply_to.is_empty() {
        &source.reply_to
    } else {
        &source.from
    };
    let mut to: Vec<EmailAddress> = primary.iter().map(to_email_address).collect();

    if mode == ReplyMode::All {
        to.extend(source.to.iter().map(to_email_address));
        to.extend(source.cc.iter().map(to_email_address));
    }
    let to = dedup_excluding(to, our_addresses);

    if to.is_empty() {
        return Err(AmxError::ReplyDerivationFailed {
            rowid,
            mode: "reply",
            reason: "no recipient could be derived (source has no From/Reply-To)".to_string(),
        });
    }

    let subject = match &source.subject {
        Some(subject) if subject.to_ascii_lowercase().starts_with("re:") => subject.clone(),
        Some(subject) => format!("Re: {subject}"),
        None => "Re:".to_string(),
    };

    let mut references = source.references.clone();
    references.push(source_message_id.clone());

    Ok(Draft {
        to,
        subject,
        text_body: source.body_text.clone(),
        html_body: source.body_html.clone(),
        in_reply_to: Some(source_message_id),
        references,
        ..Draft::default()
    })
}

/// Derives a forward [`Draft`] from `source` — a quoted-history body with no inherited threading
/// headers (a forward starts a new thread). The caller fills in `to`/`from` themselves.
///
/// Returns [`AmxError::ReplyDerivationFailed`] if `source` has no `Message-ID`, for the same
/// hard-error contract as [`derive_reply`].
pub fn derive_forward(rowid: i64, source: &ParsedMessage) -> Result<Draft, AmxError> {
    if source.message_id.is_none() {
        return Err(AmxError::ReplyDerivationFailed {
            rowid,
            mode: "forward",
            reason: "source message has no Message-ID".to_string(),
        });
    }

    let subject = match &source.subject {
        Some(subject) if subject.to_ascii_lowercase().starts_with("fwd:") => subject.clone(),
        Some(subject) => format!("Fwd: {subject}"),
        None => "Fwd:".to_string(),
    };

    Ok(Draft {
        subject,
        text_body: Some(quoted_history(source)),
        html_body: None,
        ..Draft::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use amx_core::coverage::AttachmentState;

    fn address(name: &str, email: &str) -> EmlxAddress {
        EmlxAddress {
            name: Some(name.to_string()),
            email: email.to_string(),
        }
    }

    fn base_message() -> ParsedMessage {
        ParsedMessage {
            subject: Some("Quarterly numbers".to_string()),
            from: vec![address("Alice", "alice@example.com")],
            to: vec![
                address("Bob", "bob@example.com"),
                address("Carol", "carol@example.com"),
            ],
            cc: vec![address("Dave", "dave@example.com")],
            reply_to: Vec::new(),
            date: None,
            message_id: Some("<msg-1@example.com>".to_string()),
            in_reply_to: None,
            references: Vec::new(),
            body_text: Some("Here are the numbers.".to_string()),
            body_html: None,
            footer: Default::default(),
            attachments: AttachmentState::None,
            attachment_parts: Vec::new(),
        }
    }

    #[test]
    fn reply_to_sender_addresses_only_from() {
        let message = base_message();
        let draft = derive_reply(
            1,
            &message,
            ReplyMode::Sender,
            &["bob@example.com".to_string()],
        )
        .expect("derivation should succeed");
        assert_eq!(draft.to.len(), 1);
        assert_eq!(draft.to[0].email, "alice@example.com");
        assert_eq!(draft.subject, "Re: Quarterly numbers");
        assert_eq!(draft.in_reply_to.as_deref(), Some("<msg-1@example.com>"));
        assert_eq!(draft.references, vec!["<msg-1@example.com>".to_string()]);
    }

    #[test]
    fn reply_all_folds_in_to_and_cc_minus_our_addresses() {
        let message = base_message();
        let draft = derive_reply(
            1,
            &message,
            ReplyMode::All,
            &["bob@example.com".to_string()],
        )
        .expect("derivation should succeed");
        let emails: Vec<&str> = draft.to.iter().map(|a| a.email.as_str()).collect();
        assert_eq!(
            emails,
            vec!["alice@example.com", "carol@example.com", "dave@example.com"]
        );
    }

    #[test]
    fn reply_prefers_reply_to_over_from() {
        let mut message = base_message();
        message.reply_to = vec![address("Support", "support@example.com")];
        let draft =
            derive_reply(1, &message, ReplyMode::Sender, &[]).expect("derivation should succeed");
        assert_eq!(draft.to[0].email, "support@example.com");
    }

    #[test]
    fn reply_errors_on_missing_message_id() {
        let mut message = base_message();
        message.message_id = None;
        let err = derive_reply(1, &message, ReplyMode::Sender, &[])
            .expect_err("missing Message-ID must hard-error, never degrade to a plain send");
        assert!(matches!(
            err,
            AmxError::ReplyDerivationFailed { mode: "reply", .. }
        ));
    }

    #[test]
    fn reply_all_errors_when_every_recipient_is_our_own_address() {
        let mut message = base_message();
        message.to.clear();
        message.cc.clear();
        let err = derive_reply(
            1,
            &message,
            ReplyMode::All,
            &["alice@example.com".to_string()],
        )
        .expect_err("no derivable recipient must hard-error");
        assert!(matches!(err, AmxError::ReplyDerivationFailed { .. }));
    }

    #[test]
    fn forward_has_no_inherited_threading_headers() {
        let message = base_message();
        let draft = derive_forward(1, &message).expect("derivation should succeed");
        assert_eq!(draft.subject, "Fwd: Quarterly numbers");
        assert!(draft.in_reply_to.is_none());
        assert!(draft.references.is_empty());
        assert!(draft.text_body.unwrap().contains("Here are the numbers."));
    }

    #[test]
    fn forward_errors_on_missing_message_id() {
        let mut message = base_message();
        message.message_id = None;
        let err = derive_forward(1, &message)
            .expect_err("missing Message-ID must hard-error for forward too");
        assert!(matches!(
            err,
            AmxError::ReplyDerivationFailed {
                mode: "forward",
                ..
            }
        ));
    }
}
