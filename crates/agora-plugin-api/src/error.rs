//! The structured error contract.
//!
//! Plugin authors see these codes, so they are part of the public API: a code
//! may be added, but an existing code must keep its meaning. The message is
//! for a human; the code is what a plugin is allowed to branch on.

use serde::{Deserialize, Serialize};

/// Stable, machine-readable reason a plugin request failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PluginErrorCode {
    /// The manifest could not be parsed, or violated a schema rule.
    InvalidManifest,
    /// The package could not be read, or contained an unsafe entry.
    InvalidPackage,
    /// The plugin's `apiRange` excludes the running host.
    IncompatibleApi,
    /// A declared dependency is missing, disabled, or version-incompatible.
    UnresolvedDependency,
    /// The dependency graph contains a cycle.
    DependencyCycle,
    /// Two enabled plugins contributed the same identifier.
    DuplicateContribution,
    /// The plugin asked for a method it holds no capability for.
    CapabilityDenied,
    /// The method name is not part of this host API version.
    UnknownMethod,
    /// Arguments failed validation before any service was touched.
    InvalidArguments,
    /// The plugin is not installed, or not currently activated.
    NotActivated,
    /// The plugin's script threw, or failed to load.
    ScriptError,
    /// The call exceeded its deadline and was interrupted.
    Timeout,
    /// The call was cancelled by the host or the user.
    Cancelled,
    /// The plugin exceeded a resource ceiling (memory, stack, queue depth).
    ResourceExhausted,
    /// Network access was refused by the launcher's outbound policy.
    NetworkDenied,
    /// Two plugins asked for contradictory changes to the same thing.
    Conflict,
    /// The underlying core operation failed. `message` carries its reason.
    OperationFailed,
    /// Something went wrong inside the host itself.
    Internal,
}

impl PluginErrorCode {
    /// Whether the plugin could reasonably retry the identical request.
    ///
    /// Used by the host to decide whether to offer *Retry* on a failed
    /// user-selected task, and by the CLI to pick an exit code.
    pub fn is_retryable(self) -> bool {
        matches!(
            self,
            PluginErrorCode::Timeout
                | PluginErrorCode::ResourceExhausted
                | PluginErrorCode::OperationFailed
        )
    }

    /// Whether this is the plugin's fault rather than the host's or the user's.
    ///
    /// Author-facing surfaces (the plugin log, the manager's error badge) lead
    /// with these; a `NetworkDenied` is a user setting, not a bug to report.
    pub fn is_author_error(self) -> bool {
        matches!(
            self,
            PluginErrorCode::InvalidManifest
                | PluginErrorCode::InvalidPackage
                | PluginErrorCode::IncompatibleApi
                | PluginErrorCode::UnknownMethod
                | PluginErrorCode::InvalidArguments
                | PluginErrorCode::ScriptError
        )
    }
}

/// An error crossing the plugin boundary in either direction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[serde(rename_all = "camelCase")]
#[error("{code:?}: {message}")]
pub struct PluginError {
    pub code: PluginErrorCode,
    pub message: String,
    /// Optional author-facing detail: a stack frame, the offending field, the
    /// capability that was missing. Never contains a token or a secret.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl PluginError {
    pub fn new(code: PluginErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            detail: None,
        }
    }

    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    pub fn invalid_manifest(message: impl Into<String>) -> Self {
        Self::new(PluginErrorCode::InvalidManifest, message)
    }

    pub fn invalid_arguments(message: impl Into<String>) -> Self {
        Self::new(PluginErrorCode::InvalidArguments, message)
    }

    pub fn capability_denied(capability: impl std::fmt::Display, method: &str) -> Self {
        Self::new(
            PluginErrorCode::CapabilityDenied,
            format!("`{method}` requires the `{capability}` capability"),
        )
        .with_detail(capability.to_string())
    }

    pub fn unknown_method(method: &str) -> Self {
        Self::new(
            PluginErrorCode::UnknownMethod,
            format!(
                "`{method}` is not a method of host API {}",
                crate::HOST_API_VERSION
            ),
        )
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(PluginErrorCode::Internal, message)
    }
}

pub type PluginResult<T> = Result<T, PluginError>;
