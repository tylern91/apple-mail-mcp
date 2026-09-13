//! `set_read_state`/`set_flag` (Phase 4 task 3): resolve → JXA locate → JXA mutate → re-read the
//! envelope row to verify the change actually landed → re-index the one message → commit.
//!
//! Every function here is synchronous and takes locked store handles directly — the JXA
//! round-trip in between is awaited one layer up, in `server.rs`'s `#[tool]` handlers, so no
//! `AppState` lock is ever held across an `.await`.

use std::collections::HashSet;
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use amx_automation::locate;
use amx_automation::op::MailboxAddress;
use amx_core::{AmxError, AttachmentState};
use amx_index::mutate::MutationWriter;
use amx_store::account::AccountResolver;
use amx_store::conn::RoConnection;
use amx_store::mailbox_path::MailboxPathResolver;
use amx_store::query::MessageQuery;
use amx_store::registry::MailboxRegistry;

use crate::schema::tools::{
    MoveMessagesResponse, SetFlagResponse, SetReadStateResponse, TrashMessagesResponse,
};
use crate::tools::pipeline::{ResolvedMessage, resolve_message};

/// What a mutate tool needs to address a message in Mail.app before it can act on it.
pub struct Addressed {
    pub message_id: String,
    pub mailbox: MailboxAddress,
    pub account_id: String,
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
        account_id: resolved.account_id,
    })
}

/// Resolves `rowid` and stages its re-indexed document on an already-open `writer`, without
/// committing. Split out from [`reindex`] so `triage_apply` (Phase 4 task 7) can stage many
/// items through one [`MutationWriter`] and commit once for the whole batch (D3), while the
/// single-item mutate tools still get one writer-per-call via `reindex` below.
pub(crate) fn reindex_with_writer(
    writer: &mut MutationWriter,
    conn: &RoConnection,
    registry: &MailboxRegistry,
    store_root: &Path,
    account_resolver: &AccountResolver,
    rowid: i64,
) -> Result<ResolvedMessage, AmxError> {
    let updated = resolve_message(conn, registry, store_root, account_resolver, rowid)?;
    writer.upsert(
        rowid,
        &updated.account_id,
        &updated.mailbox_url,
        &updated.classified,
    )?;
    Ok(updated)
}

