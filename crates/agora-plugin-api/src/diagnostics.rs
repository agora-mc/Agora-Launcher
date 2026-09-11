//! Findings, evidence, and typed repair proposals.
//!
//! A plugin does not fix anything. It says what it found, shows its working,
//! and proposes a repair drawn from a closed set of operations the launcher
//! already knows how to perform, validate and undo. The user approves; core
//! executes through its ordinary coordinator, with its ordinary locks.
//!
//! That closed set is the point. If a plugin could hand back "run this
//! command", the repair path would be an arbitrary-execution API wearing a
//! diagnostic's clothes.

use serde::{Deserialize, Serialize};

/// How much the user should care.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Severity {
    /// Worth knowing, nothing is wrong.
    #[default]
    Info,
    /// Likely to cause a problem. Agora warns; the user still decides.
    Warning,
    /// Known to be broken right now.
    Error,
}

/// What one diagnostic run found.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticReport {
    #[serde(default)]
    pub findings: Vec<Finding>,
    /// Set when the plugin could not complete the check. A partial report with
    /// an honest note beats a clean report that silently checked nothing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub incomplete_reason: Option<String>,
}

impl DiagnosticReport {
    /// How many findings one run may return.
    pub const MAX_FINDINGS: usize = 100;

    pub fn highest_severity(&self) -> Option<Severity> {
        self.findings.iter().map(|f| f.severity).max()
    }

    pub fn validate(&self) -> Result<(), crate::PluginError> {
        if self.findings.len() > Self::MAX_FINDINGS {
            return Err(crate::PluginError::new(
                crate::PluginErrorCode::ResourceExhausted,
                format!(
                    "a diagnostic may return {} findings; this one returned {}",
                    Self::MAX_FINDINGS,
                    self.findings.len()
                ),
            ));
        }
        for finding in &self.findings {
            if finding.title.trim().is_empty() {
                return Err(crate::PluginError::invalid_arguments(
                    "every finding needs a title",
                ));
            }
        }
        Ok(())
    }
}

/// One thing a plugin noticed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Finding {
    /// Stable within the plugin, so the host can tell "still broken" from
    /// "broken again" across runs.
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub severity: Severity,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// What the plugin looked at. Plain label/value text — this is shown to
    /// the user as the reason to believe the finding.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<Evidence>,
    /// Optional fixes, in the plugin's preferred order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub repairs: Vec<RepairProposal>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Evidence {
    pub label: String,
    pub value: String,
}

/// A fix the user may approve.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepairProposal {
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Applied in order, as one approval. An empty list is rejected — a
    /// proposal that does nothing is worse than no proposal.
    pub actions: Vec<RepairAction>,
}

impl RepairProposal {
    /// How many actions one proposal may bundle behind a single approval.
    pub const MAX_ACTIONS: usize = 25;

    pub fn validate(&self) -> Result<(), crate::PluginError> {
        if self.actions.is_empty() {
            return Err(crate::PluginError::invalid_arguments(format!(
                "repair `{}` proposes no actions",
                self.id
            )));
        }
        if self.actions.len() > Self::MAX_ACTIONS {
            return Err(crate::PluginError::new(
                crate::PluginErrorCode::ResourceExhausted,
                format!(
                    "repair `{}` bundles {} actions behind one approval; the limit is {}",
                    self.id,
                    self.actions.len(),
                    Self::MAX_ACTIONS
                ),
            ));
        }
        Ok(())
    }
}

/// The closed set of operations a repair may ask for.
///
/// Every variant maps onto an existing `agora-core` service call. Adding a
/// variant is an API change with a version bump, reviewed on its own merits —
/// which is exactly the friction that should exist before a plugin gains a new
/// way to change someone's game.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "action"
)]
pub enum RepairAction {
    /// Turn an installed item off without deleting it.
    DisableContent { instance_id: String, key: String },
    /// Turn a disabled item back on.
    EnableContent { instance_id: String, key: String },
    /// Stop offering updates for an item.
    PinContentUpdate { instance_id: String, key: String },
    /// Resume offering updates for an item.
    UnpinContentUpdate { instance_id: String, key: String },
    /// Change the heap ceiling. Bounded by core against real system memory.
    SetJvmMemory { instance_id: String, memory_mb: i64 },
    /// Clear custom JVM arguments back to Agora's defaults.
    ResetJvmArgs { instance_id: String },
    /// Take a restore point before the user does something else.
    CreateSnapshot {
        instance_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
    },
}

impl RepairAction {
    /// The instance this action touches.
    pub fn instance_id(&self) -> &str {
        match self {
            RepairAction::DisableContent { instance_id, .. }
            | RepairAction::EnableContent { instance_id, .. }
            | RepairAction::PinContentUpdate { instance_id, .. }
            | RepairAction::UnpinContentUpdate { instance_id, .. }
            | RepairAction::SetJvmMemory { instance_id, .. }
            | RepairAction::ResetJvmArgs { instance_id }
            | RepairAction::CreateSnapshot { instance_id, .. } => instance_id,
        }
    }

