pub mod account;
pub mod conn;
pub mod fixtures;
pub mod paths;
pub mod query;
pub mod registry;
#[cfg(target_os = "macos")]
pub mod tcc;

pub use account::{AccountResolver, ResolvedAccount};
pub use conn::RoConnection;
pub use paths::EmlxPathResolver;
pub use query::{MessageQuery, MessageRow};
pub use registry::{Mailbox, MailboxCounts, MailboxRegistry};
