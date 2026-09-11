//! Capabilities: what a plugin is allowed to ask the host to do.
//!
//! The model is an allowlist. A manifest names the capabilities it needs; the
//! host grants exactly those and refuses every method whose required
//! capability is absent. Nothing is implied by having been activated, and a
//! capability the host does not recognise is a hard failure when required and
//! a no-op when optional — so an old host never silently under-serves a plugin
//! that was written against a newer one.
//!
//! Capabilities are intentionally coarse, and intentionally *not* the same
//! thing as the method table. `instance:write` does not mean "arbitrary writes
//! to instances"; it means "may submit the instance-mutating operations this
//! host API exposes, each of which still runs through the ordinary core
//! operation, lock and user-decision path".

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fmt;
use std::str::FromStr;

/// A single capability a plugin may hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Capability {
    /// List and inspect instances and their configuration.
    #[serde(rename = "instance:read")]
    InstanceRead,
    /// Submit supported instance-mutating operations (rename, settings, repair).
    #[serde(rename = "instance:write")]
    InstanceWrite,
    /// Read installed content: mods, packs, shaders, resource packs.
    #[serde(rename = "content:read")]
    ContentRead,
    /// Submit supported content operations: enable, disable, and install a
    /// resolved plan the user has approved.
    #[serde(rename = "content:write")]
    ContentWrite,
    /// Read launch state and launch history.
    #[serde(rename = "launch:read")]
    LaunchRead,
    /// Contribute bounded pre-launch checks and preparation tasks.
    #[serde(rename = "launch:prepare")]
    LaunchPrepare,
    /// Publish diagnostic findings and typed repair proposals.
    #[serde(rename = "diagnostics:publish")]
    DiagnosticsPublish,
    /// Make outbound HTTP requests, brokered through the launcher's network
    /// policy. Denied outright in Lockdown Mode, like every other category.
    #[serde(rename = "network")]
    Network,
}

impl Capability {
    /// Every capability this host build understands.
    pub const ALL: &'static [Capability] = &[
        Capability::InstanceRead,
        Capability::InstanceWrite,
        Capability::ContentRead,
        Capability::ContentWrite,
        Capability::LaunchRead,
        Capability::LaunchPrepare,
        Capability::DiagnosticsPublish,
        Capability::Network,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Capability::InstanceRead => "instance:read",
            Capability::InstanceWrite => "instance:write",
            Capability::ContentRead => "content:read",
            Capability::ContentWrite => "content:write",
            Capability::LaunchRead => "launch:read",
            Capability::LaunchPrepare => "launch:prepare",
            Capability::DiagnosticsPublish => "diagnostics:publish",
            Capability::Network => "network",
        }
    }

    /// Whether holding this capability lets a plugin change the user's data.
    ///
    /// The manager surfaces these differently at install time, and the CLI
    /// prints them under a separate heading. A read-only plugin should be
    /// installable without the user reading a wall of warnings.
    pub fn is_mutating(self) -> bool {
        matches!(
            self,
            Capability::InstanceWrite | Capability::ContentWrite | Capability::LaunchPrepare
        )
    }

    /// One line explaining the capability in the user's terms, for the install
    /// prompt and the plugin manager. Deliberately concrete about the limit.
    pub fn summary(self) -> &'static str {
        match self {
            Capability::InstanceRead => "See your instances and how they are configured",
            Capability::InstanceWrite => "Change instance settings, through Agora's usual prompts",
            Capability::ContentRead => "See the mods and packs you have installed",
            Capability::ContentWrite => "Enable, disable and install content you approve",
            Capability::LaunchRead => "See launch status and launch history",
            Capability::LaunchPrepare => "Run checks before a launch, and offer fixes",
            Capability::DiagnosticsPublish => "Report problems it finds and propose repairs",
            Capability::Network => "Reach the internet, subject to your network settings",
        }
    }
}

impl fmt::Display for Capability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Capability {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Capability::ALL
            .iter()
            .copied()
            .find(|c| c.as_str() == s)
            .ok_or(())
    }
}

/// The capabilities a manifest asks for.
///
/// `required` must all be understood and granted or activation fails.
/// `optional` are granted when understood and silently skipped when not, so a
/// plugin can use a newer host's feature without dropping older hosts.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CapabilityRequest {
    #[serde(default)]
    pub required: Vec<String>,
    #[serde(default)]
    pub optional: Vec<String>,
}

/// The capabilities a plugin actually holds at runtime.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CapabilitySet(BTreeSet<Capability>);

impl CapabilitySet {
    pub fn new() -> Self {
        Self(BTreeSet::new())
    }

    pub fn from_capabilities(caps: impl IntoIterator<Item = Capability>) -> Self {
        Self(caps.into_iter().collect())
    }

    pub fn insert(&mut self, cap: Capability) {
        self.0.insert(cap);
    }

    pub fn contains(&self, cap: Capability) -> bool {
        self.0.contains(&cap)
    }

    pub fn iter(&self) -> impl Iterator<Item = Capability> + '_ {
        self.0.iter().copied()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Resolve a manifest's request against what this host understands.
    ///
    /// An unknown *required* capability is an error naming the offending
    /// string, because the honest answer is "this plugin needs something this
    /// version of Agora cannot give it", not "permission denied".
    pub fn resolve(request: &CapabilityRequest) -> Result<Self, crate::PluginError> {
        let mut set = CapabilitySet::new();
        for name in &request.required {
            match Capability::from_str(name) {
                Ok(cap) => set.insert(cap),
                Err(()) => {
                    return Err(crate::PluginError::new(
                        crate::PluginErrorCode::IncompatibleApi,
                        format!(
                            "requires the `{name}` capability, which Agora plugin API {} does not provide",
                            crate::HOST_API_VERSION
                        ),
                    )
                    .with_detail(name.clone()))
                }
            }
        }
        for name in &request.optional {
            if let Ok(cap) = Capability::from_str(name) {
                set.insert(cap);
            }
        }
        Ok(set)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_json_using_the_wire_names() {
        let json = serde_json::to_string(&Capability::InstanceRead).unwrap();
        assert_eq!(json, "\"instance:read\"");
        let back: Capability = serde_json::from_str(&json).unwrap();
        assert_eq!(back, Capability::InstanceRead);
    }

    #[test]
    fn every_capability_parses_back_from_its_own_string() {
        for cap in Capability::ALL {
            assert_eq!(Capability::from_str(cap.as_str()), Ok(*cap));
        }
    }

    #[test]
    fn unknown_required_capability_is_an_api_incompatibility_not_a_denial() {
        let request = CapabilityRequest {
            required: vec!["world:teleport".into()],
            optional: vec![],
        };
        let err = CapabilitySet::resolve(&request).unwrap_err();
        assert_eq!(err.code, crate::PluginErrorCode::IncompatibleApi);
        assert_eq!(err.detail.as_deref(), Some("world:teleport"));
    }

    #[test]
    fn unknown_optional_capability_is_skipped_so_old_hosts_still_load_new_plugins() {
        let request = CapabilityRequest {
            required: vec!["instance:read".into()],
            optional: vec!["world:teleport".into(), "network".into()],
        };
        let set = CapabilitySet::resolve(&request).unwrap();
        assert!(set.contains(Capability::InstanceRead));
        assert!(set.contains(Capability::Network));
        assert_eq!(set.len(), 2);
    }
}
