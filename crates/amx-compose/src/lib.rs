//! MIME construction for outbound mail (Phase 5, task 1).
//!
//! Compose is deliberately the *only* place in this workspace that touches RFC 2047/5322 header
//! encoding — `mail-builder` owns it as a single library responsibility. This is what
//! structurally avoids UseJunior #198 (a hand-rolled encoder double-encoding a non-ASCII
//! `Subject:` header into mojibake): there is no second encoder anywhere else to disagree with
//! this one.

mod draft;
mod mime;
mod reply;

pub use draft::{Attachment, Draft, EmailAddress};
pub use mime::{ComposedMessage, compose};
pub use reply::{ReplyMode, derive_forward, derive_reply};
