use crate::error::AmxError;

/// Standing Full Disk Access state for the mail store, per umbrella §4.4's `HealthMonitor`.
///
/// `PendingRestart` is distinct from `Denied` because macOS keys FDA to the responsible
/// process identity and only applies a grant at the next process launch — "granted but not yet
/// active" is otherwise indistinguishable from "not granted" (parasxos #3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccessState {
    Granted,
    Denied { responsible_process: String },
    PendingRestart { responsible_process: String },
}

/// The read boundary a platform-specific store crate (`amx-store`) implements.
///
/// Confining Envelope Index access behind this trait is what lets `amxcli doctor` and the
/// portable crates compile and unit-test on Linux CI without a live mailbox (umbrella §3).
/// Only the methods this phase's `doctor` screen actually calls are declared here — no
/// speculative query surface.
pub trait MailStore {
    fn access_state(&self) -> AccessState;
    fn account_count(&self) -> Result<usize, AmxError>;
    fn mailbox_count(&self) -> Result<usize, AmxError>;
}

/// The JXA-automation boundary `amx-automation` implements (Phase 4: move/flag/trash,
/// `fetch-full`). No caller in this phase invokes it — declared now, empty, only so `amx-core`
/// can name the trait object in the container diagram without depending on the JXA bridge.
///
/// lean: no methods yet, upgrade when Phase 4 adds the plan → review → apply → verify surface.
pub trait Automation {}

/// The SMTP-submission boundary `amx-send` implements (Phase 5). Same rationale as
/// [`Automation`] — declared now for the trait-object boundary, populated in Phase 5.
///
/// lean: no methods yet, upgrade when Phase 5 adds message submission.
pub trait Transport {}
