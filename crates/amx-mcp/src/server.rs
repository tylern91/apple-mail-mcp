//! The `rmcp` server: shared state, the 14 read/diagnostic tools, and the `ToolLane`-filtered
//! `tools/list` (Phase 3 task 9).
//!
//! `rusqlite::Connection` is `Send` but not `Sync`, and `RoConnection`/`AccountResolver` each wrap
//! one — concurrent tool calls (the streamable-HTTP transport allows more than one in flight)
//! serialize through a `Mutex` around each. `MailboxRegistry` is a plain in-memory snapshot and
//! `IndexReaderPool`/tantivy's `Searcher` are already safe to share, so neither is locked.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use amx_core::AmxError;
use amx_core::config::Config;
use amx_index::reader::IndexReaderPool;
use amx_index::schema::{Fields, build_schema};
use amx_store::account::AccountResolver;
use amx_store::conn::RoConnection;
use amx_store::registry::MailboxRegistry;
use rmcp::ErrorData;
use rmcp::handler::server::wrapper::{Json, Parameters};
use rmcp::model::{ListToolsResult, PaginatedRequestParams, Tool};
use rmcp::service::RequestContext;
use rmcp::{RoleServer, ServerHandler, tool, tool_router};
use rusqlite::Connection;

use crate::error::to_error_data;
use crate::health_monitor::HealthMonitor;
use crate::schema::health::HealthStatus;
use crate::schema::tools::*;
use crate::schema::{Coverage, Envelope, SearchEnvelope};
use crate::tool_lane::{ToolDescriptor, ToolLane, visible_tools};
use crate::tools;

/// The read/diagnostic tool catalog backing `tools/list`'s `ToolLane` filter. The mutate lane
/// (Phase 4) is appended by [`catalog`] only on macOS, where `amx-automation` is buildable — the
/// Linux portable-CI job builds `amx-mcp` without it (`rust.yml`'s `check-portable` job).
const READ_CATALOG: &[ToolDescriptor] = &[
    ToolDescriptor {
        name: "search_messages",
        lane: ToolLane::Read,
        read_only_hint: true,
    },
    ToolDescriptor {
        name: "get_message",
        lane: ToolLane::Read,
        read_only_hint: true,
    },
    ToolDescriptor {
        name: "list_attachments",
        lane: ToolLane::Read,
        read_only_hint: true,
    },
    ToolDescriptor {
        name: "get_attachment",
        lane: ToolLane::Read,
        read_only_hint: true,
    },
    ToolDescriptor {
        name: "extract_attachment_text",
        lane: ToolLane::Read,
        read_only_hint: true,
    },
    ToolDescriptor {
        name: "get_thread",
        lane: ToolLane::Read,
        read_only_hint: true,
    },
    ToolDescriptor {
        name: "get_message_links",
        lane: ToolLane::Read,
        read_only_hint: true,
    },
    ToolDescriptor {
        name: "count_messages",
        lane: ToolLane::Read,
        read_only_hint: true,
    },
    ToolDescriptor {
        name: "recent_messages",
        lane: ToolLane::Read,
        read_only_hint: true,
    },
    ToolDescriptor {
        name: "resolve_address",
        lane: ToolLane::Read,
        read_only_hint: true,
    },
    ToolDescriptor {
        name: "list_accounts",
        lane: ToolLane::Read,
        read_only_hint: true,
    },
    ToolDescriptor {
        name: "list_mailboxes",
        lane: ToolLane::Read,
        read_only_hint: true,
    },
    ToolDescriptor {
        name: "doctor",
        lane: ToolLane::Diagnostic,
        read_only_hint: true,
    },
    ToolDescriptor {
        name: "status",
        lane: ToolLane::Diagnostic,
        read_only_hint: true,
    },
];

/// Mutate-lane tools, macOS-only since they call into `amx-automation`.
#[cfg(target_os = "macos")]
const MUTATE_CATALOG: &[ToolDescriptor] = &[
    ToolDescriptor {
        name: "set_read_state",
        lane: ToolLane::Mutate,
        read_only_hint: false,
    },
    ToolDescriptor {
        name: "set_flag",
        lane: ToolLane::Mutate,
        read_only_hint: false,
    },
    ToolDescriptor {
        name: "move_messages",
        lane: ToolLane::Mutate,
        read_only_hint: false,
    },
    ToolDescriptor {
        name: "trash_messages",
        lane: ToolLane::Mutate,
        read_only_hint: false,
    },
    ToolDescriptor {
        name: "triage_plan",
        lane: ToolLane::Mutate,
        read_only_hint: true,
    },
];

