use std::path::Path;

use amx_core::AmxError;
use rusqlite::{Connection, OpenFlags};

/// A read-only connection to any SQLite file matching the Envelope Index schema — not tied to
/// the live `~/Library/Mail/V10/MailData/Envelope Index` path, so fixtures on Linux CI work the
/// same way a real store does.
#[derive(Debug)]
pub struct RoConnection {
    conn: Connection,
}

impl RoConnection {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, AmxError> {
        let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        Ok(Self { conn })
    }

    pub fn as_connection(&self) -> &Connection {
        &self.conn
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_missing_file_errors() {
        let result = RoConnection::open("/nonexistent/envelope-index.sqlite");
        assert!(result.is_err());
    }
}
