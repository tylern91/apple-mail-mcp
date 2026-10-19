//! Message-ID → Mail.app message handle, scoped to a single mailbox — never a global `whose`
//! over every mailbox (umbrella §2.1: an unscoped JXA address lookup costs 7-10s per message).

use amx_core::AmxError;
use amx_store::mailbox_path::MailboxPathResolver;

use crate::jxa;
use crate::op::{JxaRequest, MailboxAddress};

/// Builds the JXA-addressable identity for `mailbox_url`, reusing the same URL parsing
/// `amx-store` already does to resolve the mailbox's on-disk path.
pub fn mailbox_address(account_name: &str, mailbox_url: &str) -> Result<MailboxAddress, AmxError> {
    Ok(MailboxAddress {
        account_name: account_name.to_string(),
        mailbox_segments: MailboxPathResolver::segments(mailbox_url)?,
    })
}

/// Confirms `message_id` is currently addressable inside `mailbox` via Mail.app, without
/// mutating anything. Every mutate tool calls this first so a stale or already-moved message
/// fails with [`AmxError::MessageNotAddressable`] instead of a confusing JXA error deep in a
/// mutation script.
pub async fn locate(
    mailbox: MailboxAddress,
    message_id: String,
    rowid: i64,
) -> Result<(), AmxError> {
    let request = JxaRequest::Locate {
        mailbox,
        message_id,
    };
    let result = jxa::run(&request).await?;
    if result.found {
        Ok(())
    } else {
        Err(AmxError::MessageNotAddressable {
            rowid,
            reason: "message not found in Mail.app for the resolved mailbox".to_string(),
        })
    }
}
