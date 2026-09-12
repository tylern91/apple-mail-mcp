//! `meta.sqlite` — `amx-index`'s own bookkeeping database (writer lock here; the quarantine
//! table, task 5, joins it later). Distinct from Apple Mail's read-only Envelope Index: this
//! database is created and owned by `amx-index`, and is read-write.

use std::path::Path;

use amx_core::AmxError;
use rusqlite::Connection;

const SCHEMA_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS writer_lock (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    pid INTEGER NOT NULL,
    process TEXT NOT NULL,
    since INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS quarantine (
    message_rowid INTEGER PRIMARY KEY,
    path TEXT NOT NULL,
    error_kind TEXT NOT NULL,
    first_seen INTEGER NOT NULL,
    attempts INTEGER NOT NULL DEFAULT 1
);
"#;

/// Opens (creating if absent) the meta database at `path` and ensures its schema exists.
pub fn open(path: &Path) -> Result<Connection, AmxError> {
    let conn = Connection::open(path)?;
    conn.execute_batch(SCHEMA_SQL)?;
    Ok(conn)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opening_twice_is_idempotent() {
        let file = tempfile::NamedTempFile::new().unwrap();
        open(file.path()).unwrap();
        open(file.path()).unwrap();
    }
}