    /// What this action competes for.
    ///
    /// Two proposals sharing a conflict key are contending over the same
    /// thing. The host uses this to detect that two plugins want opposite
    /// outcomes and say so, instead of letting whichever ran last win.
    pub fn conflict_key(&self) -> String {
        match self {
            RepairAction::DisableContent { instance_id, key }
            | RepairAction::EnableContent { instance_id, key } => {
                format!("content-enabled:{instance_id}:{key}")
            }
            RepairAction::PinContentUpdate { instance_id, key }
            | RepairAction::UnpinContentUpdate { instance_id, key } => {
                format!("content-pin:{instance_id}:{key}")
            }
            RepairAction::SetJvmMemory { instance_id, .. } => format!("jvm-memory:{instance_id}"),
            RepairAction::ResetJvmArgs { instance_id } => format!("jvm-args:{instance_id}"),
            // Snapshots accumulate rather than contend; two plugins both
            // wanting one is not a disagreement.
            RepairAction::CreateSnapshot { instance_id, .. } => {
                format!("snapshot:{instance_id}:{}", uniquifier())
            }
        }
    }

    /// Whether two actions on the same key actually disagree.
    ///
    /// `Disable` then `Disable` is a duplicate and can be merged. `Disable`
    /// then `Enable` is a genuine conflict and has to reach the user.
    pub fn disagrees_with(&self, other: &RepairAction) -> bool {
        self.conflict_key() == other.conflict_key() && self != other
    }

    /// Whether the user should be asked before this runs, beyond approving the
    /// proposal itself. Reserved for actions that change what launches.
    pub fn is_destructive(&self) -> bool {
        matches!(
            self,
            RepairAction::DisableContent { .. } | RepairAction::ResetJvmArgs { .. }
        )
    }

    /// A sentence describing the action for the approval prompt.
    pub fn describe(&self) -> String {
        match self {
            RepairAction::DisableContent { key, .. } => format!("Disable `{key}`"),
            RepairAction::EnableContent { key, .. } => format!("Enable `{key}`"),
            RepairAction::PinContentUpdate { key, .. } => format!("Pin `{key}` against updates"),
            RepairAction::UnpinContentUpdate { key, .. } => format!("Allow updates for `{key}`"),
            RepairAction::SetJvmMemory { memory_mb, .. } => {
                format!("Set the memory limit to {memory_mb} MB")
            }
            RepairAction::ResetJvmArgs { .. } => "Reset custom JVM arguments".to_string(),
            RepairAction::CreateSnapshot { label, .. } => match label {
                Some(label) => format!("Create a snapshot named `{label}`"),
                None => "Create a snapshot".to_string(),
            },
        }
    }
}

/// Snapshot actions never conflict, so their key just has to be distinct.
fn uniquifier() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn disable(key: &str) -> RepairAction {
        RepairAction::DisableContent {
            instance_id: "i1".into(),
            key: key.into(),
        }
    }

    #[test]
    fn opposite_actions_on_one_item_are_a_conflict() {
        let a = disable("sodium");
        let b = RepairAction::EnableContent {
            instance_id: "i1".into(),
            key: "sodium".into(),
        };
        assert!(a.disagrees_with(&b));
    }

    #[test]
    fn the_same_action_twice_is_a_duplicate_not_a_conflict() {
        assert!(!disable("sodium").disagrees_with(&disable("sodium")));
    }

    #[test]
    fn actions_on_different_items_do_not_conflict() {
        assert!(!disable("sodium").disagrees_with(&disable("iris")));
    }

    #[test]
    fn two_memory_proposals_for_one_instance_conflict_even_at_different_values() {
        let a = RepairAction::SetJvmMemory {
            instance_id: "i1".into(),
            memory_mb: 4096,
        };
        let b = RepairAction::SetJvmMemory {
            instance_id: "i1".into(),
            memory_mb: 8192,
        };
        assert!(a.disagrees_with(&b));
    }

    #[test]
    fn two_snapshot_requests_are_never_treated_as_a_disagreement() {
        let a = RepairAction::CreateSnapshot {
            instance_id: "i1".into(),
            label: Some("before".into()),
        };
        let b = RepairAction::CreateSnapshot {
            instance_id: "i1".into(),
            label: Some("after".into()),
        };
        assert!(!a.disagrees_with(&b));
    }

    #[test]
    fn a_repair_that_does_nothing_is_rejected() {
        let proposal = RepairProposal {
            id: "noop".into(),
            title: "Do nothing".into(),
            description: None,
            actions: vec![],
        };
        assert!(proposal.validate().is_err());
    }

    #[test]
    fn highest_severity_drives_how_the_report_is_surfaced() {
        let report = DiagnosticReport {
            findings: vec![
                Finding {
                    id: "a".into(),
                    title: "Fine".into(),
                    severity: Severity::Info,
                    summary: None,
                    evidence: vec![],
                    repairs: vec![],
                },
                Finding {
                    id: "b".into(),
                    title: "Not fine".into(),
                    severity: Severity::Error,
                    summary: None,
                    evidence: vec![],
                    repairs: vec![],
                },
            ],
            incomplete_reason: None,
        };
        assert_eq!(report.highest_severity(), Some(Severity::Error));
    }

    #[test]
    fn a_repair_action_names_the_instance_it_touches() {
        assert_eq!(disable("sodium").instance_id(), "i1");
    }
}
