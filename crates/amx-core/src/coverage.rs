use std::fmt;
use std::time::SystemTime;

use crate::error::{AccountKind, ParseErrorKind};

/// A message's availability along two independent axes — body text and attachment bytes.
///
/// Ported verbatim from umbrella §4.2.4 (the "two-dimensional coverage" model). `.partial.emlx`
/// proves the axes are independent: a message can have a complete, indexable body while its
/// attachment bytes are entirely absent from disk. Conflating the two produces a confidently
/// wrong negative — the single defect this model exists to rule out.
#[derive(Debug, Clone, PartialEq)]
pub struct Availability {
    pub body: BodyState,
    pub attachments: AttachmentState,
}

#[derive(Debug, Clone, PartialEq)]
pub enum BodyState {
    Indexed,
    Pending {
        queued_at: SystemTime,
    },
    /// Terminal — retry is a category error, not a transient failure to back off from.
    Unavailable(UnavailableReason),
    Quarantined {
        error: ParseErrorKind,
    },
}

impl BodyState {
    /// `false` for [`BodyState::Unavailable`] — that state is terminal by definition and must
    /// never re-enter a retry queue (the property that rules out parasxos' six-probes-then-
    /// silent-abandonment failure mode). Every other state may still resolve to `Indexed`.
    pub fn is_retryable(&self) -> bool {
        !matches!(self, Self::Unavailable(_))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum AttachmentState {
    /// The message has no attachments.
    None,
    Extracted {
        count: usize,
    },
    /// `.partial.emlx` — fixable via `amxcli fetch-full`, unlike `BodyState::Unavailable`.
    NotDownloaded {
        declared_bytes: u64,
        on_disk_bytes: u64,
    },
    /// Bytes are present but not text-extractable (e.g. an unsupported binary format).
    Unextractable {
        count: usize,
    },
}

impl AttachmentState {
    /// `true` for [`AttachmentState::NotDownloaded`] — its remedy is real: ask Mail.app to
    /// fetch the full message via JXA. Every other state is not awaiting a future fetch.
    pub fn is_retryable(&self) -> bool {
        matches!(self, Self::NotDownloaded { .. })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum UnavailableReason {
    /// Exchange/IMAP pruned the `.emlx`; recovering it needs outbound provider auth (umbrella
    /// Q4), not a local retry.
    NotCachedLocally {
        account_kind: AccountKind,
    },
    /// TCC — user-recoverable by granting Full Disk Access.
    PermissionDenied,
    Encrypted,
}

impl fmt::Display for UnavailableReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotCachedLocally { account_kind } => {
                write!(
                    f,
                    "not cached locally ({account_kind:?} account pruned the .emlx)"
                )
            }
            Self::PermissionDenied => write!(f, "permission denied"),
            Self::Encrypted => write!(f, "encrypted"),
        }
    }
}
