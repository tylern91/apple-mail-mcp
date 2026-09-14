pub mod attachments;
pub mod browse;
pub mod directory;
pub mod doctor;
pub mod get_message;
pub mod pipeline;
pub mod search_messages;
#[cfg(target_os = "macos")]
pub mod send;
pub mod thread;

pub use attachments::{run_extract_attachment_text, run_get_attachment, run_list_attachments};
pub use browse::{run_count_messages, run_recent_messages};
pub use directory::{run_list_accounts, run_list_mailboxes, run_resolve_address};
pub use doctor::{run_doctor, run_status};
pub use get_message::run_get_message;
pub use pipeline::{ResolvedMessage, resolve_message};
pub use search_messages::run_search;
#[cfg(target_os = "macos")]
pub use send::{run_create_draft, run_forward_message, run_reply_message, run_send_message};
pub use thread::{run_get_message_links, run_get_thread};
