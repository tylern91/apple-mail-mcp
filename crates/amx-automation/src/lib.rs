#![cfg(target_os = "macos")]

//! JXA bridge to Mail.app — the only write path to the live mailbox (umbrella §2.1). Every
//! mutation is a typed [`op::JxaRequest`] run through [`jxa::run`]; there is no generic
//! "run arbitrary script" escape hatch.

pub mod jxa;
pub mod locate;
pub mod op;

pub use op::{JxaRequest, JxaResult, MailboxAddress};
