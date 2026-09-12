//! Full-snapshot reconciliation (umbrella §4.2.2) — the **only** code path allowed to emit a
//! Tantivy delete. parasxos #2 deleted 12,499 live rows because the deletion path ran against a
//! rowid subset instead of the whole table; the fix here is structural rather than a convention
//! to remember: [`Reconciler::reconcile`] always runs the same unfiltered
//! `SELECT ROWID FROM messages WHERE deleted = 0` inside one read transaction, has no parameter a
//! caller could narrow that query through, and materializes the full snapshot into a `HashSet`
//! before issuing a single delete — so a snapshot read that fails partway through (lock
//! contention, I/O error) surfaces as `Err` with zero deletes made, never a partial delete pass.

use std::collections::HashSet;

use amx_core::AmxError;
use amx_store::conn::RoConnection;
use tantivy::{IndexWriter, Searcher, Term};

use crate::schema::Fields;

pub struct Reconciler;

impl Reconciler {
    /// Deletes every indexed document whose `rowid` is absent from `conn`'s current full
    /// snapshot. Returns the number of documents deleted — `0` both when nothing needed deleting
    /// and, indistinguishably to the caller, when the snapshot read failed (in which case this
    /// returns `Err` instead, never a partial count).
    pub fn reconcile(
        conn: &RoConnection,
        searcher: &Searcher,
        writer: &mut IndexWriter,
        fields: &Fields,
    ) -> Result<usize, AmxError> {
        let live_rowids = Self::snapshot_live_rowids(conn)?;
        let indexed_rowids = Self::collect_indexed_rowids(searcher)?;

        let mut deleted = 0;
        for rowid in indexed_rowids {
            if !live_rowids.contains(&rowid) {
                writer.delete_term(Term::from_field_i64(fields.rowid, rowid));
                deleted += 1;
            }
        }
        if deleted > 0 {
            writer.commit()?;
        }
        Ok(deleted)
    }

    /// The full, unfiltered snapshot — deliberately the only query this module ever runs against
    /// `messages`. Runs inside one transaction so SQLite gives a single consistent view for its
    /// whole duration, and every row is collected before returning `Ok`; any read error aborts
    /// with nothing collected rather than an incomplete set silently standing in as complete.
    fn snapshot_live_rowids(conn: &RoConnection) -> Result<HashSet<i64>, AmxError> {
        let sql = conn.as_connection();
        let tx = sql.unchecked_transaction()?;
        let mut stmt = tx.prepare("SELECT ROWID FROM messages WHERE deleted = 0")?;
        let rows = stmt.query_map([], |row| row.get::<_, i64>(0))?;

        let mut ids = HashSet::new();
        for row in rows {
            ids.insert(row?);
        }
        Ok(ids)
    }

    /// Reads the `rowid` field by its schema-fixed name rather than through [`Fields`] --
    /// `FastFieldReaders` is a string-keyed API, so there is no typed handle to thread through
    /// here the way `Term::from_field_i64` uses `fields.rowid` above.
    fn collect_indexed_rowids(searcher: &Searcher) -> Result<HashSet<i64>, AmxError> {
        let mut ids = HashSet::new();
        for segment_reader in searcher.segment_readers() {
            let rowid_column = segment_reader.fast_fields().i64("rowid")?;
            for doc_id in segment_reader.doc_ids_alive() {
                if let Some(rowid) = rowid_column.first(doc_id) {
                    ids.insert(rowid);
                }
            }
        }
        Ok(ids)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{build_schema, register_tokenizers};
    use amx_store::fixtures::{build_schema as build_store_schema, load_fixture};
    use rusqlite::Connection as RawConnection;
    use tantivy::doc;

    const RECONCILE_PARTIAL_SQL: &str =
        include_str!("../../amx-store/tests/fixtures/store/reconcile_partial.sql");

    fn index_with_rowids(dir: &std::path::Path, rowids: &[i64]) -> (tantivy::Index, Fields) {
        let (schema, fields) = build_schema();
        let index = tantivy::Index::create_in_dir(dir, schema).unwrap();
        register_tokenizers(&index);
        let mut writer = index.writer(15_000_000).unwrap();
        for &rowid in rowids {
            writer.add_document(doc!(fields.rowid => rowid)).unwrap();
        }
        writer.commit().unwrap();
        (index, fields)
    }

    #[test]
    fn a_vanished_rowid_is_deleted_and_a_live_one_survives() {
        let store_file = tempfile::NamedTempFile::new().unwrap();
        let store_conn = RawConnection::open(store_file.path()).unwrap();
        build_store_schema(&store_conn).unwrap();
        load_fixture(&store_conn, RECONCILE_PARTIAL_SQL).unwrap();
        // Row 2 exists in the index below but not in the store snapshot -- it has vanished.
        drop(store_conn);

        let index_dir = tempfile::tempdir().unwrap();
        let (index, fields) = index_with_rowids(index_dir.path(), &[1, 2]);
        let mut writer = index.writer(15_000_000).unwrap();
        let reader = index.reader().unwrap();

        let conn = RoConnection::open(store_file.path()).unwrap();
        let deleted =
            Reconciler::reconcile(&conn, &reader.searcher(), &mut writer, &fields).unwrap();
        assert_eq!(deleted, 1);

        reader.reload().unwrap();
        let searcher = reader.searcher();
        assert_eq!(searcher.num_docs(), 1);

        let remaining_rowid = searcher
            .segment_readers()
            .iter()
            .flat_map(|sr| {
                let column = sr.fast_fields().i64("rowid").unwrap();
                sr.doc_ids_alive()
                    .filter_map(move |doc_id| column.first(doc_id))
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        assert_eq!(remaining_rowid, vec![1]);
    }

    /// parasxos #2 regression: a reconcile whose snapshot read cannot complete -- here, the
    /// `messages` table it depends on is gone -- must abort with zero deletions rather than act
    /// on whatever it read before the failure. `snapshot_live_rowids` collects every row before
    /// returning `Ok`, so any read error short-circuits with nothing collected; this exercises
    /// exactly that path rather than assuming it from the code alone.
    #[test]
    fn a_snapshot_read_that_fails_deletes_nothing() {
        let store_file = tempfile::NamedTempFile::new().unwrap();
        let store_conn = RawConnection::open(store_file.path()).unwrap();
        build_store_schema(&store_conn).unwrap();
        load_fixture(&store_conn, RECONCILE_PARTIAL_SQL).unwrap();
        store_conn.execute_batch("DROP TABLE messages;").unwrap();
        drop(store_conn);

        let index_dir = tempfile::tempdir().unwrap();
        let (index, fields) = index_with_rowids(index_dir.path(), &[1, 2]);
        let mut writer = index.writer(15_000_000).unwrap();
        let reader = index.reader().unwrap();

        let conn = RoConnection::open(store_file.path()).unwrap();
        let result = Reconciler::reconcile(&conn, &reader.searcher(), &mut writer, &fields);
        assert!(result.is_err());

        reader.reload().unwrap();
        assert_eq!(reader.searcher().num_docs(), 2);
    }
}
