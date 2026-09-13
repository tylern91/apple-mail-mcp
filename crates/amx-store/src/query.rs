//! Filters `messages` by mailbox, account, date range, and flags (§5.2).
//!
//! A mailbox or account filter that matches nothing returns
//! [`amx_core::AmxError::MailboxFilterUnmatched`] with fuzzy suggestions, rather than silently
//! succeeding with an empty result — an agent acting on a mistyped mailbox name needs to know it
//! was mistyped, not that the mailbox is simply empty.

use amx_core::AmxError;
use rusqlite::types::ToSql;

use crate::conn::RoConnection;
use crate::registry::MailboxRegistry;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageRow {
    pub rowid: i64,
    pub mailbox: i64,
    pub date_sent: Option<i64>,
    pub date_received: Option<i64>,
    pub read: bool,
    pub flagged: bool,
    pub deleted: bool,
}

/// Fuzzy suggestions carried on an unmatched filter, per [`amx_core::AmxError::MailboxFilterUnmatched`].
const SUGGESTION_COUNT: usize = 3;

#[derive(Debug, Clone, Default)]
pub struct MessageQuery {
    rowid: Option<i64>,
    mailbox_url: Option<String>,
    account_identifier: Option<String>,
    date_sent_from: Option<i64>,
    date_sent_to: Option<i64>,
    read: Option<bool>,
    flagged: Option<bool>,
    deleted: Option<bool>,
}

impl MessageQuery {
    pub fn new() -> Self {
        Self::default()
    }

    /// Restricts the query to a single message, e.g. for `get_message`/`list_attachments` — a
    /// mailbox/account filter set alongside this still applies, so a rowid outside the filtered
    /// set correctly yields no rows rather than bypassing the filter.
    pub fn rowid(mut self, rowid: i64) -> Self {
        self.rowid = Some(rowid);
        self
    }

    pub fn mailbox(mut self, url: impl Into<String>) -> Self {
        self.mailbox_url = Some(url.into());
        self
    }

    pub fn account(mut self, account_identifier: impl Into<String>) -> Self {
        self.account_identifier = Some(account_identifier.into());
        self
    }

    pub fn date_sent_range(mut self, from: Option<i64>, to: Option<i64>) -> Self {
        self.date_sent_from = from;
        self.date_sent_to = to;
        self
    }

    pub fn read(mut self, read: bool) -> Self {
        self.read = Some(read);
        self
    }

    pub fn flagged(mut self, flagged: bool) -> Self {
        self.flagged = Some(flagged);
        self
    }

    pub fn deleted(mut self, deleted: bool) -> Self {
        self.deleted = Some(deleted);
        self
    }

    pub fn execute(
        &self,
        conn: &RoConnection,
        registry: &MailboxRegistry,
    ) -> Result<Vec<MessageRow>, AmxError> {
        let mailbox_rowids = self.resolve_mailbox_rowids(registry)?;

        let mut clauses: Vec<String> = Vec::new();
        let mut params: Vec<Box<dyn ToSql>> = Vec::new();

        if let Some(rowid) = self.rowid {
            clauses.push("ROWID = ?".to_string());
            params.push(Box::new(rowid));
        }
        if let Some(rowids) = &mailbox_rowids {
            let placeholders = rowids.iter().map(|_| "?").collect::<Vec<_>>().join(", ");
            clauses.push(format!("mailbox IN ({placeholders})"));
            for rowid in rowids {
                params.push(Box::new(*rowid));
            }
        }
        if let Some(from) = self.date_sent_from {
            clauses.push("date_sent >= ?".to_string());
            params.push(Box::new(from));
        }
        if let Some(to) = self.date_sent_to {
            clauses.push("date_sent <= ?".to_string());
            params.push(Box::new(to));
        }
        if let Some(read) = self.read {
            clauses.push("read = ?".to_string());
            params.push(Box::new(read as i64));
        }
        if let Some(flagged) = self.flagged {
            clauses.push("flagged = ?".to_string());
            params.push(Box::new(flagged as i64));
        }
        if let Some(deleted) = self.deleted {
            clauses.push("deleted = ?".to_string());
            params.push(Box::new(deleted as i64));
        }

        let where_clause = if clauses.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", clauses.join(" AND "))
        };
        let sql = format!(
            "SELECT ROWID, mailbox, date_sent, date_received, read, flagged, deleted \
             FROM messages {where_clause}"
        );

        let mut stmt = conn.as_connection().prepare(&sql)?;
        let param_refs: Vec<&dyn ToSql> = params.iter().map(|p| p.as_ref()).collect();
        let rows = stmt.query_map(param_refs.as_slice(), |row| {
            Ok(MessageRow {
                rowid: row.get(0)?,
                mailbox: row.get(1)?,
                date_sent: row.get(2)?,
                date_received: row.get(3)?,
                read: row.get::<_, i64>(4)? != 0,
                flagged: row.get::<_, i64>(5)? != 0,
                deleted: row.get::<_, i64>(6)? != 0,
            })
        })?;

