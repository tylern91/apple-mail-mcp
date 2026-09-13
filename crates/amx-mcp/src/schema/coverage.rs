//! Wire-shape mirror of `amx_index::coverage::CoverageEnvelope` (umbrella §4.2.4).
//!
//! `amx-index` stays free of `schemars` — the query builder is a portable, Linux-CI-testable
//! module with no MCP-specific concerns. This mirror is what `search_messages` actually returns.

use amx_index::coverage::{AttachmentCoverage, BodyCoverage, CoverageEnvelope};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BodyCoverageDto {
    pub indexed: usize,
    pub pending: usize,
    pub unavailable: usize,
    pub quarantined: usize,
}

impl From<BodyCoverage> for BodyCoverageDto {
    fn from(c: BodyCoverage) -> Self {
        Self {
            indexed: c.indexed,
            pending: c.pending,
            unavailable: c.unavailable,
            quarantined: c.quarantined,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AttachmentCoverageDto {
    pub messages_with_attachments: usize,
    pub extracted: usize,
    pub not_downloaded: usize,
    pub unextractable: usize,
}

impl From<AttachmentCoverage> for AttachmentCoverageDto {
    fn from(c: AttachmentCoverage) -> Self {
        Self {
            messages_with_attachments: c.messages_with_attachments,
            extracted: c.extracted,
            not_downloaded: c.not_downloaded,
            unextractable: c.unextractable,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Coverage {
    pub corpus_messages: usize,
    pub body: BodyCoverageDto,
    pub attachments: AttachmentCoverageDto,
    pub body_searched_fraction: f64,
    pub attachment_searched_fraction: f64,
    pub warnings: Vec<String>,
}

impl From<CoverageEnvelope> for Coverage {
    fn from(e: CoverageEnvelope) -> Self {
        Self {
            corpus_messages: e.corpus_messages,
            body: e.body.into(),
            attachments: e.attachments.into(),
            body_searched_fraction: e.body_searched_fraction,
            attachment_searched_fraction: e.attachment_searched_fraction,
            warnings: e.warnings,
        }
    }
}
