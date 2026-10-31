//! The error every `games_*` command returns.
//!
//! `code` is a stable string the frontend may branch on, so a code is added here as a
//! constant rather than typed ad hoc at a call site. The message is for people.

use serde::Serialize;

/// A command that is registered but not built yet. Slices replace these.
pub const NOT_IMPLEMENTED: &str = "not_implemented";
/// agora-core refused the request; `core_code` carries its `ERR_...` code.
pub const CORE_FAILED: &str = "core_failed";
/// The blocking worker that did the work panicked or was cancelled.
pub const TASK_FAILED: &str = "task_failed";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GamesError {
    pub code: String,
    pub message: String,
    /// The agora-core code (`ERR_...`) when the failure came from core.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub core_code: Option<String>,
}

impl GamesError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_string(),
            message: message.into(),
            core_code: None,
        }
    }

    /// The failure a stub returns. Names the command so a caller can see which one.
    pub fn not_implemented(command: &str) -> Self {
        Self::new(
            NOT_IMPLEMENTED,
            format!("{command} is not implemented yet."),
        )
    }
}

impl From<agora_core::error::LauncherError> for GamesError {
    fn from(error: agora_core::error::LauncherError) -> Self {
        Self {
            code: CORE_FAILED.to_string(),
            message: error.to_string(),
            core_code: Some(error.code()),
        }
    }
}