/// Resolves `rowid`, re-indexes it from its current on-disk state, and commits — shared by every
/// single-item mutate tool's post-mutation step so `search_messages` never sees a stale document
/// (imdinu #66). The caller must reload its `IndexReaderPool` after this returns (D3 leaves that
/// one step to the caller, since the pool is `AppState`'s, not this module's).
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
    let mut writer = MutationWriter::open(index_dir, meta_path)?;
    let updated = reindex_with_writer(
        &mut writer,
        conn,
        registry,
        store_root,
        account_resolver,
        rowid,
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

/// Called after the JXA `fetch_full` mutation has been issued (Phase 4 task 8, `amxcli
/// fetch-full` — deliberately not a 7th MCP tool): re-parses the now-fully-downloaded `.emlx`
/// through [`resolve_message`]'s own [`AvailabilityClassifier`](amx_index::classify::AvailabilityClassifier)
/// call and re-indexes. Fails if the attachment state is still `NotDownloaded` after the fetch —
/// Mail.app declined, or the download did not complete.
#[allow(clippy::too_many_arguments)]
pub fn finish_fetch_full(
    conn: &RoConnection,
    registry: &MailboxRegistry,
    store_root: &Path,
    account_resolver: &AccountResolver,
    index_dir: &Path,
    meta_path: &Path,
    rowid: i64,
) -> Result<AttachmentState, AmxError> {
    let updated = reindex(
        conn,
        registry,
        store_root,
        account_resolver,
        index_dir,
        meta_path,
        rowid,
    )?;
    if matches!(
        updated.classified.attachments,
        AttachmentState::NotDownloaded { .. }
    ) {
        return Err(AmxError::MutationVerificationFailed {
            rowid,
            expected: "attachments downloaded".to_string(),
            observed: "still NotDownloaded after fetch_full".to_string(),
        });
    }
    Ok(updated.classified.attachments)
}

/// Everything a move/trash mutate tool needs before its JXA round-trip: the message's current
/// address, plus a rowid snapshot of every mailbox in `snapshot_account_id` taken *before* the
/// mutation, so the post-mutation reconciliation step can tell which rowid is newly-created
/// (§"Open question resolved by design": a Mail.app move does not preserve `ROWID` — confirmed
/// empirically against `AMX-TEST` — so a vanished old rowid must be matched to its replacement by
/// diffing the account's rowid set, not by re-deriving an identifier).
pub struct RelocatePrep {
    pub addressed: Addressed,
    pub snapshot_account_id: String,
    pub before: HashSet<i64>,
}

/// Resolves `rowid` and snapshots the target account's rowids, ahead of a `move`/`trash` JXA
/// call. Pass `snapshot_account_id` explicitly for `move_messages` (the *destination* account — a
/// cross-account move surfaces its new rowid there); pass `None` for `trash_messages`, which
/// snapshots the message's own account (Mail.app's `delete` files to that same account's Trash
/// mailbox).
pub fn prepare_relocate(
    conn: &RoConnection,
    registry: &MailboxRegistry,
    store_root: &Path,
    account_resolver: &AccountResolver,
    rowid: i64,
    snapshot_account_id: Option<&str>,
) -> Result<RelocatePrep, AmxError> {
    let addressed = resolve_and_address(conn, registry, store_root, account_resolver, rowid)?;
    let snapshot_account_id = snapshot_account_id.unwrap_or(&addressed.account_id);
    let before = account_rowid_snapshot(conn, registry, snapshot_account_id)?;
    Ok(RelocatePrep {
        snapshot_account_id: snapshot_account_id.to_string(),
        before,
        addressed,
    })
}

/// Resolves a caller-supplied destination mailbox URL to its JXA address and owning account id,
/// validating it against the registry first so an unknown mailbox fails with suggestions rather
/// than a confusing JXA error.
pub fn resolve_destination(
    registry: &MailboxRegistry,
    account_resolver: &AccountResolver,
    destination_mailbox: &str,
) -> Result<(MailboxAddress, String), AmxError> {
    let mailbox =
        registry
            .resolve(destination_mailbox)
            .ok_or_else(|| AmxError::MailboxFilterUnmatched {
                requested: destination_mailbox.to_string(),
                available: registry.len(),
                suggestions: registry.suggest(destination_mailbox, 3),
            })?;
    let account_id = MailboxPathResolver::account_identifier(&mailbox.url)?;
    let display_name = account_resolver
        .resolve(&account_id)?
        .ok_or_else(|| AmxError::MailboxFilterUnmatched {
            requested: destination_mailbox.to_string(),
            available: registry.len(),
            suggestions: registry.suggest(destination_mailbox, 3),
        })?
        .display_name;
    let address = locate::mailbox_address(&display_name, &mailbox.url)?;
    Ok((address, account_id))
}

/// Every rowid currently on the books for `account_id`, across all of its mailboxes — the
/// before/after snapshot the reconciliation diff runs against.
fn account_rowid_snapshot(
    conn: &RoConnection,
    registry: &MailboxRegistry,
    account_id: &str,
) -> Result<HashSet<i64>, AmxError> {
    let mut rowids = HashSet::new();
    for mailbox in registry.mailboxes_for_account(account_id) {
        let rows = MessageQuery::new()
            .mailbox(mailbox.url.clone())
            .execute(conn, registry)?;
        rowids.extend(rows.into_iter().map(|row| row.rowid));
    }
    Ok(rowids)
}

/// How long to wait for Mail.app's own envelope-index writeback to catch up with a `move`/`trash`
/// before concluding the rowid genuinely never changed. Mail.app's `delete`/`move` verbs return
/// control to `osascript` before the underlying `.emlx` relocation and envelope-index update have
/// finished — observed empirically as a `trash_messages` call whose immediate post-JXA read still
/// saw the old rowid in its old mailbox, even though the message had in fact already moved to
/// `Deleted Messages` under a new rowid a moment later.
const RELOCATE_SETTLE_BUDGET: Duration = Duration::from_secs(3);
const RELOCATE_POLL_INTERVAL: Duration = Duration::from_millis(150);

/// Polls the envelope index until the message's post-mutation rowid is unambiguous: either the
/// old rowid disappears and exactly one new rowid appears in `snapshot_account_id` (a real
/// reallocation), or the old rowid is still present once the settle budget is exhausted (Mail.app
/// updated the row in place without reallocating `ROWID`).
fn wait_for_relocated_rowid(
    conn: &RoConnection,
    registry: &MailboxRegistry,
    old_rowid: i64,
    snapshot_account_id: &str,
    before: &HashSet<i64>,
) -> Result<i64, AmxError> {
    let deadline = Instant::now() + RELOCATE_SETTLE_BUDGET;
    loop {
        let still_present = !MessageQuery::new()
            .rowid(old_rowid)
            .execute(conn, registry)?
            .is_empty();

        if !still_present {
            let after = account_rowid_snapshot(conn, registry, snapshot_account_id)?;
            let new_rowids: Vec<i64> = after.difference(before).copied().collect();
            match new_rowids.len() {
                1 => return Ok(new_rowids[0]),
                0 if Instant::now() < deadline => {
                    thread::sleep(RELOCATE_POLL_INTERVAL);
                    continue;
                }
                n => {
                    return Err(AmxError::MutationVerificationFailed {
                        rowid: old_rowid,
                        expected: "exactly one newly-created rowid in the account".to_string(),
                        observed: format!("{n} newly-created rowids"),
                    });
                }
            }
        }

        if Instant::now() >= deadline {
            return Ok(old_rowid);
        }
        thread::sleep(RELOCATE_POLL_INTERVAL);
    }
}

/// Called after a `move`/`trash` JXA mutation has succeeded: figures out the message's rowid now
/// (unchanged, or newly-created per the account rowid diff), stages its re-indexed document on an
/// already-open `writer` (removing the stale document for `old_rowid` if it moved to a new one),
/// without committing. Split out from [`finish_relocate`] for the same reason as
/// [`reindex_with_writer`] — `triage_apply` batches many relocations through one writer and one
/// commit (D3).
#[allow(clippy::too_many_arguments)]
pub(crate) fn finish_relocate_with_writer(
    writer: &mut MutationWriter,
    conn: &RoConnection,
    registry: &MailboxRegistry,
    store_root: &Path,
    account_resolver: &AccountResolver,
    old_rowid: i64,
    snapshot_account_id: &str,
    before: &HashSet<i64>,
) -> Result<(i64, String), AmxError> {
    let final_rowid =
        wait_for_relocated_rowid(conn, registry, old_rowid, snapshot_account_id, before)?;

    let updated = resolve_message(conn, registry, store_root, account_resolver, final_rowid)?;

    if final_rowid != old_rowid {
        writer.remove(old_rowid);
    }
    writer.upsert(
        final_rowid,
        &updated.account_id,
        &updated.mailbox_url,
        &updated.classified,
    )?;

    Ok((final_rowid, updated.mailbox_url))
}

/// Called after a `move`/`trash` JXA mutation has succeeded, for the single-item mutate tools:
/// opens one [`MutationWriter`], stages the relocation, and commits (D3).
#[allow(clippy::too_many_arguments)]
fn finish_relocate(
    conn: &RoConnection,
    registry: &MailboxRegistry,
    store_root: &Path,
    account_resolver: &AccountResolver,
    index_dir: &Path,
    meta_path: &Path,
    old_rowid: i64,
    snapshot_account_id: &str,
    before: &HashSet<i64>,
) -> Result<(i64, String), AmxError> {
    let mut writer = MutationWriter::open(index_dir, meta_path)?;
    let result = finish_relocate_with_writer(
        &mut writer,
        conn,
        registry,
        store_root,
        account_resolver,
        old_rowid,
        snapshot_account_id,
        before,
    )?;
    writer.commit()?;
    Ok(result)
}

/// Called after the JXA `move` mutation has been issued.
#[allow(clippy::too_many_arguments)]
pub fn finish_move(
    conn: &RoConnection,
    registry: &MailboxRegistry,
    store_root: &Path,
    account_resolver: &AccountResolver,
    index_dir: &Path,
    meta_path: &Path,
    rowid: i64,
    snapshot_account_id: &str,
    before: &HashSet<i64>,
) -> Result<MoveMessagesResponse, AmxError> {
    let (new_rowid, mailbox) = finish_relocate(
        conn,
        registry,
        store_root,
        account_resolver,
        index_dir,
        meta_path,
        rowid,
        snapshot_account_id,
        before,
    )?;
    Ok(MoveMessagesResponse {
        rowid,
        new_rowid,
        mailbox,
    })
}

/// Called after the JXA `trash` mutation has been issued.
#[allow(clippy::too_many_arguments)]
pub fn finish_trash(
    conn: &RoConnection,
    registry: &MailboxRegistry,
    store_root: &Path,
    account_resolver: &AccountResolver,
    index_dir: &Path,
    meta_path: &Path,
    rowid: i64,
    snapshot_account_id: &str,
    before: &HashSet<i64>,
) -> Result<TrashMessagesResponse, AmxError> {
    let (new_rowid, mailbox) = finish_relocate(
        conn,
        registry,
        store_root,
        account_resolver,
        index_dir,
        meta_path,
        rowid,
        snapshot_account_id,
        before,
    )?;
    Ok(TrashMessagesResponse {
        rowid,
        new_rowid,
        mailbox,
    })
}
