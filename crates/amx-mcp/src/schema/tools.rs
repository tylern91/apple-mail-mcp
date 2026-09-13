//! Request/response schemas for the 12 read tools + `doctor`/`status` (umbrella §4.3.1).
//!
//! Every type here derives `schemars::JsonSchema` so the MCP tool registration (task 9) can
//! publish a JSON schema without hand-writing one. Handlers that populate these types are built
//! in tasks 4-8; this module only fixes the wire shape.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::coverage::Coverage;
use super::health::HealthStatus;

// ---- search_messages (task 4) ----

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct SearchMessagesRequest {
    pub query: String,
    pub mailbox: Option<String>,
    pub account: Option<String>,
    pub sender: Option<String>,
    pub date_sent_from: Option<i64>,
    pub date_sent_to: Option<i64>,
    #[serde(default)]
    pub limit: Option<usize>,
    #[serde(default)]
    pub offset: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SearchHit {
    pub rowid: i64,
    pub score: f32,
    pub subject: Option<String>,
    pub sender: Option<String>,
    pub mailbox: String,
    pub account: String,
    pub date_sent: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SearchMessagesResponse {
    pub hits: Vec<SearchHit>,
}

// ---- get_message / list_attachments / get_attachment / extract_attachment_text (task 5) ----

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GetMessageRequest {
    pub rowid: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GetMessageResponse {
    pub rowid: i64,
    pub subject: Option<String>,
    pub sender: Option<String>,
    pub recipients: Vec<String>,
    pub date_sent: Option<i64>,
    pub date_received: Option<i64>,
    pub mailbox: String,
    pub account: String,
    pub read: bool,
    pub flagged: bool,
    pub body: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ListAttachmentsRequest {
    pub rowid: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AttachmentSummary {
    pub name: String,
    pub content_type: Option<String>,
    pub declared_bytes: u64,
    pub on_disk_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ListAttachmentsResponse {
    pub attachments: Vec<AttachmentSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GetAttachmentRequest {
    pub rowid: i64,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GetAttachmentResponse {
    pub name: String,
    pub content_type: Option<String>,
    /// Base64-encoded attachment bytes.
    pub bytes_base64: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ExtractAttachmentTextRequest {
    pub rowid: i64,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ExtractAttachmentTextResponse {
    pub name: String,
    pub text: String,
}

// ---- get_thread / get_message_links / count_messages / recent_messages / resolve_address /
//      list_accounts / list_mailboxes (task 6) ----

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GetThreadRequest {
    pub thread_id: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ThreadMessageSummary {
    pub rowid: i64,
    pub subject: Option<String>,
    pub sender: Option<String>,
    pub date_sent: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GetThreadResponse {
    pub thread_id: i64,
    pub messages: Vec<ThreadMessageSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GetMessageLinksRequest {
    pub rowid: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GetMessageLinksResponse {
    pub message_id: Option<String>,
    pub in_reply_to: Option<String>,
    pub references: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct CountMessagesRequest {
    pub mailbox: Option<String>,
    pub account: Option<String>,
    pub read: Option<bool>,
    pub flagged: Option<bool>,
    pub date_sent_from: Option<i64>,
    pub date_sent_to: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CountMessagesResponse {
    pub count: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct RecentMessagesRequest {
    pub mailbox: Option<String>,
    pub account: Option<String>,
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MessageSummary {
    pub rowid: i64,
    pub subject: Option<String>,
    pub sender: Option<String>,
    pub date_sent: Option<i64>,
    pub mailbox: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RecentMessagesResponse {
    pub messages: Vec<MessageSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ResolveAddressRequest {
    pub address: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ResolveAddressResponse {
    pub address: String,
    pub display_name: Option<String>,
    pub accounts: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct ListAccountsRequest {}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AccountSummary {
    pub identifier: String,
    pub display_name: String,
    /// `None` for accounts with no remote protocol (e.g. "On My Mac").
    pub kind: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ListAccountsResponse {
    pub accounts: Vec<AccountSummary>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct ListMailboxesRequest {
    pub account: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MailboxSummary {
    pub url: String,
    pub total: i64,
    pub unread: i64,
    pub deleted: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ListMailboxesResponse {
    pub mailboxes: Vec<MailboxSummary>,
}

// ---- set_read_state / set_flag (Phase 4 task 3) ----

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SetReadStateRequest {
    pub rowid: i64,
    pub read: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SetReadStateResponse {
    pub rowid: i64,
    pub read: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SetFlagRequest {
    pub rowid: i64,
    pub flagged: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SetFlagResponse {
    pub rowid: i64,
    pub flagged: bool,
}

// ---- move_messages / trash_messages (Phase 4 task 4) ----

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MoveMessagesRequest {
    pub rowid: i64,
    pub destination_mailbox: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MoveMessagesResponse {
    pub rowid: i64,
    /// The rowid the message now has — Apple Mail does not always preserve `ROWID` across a
    /// move, so this may differ from the request's `rowid`. Callers must re-address the message
    /// by this value, not the original.
    pub new_rowid: i64,
    pub mailbox: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TrashMessagesRequest {
    pub rowid: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TrashMessagesResponse {
    pub rowid: i64,
    /// See [`MoveMessagesResponse::new_rowid`].
    pub new_rowid: i64,
    pub mailbox: String,
}

// ---- triage_plan (Phase 4 task 6) / triage_apply (Phase 4 task 7) ----

/// The operation a triage plan will apply to every item once `triage_apply` runs it. Kept as a
/// closed set (not a free-form JXA request) so a frozen plan's hash is meaningful — hashing an
/// arbitrary operation payload would let two semantically-identical plans hash differently.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TriageOperation {
    SetReadState { read: bool },
    SetFlag { flagged: bool },
    Move { destination_mailbox: String },
    Trash,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TriagePlanRequest {
    /// The ordered rowids the plan covers. Order is part of the frozen, hashed content.
    pub rowids: Vec<i64>,
    pub operation: TriageOperation,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TriagePlanResponse {
    /// SHA-256 (hex) over the plan's rowids and operation. Pass this to `triage_apply`.
    pub plan_hash: String,
    pub rowids: Vec<i64>,
    pub operation: TriageOperation,
}

// ---- doctor / status (tasks 7-8) ----
//
// These two diagnostics already report health/coverage as their own subject matter, so unlike
// the read tools above they are not wrapped in `Envelope`/`SearchEnvelope` — they carry
// `store_access` and `coverage` as first-class fields instead of a generic stamp.

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct DoctorRequest {}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DoctorResponse {
    pub store_access: HealthStatus,
    /// Set once a transition into the current `store_access` state has been recorded (task 7).
    pub degraded_since: Option<String>,
    pub account_count: usize,
    pub mailbox_count: usize,
    pub coverage: Coverage,
    pub index_writer_busy: Option<String>,
    pub quarantined_count: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct StatusRequest {}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct StatusResponse {
    pub store_access: HealthStatus,
    pub degraded_since: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use schemars::schema_for;

    /// Task 3's proof obligation: every request/response type in the 14-tool contract must
    /// actually derive a JSON schema, not just compile as a Rust struct.
    #[test]
    fn every_tool_request_and_response_type_generates_a_json_schema() {
        macro_rules! assert_schema {
            ($ty:ty) => {
                let schema = schema_for!($ty);
                assert!(
                    serde_json::to_value(&schema).is_ok(),
                    "{} did not produce a serializable schema",
                    stringify!($ty)
                );
            };
        }

        assert_schema!(SearchMessagesRequest);
        assert_schema!(SearchMessagesResponse);
        assert_schema!(GetMessageRequest);
        assert_schema!(GetMessageResponse);
        assert_schema!(ListAttachmentsRequest);
        assert_schema!(ListAttachmentsResponse);
        assert_schema!(GetAttachmentRequest);
        assert_schema!(GetAttachmentResponse);
        assert_schema!(ExtractAttachmentTextRequest);
        assert_schema!(ExtractAttachmentTextResponse);
        assert_schema!(GetThreadRequest);
        assert_schema!(GetThreadResponse);
        assert_schema!(GetMessageLinksRequest);
        assert_schema!(GetMessageLinksResponse);
        assert_schema!(CountMessagesRequest);
        assert_schema!(CountMessagesResponse);
        assert_schema!(RecentMessagesRequest);
        assert_schema!(RecentMessagesResponse);
        assert_schema!(ResolveAddressRequest);
        assert_schema!(ResolveAddressResponse);
        assert_schema!(ListAccountsRequest);
        assert_schema!(ListAccountsResponse);
        assert_schema!(ListMailboxesRequest);
        assert_schema!(ListMailboxesResponse);
        assert_schema!(SetReadStateRequest);
        assert_schema!(SetReadStateResponse);
        assert_schema!(SetFlagRequest);
        assert_schema!(SetFlagResponse);
        assert_schema!(MoveMessagesRequest);
        assert_schema!(MoveMessagesResponse);
        assert_schema!(TrashMessagesRequest);
        assert_schema!(TrashMessagesResponse);
        assert_schema!(TriagePlanRequest);
        assert_schema!(TriagePlanResponse);
        assert_schema!(DoctorRequest);
        assert_schema!(DoctorResponse);
        assert_schema!(StatusRequest);
        assert_schema!(StatusResponse);
    }
}