        rows.collect::<Result<Vec<_>, _>>().map_err(AmxError::from)
    }

    /// `None` means "no mailbox/account filter was set" (match every mailbox); `Some(rowids)` is
    /// the set the `IN (...)` clause restricts to.
    fn resolve_mailbox_rowids(
        &self,
        registry: &MailboxRegistry,
    ) -> Result<Option<Vec<i64>>, AmxError> {
        let mut rowids = match &self.mailbox_url {
            Some(url) => match registry.resolve(url) {
                Some(mailbox) => Some(vec![mailbox.rowid]),
                None => {
                    return Err(AmxError::MailboxFilterUnmatched {
                        requested: url.clone(),
                        available: registry.len(),
                        suggestions: registry.suggest(url, SUGGESTION_COUNT),
                    });
                }
            },
            None => None,
        };

        if let Some(account_identifier) = &self.account_identifier {
            let matches: Vec<i64> = registry
                .mailboxes_for_account(account_identifier)
                .map(|mailbox| mailbox.rowid)
                .collect();
            if matches.is_empty() {
                return Err(AmxError::MailboxFilterUnmatched {
                    requested: account_identifier.clone(),
                    available: registry.len(),
                    suggestions: registry.suggest(account_identifier, SUGGESTION_COUNT),
                });
            }
            rowids = Some(match rowids {
                Some(existing) => existing
                    .into_iter()
                    .filter(|r| matches.contains(r))
                    .collect(),
                None => matches,
            });
        }

        Ok(rowids)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn seed_store(conn: &Connection) {
        crate::fixtures::build_schema(conn).unwrap();
        conn.execute_batch(
            r#"
            INSERT INTO mailboxes (ROWID, url, total_count, unread_count, deleted_count)
            VALUES (1, 'imap://ACCOUNT-A/INBOX', 2, 1, 0);
            INSERT INTO mailboxes (ROWID, url, total_count, unread_count, deleted_count)
            VALUES (2, 'imap://ACCOUNT-B/INBOX', 1, 0, 0);

            INSERT INTO subjects (ROWID, subject) VALUES (1, 'hi');

            INSERT INTO messages (ROWID, subject, mailbox, date_sent, read, flagged, deleted)
            VALUES (10, 1, 1, 100, 0, 1, 0);
            INSERT INTO messages (ROWID, subject, mailbox, date_sent, read, flagged, deleted)
            VALUES (11, 1, 1, 200, 1, 0, 0);
            INSERT INTO messages (ROWID, subject, mailbox, date_sent, read, flagged, deleted)
            VALUES (12, 1, 2, 150, 0, 0, 0);
            "#,
        )
        .unwrap();
    }

    fn store_and_registry(path: &std::path::Path) -> (RoConnection, MailboxRegistry) {
        seed_store(&Connection::open(path).unwrap());
        let conn = RoConnection::open(path).unwrap();
        let registry = MailboxRegistry::load(&conn).unwrap();
        (conn, registry)
    }

    #[test]
    fn filters_by_mailbox() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let (conn, registry) = store_and_registry(file.path());

        let results = MessageQuery::new()
            .mailbox("imap://ACCOUNT-A/INBOX")
            .execute(&conn, &registry)
            .unwrap();
        assert_eq!(results.len(), 2);
        assert!(results.iter().all(|m| m.mailbox == 1));
    }

    #[test]
    fn filters_by_rowid() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let (conn, registry) = store_and_registry(file.path());

        let results = MessageQuery::new()
            .rowid(11)
            .execute(&conn, &registry)
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].rowid, 11);
    }

    #[test]
    fn filters_by_account() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let (conn, registry) = store_and_registry(file.path());

        let results = MessageQuery::new()
            .account("ACCOUNT-B")
            .execute(&conn, &registry)
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].rowid, 12);
    }

    #[test]
    fn filters_by_flagged_and_date_range() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let (conn, registry) = store_and_registry(file.path());

        let results = MessageQuery::new()
            .flagged(true)
            .date_sent_range(Some(50), Some(150))
            .execute(&conn, &registry)
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].rowid, 10);
    }

    #[test]
    fn unmatched_mailbox_returns_error_with_suggestions_not_empty_list() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let (conn, registry) = store_and_registry(file.path());

        let err = MessageQuery::new()
            .mailbox("imap://ACCOUNT-A/INBOKS")
            .execute(&conn, &registry)
            .unwrap_err();

        match err {
            AmxError::MailboxFilterUnmatched {
                available,
                suggestions,
                ..
            } => {
                assert_eq!(available, 2);
                assert!(!suggestions.is_empty());
                assert!(suggestions.contains(&"imap://ACCOUNT-A/INBOX".to_string()));
            }
            other => panic!("expected MailboxFilterUnmatched, got {other:?}"),
        }
    }
}
