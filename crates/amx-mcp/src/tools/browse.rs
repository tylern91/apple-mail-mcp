//! `count_messages` / `recent_messages` (Phase 3 task 6).
//!
//! Both are thin wraps of `MessageQuery` — unlike `search_messages`, they don't pre-resolve
//! `mailbox`/`account` themselves: `MessageQuery::execute` already resolves those filters
//! against `MailboxRegistry` internally and surfaces `AmxError::MailboxFilterUnmatched` /
//! `AccountFilterUnmatched` on a miss (see `query.rs`'s own doc comment), so re-deriving that
//! resolution here would just duplicate it.

use std::path::Path;

use amx_core::AmxError;
use amx_store::account::AccountResolver;
use amx_store::conn::RoConnection;
use amx_store::query::MessageQuery;
use amx_store::registry::MailboxRegistry;

use super::get_message::format_address;
use super::pipeline::resolve_message;
use crate::schema::tools::{
    CountMessagesRequest, CountMessagesResponse, MessageSummary, RecentMessagesRequest,
    RecentMessagesResponse,
};

const DEFAULT_RECENT_LIMIT: usize = 20;

fn build_query(mailbox: &Option<String>, account: &Option<String>) -> MessageQuery {
    let mut query = MessageQuery::new();
    if let Some(mailbox) = mailbox {
        query = query.mailbox(mailbox.clone());
    }
    if let Some(account) = account {
        query = query.account(account.clone());
    }
    query
}

pub fn run_count_messages(
    conn: &RoConnection,
    registry: &MailboxRegistry,
    request: &CountMessagesRequest,
) -> Result<CountMessagesResponse, AmxError> {
    let mut query = build_query(&request.mailbox, &request.account)
        .date_sent_range(request.date_sent_from, request.date_sent_to);
    if let Some(read) = request.read {
        query = query.read(read);
    }
    if let Some(flagged) = request.flagged {
        query = query.flagged(flagged);
    }

    let count = query.execute(conn, registry)?.len();
    Ok(CountMessagesResponse { count })
}

