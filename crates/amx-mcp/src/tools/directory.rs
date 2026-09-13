//! `resolve_address` / `list_accounts` / `list_mailboxes` (Phase 3 task 6).
//!
//! There is no address-to-account table anywhere in the store — `resolve_address` uses
//! `Accounts4.sqlite`'s `ZACCOUNT.ZUSERNAME`, confirmed live to hold an account's own login/email
//! address (e.g. an iCloud address can back more than one account row, one Mail-capable and one
//! calendar-only), via `AccountResolver::resolve_by_address`.

use std::collections::BTreeSet;

use amx_core::{AccountKind, AmxError};
use amx_store::account::AccountResolver;
use amx_store::mailbox_path::MailboxPathResolver;
use amx_store::registry::MailboxRegistry;

use crate::schema::tools::{
    AccountSummary, ListAccountsResponse, ListMailboxesRequest, ListMailboxesResponse,
    MailboxSummary, ResolveAddressRequest, ResolveAddressResponse,
};

fn kind_label(kind: AccountKind) -> &'static str {
    match kind {
        AccountKind::Imap => "imap",
        AccountKind::Exchange => "exchange",
        AccountKind::ICloud => "icloud",
        AccountKind::Pop => "pop",
    }
}

pub fn run_resolve_address(
    account_resolver: &AccountResolver,
    request: &ResolveAddressRequest,
) -> Result<ResolveAddressResponse, AmxError> {
    let matches = account_resolver.resolve_by_address(&request.address)?;

    Ok(ResolveAddressResponse {
        address: request.address.clone(),
        display_name: matches.first().map(|account| account.display_name.clone()),
        accounts: matches
            .into_iter()
            .map(|account| account.identifier)
            .collect(),
    })
}

/// Every account identifier the mailbox registry references, deduplicated — `MailboxRegistry`
/// only tracks mailbox identity, so account identity is derived the same way
/// `search_messages.rs`'s `known_accounts` does, rather than duplicating that into the registry.
fn known_account_identifiers(registry: &MailboxRegistry) -> BTreeSet<String> {
    registry
        .iter()
        .filter_map(|mailbox| MailboxPathResolver::account_identifier(&mailbox.url).ok())
        .collect()
}

pub fn run_list_accounts(
    registry: &MailboxRegistry,
    account_resolver: &AccountResolver,
) -> Result<ListAccountsResponse, AmxError> {
    let accounts = known_account_identifiers(registry)
        .into_iter()
        .map(|identifier| match account_resolver.resolve(&identifier)? {
            Some(resolved) => Ok(AccountSummary {
                identifier: resolved.identifier,
                display_name: resolved.display_name,
                kind: resolved.kind.map(kind_label).map(str::to_string),
            }),
            None => Ok(AccountSummary {
                identifier: identifier.clone(),
                display_name: identifier,
                kind: None,
            }),
        })
        .collect::<Result<Vec<_>, AmxError>>()?;

    Ok(ListAccountsResponse { accounts })
}

pub fn run_list_mailboxes(
    registry: &MailboxRegistry,
    request: &ListMailboxesRequest,
) -> Result<ListMailboxesResponse, AmxError> {
    let mailboxes: Vec<MailboxSummary> = match &request.account {
        Some(account) => {
            let matches: Vec<MailboxSummary> = registry
                .mailboxes_for_account(account)
                .map(|mailbox| MailboxSummary {
                    url: mailbox.url.clone(),
                    total: mailbox.counts.total,
                    unread: mailbox.counts.unread,
                    deleted: mailbox.counts.deleted,
                })
                .collect();
            if matches.is_empty() {
                return Err(AmxError::AccountFilterUnmatched {
                    requested: account.clone(),
                    available: known_account_identifiers(registry).len(),
                    suggestions: Vec::new(),
                });
            }
            matches
        }
        None => registry
            .iter()
            .map(|mailbox| MailboxSummary {
                url: mailbox.url.clone(),
                total: mailbox.counts.total,
                unread: mailbox.counts.unread,
                deleted: mailbox.counts.deleted,
            })
            .collect(),
    };

    Ok(ListMailboxesResponse { mailboxes })
}

#[cfg(test)]
mod tests {
    use super::*;
    use amx_store::conn::RoConnection;
    use rusqlite::{Connection, params};

    const ACCOUNT: &str = "AB4FC904-21FE-4AC0-A089-716246CE5C46";

