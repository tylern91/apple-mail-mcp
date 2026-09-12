//! A schema-faithful synthetic Envelope Index, for tests and Linux CI that have no live
//! `~/Library/Mail/V10/MailData/Envelope Index` to read.
//!
//! The column set is verified against a live store's `.schema mailboxes` / `.schema messages` /
//! `.schema subjects`. Triggers and the duplicate-count bookkeeping tables (`labels`,
//! `duplicates_unread_count`, `message_global_data`, ...) are omitted: this phase is read-only
//! and never exercises the write path those triggers maintain.

use rusqlite::{Connection, Result};

pub const SCHEMA_SQL: &str = r#"
CREATE TABLE mailboxes (
    ROWID INTEGER PRIMARY KEY AUTOINCREMENT,
    url TEXT NOT NULL,
    total_count INTEGER NOT NULL DEFAULT 0,
    unread_count INTEGER NOT NULL DEFAULT 0,
    deleted_count INTEGER NOT NULL DEFAULT 0,
    change_identifier TEXT,
    UNIQUE(url)
);

CREATE TABLE subjects (
    ROWID INTEGER PRIMARY KEY AUTOINCREMENT,
    subject TEXT NOT NULL,
    UNIQUE(subject)
);

CREATE TABLE messages (
    ROWID INTEGER PRIMARY KEY AUTOINCREMENT,
    subject INTEGER NOT NULL,
    mailbox INTEGER NOT NULL,
    date_sent INTEGER,
    date_received INTEGER,
    flags INTEGER NOT NULL DEFAULT 0,
    read INTEGER NOT NULL DEFAULT 0,
    flagged INTEGER NOT NULL DEFAULT 0,
    deleted INTEGER NOT NULL DEFAULT 0
);
"#;

/// Creates the synthetic schema in `conn` — typically an in-memory connection for a unit test,
/// or a temp file for a fixture the macOS integration job also loads.
pub fn build_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(SCHEMA_SQL)
}

/// Loads a `.sql` fixture (`INSERT` statements matching [`SCHEMA_SQL`]) into `conn`.
pub fn load_fixture(conn: &Connection, sql: &str) -> Result<()> {
    conn.execute_batch(sql)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NULL_CHANGE_IDENTIFIER: &str =
        include_str!("../tests/fixtures/store/null_change_identifier.sql");

    #[test]
    fn null_change_identifier_fixture_loads() {
        let conn = Connection::open_in_memory().unwrap();
        build_schema(&conn).unwrap();
        load_fixture(&conn, NULL_CHANGE_IDENTIFIER).unwrap();

        let change_identifier: Option<String> = conn
            .query_row(
                "SELECT change_identifier FROM mailboxes WHERE url = ?1",
                ["imap://user@example.com/INBOX"],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(change_identifier, None);
    }
}
