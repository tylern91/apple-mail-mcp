pub mod account;
pub mod conn;
pub mod fixtures;
pub mod mailbox_path;
pub mod paths;
pub mod query;
pub mod readiness;
pub mod registry;
#[cfg(target_os = "macos")]
pub mod tcc;
#[cfg(target_os = "macos")]
pub mod watch;

pub use account::{AccountResolver, ResolvedAccount};
pub use conn::RoConnection;
pub use mailbox_path::MailboxPathResolver;
pub use paths::EmlxPathResolver;
pub use query::{MessageQuery, MessageRow};
pub use registry::{Mailbox, MailboxCounts, MailboxRegistry};