    fn seed_accounts_db(path: &std::path::Path) {
        Connection::open(path)
            .unwrap()
            .execute_batch(&format!(
                r#"
                CREATE TABLE ZACCOUNTTYPE (Z_PK INTEGER PRIMARY KEY, ZIDENTIFIER TEXT);
                CREATE TABLE ZACCOUNT (
                    Z_PK INTEGER PRIMARY KEY, ZIDENTIFIER TEXT, ZACCOUNTDESCRIPTION TEXT,
                    ZUSERNAME TEXT, ZACCOUNTTYPE INTEGER
                );
                CREATE TABLE ZACCOUNTPROPERTY (Z_PK INTEGER PRIMARY KEY, ZOWNER INTEGER, ZKEY TEXT);

                INSERT INTO ZACCOUNTTYPE VALUES (1, 'com.apple.account.IMAP');
                INSERT INTO ZACCOUNT VALUES (10, '{ACCOUNT}', NULL, 'alice@example.com', 1);
                INSERT INTO ZACCOUNTPROPERTY VALUES (100, 10, 'Hostname');
                "#
            ))
            .unwrap();
    }

    fn seed_registry(path: &std::path::Path) -> MailboxRegistry {
        let conn = Connection::open(path).unwrap();
        amx_store::fixtures::build_schema(&conn).unwrap();
        conn.execute(
            "INSERT INTO mailboxes (url, total_count, unread_count, deleted_count) \
             VALUES (?1, 5, 2, 0)",
            params![format!("imap://{ACCOUNT}/INBOX")],
        )
        .unwrap();
        let ro = RoConnection::open(path).unwrap();
        MailboxRegistry::load(&ro).unwrap()
    }

    #[test]
    fn resolve_address_finds_the_owning_account() {
        let file = tempfile::NamedTempFile::new().unwrap();
        seed_accounts_db(file.path());
        let resolver = AccountResolver::open(file.path()).unwrap();

        let response = run_resolve_address(
            &resolver,
            &ResolveAddressRequest {
                address: "Alice@Example.com".to_string(),
            },
        )
        .unwrap();

        assert_eq!(response.accounts, vec![ACCOUNT.to_string()]);
        assert!(response.display_name.is_some());
    }

    #[test]
    fn resolve_address_with_no_match_is_empty_not_an_error() {
        let file = tempfile::NamedTempFile::new().unwrap();
        seed_accounts_db(file.path());
        let resolver = AccountResolver::open(file.path()).unwrap();

        let response = run_resolve_address(
            &resolver,
            &ResolveAddressRequest {
                address: "nobody@example.com".to_string(),
            },
        )
        .unwrap();

        assert!(response.accounts.is_empty());
        assert!(response.display_name.is_none());
    }

    #[test]
    fn list_accounts_reports_every_account_the_registry_references() {
        let accounts_file = tempfile::NamedTempFile::new().unwrap();
        seed_accounts_db(accounts_file.path());
        let resolver = AccountResolver::open(accounts_file.path()).unwrap();

        let registry_file = tempfile::NamedTempFile::new().unwrap();
        let registry = seed_registry(registry_file.path());

        let response = run_list_accounts(&registry, &resolver).unwrap();
        assert_eq!(response.accounts.len(), 1);
        assert_eq!(response.accounts[0].identifier, ACCOUNT);
        assert_eq!(response.accounts[0].kind.as_deref(), Some("imap"));
    }

    #[test]
    fn list_mailboxes_with_no_filter_returns_everything() {
        let registry_file = tempfile::NamedTempFile::new().unwrap();
        let registry = seed_registry(registry_file.path());

        let response = run_list_mailboxes(&registry, &ListMailboxesRequest::default()).unwrap();
        assert_eq!(response.mailboxes.len(), 1);
        assert_eq!(response.mailboxes[0].total, 5);
    }

    #[test]
    fn list_mailboxes_unmatched_account_is_a_typed_error() {
        let registry_file = tempfile::NamedTempFile::new().unwrap();
        let registry = seed_registry(registry_file.path());

        let request = ListMailboxesRequest {
            account: Some("00000000-0000-0000-0000-000000000000".to_string()),
        };
        let err = run_list_mailboxes(&registry, &request).unwrap_err();
        assert!(matches!(err, AmxError::AccountFilterUnmatched { .. }));
    }
}
