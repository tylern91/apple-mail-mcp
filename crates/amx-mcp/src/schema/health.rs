//! Wire-shape mirror of `amx_core::traits::AccessState` (umbrella §4.4's `HealthMonitor`).
//!
//! `AccessState` itself stays free of `serde`/`schemars` — it's amx-core's portable domain type.
//! This mirror is what actually crosses the MCP wire, stamped onto every response per the
//! umbrella's "stamp every response while degraded, not just the ones that touch the store"
//! requirement.

use amx_core::traits::AccessState;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum HealthStatus {
    Granted,
    Denied { responsible_process: String },
    PendingRestart { responsible_process: String },
}

impl From<&AccessState> for HealthStatus {
    fn from(state: &AccessState) -> Self {
        match state {
            AccessState::Granted => Self::Granted,
            AccessState::Denied {
                responsible_process,
            } => Self::Denied {
                responsible_process: responsible_process.clone(),
            },
            AccessState::PendingRestart {
                responsible_process,
            } => Self::PendingRestart {
                responsible_process: responsible_process.clone(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mirrors_every_access_state_variant() {
        assert_eq!(
            HealthStatus::from(&AccessState::Granted),
            HealthStatus::Granted
        );
        assert_eq!(
            HealthStatus::from(&AccessState::Denied {
                responsible_process: "amxcli".to_string()
            }),
            HealthStatus::Denied {
                responsible_process: "amxcli".to_string()
            }
        );
        assert_eq!(
            HealthStatus::from(&AccessState::PendingRestart {
                responsible_process: "amxcli".to_string()
            }),
            HealthStatus::PendingRestart {
                responsible_process: "amxcli".to_string()
            }
        );
    }
}
