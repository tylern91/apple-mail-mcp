//! `search_messages` (Phase 3 task 4): resolves the request's `mailbox`/`account` filters against
//! `MailboxRegistry`/the store's known accounts *before* building an `amx_index::search::SearchRequest`
//! — an unmatched filter is `MailboxFilterUnmatched`/`AccountFilterUnmatched` with suggestions,
//! never a silent `hits: []` (umbrella §5.2). `amx_index::search::Search` itself only ever sees
//! already-resolved filter values (see that module's own doc comment).
//!
//! This handler deliberately returns the raw response + coverage, not a `SearchEnvelope` — the
//! `health` stamp is applied uniformly by the dispatch layer (task 7's `HealthMonitor`), not by
//! each handler.

use std::collections::HashMap;

use amx_core::AmxError;
use amx_index::schema::Fields;
use amx_index::search::{Search, SearchRequest};
use amx_store::mailbox_path::MailboxPathResolver;
use amx_store::registry::MailboxRegistry;
use tantivy::Searcher;

use crate::schema::coverage::Coverage;
use crate::schema::tools::{SearchHit, SearchMessagesRequest, SearchMessagesResponse};

const SUGGESTION_LIMIT: usize = 5;

/// Filter values already resolved against the store — the only shape `SearchRequest` accepts.
struct ResolvedFilters {
    mailbox_key: Option<String>,
    account_id: Option<String>,
}

/// Resolves `request`'s `mailbox`/`account` filters against `registry`, returning a typed,
/// suggestion-bearing error the moment either one doesn't match a real mailbox/account.
fn resolve_filters(
    registry: &MailboxRegistry,
    request: &SearchMessagesRequest,
) -> Result<ResolvedFilters, AmxError> {
    let mailbox_key = request
        .mailbox
        .as_deref()
        .map(|mailbox| resolve_mailbox(registry, mailbox))
        .transpose()?;

    let account_id = request
        .account
        .as_deref()
        .map(|account| resolve_account(registry, account))
        .transpose()?;

    Ok(ResolvedFilters {
        mailbox_key,
        account_id,
    })
}

fn resolve_mailbox(registry: &MailboxRegistry, requested: &str) -> Result<String, AmxError> {
    match registry.resolve(requested) {
        Some(mailbox) => Ok(MailboxRegistry::normalize_url(&mailbox.url)),
        None => Err(AmxError::MailboxFilterUnmatched {
            requested: requested.to_string(),
            available: registry.len(),
            suggestions: registry.suggest(requested, SUGGESTION_LIMIT),
        }),
    }
}

/// Every mailbox's account UUID segment, deduplicated and keyed by lowercase for
/// case-insensitive matching — `MailboxRegistry` only tracks mailbox identity, so account
/// identity is derived here rather than duplicated into the registry itself.
fn known_accounts(registry: &MailboxRegistry) -> HashMap<String, String> {
    let mut accounts = HashMap::new();
    for mailbox in registry.iter() {
        if let Ok(identifier) = MailboxPathResolver::account_identifier(&mailbox.url) {
            accounts
                .entry(identifier.to_lowercase())
                .or_insert(identifier);
        }
    }
    accounts
}

fn resolve_account(registry: &MailboxRegistry, requested: &str) -> Result<String, AmxError> {
    let accounts = known_accounts(registry);
    if let Some(identifier) = accounts.get(&requested.to_lowercase()) {
        return Ok(identifier.clone());
    }

    let mut scored: Vec<(usize, &String)> = accounts
        .values()
        .map(|identifier| (levenshtein(requested, identifier), identifier))
        .collect();
    scored.sort_by_key(|(distance, _)| *distance);

    Err(AmxError::AccountFilterUnmatched {
        requested: requested.to_string(),
        available: accounts.len(),
        suggestions: scored
            .into_iter()
            .take(SUGGESTION_LIMIT)
            .map(|(_, identifier)| identifier.clone())
            .collect(),
    })
}

