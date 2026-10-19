//! Typed JXA request contract. One variant per Mail.app automation operation, each carrying
//! exactly the fields its script needs — no generic "run this script" escape hatch.

use serde::{Deserialize, Serialize};

/// Addresses a single mailbox the same way `amx-store` resolves it from the Envelope Index: the
/// account's Mail.app display name (`Mail.accounts.whose({name: ...})`) and the mailbox's
/// nested folder-name path (`amx_store::mailbox_path::MailboxPathResolver::segments`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MailboxAddress {
    pub account_name: String,
    pub mailbox_segments: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum JxaRequest {
    Locate {
        mailbox: MailboxAddress,
        message_id: String,
    },
    SetReadState {
        mailbox: MailboxAddress,
        message_id: String,
        read: bool,
    },
    SetFlag {
        mailbox: MailboxAddress,
        message_id: String,
        flagged: bool,
    },
    Move {
        mailbox: MailboxAddress,
        message_id: String,
        destination: MailboxAddress,
    },
    Trash {
        mailbox: MailboxAddress,
        message_id: String,
    },
    FetchFull {
        mailbox: MailboxAddress,
        message_id: String,
    },
}

impl JxaRequest {
    /// Short operation name, used in `AmxError::JxaFailed`/`JxaTimeout` for diagnostics.
    pub fn op_name(&self) -> &'static str {
        match self {
            JxaRequest::Locate { .. } => "locate",
            JxaRequest::SetReadState { .. } => "set_read_state",
            JxaRequest::SetFlag { .. } => "set_flag",
            JxaRequest::Move { .. } => "move",
            JxaRequest::Trash { .. } => "trash",
            JxaRequest::FetchFull { .. } => "fetch_full",
        }
    }

    /// The embedded JXA source that implements this operation.
    pub fn script(&self) -> &'static str {
        match self {
            JxaRequest::Locate { .. } => include_str!("../script/locate.js"),
            JxaRequest::SetReadState { .. } => include_str!("../script/set_read_state.js"),
            JxaRequest::SetFlag { .. } => include_str!("../script/set_flag.js"),
            JxaRequest::Move { .. } => include_str!("../script/move.js"),
            JxaRequest::Trash { .. } => include_str!("../script/trash.js"),
            JxaRequest::FetchFull { .. } => include_str!("../script/fetch_full.js"),
        }
    }
}

/// A JXA script's JSON stdout, shared across every operation: `found` is meaningful only for
/// [`JxaRequest::Locate`] and defaults to `false` elsewhere.
#[derive(Debug, Clone, Deserialize)]
pub struct JxaResult {
    pub ok: bool,
    #[serde(default)]
    pub found: bool,
    #[serde(default)]
    pub error: Option<String>,
}
