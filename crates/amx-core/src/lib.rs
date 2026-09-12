pub mod config;
pub mod coverage;
pub mod error;
pub mod traits;

pub use coverage::{AttachmentState, Availability, BodyState, UnavailableReason};
pub use error::{AccountKind, AmxError, ParseError, ParseErrorKind};
pub use traits::{Automation, MailStore, Transport};