/// Plain Levenshtein edit distance — mirrors `amx_store::registry`'s private helper; account
/// identifier suggestions are a distinct concern (account UUIDs, not mailbox URLs) and small
/// enough that reimplementing beats exposing the other one just for this.
fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut prev = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let temp = row[j + 1];
            row[j + 1] = if ca == cb {
                prev
            } else {
                1 + prev.min(row[j]).min(row[j + 1])
            };
            prev = temp;
        }
    }
    row[b.len()]
}

/// Resolves `request`'s filters against `registry`, then runs the search — the full
/// `search_messages` tool body, minus the `health` stamp (applied by the dispatch layer).
pub fn run_search(
    searcher: &Searcher,
    fields: &Fields,
    registry: &MailboxRegistry,
    request: &SearchMessagesRequest,
) -> Result<(SearchMessagesResponse, Coverage), AmxError> {
    let resolved = resolve_filters(registry, request)?;

    let search_request = SearchRequest {
        query: request.query.clone(),
        mailbox_key: resolved.mailbox_key,
        account_id: resolved.account_id,
        sender: request.sender.clone(),
        date_sent_from: request.date_sent_from,
        date_sent_to: request.date_sent_to,
        limit: request.limit.unwrap_or(20),
        offset: request.offset.unwrap_or(0),
    };

    let response = Search::run(searcher, fields, &search_request)?;

    let hits = response
        .hits
        .into_iter()
        .map(|hit| SearchHit {
            rowid: hit.rowid,
            score: hit.score,
            subject: Some(hit.subject).filter(|s| !s.is_empty()),
            sender: Some(hit.sender).filter(|s| !s.is_empty()),
            mailbox: hit.mailbox_key,
            account: hit.account_id,
            date_sent: hit.date_sent,
        })
        .collect();

    Ok((
        SearchMessagesResponse { hits },
        Coverage::from(response.coverage),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use amx_index::schema::{build_schema, register_tokenizers};
    use amx_store::conn::RoConnection;
    use rusqlite::{Connection, params};
    use tantivy::directory::RamDirectory;
    use tantivy::{DateTime, doc};
    use unicode_normalization::UnicodeNormalization;

    const ACCOUNT: &str = "AB4FC904-21FE-4AC0-A089-716246CE5C46";

    /// The three real Vietnamese IMAP folder names this store exercises (umbrella §10), stored
    /// NFD-encoded on disk the way Apple Mail actually writes them (imdinu #120).
    fn seed_registry(path: &std::path::Path) -> MailboxRegistry {
        let conn = Connection::open(path).unwrap();
        amx_store::fixtures::build_schema(&conn).unwrap();
        for name in [
            "[Gmail]/Tất cả thư",
            "[Gmail]/Quan trọng",
            "[Gmail]/Thư rác",
        ] {
            let nfd_name: String = name.nfd().collect();
            let url = format!("imap://{ACCOUNT}/{nfd_name}");
            conn.execute(
                "INSERT INTO mailboxes (url, total_count, unread_count, deleted_count) \
                 VALUES (?1, 0, 0, 0)",
                params![url],
            )
            .unwrap();
        }
        let ro = RoConnection::open(path).unwrap();
        MailboxRegistry::load(&ro).unwrap()
    }

    fn seeded_index() -> (tantivy::Index, Fields) {
        let (schema, fields) = build_schema();
        let index = tantivy::Index::create(RamDirectory::create(), schema, Default::default())
            .expect("in-memory index");
        register_tokenizers(&index);

        let mut writer = index.writer(15_000_000).expect("writer");
        let mailbox_key =
            MailboxRegistry::normalize_url(&format!("imap://{ACCOUNT}/[Gmail]/Tất cả thư"));
        writer
            .add_document(doc!(
                fields.rowid => 1i64,
                fields.account_id => ACCOUNT,
                fields.mailbox_key => mailbox_key,
                fields.subject => "quarterly roadmap",
                fields.sender => "Alice <alice@example.com>",
                fields.body => "nothing relevant here",
                fields.date_sent => DateTime::from_timestamp_secs(1_700_000_000),
                fields.body_state => 0u64,
                fields.attachment_state => 0u64,
            ))
            .unwrap();
        writer.commit().unwrap();
        (index, fields)
    }

    #[test]
    fn nfc_typed_mailbox_filter_resolves_against_nfd_stored_url() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let registry = seed_registry(file.path());
        let (index, fields) = seeded_index();
        let searcher = index.reader().unwrap().searcher();

        let request = SearchMessagesRequest {
            query: "roadmap".to_string(),
            mailbox: Some(format!("imap://{ACCOUNT}/[Gmail]/Tất cả thư")),
            ..Default::default()
        };
        let (response, _coverage) =
            run_search(&searcher, &fields, &registry, &request).expect("resolves and searches");
        assert_eq!(response.hits.len(), 1);
        assert_eq!(response.hits[0].rowid, 1);
    }

    #[test]
    fn second_nfc_case_quan_trong_resolves_as_an_empty_but_legitimate_filter() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let registry = seed_registry(file.path());
        let (index, fields) = seeded_index();
        let searcher = index.reader().unwrap().searcher();

        let request = SearchMessagesRequest {
            query: String::new(),
            mailbox: Some(format!("imap://{ACCOUNT}/[Gmail]/Quan trọng")),
            ..Default::default()
        };
        let (response, _coverage) =
            run_search(&searcher, &fields, &registry, &request).expect("filter resolves");
        assert!(
            response.hits.is_empty(),
            "no documents indexed under this mailbox"
        );
    }

    #[test]
    fn unmatched_mailbox_filter_is_a_typed_error_with_suggestions() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let registry = seed_registry(file.path());
        let (index, fields) = seeded_index();
        let searcher = index.reader().unwrap().searcher();

        let request = SearchMessagesRequest {
            query: String::new(),
            mailbox: Some("imap://nonexistent/INBOX".to_string()),
            ..Default::default()
        };
        let err = run_search(&searcher, &fields, &registry, &request).unwrap_err();
        match err {
            AmxError::MailboxFilterUnmatched {
                available,
                suggestions,
                ..
            } => {
                assert_eq!(available, 3);
                assert!(!suggestions.is_empty());
            }
            other => panic!("expected MailboxFilterUnmatched, got {other:?}"),
        }
    }

    #[test]
    fn unmatched_account_filter_is_a_typed_error_with_suggestions() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let registry = seed_registry(file.path());
        let (index, fields) = seeded_index();
        let searcher = index.reader().unwrap().searcher();

        let request = SearchMessagesRequest {
            query: String::new(),
            account: Some("00000000-0000-0000-0000-000000000000".to_string()),
            ..Default::default()
        };
        let err = run_search(&searcher, &fields, &registry, &request).unwrap_err();
        match err {
            AmxError::AccountFilterUnmatched {
                available,
                suggestions,
                ..
            } => {
                assert_eq!(available, 1);
                assert_eq!(suggestions, vec![ACCOUNT.to_string()]);
            }
            other => panic!("expected AccountFilterUnmatched, got {other:?}"),
        }
    }

    #[test]
    fn account_filter_matches_case_insensitively() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let registry = seed_registry(file.path());
        let (index, fields) = seeded_index();
        let searcher = index.reader().unwrap().searcher();

        let request = SearchMessagesRequest {
            query: "roadmap".to_string(),
            account: Some(ACCOUNT.to_lowercase()),
            ..Default::default()
        };
        let (response, _coverage) =
            run_search(&searcher, &fields, &registry, &request).expect("case-insensitive match");
        assert_eq!(response.hits.len(), 1);
    }
}
