//! Per-file isolation for parse failures (imdinu #125): a batch continues past any single
//! failing message rather than aborting the whole pass. Failures are recorded in `meta.sqlite`'s
//! `quarantine` table (`meta.rs`) so a later listing/retry pass can find them without
//! re-parsing every message first.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use amx_core::{AmxError, ParseErrorKind};
use amx_parse::{ParsedMessage, parse_emlx};
use rusqlite::{Connection, params};

pub struct BatchItem {
    pub rowid: i64,
    pub path: PathBuf,
    pub bytes: Vec<u8>,
}

/// One item's outcome from [`run_batch`] — the message it parsed, or `None` if it was
/// quarantined instead.
pub struct BatchOutcome {
    pub rowid: i64,
    pub parsed: Option<ParsedMessage>,
}

/// Parses every item in `batch` against `conn` (a `meta.sqlite` connection), quarantining —
/// never aborting on — any single failure. Returns one [`BatchOutcome`] per input item, in
/// order.
pub fn run_batch(conn: &Connection, batch: &[BatchItem]) -> Result<Vec<BatchOutcome>, AmxError> {
    let mut outcomes = Vec::with_capacity(batch.len());
    for item in batch {
        match parse_emlx(&item.path, &item.bytes) {
            Ok(parsed) => outcomes.push(BatchOutcome {
                rowid: item.rowid,
                parsed: Some(parsed),
            }),
            Err(err) => {
                record(conn, item.rowid, &item.path, ParseErrorKind::from(&err))?;
                outcomes.push(BatchOutcome {
                    rowid: item.rowid,
                    parsed: None,
                });
            }
        }
    }
    Ok(outcomes)
}

fn record(
    conn: &Connection,
    rowid: i64,
    path: &Path,
    error_kind: ParseErrorKind,
) -> Result<(), AmxError> {
    conn.execute(
        "INSERT INTO quarantine (message_rowid, path, error_kind, first_seen, attempts)
         VALUES (?1, ?2, ?3, ?4, 1)
         ON CONFLICT(message_rowid) DO UPDATE SET attempts = attempts + 1",
        params![
            rowid,
            path.to_string_lossy(),
            error_kind_str(error_kind),
            unix_now()
        ],
    )?;
    Ok(())
}

fn error_kind_str(kind: ParseErrorKind) -> &'static str {
    match kind {
        ParseErrorKind::MalformedEnvelope => "malformed_envelope",
        ParseErrorKind::InvalidMessage => "invalid_message",
        ParseErrorKind::MalformedFooter => "malformed_footer",
        ParseErrorKind::Io => "io",
    }
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meta;

    fn fixture(name: &str) -> Vec<u8> {
        let path = format!(
            "{}/../amx-parse/tests/fixtures/emlx/{name}",
            env!("CARGO_MANIFEST_DIR")
        );
        std::fs::read(path).unwrap()
    }

    /// A malformed-footer message, built the same way `amx-parse`'s own
    /// `malformed_footer_is_reported_distinctly` test does — `date_header_raw_byte.emlx` was
    /// considered as the bad fixture here (imdinu #125's original repro), but `mail_parser`
    /// 0.11.9 lossily recovers a `Date:` header containing a raw byte and returns `Ok`, so it
    /// cannot exercise the quarantine path; a message with no valid plist footer reliably does.
    fn malformed_footer_bytes() -> Vec<u8> {
        let message = b"Subject: bad\n\nbody";
        let mut bytes = format!("{:<10}\n", message.len()).into_bytes();
        bytes.extend_from_slice(message);
        bytes.extend_from_slice(b"not a plist");
        bytes
    }

    #[test]
    fn a_batch_with_one_bad_message_indexes_the_good_ones_and_quarantines_exactly_one() {
        let dir = tempfile::tempdir().unwrap();
        let conn = meta::open(&dir.path().join("meta.sqlite")).unwrap();

        let batch = vec![
            BatchItem {
                rowid: 1,
                path: "well_formed.emlx".into(),
                bytes: fixture("well_formed.emlx"),
            },
            BatchItem {
                rowid: 2,
                path: "bad-footer.emlx".into(),
                bytes: malformed_footer_bytes(),
            },
            BatchItem {
                rowid: 3,
                path: "partial_no_attachment.emlx".into(),
                bytes: fixture("partial_no_attachment.emlx"),
            },
        ];

        let outcomes = run_batch(&conn, &batch).unwrap();
        assert_eq!(outcomes.len(), 3);
        let succeeded: Vec<_> = outcomes
            .iter()
            .filter(|o| o.parsed.is_some())
            .map(|o| o.rowid)
            .collect();
        assert_eq!(succeeded, vec![1, 3]);

        let quarantined_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM quarantine", [], |row| row.get(0))
            .unwrap();
        assert_eq!(quarantined_count, 1);

        let (quarantined_rowid, error_kind): (i64, String) = conn
            .query_row(
                "SELECT message_rowid, error_kind FROM quarantine",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(quarantined_rowid, 2);
        assert_eq!(error_kind, "malformed_footer");
    }

    #[test]
    fn recording_the_same_rowid_twice_bumps_attempts_instead_of_duplicating() {
        let dir = tempfile::tempdir().unwrap();
        let conn = meta::open(&dir.path().join("meta.sqlite")).unwrap();

        let batch = vec![BatchItem {
            rowid: 5,
            path: "bad-footer.emlx".into(),
            bytes: malformed_footer_bytes(),
        }];

        run_batch(&conn, &batch).unwrap();
        run_batch(&conn, &batch).unwrap();

        let (count, attempts): (i64, i64) = conn
            .query_row(
                "SELECT COUNT(*), MAX(attempts) FROM quarantine",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(count, 1);
        assert_eq!(attempts, 2);
    }
}
