//! The input to [`crate::compose`] — provider- and transport-agnostic, so callers building a
//! plain send, a reply, or a forward all produce the same shape.

/// An RFC 5322 mailbox: an email address with an optional display name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmailAddress {
    pub name: Option<String>,
    pub email: String,
}

impl EmailAddress {
    pub fn new(email: impl Into<String>) -> Self {
        Self {
            name: None,
            email: email.into(),
        }
    }

    pub fn with_name(name: impl Into<String>, email: impl Into<String>) -> Self {
        Self {
            name: Some(name.into()),
            email: email.into(),
        }
    }
}

/// A binary attachment to be MIME-encoded into the message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attachment {
    pub filename: String,
    pub content_type: String,
    pub bytes: Vec<u8>,
}

/// A message to be composed, before MIME construction. At least one of `text_body`/`html_body`
/// must be set — [`crate::compose`] rejects a body-less draft rather than silently sending an
/// empty message.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Draft {
    pub from: Option<EmailAddress>,
    pub to: Vec<EmailAddress>,
    pub cc: Vec<EmailAddress>,
    pub bcc: Vec<EmailAddress>,
    pub subject: String,
    pub text_body: Option<String>,
    pub html_body: Option<String>,
    /// The source message's `Message-ID`, for a reply. `None` for a plain send or a forward.
    pub in_reply_to: Option<String>,
    /// The full ancestor chain (oldest first), ending with the immediate parent's `Message-ID`.
    pub references: Vec<String>,
    pub attachments: Vec<Attachment>,
}
