use std::path::PathBuf;
use std::time::SystemTime;

use crate::coverage::UnavailableReason;

/// Errors from parsing a `.emlx` file's envelope, RFC 822 body, or plist footer.
///
/// Every variant is a per-file failure — a `parse_emlx` call always returns a `Result`, never
/// panics, so one bad message can be quarantined without aborting the rest of a sync pass.
#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("malformed .emlx envelope in {path}: {reason}")]
    MalformedEnvelope { path: PathBuf, reason: String },

    #[error("invalid RFC 822 message in {path}: {reason}")]
    InvalidMessage { path: PathBuf, reason: String },

    #[error("malformed plist footer in {path}: {reason}")]
    MalformedFooter { path: PathBuf, reason: String },

    #[error("could not read {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// The kind of a [`ParseError`], stored in the `quarantine` table without the full error
/// (which may carry a non-`'static` path) so quarantined messages can be listed cheaply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseErrorKind {
    MalformedEnvelope,
    InvalidMessage,
    MalformedFooter,
    Io,
}

impl From<&ParseError> for ParseErrorKind {
    fn from(err: &ParseError) -> Self {
        match err {
            ParseError::MalformedEnvelope { .. } => Self::MalformedEnvelope,
            ParseError::InvalidMessage { .. } => Self::InvalidMessage,
            ParseError::MalformedFooter { .. } => Self::MalformedFooter,
            ParseError::Io { .. } => Self::Io,
        }
    }
}

/// The account's remote protocol — determines whether a missing `.emlx` means "never
/// downloaded" ([`UnavailableReason::NotCachedLocally`]) or something else.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountKind {
    Imap,
    Exchange,
    ICloud,
    Pop,
}

/// The full error taxonomy for apple-mail-mcp, ported verbatim from the umbrella spec (§4.5).
///
/// Every variant carries its remediation in `Display`, so an agent relaying the error relays
/// the fix. This is the direct answer to imdinu #109: two exception handlers one class away
/// from correct, with no type-system help — here an exhaustive `match` forces every handler
/// to decide about a new variant.
#[derive(Debug, thiserror::Error)]
pub enum AmxError {
    #[error(
        "Full Disk Access denied for {responsible_process} — grant it in System Settings → \
         Privacy & Security → Full Disk Access, then restart {responsible_process}"
    )]
    StoreAccessDenied {
        responsible_process: String,
        path: PathBuf,
    },

    #[error("granted but not yet active — {0} must restart to pick up the TCC grant")]
    StoreAccessPendingRestart(String),

    #[error(
        "mailbox {requested:?} matched none of {available} mailboxes \
         (percent-decoded, NFC-normalised); closest: {suggestions:?}"
    )]
    MailboxFilterUnmatched {
        requested: String,
        available: usize,
        suggestions: Vec<String>,
    },

    #[error(
        "account {requested:?} matched none of {available} accounts \
         (percent-decoded, NFC-normalised); closest: {suggestions:?}"
    )]
    AccountFilterUnmatched {
        requested: String,
        available: usize,
        suggestions: Vec<String>,
    },

    #[error(
        "attachment {name:?} is not on disk: message is a partial download \
         ({on_disk} of {declared} bytes). Run `amxcli fetch-full --rowid {rowid}`"
    )]
    AttachmentNotDownloaded {
        rowid: i64,
        name: String,
        on_disk: u64,
        declared: u64,
    },

    #[error("message {0} not found in the local store")]
    MessageNotFound(i64),

    #[error("message {rowid} has no attachment named {name:?}")]
    AttachmentNotFound { rowid: i64, name: String },

    #[error("attachment {name:?} on message {rowid} could not be extracted to text: {reason}")]
    AttachmentUnextractable {
        rowid: i64,
        name: String,
        reason: String,
    },

    #[error("message {rowid} body unavailable: {reason}")]
    BodyUnavailable {
        rowid: i64,
        reason: UnavailableReason,
    },

    #[error("index writer lock held by pid {pid} ({process}) since {since:?}")]
    IndexWriterBusy {
        pid: u32,
        process: String,
        since: SystemTime,
    },

    #[error(
        "Automation permission denied for Mail.app — System Settings → Privacy & Security → Automation"
    )]
    AutomationDenied,

    #[error("Mail store not found at {0} — is Mail.app configured on this Mac?")]
    StoreNotFound(PathBuf),

    #[error("mailbox url {url:?} is not a `scheme://account/segment...` url")]
    MailboxUrlMalformed { url: String },

    #[error("could not locate mailbox {url:?} on disk at {path} ({reason})")]
    MailboxPathUnresolvable {
        url: String,
        path: PathBuf,
        reason: String,
    },

    #[error("search query {query:?} could not be parsed: {reason}")]
    SearchQueryInvalid { query: String, reason: String },

    #[error("automation script for {op} failed: {stderr}")]
    JxaFailed { op: String, stderr: String },

    #[error("automation script for {op} timed out")]
    JxaTimeout { op: String },

    #[error("message {rowid} is not addressable in Mail.app: {reason}")]
    MessageNotAddressable { rowid: i64, reason: String },

    #[error(
        "mutation on message {rowid} could not be verified: expected {expected}, observed {observed}"
    )]
    MutationVerificationFailed {
        rowid: i64,
        expected: String,
        observed: String,
    },

    #[error(transparent)]
    Parse(#[from] ParseError),

    #[cfg(feature = "sqlite")]
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),

    #[cfg(feature = "tantivy")]
    #[error(transparent)]
    Tantivy(#[from] tantivy::TantivyError),
}