#[allow(clippy::too_many_arguments)]
pub fn run_recent_messages(
    conn: &RoConnection,
    registry: &MailboxRegistry,
    store_root: &Path,
    account_resolver: &AccountResolver,
    request: &RecentMessagesRequest,
) -> Result<RecentMessagesResponse, AmxError> {
    let query = build_query(&request.mailbox, &request.account);
    let mut rows = query.execute(conn, registry)?;
    rows.sort_by_key(|row| std::cmp::Reverse(row.date_sent));

    let limit = request.limit.unwrap_or(DEFAULT_RECENT_LIMIT);
    let messages = rows
        .into_iter()
        .take(limit)
        .map(|row| {
            let resolved =
                resolve_message(conn, registry, store_root, account_resolver, row.rowid)?;
            let (subject, sender) = match &resolved.classified.parsed {
                Some(parsed) => (
                    parsed.subject.clone(),
                    parsed.from.first().map(format_address),
                ),
                None => (None, None),
            };
            Ok(MessageSummary {
                rowid: row.rowid,
                subject,
                sender,
                date_sent: row.date_sent,
                mailbox: resolved.mailbox_url,
            })
        })
        .collect::<Result<Vec<_>, AmxError>>()?;

    Ok(RecentMessagesResponse { messages })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::{Connection, params};

    const ACCOUNT: &str = "AB4FC904-21FE-4AC0-A089-716246CE5C46";

    fn seed_store(path: &std::path::Path) {
        let conn = Connection::open(path).unwrap();
        amx_store::fixtures::build_schema(&conn).unwrap();
        conn.execute(
            "INSERT INTO mailboxes (ROWID, url, total_count, unread_count, deleted_count) \
             VALUES (1, ?1, 2, 1, 0)",
            params![format!("imap://{ACCOUNT}/INBOX")],
        )
        .unwrap();
        conn.execute_batch(
            "INSERT INTO subjects (ROWID, subject) VALUES (1, 'hi');
             INSERT INTO messages (ROWID, subject, mailbox, date_sent, read, flagged, deleted)
             VALUES (10, 1, 1, 100, 0, 1, 0);
             INSERT INTO messages (ROWID, subject, mailbox, date_sent, read, flagged, deleted)
             VALUES (11, 1, 1, 200, 1, 0, 0);",
        )
        .unwrap();
    }

    fn store_and_registry(path: &std::path::Path) -> (RoConnection, MailboxRegistry) {
        seed_store(path);
        let conn = RoConnection::open(path).unwrap();
        let registry = MailboxRegistry::load(&conn).unwrap();
        (conn, registry)
    }

    #[test]
    fn count_messages_counts_matching_rows() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let (conn, registry) = store_and_registry(file.path());

        let response =
            run_count_messages(&conn, &registry, &CountMessagesRequest::default()).unwrap();
        assert_eq!(response.count, 2);
    }

    #[test]
    fn count_messages_applies_flagged_filter() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let (conn, registry) = store_and_registry(file.path());

        let request = CountMessagesRequest {
            flagged: Some(true),
            ..Default::default()
        };
        let response = run_count_messages(&conn, &registry, &request).unwrap();
        assert_eq!(response.count, 1);
    }

    #[test]
    fn count_messages_unmatched_mailbox_is_a_typed_error() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let (conn, registry) = store_and_registry(file.path());

        let request = CountMessagesRequest {
            mailbox: Some("imap://nonexistent/INBOX".to_string()),
            ..Default::default()
        };
        let err = run_count_messages(&conn, &registry, &request).unwrap_err();
        assert!(matches!(err, AmxError::MailboxFilterUnmatched { .. }));
    }

    /// End-to-end: a real `.mbox/<generation>` tree + `.emlx` fixture on disk, proving
    /// `run_recent_messages` sorts newest-first and stops at `limit` — logic this handler owns,
    /// unlike the mailbox/account resolution `MessageQuery` already covers on its own.
    #[test]
    fn recent_messages_sorts_newest_first_and_respects_limit() {
        let store_dir = tempfile::tempdir().unwrap();
        let mbox_dir = store_dir
            .path()
            .join(ACCOUNT)
            .join("INBOX.mbox")
            .join("F645E2FB-05D2-4A20-86B7-33FB1C8BFD18");
        std::fs::create_dir_all(&mbox_dir).unwrap();

        let fixture = std::fs::read(format!(
            "{}/../amx-parse/tests/fixtures/emlx/well_formed.emlx",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap();
        for rowid in [10, 11] {
            std::fs::write(mbox_dir.join(format!("{rowid}.emlx")), &fixture).unwrap();
        }

        let store_file = tempfile::NamedTempFile::new().unwrap();
        let (conn, registry) = store_and_registry(store_file.path());

        let accounts_file = tempfile::NamedTempFile::new().unwrap();
        rusqlite::Connection::open(accounts_file.path())
            .unwrap()
            .execute_batch(
                "CREATE TABLE ZACCOUNTTYPE (Z_PK INTEGER PRIMARY KEY, ZIDENTIFIER TEXT);
                 CREATE TABLE ZACCOUNT (
                     Z_PK INTEGER PRIMARY KEY, ZIDENTIFIER TEXT, ZACCOUNTDESCRIPTION TEXT,
                     ZUSERNAME TEXT, ZACCOUNTTYPE INTEGER
                 );
                 CREATE TABLE ZACCOUNTPROPERTY (Z_PK INTEGER PRIMARY KEY, ZOWNER INTEGER, ZKEY TEXT);",
            )
            .unwrap();
        let account_resolver = AccountResolver::open(accounts_file.path()).unwrap();

        let response = run_recent_messages(
            &conn,
            &registry,
            store_dir.path(),
            &account_resolver,
            &RecentMessagesRequest {
                limit: Some(1),
                ..Default::default()
            },
        )
        .unwrap();

        assert_eq!(response.messages.len(), 1);
        assert_eq!(response.messages[0].rowid, 11, "newest date_sent first");
    }
}
