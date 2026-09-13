//! `set_read_state`/`set_flag` (Phase 4 task 3): resolve → JXA locate → JXA mutate → re-read the
//! envelope row to verify the change actually landed → re-index the one message → commit.
//!
//! Every function here is synchronous and takes locked store handles directly — the JXA
//! round-trip in between is awaited one layer up, in `server.rs`'s `#[tool]` handlers, so no
//! `AppState` lock is ever held across an `.await`.

use std::path::Path;

use amx_automation::locate;
use amx_automation::op::MailboxAddress;
use amx_core::AmxError;
use amx_index::mutate::MutationWriter;
use amx_store::account::AccountResolver;
use amx_store::conn::RoConnection;
use amx_store::registry::MailboxRegistry;

use crate::schema::tools::{SetFlagResponse, SetReadStateResponse};
use crate::tools::pipeline::{ResolvedMessage, resolve_message};

/// What a mutate tool needs to address a message in Mail.app before it can act on it.
pub struct Addressed {
    pub message_id: String,
    pub mailbox: MailboxAddress,
}

/// Resolves `rowid` and derives its JXA address. Called before the JXA round-trip.
pub fn resolve_and_address(
    conn: &RoConnection,
    registry: &MailboxRegistry,
    store_root: &Path,
    account_resolver: &AccountResolver,
    rowid: i64,
) -> Result<Addressed, AmxError> {
    let resolved = resolve_message(conn, registry, store_root, account_resolver, rowid)?;

    let message_id = resolved
        .classified
        .parsed
        .as_ref()
        .and_then(|parsed| parsed.message_id.clone())
        .ok_or_else(|| AmxError::MessageNotAddressable {
            rowid,
            reason: "message has no Message-ID header".to_string(),
        })?;

    let display_name = account_resolver
        .resolve(&resolved.account_id)?
        .ok_or_else(|| AmxError::MessageNotAddressable {
            rowid,
            reason: format!(
                "account {} not found in Accounts4.sqlite",
                resolved.account_id
            ),
        })?
        .display_name;

    let mailbox = locate::mailbox_address(&display_name, &resolved.mailbox_url)?;
    Ok(Addressed {
        message_id,
        mailbox,
    })
}

/// Resolves `rowid`, re-indexes it from its current on-disk state, and commits — shared by every
/// mutate tool's post-mutation step so `search_messages` never sees a stale document (imdinu #66).
/// The caller must reload its `IndexReaderPool` after this returns (D3 leaves that one step to
/// the caller, since the pool is `AppState`'s, not this module's).
#[allow(clippy::too_many_arguments)]
fn reindex(
    conn: &RoConnection,
    registry: &MailboxRegistry,
    store_root: &Path,
    account_resolver: &AccountResolver,
    index_dir: &Path,
    meta_path: &Path,
    rowid: i64,
) -> Result<ResolvedMessage, AmxError> {
    let updated = resolve_message(conn, registry, store_root, account_resolver, rowid)?;
    let mut writer = MutationWriter::open(index_dir, meta_path)?;
    writer.upsert(
        rowid,
        &updated.account_id,
        &updated.mailbox_url,
        &updated.classified,
    )?;
    writer.commit()?;
    Ok(updated)
}

/// Called after the JXA `set_read_state` mutation has been issued: re-reads the envelope row to
/// confirm `read` actually landed, then re-indexes.
#[allow(clippy::too_many_arguments)]
pub fn finish_set_read_state(
    conn: &RoConnection,
    registry: &MailboxRegistry,
    store_root: &Path,
    account_resolver: &AccountResolver,
    index_dir: &Path,
    meta_path: &Path,
    rowid: i64,
    read: bool,
) -> Result<SetReadStateResponse, AmxError> {
    let updated = reindex(
        conn,
        registry,
        store_root,
        account_resolver,
        index_dir,
        meta_path,
        rowid,
    )?;
    if updated.row.read != read {
        return Err(AmxError::MutationVerificationFailed {
            rowid,
            expected: read.to_string(),
            observed: updated.row.read.to_string(),
        });
    }
    Ok(SetReadStateResponse { rowid, read })
}

/// Called after the JXA `set_flag` mutation has been issued: re-reads the envelope row to
/// confirm `flagged` actually landed, then re-indexes.
#[allow(clippy::too_many_arguments)]
pub fn finish_set_flag(
    conn: &RoConnection,
    registry: &MailboxRegistry,
    store_root: &Path,
    account_resolver: &AccountResolver,
    index_dir: &Path,
    meta_path: &Path,
    rowid: i64,
    flagged: bool,
) -> Result<SetFlagResponse, AmxError> {
    let updated = reindex(
        conn,
        registry,
        store_root,
        account_resolver,
        index_dir,
        meta_path,
        rowid,
    )?;
    if updated.row.flagged != flagged {
        return Err(AmxError::MutationVerificationFailed {
            rowid,
            expected: flagged.to_string(),
            observed: updated.row.flagged.to_string(),
        });
    }
    Ok(SetFlagResponse { rowid, flagged })
}
