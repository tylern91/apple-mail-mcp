//! The imdinu #66 regression: a `move_messages` call must leave the search index consistent
//! *before* it returns. This drives the same `tools::mutate::prepare_relocate`/`finish_move`
//! functions `server.rs`'s `move_messages` handler calls, against a synthetic envelope index +
//! on-disk `.emlx` fixture, then queries an `IndexReaderPool` immediately afterward (no sleep,
//! no extra reader-reload wait beyond the one `reload()` call the real handler itself makes) and
//! asserts the stale mailbox is gone. The test fails if `finish_move`'s `commit()` is ever moved
//! to run after the tool returns, or dropped altogether.

use std::fs;
use std::path::Path;

use amx_index::mutate::MutationWriter;
use amx_index::reader::IndexReaderPool;
use amx_index::schema::build_schema;
use amx_mcp::tools::mutate::{finish_move, prepare_relocate};
use amx_mcp::tools::resolve_message;
use amx_store::account::AccountResolver;
use amx_store::conn::RoConnection;
use amx_store::paths::EmlxPathResolver;
use amx_store::registry::MailboxRegistry;
use rusqlite::{Connection, params};
use tantivy::Term;
use tantivy::collector::Count;
use tantivy::query::TermQuery;
use tantivy::schema::IndexRecordOption;

const ACCOUNT: &str = "F00DCAFE-0000-4000-8000-000000000001";
const SOURCE_URL: &str = "local://F00DCAFE-0000-4000-8000-000000000001/AMX-TEST";
const DEST_URL: &str = "local://F00DCAFE-0000-4000-8000-000000000001/AMX-TEST-DEST";
const OLD_ROWID: i64 = 42;
const NEW_ROWID: i64 = 43;

const WELL_FORMED_EMLX: &[u8] =
    include_bytes!("../../amx-parse/tests/fixtures/emlx/well_formed.emlx");

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

        INSERT INTO ZACCOUNTTYPE VALUES (1, 'com.apple.account.LocalOnMyMac');
        "#,
    )
    .unwrap();
    conn.execute(
        "INSERT INTO ZACCOUNT (Z_PK, ZIDENTIFIER, ZACCOUNTDESCRIPTION, ZUSERNAME, ZACCOUNTTYPE) \
         VALUES (1, ?1, 'On My Mac', NULL, 1)",
        params![ACCOUNT],
    )
    .unwrap();
}

/// Writes `WELL_FORMED_EMLX` to `mailbox_dir`'s computed path for `rowid`, mirroring the layout
/// a real `.mbox/<GenerationUUID>` directory has (`amx_store::paths::EmlxPathResolver`).
fn write_emlx(mailbox_dir: &Path, rowid: i64) {
    let path = EmlxPathResolver::resolve(mailbox_dir, rowid, false);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, WELL_FORMED_EMLX).unwrap();
}