/// The full tool catalog for this platform build.
fn catalog() -> Vec<ToolDescriptor> {
    #[allow(unused_mut)]
    let mut all = READ_CATALOG.to_vec();
    #[cfg(target_os = "macos")]
    all.extend_from_slice(MUTATE_CATALOG);
    all
}

/// Derives `~/Library/Accounts/Accounts4.sqlite` as a sibling of `store_path`'s grandparent —
/// the same derivation `amxcli`'s `main.rs` uses; duplicated rather than shared since `amx-mcp`
/// cannot depend on `amx-cli` (the dependency runs the other way).
fn accounts_db_path(store_path: &Path) -> Result<PathBuf, AmxError> {
    store_path
        .parent()
        .and_then(Path::parent)
        .map(|library_dir| library_dir.join("Accounts").join("Accounts4.sqlite"))
        .ok_or_else(|| AmxError::StoreNotFound(store_path.to_path_buf()))
}

pub struct AppState {
    store_conn: Mutex<RoConnection>,
    meta_conn: Mutex<Connection>,
    account_resolver: Mutex<AccountResolver>,
    mailbox_registry: MailboxRegistry,
    index_pool: IndexReaderPool,
    fields: Fields,
    health: HealthMonitor,
    store_path: PathBuf,
    read_only: bool,
    /// Raw `<cache_dir>/index` and `<cache_dir>/meta.sqlite` paths — kept alongside `index_pool`
    /// (which owns a long-lived reader) so a mutate tool can open its own transient
    /// [`amx_index::mutate::MutationWriter`] without re-deriving `Config`'s path convention.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    index_dir: PathBuf,
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    meta_path: PathBuf,
}

impl AppState {
    /// Opens every backing store from `config`, mirroring `amxcli sync`'s path derivation
    /// (envelope index, `Accounts4.sqlite`, `<cache_dir>/index`, `<cache_dir>/meta.sqlite`).
    pub fn open(config: &Config) -> Result<Self, AmxError> {
        let store_path = config
            .store_path
            .clone()
            .ok_or_else(|| AmxError::StoreNotFound(PathBuf::from("<unconfigured>")))?;

        let envelope_index_path = store_path.join("MailData").join("Envelope Index");
        let store_conn = RoConnection::open(&envelope_index_path)?;
        let mailbox_registry = MailboxRegistry::load(&store_conn)?;

        let accounts_db = accounts_db_path(&store_path)?;
        let account_resolver = AccountResolver::open(&accounts_db)?;

        let index_dir = config.cache_dir.join("index");
        let meta_path = config.cache_dir.join("meta.sqlite");
        std::fs::create_dir_all(&index_dir).map_err(|source| {
            AmxError::Parse(amx_core::ParseError::Io {
                path: index_dir.clone(),
                source,
            })
        })?;

        let index_pool = IndexReaderPool::open_or_create(&index_dir)?;
        let (_schema, fields) = build_schema();
        let meta_conn = amx_index::meta::open(&meta_path)?;

        let health = HealthMonitor::start(store_path.clone(), &meta_conn);

        Ok(Self {
            store_conn: Mutex::new(store_conn),
            meta_conn: Mutex::new(meta_conn),
            account_resolver: Mutex::new(account_resolver),
            mailbox_registry,
            index_pool,
            fields,
            health,
            store_path,
            read_only: config.read_only,
            index_dir,
            meta_path,
        })
    }

    fn health_status(&self) -> HealthStatus {
        HealthStatus::from(&self.health.current())
    }
}

#[derive(Clone)]
pub struct AmxServer {
    state: Arc<AppState>,
    tool_router: rmcp::handler::server::router::tool::ToolRouter<Self>,
}

impl AmxServer {
    pub fn new(state: Arc<AppState>) -> Self {
        Self {
            state,
            tool_router: Self::tool_router(),
        }
    }

    fn envelope<T>(&self, result: T) -> Envelope<T> {
        Envelope::new(result, self.state.health_status())
    }

    fn search_envelope<T>(&self, result: T, coverage: Coverage) -> SearchEnvelope<T> {
        SearchEnvelope::new(result, self.state.health_status(), coverage)
    }

    fn resolve(&self, rowid: i64) -> Result<tools::ResolvedMessage, AmxError> {
        let conn = self.state.store_conn.lock().expect("store_conn poisoned");
        let resolver = self
            .state
            .account_resolver
            .lock()
            .expect("account_resolver poisoned");
        tools::resolve_message(
            &conn,
            &self.state.mailbox_registry,
            &self.state.store_path,
            &resolver,
            rowid,
        )
    }
}

