//! `triage_plan` (Phase 4 task 6) — freezes an ordered rowid list plus a requested
//! [`TriageOperation`] into a content-addressed plan that `triage_apply` (task 7) can later look
//! up by hash and apply. Touches nothing in Mail.app: this is pure computation plus a
//! `meta.sqlite` write.

use std::time::{SystemTime, UNIX_EPOCH};

use amx_core::AmxError;
use rusqlite::Connection;
use sha2::{Digest, Sha256};

use crate::schema::tools::{TriageOperation, TriagePlanRequest, TriagePlanResponse};

/// Computes the plan's content-addressed hash over its rowids and operation, then persists it in
/// `meta.sqlite` so `triage_apply` can look it up on a separate connection.
pub fn run_triage_plan(
    meta_conn: &Connection,
    request: TriagePlanRequest,
) -> Result<TriagePlanResponse, AmxError> {
    let rowids_json =
        serde_json::to_string(&request.rowids).expect("Vec<i64> is always JSON-serializable");
    let operation_json = serde_json::to_string(&request.operation)
        .expect("TriageOperation is always JSON-serializable");

    let mut hasher = Sha256::new();
    hasher.update(rowids_json.as_bytes());
    hasher.update(operation_json.as_bytes());
    let plan_hash = format!("{:x}", hasher.finalize());

    let created_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    amx_index::meta::save_triage_plan(
        meta_conn,
        &plan_hash,
        &rowids_json,
        &operation_json,
        created_at,
    )?;

    Ok(TriagePlanResponse {
        plan_hash,
        rowids: request.rowids,
        operation: request.operation,
    })
}

/// Loads a frozen plan by its content hash and deserializes it back into `triage_apply`'s
/// working types. An unknown hash (never frozen, or mistyped) fails closed rather than applying
/// a guessed-at operation to a caller-supplied rowid list.
pub fn load_plan(
    meta_conn: &Connection,
    plan_hash: &str,
) -> Result<(Vec<i64>, TriageOperation), AmxError> {
    let (rowids_json, operation_json) = amx_index::meta::load_triage_plan(meta_conn, plan_hash)?
        .ok_or_else(|| AmxError::TriagePlanNotFound {
            hash: plan_hash.to_string(),
        })?;

    let rowids: Vec<i64> = serde_json::from_str(&rowids_json)
        .expect("rowids_json was produced by run_triage_plan's own serialization");
    let operation: TriageOperation = serde_json::from_str(&operation_json)
        .expect("operation_json was produced by run_triage_plan's own serialization");
    Ok((rowids, operation))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Keeps the backing tempfile alive alongside the `Connection` — dropping it deletes the
    /// file out from under an open connection, which surfaces as a spurious "readonly database"
    /// write error rather than an obviously-missing-file one.
    fn meta_conn() -> (tempfile::NamedTempFile, Connection) {
        let file = tempfile::NamedTempFile::new().unwrap();
        let conn = amx_index::meta::open(file.path()).unwrap();
        (file, conn)
    }

    #[test]
    fn the_same_rowids_and_operation_hash_identically() {
        let (_file, conn) = meta_conn();
        let request = || TriagePlanRequest {
            rowids: vec![1, 2, 3],
            operation: TriageOperation::SetFlag { flagged: true },
        };

        let first = run_triage_plan(&conn, request()).unwrap();
        let second = run_triage_plan(&conn, request()).unwrap();
        assert_eq!(first.plan_hash, second.plan_hash);
    }

    #[test]
    fn a_different_rowid_order_hashes_differently() {
        let (_file, conn) = meta_conn();
        let forward = run_triage_plan(
            &conn,
            TriagePlanRequest {
                rowids: vec![1, 2],
                operation: TriageOperation::Trash,
            },
        )
        .unwrap();
        let reversed = run_triage_plan(
            &conn,
            TriagePlanRequest {
                rowids: vec![2, 1],
                operation: TriageOperation::Trash,
            },
        )
        .unwrap();
        assert_ne!(forward.plan_hash, reversed.plan_hash);
    }

    #[test]
    fn a_different_operation_hashes_differently() {
        let (_file, conn) = meta_conn();
        let read = run_triage_plan(
            &conn,
            TriagePlanRequest {
                rowids: vec![1],
                operation: TriageOperation::SetReadState { read: true },
            },
        )
        .unwrap();
        let unread = run_triage_plan(
            &conn,
            TriagePlanRequest {
                rowids: vec![1],
                operation: TriageOperation::SetReadState { read: false },
            },
        )
        .unwrap();
        assert_ne!(read.plan_hash, unread.plan_hash);
    }

    #[test]
    fn the_plan_is_loadable_from_meta_by_its_hash() {
        let (_file, conn) = meta_conn();
        let response = run_triage_plan(
            &conn,
            TriagePlanRequest {
                rowids: vec![42, 43],
                operation: TriageOperation::Move {
                    destination_mailbox: "AMX-TEST-DEST".to_string(),
                },
            },
        )
        .unwrap();

        let (rowids_json, operation_json) =
            amx_index::meta::load_triage_plan(&conn, &response.plan_hash)
                .unwrap()
                .expect("plan was just saved");
        assert_eq!(rowids_json, "[42,43]");
        assert!(operation_json.contains("AMX-TEST-DEST"));
    }

    #[test]
    fn load_plan_round_trips_the_frozen_types() {
        let (_file, conn) = meta_conn();
        let response = run_triage_plan(
            &conn,
            TriagePlanRequest {
                rowids: vec![7, 8, 9],
                operation: TriageOperation::SetReadState { read: false },
            },
        )
        .unwrap();

        let (rowids, operation) = load_plan(&conn, &response.plan_hash).unwrap();
        assert_eq!(rowids, vec![7, 8, 9]);
        assert_eq!(operation, TriageOperation::SetReadState { read: false });
    }

    #[test]
    fn load_plan_rejects_an_unknown_hash() {
        let (_file, conn) = meta_conn();
        let err = load_plan(&conn, "not-a-real-hash").unwrap_err();
        assert!(matches!(err, AmxError::TriagePlanNotFound { .. }));
    }
}