/// Builds one `.mbox/<GenerationUUID>` directory under `store_root/<ACCOUNT>` and returns it.
fn mailbox_dir(store_root: &Path, mailbox_name: &str) -> std::path::PathBuf {
    let dir = store_root
        .join(ACCOUNT)
        .join(format!("{mailbox_name}.mbox"))
        .join("11111111-1111-4111-8111-111111111111");
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn count_rowid(pool: &IndexReaderPool, fields: &amx_index::schema::Fields, rowid: i64) -> usize {
    let searcher = pool.searcher();
    let query = TermQuery::new(
        Term::from_field_i64(fields.rowid, rowid),
        IndexRecordOption::Basic,
    );
    searcher.search(&query, &Count).unwrap()
}

#[test]
fn a_move_leaves_no_ghost_result_immediately_after_it_returns() {
    let dir = tempfile::tempdir().unwrap();
    let store_root = dir.path().join("store");
    fs::create_dir_all(&store_root).unwrap();

    let source_dir = mailbox_dir(&store_root, "AMX-TEST");
    let dest_dir = mailbox_dir(&store_root, "AMX-TEST-DEST");
    write_emlx(&source_dir, OLD_ROWID);

    let accounts_db = dir.path().join("Accounts4.sqlite");
    seed_accounts_db(&accounts_db);
    let account_resolver = AccountResolver::open(&accounts_db).unwrap();

    let envelope_index = dir.path().join("Envelope Index");
    let rw_conn = Connection::open(&envelope_index).unwrap();
    amx_store::fixtures::build_schema(&rw_conn).unwrap();
    rw_conn
        .execute(
            "INSERT INTO mailboxes (ROWID, url, total_count, unread_count, deleted_count) \
             VALUES (1, ?1, 1, 0, 0)",
            params![SOURCE_URL],
        )
        .unwrap();
    rw_conn
        .execute(
            "INSERT INTO mailboxes (ROWID, url, total_count, unread_count, deleted_count) \
             VALUES (2, ?1, 0, 0, 0)",
            params![DEST_URL],
        )
        .unwrap();
    rw_conn
        .execute(
            "INSERT INTO subjects (ROWID, subject) VALUES (1, 'fixture subject')",
            [],
        )
        .unwrap();
    rw_conn
        .execute(
            "INSERT INTO messages (ROWID, subject, mailbox, date_sent, read, flagged, deleted) \
             VALUES (?1, 1, 1, 0, 0, 0, 0)",
            params![OLD_ROWID],
        )
        .unwrap();

    let ro_conn = RoConnection::open(&envelope_index).unwrap();
    let registry = MailboxRegistry::load(&ro_conn).unwrap();

    let index_dir = dir.path().join("index");
    fs::create_dir(&index_dir).unwrap();
    let meta_path = dir.path().join("meta.sqlite");

    // Seed the index with the message's pre-move document, as an `amxcli sync` run would have.
    let seed = resolve_message(
        &ro_conn,
        &registry,
        &store_root,
        &account_resolver,
        OLD_ROWID,
    )
    .unwrap();
    let mut writer = MutationWriter::open(&index_dir, &meta_path).unwrap();
    writer
        .upsert(
            OLD_ROWID,
            &seed.account_id,
            &seed.mailbox_url,
            &seed.classified,
        )
        .unwrap();
    writer.commit().unwrap();

    let pool = IndexReaderPool::open_or_create(&index_dir).unwrap();
    pool.reload().unwrap();
    let (_schema, fields) = build_schema();
    assert_eq!(count_rowid(&pool, &fields, OLD_ROWID), 1);

    // Snapshot the destination account's rowids *before* the move, as `move_messages`' handler
    // does ahead of its JXA round-trip.
    let prep = prepare_relocate(
        &ro_conn,
        &registry,
        &store_root,
        &account_resolver,
        OLD_ROWID,
        Some(ACCOUNT),
    )
    .unwrap();

    // Simulate what Mail.app's JXA `move` verb does to the store: relocate the `.emlx` file under
    // a newly-allocated rowid in the destination mailbox (§"ROWID does not survive a move",
    // confirmed empirically against `AMX-TEST` this phase) and drop the old envelope row.
    write_emlx(&dest_dir, NEW_ROWID);
    rw_conn
        .execute("DELETE FROM messages WHERE ROWID = ?1", params![OLD_ROWID])
        .unwrap();
    rw_conn
        .execute(
            "INSERT INTO messages (ROWID, subject, mailbox, date_sent, read, flagged, deleted) \
             VALUES (?1, 1, 2, 0, 0, 0, 0)",
            params![NEW_ROWID],
        )
        .unwrap();

    let response = finish_move(
        &ro_conn,
        &registry,
        &store_root,
        &account_resolver,
        &index_dir,
        &meta_path,
        OLD_ROWID,
        &prep.snapshot_account_id,
        &prep.before,
    )
    .unwrap();
    assert_eq!(response.new_rowid, NEW_ROWID);

    // The property under test: immediately after `finish_move` returns, with no sleep, the
    // caller's own reload (the one line `move_messages`' handler makes right after this call)
    // must already see the committed change — never a stale reader lagging the mutation.
    pool.reload().unwrap();
    assert_eq!(
        count_rowid(&pool, &fields, OLD_ROWID),
        0,
        "old rowid must not remain searchable once the move has been reconciled"
    );
    assert_eq!(count_rowid(&pool, &fields, NEW_ROWID), 1);
}