#[tool_router(router = tool_router)]
impl AmxServer {
    #[tool(
        name = "search_messages",
        description = "Full-text search over mail with ranked hits and a coverage envelope."
    )]
    pub async fn search_messages(
        &self,
        params: Parameters<SearchMessagesRequest>,
    ) -> Result<Json<SearchEnvelope<SearchMessagesResponse>>, ErrorData> {
        let searcher = self.state.index_pool.searcher();
        let (response, coverage) = tools::run_search(
            &searcher,
            &self.state.fields,
            &self.state.mailbox_registry,
            &params.0,
        )
        .map_err(to_error_data)?;
        Ok(Json(self.search_envelope(response, coverage)))
    }

    #[tool(name = "get_message", description = "Fetch one message by rowid.")]
    pub async fn get_message(
        &self,
        params: Parameters<GetMessageRequest>,
    ) -> Result<Json<Envelope<GetMessageResponse>>, ErrorData> {
        let resolved = self.resolve(params.0.rowid).map_err(to_error_data)?;
        let response = tools::run_get_message(&resolved).map_err(to_error_data)?;
        Ok(Json(self.envelope(response)))
    }

    #[tool(
        name = "list_attachments",
        description = "List attachment metadata for one message."
    )]
    pub async fn list_attachments(
        &self,
        params: Parameters<ListAttachmentsRequest>,
    ) -> Result<Json<Envelope<ListAttachmentsResponse>>, ErrorData> {
        let resolved = self.resolve(params.0.rowid).map_err(to_error_data)?;
        let response = tools::run_list_attachments(&resolved).map_err(to_error_data)?;
        Ok(Json(self.envelope(response)))
    }

    #[tool(
        name = "get_attachment",
        description = "Fetch one attachment's bytes (base64) by message rowid and name."
    )]
    pub async fn get_attachment(
        &self,
        params: Parameters<GetAttachmentRequest>,
    ) -> Result<Json<Envelope<GetAttachmentResponse>>, ErrorData> {
        let resolved = self.resolve(params.0.rowid).map_err(to_error_data)?;
        let response =
            tools::run_get_attachment(&resolved, &params.0.name).map_err(to_error_data)?;
        Ok(Json(self.envelope(response)))
    }

    #[tool(
        name = "extract_attachment_text",
        description = "Extract plain text from one attachment, when a text extractor exists for its MIME type."
    )]
    pub async fn extract_attachment_text(
        &self,
        params: Parameters<ExtractAttachmentTextRequest>,
    ) -> Result<Json<Envelope<ExtractAttachmentTextResponse>>, ErrorData> {
        let resolved = self.resolve(params.0.rowid).map_err(to_error_data)?;
        let response =
            tools::run_extract_attachment_text(&resolved, &params.0.name).map_err(to_error_data)?;
        Ok(Json(self.envelope(response)))
    }

    #[tool(name = "get_thread", description = "Fetch every message in a thread.")]
    pub async fn get_thread(
        &self,
        params: Parameters<GetThreadRequest>,
    ) -> Result<Json<Envelope<GetThreadResponse>>, ErrorData> {
        let searcher = self.state.index_pool.searcher();
        let response = tools::run_get_thread(&searcher, &self.state.fields, params.0.thread_id)
            .map_err(to_error_data)?;
        Ok(Json(self.envelope(response)))
    }

    #[tool(
        name = "get_message_links",
        description = "Fetch a message's Message-ID/In-Reply-To/References."
    )]
    pub async fn get_message_links(
        &self,
        params: Parameters<GetMessageLinksRequest>,
    ) -> Result<Json<Envelope<GetMessageLinksResponse>>, ErrorData> {
        let resolved = self.resolve(params.0.rowid).map_err(to_error_data)?;
        let response = tools::run_get_message_links(&resolved).map_err(to_error_data)?;
        Ok(Json(self.envelope(response)))
    }

    #[tool(
        name = "count_messages",
        description = "Count messages matching mailbox/account/read/flagged/date filters."
    )]
    pub async fn count_messages(
        &self,
        params: Parameters<CountMessagesRequest>,
    ) -> Result<Json<Envelope<CountMessagesResponse>>, ErrorData> {
        let conn = self.state.store_conn.lock().expect("store_conn poisoned");
        let response = tools::run_count_messages(&conn, &self.state.mailbox_registry, &params.0)
            .map_err(to_error_data)?;
        Ok(Json(self.envelope(response)))
    }

    #[tool(
        name = "recent_messages",
        description = "The most recent messages in a mailbox/account, newest first."
    )]
    pub async fn recent_messages(
        &self,
        params: Parameters<RecentMessagesRequest>,
    ) -> Result<Json<Envelope<RecentMessagesResponse>>, ErrorData> {
        let conn = self.state.store_conn.lock().expect("store_conn poisoned");
        let resolver = self
            .state
            .account_resolver
            .lock()
            .expect("account_resolver poisoned");
        let response = tools::run_recent_messages(
            &conn,
            &self.state.mailbox_registry,
            &self.state.store_path,
            &resolver,
            &params.0,
        )
        .map_err(to_error_data)?;
        Ok(Json(self.envelope(response)))
    }

    #[tool(
        name = "resolve_address",
        description = "Resolve an email address to the account(s) it belongs to."
    )]
    pub async fn resolve_address(
        &self,
        params: Parameters<ResolveAddressRequest>,
    ) -> Result<Json<Envelope<ResolveAddressResponse>>, ErrorData> {
        let resolver = self
            .state
            .account_resolver
            .lock()
            .expect("account_resolver poisoned");
        let response = tools::run_resolve_address(&resolver, &params.0).map_err(to_error_data)?;
        Ok(Json(self.envelope(response)))
    }

    #[tool(
        name = "list_accounts",
        description = "List every account the store references."
    )]
    pub async fn list_accounts(&self) -> Result<Json<Envelope<ListAccountsResponse>>, ErrorData> {
        let resolver = self
            .state
            .account_resolver
            .lock()
            .expect("account_resolver poisoned");
        let response = tools::run_list_accounts(&self.state.mailbox_registry, &resolver)
            .map_err(to_error_data)?;
        Ok(Json(self.envelope(response)))
    }

    #[tool(
        name = "list_mailboxes",
        description = "List mailboxes, optionally filtered to one account."
    )]
    pub async fn list_mailboxes(
        &self,
        params: Parameters<ListMailboxesRequest>,
    ) -> Result<Json<Envelope<ListMailboxesResponse>>, ErrorData> {
        let response = tools::run_list_mailboxes(&self.state.mailbox_registry, &params.0)
            .map_err(to_error_data)?;
        Ok(Json(self.envelope(response)))
    }

    #[cfg(target_os = "macos")]
    #[tool(
        name = "set_read_state",
        description = "Mark a message read or unread in Mail.app, keeping the search index in sync.",
        annotations(read_only_hint = false)
    )]
    pub async fn set_read_state(
        &self,
        params: Parameters<SetReadStateRequest>,
    ) -> Result<Json<Envelope<SetReadStateResponse>>, ErrorData> {
        let rowid = params.0.rowid;
        let read = params.0.read;

        let addressed = {
            let conn = self.state.store_conn.lock().expect("store_conn poisoned");
            let resolver = self
                .state
                .account_resolver
                .lock()
                .expect("account_resolver poisoned");
            tools::mutate::resolve_and_address(
                &conn,
                &self.state.mailbox_registry,
                &self.state.store_path,
                &resolver,
                rowid,
            )
        }
        .map_err(to_error_data)?;

        amx_automation::locate::locate(
            addressed.mailbox.clone(),
            addressed.message_id.clone(),
            rowid,
        )
        .await
        .map_err(to_error_data)?;
        amx_automation::jxa::run(&amx_automation::JxaRequest::SetReadState {
            mailbox: addressed.mailbox,
            message_id: addressed.message_id,
            read,
        })
        .await
        .map_err(to_error_data)?;

        let response = {
            let conn = self.state.store_conn.lock().expect("store_conn poisoned");
            let resolver = self
                .state
                .account_resolver
                .lock()
                .expect("account_resolver poisoned");
            tools::mutate::finish_set_read_state(
                &conn,
                &self.state.mailbox_registry,
                &self.state.store_path,
                &resolver,
                &self.state.index_dir,
                &self.state.meta_path,
                rowid,
                read,
            )
        }
        .map_err(to_error_data)?;
        self.state.index_pool.reload().map_err(to_error_data)?;

        Ok(Json(self.envelope(response)))
    }

    #[cfg(target_os = "macos")]
    #[tool(
        name = "set_flag",
        description = "Flag or unflag a message in Mail.app, keeping the search index in sync.",
        annotations(read_only_hint = false)
    )]
    pub async fn set_flag(
        &self,
        params: Parameters<SetFlagRequest>,
    ) -> Result<Json<Envelope<SetFlagResponse>>, ErrorData> {
        let rowid = params.0.rowid;
        let flagged = params.0.flagged;

        let addressed = {
            let conn = self.state.store_conn.lock().expect("store_conn poisoned");
            let resolver = self
                .state
                .account_resolver
                .lock()
                .expect("account_resolver poisoned");
            tools::mutate::resolve_and_address(
                &conn,
                &self.state.mailbox_registry,
                &self.state.store_path,
                &resolver,
                rowid,
            )
        }
        .map_err(to_error_data)?;

        amx_automation::locate::locate(
            addressed.mailbox.clone(),
            addressed.message_id.clone(),
            rowid,
        )
        .await
        .map_err(to_error_data)?;
        amx_automation::jxa::run(&amx_automation::JxaRequest::SetFlag {
            mailbox: addressed.mailbox,
            message_id: addressed.message_id,
            flagged,
        })
        .await
        .map_err(to_error_data)?;

        let response = {
            let conn = self.state.store_conn.lock().expect("store_conn poisoned");
            let resolver = self
                .state
                .account_resolver
                .lock()
                .expect("account_resolver poisoned");
            tools::mutate::finish_set_flag(
                &conn,
                &self.state.mailbox_registry,
                &self.state.store_path,
                &resolver,
                &self.state.index_dir,
                &self.state.meta_path,
                rowid,
                flagged,
            )
        }
        .map_err(to_error_data)?;
        self.state.index_pool.reload().map_err(to_error_data)?;

        Ok(Json(self.envelope(response)))
    }

    #[cfg(target_os = "macos")]
    #[tool(
        name = "move_messages",
        description = "Move a message to another mailbox in Mail.app, keeping the search index in sync.",
        annotations(read_only_hint = false)
    )]
    pub async fn move_messages(
        &self,
        params: Parameters<MoveMessagesRequest>,
    ) -> Result<Json<Envelope<MoveMessagesResponse>>, ErrorData> {
        let rowid = params.0.rowid;
        let destination_mailbox = params.0.destination_mailbox;

        let (addressed, destination, snapshot_account_id, before) = {
            let conn = self.state.store_conn.lock().expect("store_conn poisoned");
            let resolver = self
                .state
                .account_resolver
                .lock()
                .expect("account_resolver poisoned");
            let (destination, snapshot_account_id) = tools::mutate::resolve_destination(
                &self.state.mailbox_registry,
                &resolver,
                &destination_mailbox,
            )
            .map_err(to_error_data)?;
            let prep = tools::mutate::prepare_relocate(
                &conn,
                &self.state.mailbox_registry,
                &self.state.store_path,
                &resolver,
                rowid,
                Some(&snapshot_account_id),
            )
            .map_err(to_error_data)?;
            (
                prep.addressed,
                destination,
                prep.snapshot_account_id,
                prep.before,
            )
        };

        amx_automation::locate::locate(
            addressed.mailbox.clone(),
            addressed.message_id.clone(),
            rowid,
        )
        .await
        .map_err(to_error_data)?;
        amx_automation::jxa::run(&amx_automation::JxaRequest::Move {
            mailbox: addressed.mailbox,
            message_id: addressed.message_id,
            destination,
        })
        .await
        .map_err(to_error_data)?;

        let response = {
            let conn = self.state.store_conn.lock().expect("store_conn poisoned");
            let resolver = self
                .state
                .account_resolver
                .lock()
                .expect("account_resolver poisoned");
            tools::mutate::finish_move(
                &conn,
                &self.state.mailbox_registry,
                &self.state.store_path,
                &resolver,
                &self.state.index_dir,
                &self.state.meta_path,
                rowid,
                &snapshot_account_id,
                &before,
            )
        }
        .map_err(to_error_data)?;
        self.state.index_pool.reload().map_err(to_error_data)?;

        Ok(Json(self.envelope(response)))
    }

    #[cfg(target_os = "macos")]
    #[tool(
        name = "trash_messages",
        description = "Move a message to Mail.app's Trash mailbox, keeping the search index in sync. Never permanently erases.",
        annotations(read_only_hint = false, destructive_hint = true)
    )]
    pub async fn trash_messages(
        &self,
        params: Parameters<TrashMessagesRequest>,
    ) -> Result<Json<Envelope<TrashMessagesResponse>>, ErrorData> {
        let rowid = params.0.rowid;

        let (addressed, snapshot_account_id, before) = {
            let conn = self.state.store_conn.lock().expect("store_conn poisoned");
            let resolver = self
                .state
                .account_resolver
                .lock()
                .expect("account_resolver poisoned");
            let prep = tools::mutate::prepare_relocate(
                &conn,
                &self.state.mailbox_registry,
                &self.state.store_path,
                &resolver,
                rowid,
                None,
            )
            .map_err(to_error_data)?;
            (prep.addressed, prep.snapshot_account_id, prep.before)
        };

        amx_automation::locate::locate(
            addressed.mailbox.clone(),
            addressed.message_id.clone(),
            rowid,
        )
        .await
        .map_err(to_error_data)?;
        amx_automation::jxa::run(&amx_automation::JxaRequest::Trash {
            mailbox: addressed.mailbox,
            message_id: addressed.message_id,
        })
        .await
        .map_err(to_error_data)?;

        let response = {
            let conn = self.state.store_conn.lock().expect("store_conn poisoned");
            let resolver = self
                .state
                .account_resolver
                .lock()
                .expect("account_resolver poisoned");
            tools::mutate::finish_trash(
                &conn,
                &self.state.mailbox_registry,
                &self.state.store_path,
                &resolver,
                &self.state.index_dir,
                &self.state.meta_path,
                rowid,
                &snapshot_account_id,
                &before,
            )
        }
        .map_err(to_error_data)?;
        self.state.index_pool.reload().map_err(to_error_data)?;

        Ok(Json(self.envelope(response)))
    }

    #[cfg(target_os = "macos")]
    #[tool(
        name = "triage_plan",
        description = "Freeze an ordered rowid list and an operation into a content-addressed plan for triage_apply. Touches nothing.",
        annotations(read_only_hint = true)
    )]
    pub async fn triage_plan(
        &self,
        params: Parameters<TriagePlanRequest>,
    ) -> Result<Json<Envelope<TriagePlanResponse>>, ErrorData> {
        let meta_conn = self.state.meta_conn.lock().expect("meta_conn poisoned");
        let response = tools::run_triage_plan(&meta_conn, params.0).map_err(to_error_data)?;
        Ok(Json(self.envelope(response)))
    }

    #[tool(
        name = "doctor",
        description = "Full readiness screen: access, accounts, mailboxes, coverage, writer/quarantine state."
    )]
    pub async fn doctor(&self) -> Result<Json<DoctorResponse>, ErrorData> {
        let meta_conn = self.state.meta_conn.lock().expect("meta_conn poisoned");
        let searcher = self.state.index_pool.searcher();
        let response = tools::run_doctor(
            &self.state.health,
            &meta_conn,
            &self.state.store_path,
            &self.state.mailbox_registry,
            &searcher,
            &self.state.fields,
        )
        .map_err(to_error_data)?;
        Ok(Json(response))
    }

    #[tool(name = "status", description = "Cheap access-only health check.")]
    pub async fn status(&self) -> Result<Json<StatusResponse>, ErrorData> {
        let meta_conn = self.state.meta_conn.lock().expect("meta_conn poisoned");
        let response = tools::run_status(&self.state.health, &meta_conn).map_err(to_error_data)?;
        Ok(Json(response))
    }
}

#[rmcp::tool_handler(router = self.tool_router)]
impl ServerHandler for AmxServer {
    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        let visible_names: std::collections::HashSet<&str> =
            visible_tools(&catalog(), self.state.read_only)
                .into_iter()
                .map(|descriptor| descriptor.name)
                .collect();

        let tools: Vec<Tool> = self
            .tool_router
            .list_all()
            .into_iter()
            .filter(|tool| visible_names.contains(tool.name.as_ref()))
            .collect();

        // The catalog is fixed for this process's lifetime (it only varies by `read_only` at
        // startup), so it's safe to advertise as publicly cacheable for a while.
        Ok(ListToolsResult::with_all_items(tools)
            .with_ttl_ms(60_000)
            .with_cache_scope(rmcp::model::CacheScope::Public))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_names_match_every_registered_tool() {
        let router = AmxServer::tool_router();
        let registered: std::collections::HashSet<_> =
            router.list_all().into_iter().map(|t| t.name).collect();
        let catalog = catalog();
        for descriptor in &catalog {
            assert!(
                registered.contains(descriptor.name),
                "catalog entry {:?} has no matching #[tool]",
                descriptor.name
            );
        }
        assert_eq!(registered.len(), catalog.len());
    }
}
