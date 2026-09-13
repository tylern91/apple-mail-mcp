//! `AmxError` → `rmcp::ErrorData` (Phase 3 task 9). Every `AmxError` variant's `Display` already
//! carries its remediation (umbrella §4.5), so the MCP error message is that string verbatim —
//! there is no separate error-code taxonomy to maintain here.

use amx_core::AmxError;
use rmcp::ErrorData;

pub fn to_error_data(err: AmxError) -> ErrorData {
    ErrorData::internal_error(err.to_string(), None)
}
