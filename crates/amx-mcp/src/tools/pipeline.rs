//! Shared rowid → classified-message resolution for `get_message`/`list_attachments`/
//! `get_attachment`/`extract_attachment_text` (task 5) — every one of those handlers needs the
//! same envelope-index row, mailbox/account identity, and body/attachment classification, so it
//! is resolved once here rather than four times.

use std::path::Path;

use amx_core::AmxError;
use amx_index::classify::{AvailabilityClassifier, Classified};
use amx_index::fetch::{BodyFetcher, resolved_emlx_path};
use amx_store::account::AccountResolver;
use amx_store::conn::RoConnection;
use amx_store::mailbox_path::MailboxPathResolver;
use amx_store::query::{MessageQuery, MessageRow};
use amx_store::registry::MailboxRegistry;

pub struct ResolvedMessage {
    pub row: MessageRow,
    pub mailbox_url: String,
    pub account_id: String,
    pub classified: Classified,
}

/// Resolves `rowid` to its envelope-index row, mailbox/account identity, and body/attachment
/// classification. Returns [`AmxError::MessageNotFound`] when no such row exists, or when its
/// mailbox has since vanished from the registry.
pub fn resolve_message(
    store_conn: &RoConnection,
    registry: &MailboxRegistry,
    store_root: &Path,
    account_resolver: &AccountResolver,
    rowid: i64,
) -> Result<ResolvedMessage, AmxError> {
    let row = MessageQuery::new()
        .rowid(rowid)
        .execute(store_conn, registry)?
        .into_iter()
        .next()
        .ok_or(AmxError::MessageNotFound(rowid))?;

    let mailbox = registry
        .iter()
        .find(|m| m.rowid == row.mailbox)
        .ok_or(AmxError::MessageNotFound(rowid))?;

    let mailbox_dir = MailboxPathResolver::resolve(store_root, &mailbox.url)?;
    let account_id = MailboxPathResolver::account_identifier(&mailbox.url)?;
    let account_kind = account_resolver.resolve(&account_id)?.and_then(|a| a.kind);

    let fetched = BodyFetcher::fetch(&mailbox_dir, rowid);
    let emlx_path = resolved_emlx_path(&mailbox_dir, rowid, &fetched);
    let classified = AvailabilityClassifier::classify(&emlx_path, fetched, account_kind);

    Ok(ResolvedMessage {
        mailbox_url: mailbox.url.clone(),
        account_id,
        classified,
        row,
    })
}
