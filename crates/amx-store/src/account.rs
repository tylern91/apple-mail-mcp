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

/// The SMTP endpoint and credential mechanism an account submits outbound mail through.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SendingSettings {
    pub hostname: String,
    pub port: u16,
    pub ssl_enabled: bool,
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
                "SELECT Z_PK, ZIDENTIFIER, ZACCOUNTDESCRIPTION, ZUSERNAME, ZACCOUNTTYPE \
                 FROM ZACCOUNT WHERE ZIDENTIFIER = ?1",
                [identifier],
                Self::row_to_tuple,
            )
            .optional()?;

        row.map(|row| self.build_resolved(row)).transpose()
    }

    /// Every account whose `ZUSERNAME` (the account's own login/email address) matches `address`
    /// case-insensitively — resolves [`crate::query`]'s `resolve_address` tool. A single address
    /// can legitimately own more than one account row (observed live: the same iCloud address
    /// backing both a Mail and a Calendar-only account), so this returns all matches rather than
    /// assuming uniqueness.
    pub fn resolve_by_address(&self, address: &str) -> Result<Vec<ResolvedAccount>, AmxError> {
        let conn = self.conn.as_connection();
        let mut stmt = conn.prepare(
            "SELECT Z_PK, ZIDENTIFIER, ZACCOUNTDESCRIPTION, ZUSERNAME, ZACCOUNTTYPE \
             FROM ZACCOUNT WHERE ZUSERNAME IS NOT NULL AND LOWER(ZUSERNAME) = LOWER(?1)",
        )?;
        let rows = stmt.query_map([address], Self::row_to_tuple)?;

        rows.collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .map(|row| self.build_resolved(row))
            .collect()
    }

    #[allow(clippy::type_complexity)]
    fn row_to_tuple(
        row: &rusqlite::Row<'_>,
    ) -> rusqlite::Result<(i64, String, Option<String>, Option<String>, i64)> {
        Ok((
            row.get(0)?,
            row.get(1)?,
            row.get(2)?,
            row.get(3)?,
            row.get(4)?,
        ))
    }

    fn build_resolved(
        &self,
        (account_pk, identifier, description, username, account_type_pk): (
            i64,
            String,
            Option<String>,
            Option<String>,
            i64,
        ),
    ) -> Result<ResolvedAccount, AmxError> {
        let conn = self.conn.as_connection();
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
            .unwrap_or_else(|| identifier.clone());

        Ok(ResolvedAccount {
            identifier,
            display_name,
            kind,
        })
    }

    /// Resolves `identifier`'s outbound SMTP settings by following its `SendingAccountIdentifier`
    /// property to a second `ZACCOUNT` row (the SMTP account) and reading that row's `Hostname` /
    /// `PortNumber` / `SSLEnabled` properties.
    ///
    /// Those three properties are NSKeyedArchiver-encoded plists, not plain SQLite columns —
    /// decoded via [`Self::keyed_archiver_string`]/[`Self::keyed_archiver_integer`]/
    /// [`Self::keyed_archiver_bool`]. iCloud's stored `Hostname` is a non-string archived value
    /// (observed live: an archived integer `1`, not a hostname) — when the decoded hostname isn't
    /// a string, this falls back to a known-provider table keyed on [`AccountKind`] rather than
    /// erroring, and only errors when neither the store nor the fallback table has an answer.
    ///
    /// Returns `Ok(None)` if `identifier` doesn't resolve to an account, or the account has no
    /// `SendingAccountIdentifier` at all (e.g. a receive-only or local account).
    pub fn sending_settings(&self, identifier: &str) -> Result<Option<SendingSettings>, AmxError> {
        let conn = self.conn.as_connection();
        let account_pk: Option<i64> = conn
            .query_row(
                "SELECT Z_PK FROM ZACCOUNT WHERE ZIDENTIFIER = ?1",
                [identifier],
                |row| row.get(0),
            )
            .optional()?;
        let Some(account_pk) = account_pk else {
            return Ok(None);
        };

        let sending_identifier =
            self.keyed_archiver_string(account_pk, "SendingAccountIdentifier")?;
        let Some(sending_identifier) = sending_identifier else {
            return Ok(None);
        };

        let smtp_pk: Option<i64> = conn
            .query_row(
                "SELECT Z_PK FROM ZACCOUNT WHERE ZIDENTIFIER = ?1",
                [&sending_identifier],
                |row| row.get(0),
            )
            .optional()?;
        let Some(smtp_pk) = smtp_pk else {
            return Ok(None);
        };

        let hostname = match self.keyed_archiver_string(smtp_pk, "Hostname")? {
            Some(hostname) => hostname,
            None => {
                let kind = self.resolve(identifier)?.and_then(|account| account.kind);
                Self::fallback_smtp_hostname(kind).ok_or_else(|| {
                    AmxError::SendingSettingsUnresolvable {
                        identifier: identifier.to_string(),
                        reason: "stored Hostname is not a string, and no known-provider fallback \
                                 applies to this account kind"
                            .to_string(),
                    }
                })?
            }
        };

        let port = self
            .keyed_archiver_integer(smtp_pk, "PortNumber")?
            .and_then(|port| u16::try_from(port).ok())
            .unwrap_or(587);
        let ssl_enabled = self
            .keyed_archiver_bool(smtp_pk, "SSLEnabled")?
            .unwrap_or(true);

        Ok(Some(SendingSettings {
            hostname,
            port,
            ssl_enabled,
        }))
    }

    fn fallback_smtp_hostname(kind: Option<AccountKind>) -> Option<String> {
        match kind {
            Some(AccountKind::ICloud) => Some("smtp.mail.me.com".to_string()),
            _ => None,
        }
    }

    /// Reads `ZKEY = key`'s `ZVALUE` for `owner_pk`, decodes it as an NSKeyedArchiver plist, and
    /// returns the archive's root object (`$objects[1]` — `$objects[0]` is always `$null`).
    fn keyed_archiver_root(
        &self,
        owner_pk: i64,
        key: &str,
    ) -> Result<Option<plist::Value>, AmxError> {
        let raw: Option<Vec<u8>> = self
            .conn
            .as_connection()
            .query_row(
                "SELECT ZVALUE FROM ZACCOUNTPROPERTY WHERE ZOWNER = ?1 AND ZKEY = ?2",
                rusqlite::params![owner_pk, key],
                |row| row.get(0),
            )
            .optional()?;
        let Some(raw) = raw else {
            return Ok(None);
        };
        let Ok(archive) = plist::Value::from_reader(std::io::Cursor::new(raw)) else {
            return Ok(None);
        };
        let root = archive
            .as_dictionary()
            .and_then(|dict| dict.get("$objects"))
            .and_then(|objects| objects.as_array())
            .and_then(|objects| objects.get(1))
            .cloned();
        Ok(root)
    }

    fn keyed_archiver_string(&self, owner_pk: i64, key: &str) -> Result<Option<String>, AmxError> {
        Ok(self
            .keyed_archiver_root(owner_pk, key)?
            .and_then(|value| value.into_string()))
    }

    fn keyed_archiver_integer(&self, owner_pk: i64, key: &str) -> Result<Option<i64>, AmxError> {
        Ok(self
            .keyed_archiver_root(owner_pk, key)?
            .and_then(|value| value.as_signed_integer()))
    }

    fn keyed_archiver_bool(&self, owner_pk: i64, key: &str) -> Result<Option<bool>, AmxError> {
        Ok(self
            .keyed_archiver_root(owner_pk, key)?
            .and_then(|value| value.as_boolean()))
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
            CREATE TABLE ZACCOUNTPROPERTY (Z_PK INTEGER PRIMARY KEY, ZOWNER INTEGER, ZKEY TEXT, ZVALUE BLOB);

            INSERT INTO ZACCOUNTTYPE VALUES (1, 'com.apple.account.IMAP');
            INSERT INTO ZACCOUNTTYPE VALUES (2, 'com.apple.account.Exchange');
            INSERT INTO ZACCOUNTTYPE VALUES (3, 'com.apple.account.LocalOnMyMac');
            INSERT INTO ZACCOUNTTYPE VALUES (4, 'com.apple.account.SMTP');

            INSERT INTO ZACCOUNT VALUES (10, 'ICLOUD-UUID', NULL, NULL, 1);
            INSERT INTO ZACCOUNTPROPERTY (Z_PK, ZOWNER, ZKEY) VALUES (100, 10, 'UseMailDrop');

            INSERT INTO ZACCOUNT VALUES (20, 'GMAIL-UUID', NULL, NULL, 1);
            INSERT INTO ZACCOUNTPROPERTY (Z_PK, ZOWNER, ZKEY) VALUES (200, 20, 'Hostname');

            INSERT INTO ZACCOUNT VALUES (30, 'EXCHANGE-UUID', 'Work Exchange', NULL, 2);
            INSERT INTO ZACCOUNT VALUES (40, 'LOCAL-UUID', 'On My Mac', NULL, 3);
            "#,
        )
        .unwrap();
    }

    /// Encodes `value` the way this fixture stands in for an NSKeyedArchiver-serialized
    /// `ZVALUE`: a `$objects` array whose index 0 is always `$null` and whose index 1 is the
    /// archive's root object — the shape [`AccountResolver::keyed_archiver_root`] expects.
    fn keyed_archiver_blob(value: plist::Value) -> Vec<u8> {
        let mut objects = plist::Dictionary::new();
        objects.insert(
            "$objects".to_string(),
            plist::Value::Array(vec![plist::Value::String("$null".to_string()), value]),
        );
        let archive = plist::Value::Dictionary(objects);
        let mut bytes = Vec::new();
        archive.to_writer_binary(&mut bytes).unwrap();
        bytes
    }

    fn seed_sending_settings(path: &Path) {
        let conn = Connection::open(path).unwrap();
        conn.execute(
            "INSERT INTO ZACCOUNTPROPERTY (Z_PK, ZOWNER, ZKEY, ZVALUE) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![
                300,
                20,
                "SendingAccountIdentifier",
                keyed_archiver_blob(plist::Value::String("GMAIL-SMTP-UUID".to_string())),
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO ZACCOUNT VALUES (21, 'GMAIL-SMTP-UUID', NULL, NULL, 4)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO ZACCOUNTPROPERTY (Z_PK, ZOWNER, ZKEY, ZVALUE) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![
                301,
                21,
                "Hostname",
                keyed_archiver_blob(plist::Value::String("smtp.gmail.com".to_string())),
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO ZACCOUNTPROPERTY (Z_PK, ZOWNER, ZKEY, ZVALUE) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![
                302,
                21,
                "PortNumber",
                keyed_archiver_blob(plist::Value::Integer(587.into())),
            ],
        )
        .unwrap();

        conn.execute(
            "INSERT INTO ZACCOUNTPROPERTY (Z_PK, ZOWNER, ZKEY, ZVALUE) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![
                310,
                10,
                "SendingAccountIdentifier",
                keyed_archiver_blob(plist::Value::String("ICLOUD-SMTP-UUID".to_string())),
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO ZACCOUNT VALUES (11, 'ICLOUD-SMTP-UUID', NULL, NULL, 4)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO ZACCOUNTPROPERTY (Z_PK, ZOWNER, ZKEY, ZVALUE) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![
                311,
                11,
                "Hostname",
                keyed_archiver_blob(plist::Value::Integer(1.into())),
            ],
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

    #[test]
    fn gmail_sending_settings_resolve_from_the_store_directly() {
        let file = NamedTempFile::new().unwrap();
        seed_accounts_db(file.path());
        seed_sending_settings(file.path());
        let resolver = AccountResolver::open(file.path()).unwrap();
        let settings = resolver
            .sending_settings("GMAIL-UUID")
            .unwrap()
            .expect("gmail should resolve sending settings");
        assert_eq!(settings.hostname, "smtp.gmail.com");
        assert_eq!(settings.port, 587);
    }

    #[test]
    fn icloud_sending_settings_fall_back_to_the_known_provider_table() {
        let file = NamedTempFile::new().unwrap();
        seed_accounts_db(file.path());
        seed_sending_settings(file.path());
        let resolver = AccountResolver::open(file.path()).unwrap();
        let settings = resolver
            .sending_settings("ICLOUD-UUID")
            .unwrap()
            .expect("icloud's junk Hostname should fall back, not error");
        assert_eq!(settings.hostname, "smtp.mail.me.com");
        // PortNumber was never seeded for the iCloud SMTP row — falls back to 587.
        assert_eq!(settings.port, 587);
        assert!(settings.ssl_enabled);
    }

    #[test]
    fn account_with_no_sending_identifier_resolves_to_none() {
        let file = NamedTempFile::new().unwrap();
        seed_accounts_db(file.path());
        let resolver = AccountResolver::open(file.path()).unwrap();
        assert_eq!(resolver.sending_settings("EXCHANGE-UUID").unwrap(), None);
    }

    #[test]
    fn unresolvable_account_returns_none() {
        let file = NamedTempFile::new().unwrap();
        seed_accounts_db(file.path());
        let resolver = AccountResolver::open(file.path()).unwrap();
        assert_eq!(resolver.sending_settings("NOT-A-REAL-UUID").unwrap(), None);
    }
}
