//! Resolves a Mail store account UUID (a `~/Library/Mail/V10/<UUID>` directory name) to a
//! display name and [`AccountKind`], by reading `~/Library/Accounts/Accounts4.sqlite`.
//!
//! Accounts4.sqlite lives outside the Mail store root — a second Full Disk Access-relevant
//! location `TccProbe` has to account for separately (umbrella §0 correction C13).

use std::path::Path;

use amx_core::{AccountKind, AmxError};
use rusqlite::OptionalExtension;

use crate::conn::RoConnection;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedAccount {
    pub identifier: String,
    pub display_name: String,
    /// `None` for accounts with no remote protocol (e.g. macOS's "On My Mac" local account) —
    /// [`amx_core::UnavailableReason::NotCachedLocally`] never applies to them.
    pub kind: Option<AccountKind>,
}

pub struct AccountResolver {
    conn: RoConnection,
}

impl AccountResolver {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, AmxError> {
        Ok(Self {
            conn: RoConnection::open(path)?,
        })
    }

    /// Resolves `identifier` (the Mail store's account UUID) against Accounts4.sqlite.
    pub fn resolve(&self, identifier: &str) -> Result<Option<ResolvedAccount>, AmxError> {
        let conn = self.conn.as_connection();
        let row = conn
            .query_row(
                "SELECT Z_PK, ZACCOUNTDESCRIPTION, ZUSERNAME, ZACCOUNTTYPE \
                 FROM ZACCOUNT WHERE ZIDENTIFIER = ?1",
                [identifier],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, i64>(3)?,
                    ))
                },
            )
            .optional()?;

        let Some((account_pk, description, username, account_type_pk)) = row else {
            return Ok(None);
        };

        let type_identifier: Option<String> = conn
            .query_row(
                "SELECT ZIDENTIFIER FROM ZACCOUNTTYPE WHERE Z_PK = ?1",
                [account_type_pk],
                |row| row.get(0),
            )
            .optional()?;

        let kind = match type_identifier.as_deref() {
            Some("com.apple.account.Exchange") => Some(AccountKind::Exchange),
            Some("com.apple.account.POP") => Some(AccountKind::Pop),
            Some("com.apple.account.IMAP") | Some("com.apple.account.IMAPMail") => {
                if self.has_mail_drop(account_pk)? {
                    Some(AccountKind::ICloud)
                } else {
                    Some(AccountKind::Imap)
                }
            }
            _ => None,
        };

        let display_name = description
            .filter(|s| !s.is_empty())
            .or_else(|| username.filter(|s| !s.is_empty()))
            .unwrap_or_else(|| identifier.to_string());

        Ok(Some(ResolvedAccount {
            identifier: identifier.to_string(),
            display_name,
            kind,
        }))
    }

    /// `UseMailDrop` is set only on iCloud Mail accounts among the IMAP-typed rows — verified
    /// against a live store: three Gmail IMAP accounts carry a `Hostname` property and no
    /// `UseMailDrop`; the iCloud account carries `UseMailDrop` and no `Hostname` (Mail hardcodes
    /// `imap.mail.me.com` for it instead of storing it as a property).
    ///
    /// lean: a heuristic inferred from one live store, not a documented Apple contract —
    /// revisit if a non-iCloud IMAP provider is ever observed setting this key.
    fn has_mail_drop(&self, account_pk: i64) -> Result<bool, AmxError> {
        let exists: Option<i64> = self
            .conn
            .as_connection()
            .query_row(
                "SELECT 1 FROM ZACCOUNTPROPERTY WHERE ZOWNER = ?1 AND ZKEY = 'UseMailDrop'",
                [account_pk],
                |row| row.get(0),
            )
            .optional()?;
        Ok(exists.is_some())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;
    use tempfile::NamedTempFile;

    fn seed_accounts_db(path: &Path) {
        let conn = Connection::open(path).unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE ZACCOUNTTYPE (Z_PK INTEGER PRIMARY KEY, ZIDENTIFIER TEXT);
            CREATE TABLE ZACCOUNT (
                Z_PK INTEGER PRIMARY KEY,
                ZIDENTIFIER TEXT,
                ZACCOUNTDESCRIPTION TEXT,
                ZUSERNAME TEXT,
                ZACCOUNTTYPE INTEGER
            );
            CREATE TABLE ZACCOUNTPROPERTY (Z_PK INTEGER PRIMARY KEY, ZOWNER INTEGER, ZKEY TEXT);

            INSERT INTO ZACCOUNTTYPE VALUES (1, 'com.apple.account.IMAP');
            INSERT INTO ZACCOUNTTYPE VALUES (2, 'com.apple.account.Exchange');
            INSERT INTO ZACCOUNTTYPE VALUES (3, 'com.apple.account.LocalOnMyMac');

            INSERT INTO ZACCOUNT VALUES (10, 'ICLOUD-UUID', NULL, NULL, 1);
            INSERT INTO ZACCOUNTPROPERTY VALUES (100, 10, 'UseMailDrop');

            INSERT INTO ZACCOUNT VALUES (20, 'GMAIL-UUID', NULL, NULL, 1);
            INSERT INTO ZACCOUNTPROPERTY VALUES (200, 20, 'Hostname');

            INSERT INTO ZACCOUNT VALUES (30, 'EXCHANGE-UUID', 'Work Exchange', NULL, 2);
            INSERT INTO ZACCOUNT VALUES (40, 'LOCAL-UUID', 'On My Mac', NULL, 3);
            "#,
        )
        .unwrap();
    }

    #[test]
    fn resolves_icloud_via_mail_drop() {
        let file = NamedTempFile::new().unwrap();
        seed_accounts_db(file.path());
        let resolver = AccountResolver::open(file.path()).unwrap();
        let account = resolver.resolve("ICLOUD-UUID").unwrap().unwrap();
        assert_eq!(account.kind, Some(AccountKind::ICloud));
    }

    #[test]
    fn resolves_plain_imap_without_mail_drop() {
        let file = NamedTempFile::new().unwrap();
        seed_accounts_db(file.path());
        let resolver = AccountResolver::open(file.path()).unwrap();
        let account = resolver.resolve("GMAIL-UUID").unwrap().unwrap();
        assert_eq!(account.kind, Some(AccountKind::Imap));
    }

    #[test]
    fn resolves_exchange_by_display_name() {
        let file = NamedTempFile::new().unwrap();
        seed_accounts_db(file.path());
        let resolver = AccountResolver::open(file.path()).unwrap();
        let account = resolver.resolve("EXCHANGE-UUID").unwrap().unwrap();
        assert_eq!(account.kind, Some(AccountKind::Exchange));
        assert_eq!(account.display_name, "Work Exchange");
    }

    #[test]
    fn local_account_has_no_kind() {
        let file = NamedTempFile::new().unwrap();
        seed_accounts_db(file.path());
        let resolver = AccountResolver::open(file.path()).unwrap();
        let account = resolver.resolve("LOCAL-UUID").unwrap().unwrap();
        assert_eq!(account.kind, None);
    }

    #[test]
    fn unknown_uuid_resolves_to_none() {
        let file = NamedTempFile::new().unwrap();
        seed_accounts_db(file.path());
        let resolver = AccountResolver::open(file.path()).unwrap();
        assert_eq!(resolver.resolve("NOT-A-REAL-UUID").unwrap(), None);
    }
}
