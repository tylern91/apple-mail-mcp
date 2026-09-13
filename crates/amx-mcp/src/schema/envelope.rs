//! The response envelope every tool shares: a `health` stamp on every response (umbrella §4.4),
//! and a `coverage` envelope on every search-shaped response (umbrella §4.2.4).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::coverage::Coverage;
use super::health::HealthStatus;

/// Wraps a tool's own response fields with the `health` stamp every response carries.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Envelope<T> {
    #[serde(flatten)]
    pub result: T,
    pub health: HealthStatus,
}

impl<T> Envelope<T> {
    pub fn new(result: T, health: HealthStatus) -> Self {
        Self { result, health }
    }
}

/// [`Envelope`] plus the coverage envelope — used only by search-shaped tools
/// (`search_messages`).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SearchEnvelope<T> {
    #[serde(flatten)]
    pub result: T,
    pub health: HealthStatus,
    pub coverage: Coverage,
}

impl<T> SearchEnvelope<T> {
    pub fn new(result: T, health: HealthStatus, coverage: Coverage) -> Self {
        Self {
            result,
            health,
            coverage,
        }
    }
}
