//! The plugin manifest: `agora-plugin.json`.
//!
//! Parsed before anything else happens — before a package is unpacked, before
//! a line of plugin script is read. Everything the host needs in order to
//! decide *whether* to run a plugin lives here, in data.
//!
//! Validation is deliberately strict and deliberately specific. "Invalid
//! manifest" is useless to an author; "`entrypoint` must stay inside the
//! package, but `../../etc/passwd` climbs out of it" is not.

use crate::capability::CapabilityRequest;
use crate::contributions::Contributions;
use crate::error::{PluginError, PluginErrorCode, PluginResult};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

/// The manifest schema version this host reads.
///
/// Distinct from [`crate::HOST_API_VERSION`]: the schema version governs the
/// *shape of this file*, the API version governs *the methods a plugin may
/// call*. A manifest for a schema we do not know is rejected without guessing.
pub const MANIFEST_SCHEMA_VERSION: u32 = 1;

/// Canonical manifest filename inside a package or development folder.
pub const MANIFEST_FILENAME: &str = "agora-plugin.json";

// ---------------------------------------------------------------------------
// Plugin identity
// ---------------------------------------------------------------------------

/// A stable `publisher.plugin` identifier.
///
/// The publisher segment exists so that two people can both write a plugin
/// called `dashboard` without a registry to arbitrate between them. It is not
/// verified — Agora has no sign-in and no central approval — so it is a
/// namespace, not a claim of authorship.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct PluginId(String);

impl PluginId {
    /// Longest a full identifier may be. Keeps namespaced contribution ids,
    /// storage keys and log filenames comfortably inside path limits.
    pub const MAX_LEN: usize = 96;

    pub fn parse(raw: &str) -> PluginResult<Self> {
        if raw.len() > Self::MAX_LEN {
            return Err(PluginError::invalid_manifest(format!(
                "plugin id `{raw}` is {} characters; the limit is {}",
                raw.len(),
                Self::MAX_LEN
            )));
        }
        let Some((publisher, name)) = raw.split_once('.') else {
            return Err(PluginError::invalid_manifest(format!(
                "plugin id `{raw}` must be `publisher.plugin`, for example `acme.dashboard`"
            )));
        };
        validate_segment("publisher", publisher)?;
        validate_segment("plugin name", name)?;
        Ok(Self(raw.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn publisher(&self) -> &str {
        self.0.split_once('.').map(|(p, _)| p).unwrap_or(&self.0)
    }

    pub fn name(&self) -> &str {
        self.0.split_once('.').map(|(_, n)| n).unwrap_or("")
    }

    /// Namespace a local contribution identifier: `publisher.plugin/local`.
    pub fn qualify(&self, local: &str) -> String {
        format!("{}/{}", self.0, local)
    }
}

fn validate_segment(label: &str, segment: &str) -> PluginResult<()> {
    if segment.is_empty() {
        return Err(PluginError::invalid_manifest(format!(
            "{label} segment of the plugin id is empty"
        )));
    }
    let ok = segment
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !segment.starts_with('-')
        && !segment.ends_with('-')
        && segment.chars().next().is_some_and(|c| c != '-');
    if !ok {
        return Err(PluginError::invalid_manifest(format!(
            "{label} segment `{segment}` must be lowercase letters, digits and inner hyphens"
        )));
    }
    Ok(())
}

impl TryFrom<String> for PluginId {
    type Error = PluginError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        PluginId::parse(&value)
    }
}

impl From<PluginId> for String {
    fn from(value: PluginId) -> Self {
        value.0
    }
}

impl FromStr for PluginId {
    type Err = PluginError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        PluginId::parse(s)
    }
}

impl fmt::Display for PluginId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

// ---------------------------------------------------------------------------
// Activation
// ---------------------------------------------------------------------------

/// When the host should load a plugin's script.
///
/// Nothing runs at startup unless the plugin says so. A plugin that only adds
/// a page costs nothing until the user opens that page, which is what keeps a
/// dozen installed plugins from becoming a dozen runtimes at boot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum ActivationEvent {
    /// Load as soon as the launcher is ready. Use sparingly.
    Startup,
    /// Load when a contributed page or instance panel is opened.
    View(String),
    /// Load when a contributed command is invoked.
    Command(String),
    /// Load when a documented launcher event fires.
    Event(String),
    /// Load when the user opens any instance.
    InstanceOpened,
}

impl ActivationEvent {
    pub fn parse(raw: &str) -> PluginResult<Self> {
        match raw.split_once(':') {
            None => match raw {
                "onStartup" => Ok(ActivationEvent::Startup),
                "onInstanceOpened" => Ok(ActivationEvent::InstanceOpened),
                other => Err(PluginError::invalid_manifest(format!(
                    "`{other}` is not an activation event; expected onStartup, onInstanceOpened, onView:<id>, onCommand:<id> or onEvent:<name>"
                ))),
            },
            Some(("onView", id)) => non_empty("onView", id).map(ActivationEvent::View),
            Some(("onCommand", id)) => non_empty("onCommand", id).map(ActivationEvent::Command),
            Some(("onEvent", name)) => non_empty("onEvent", name).map(ActivationEvent::Event),
            Some((prefix, _)) => Err(PluginError::invalid_manifest(format!(
                "`{prefix}:` is not an activation event prefix"
            ))),
        }
    }

    pub fn as_string(&self) -> String {
        match self {
            ActivationEvent::Startup => "onStartup".into(),
            ActivationEvent::InstanceOpened => "onInstanceOpened".into(),
            ActivationEvent::View(id) => format!("onView:{id}"),
            ActivationEvent::Command(id) => format!("onCommand:{id}"),
            ActivationEvent::Event(name) => format!("onEvent:{name}"),
        }
    }
}

fn non_empty(prefix: &str, value: &str) -> PluginResult<String> {
    if value.is_empty() {
        Err(PluginError::invalid_manifest(format!(
            "`{prefix}:` needs an identifier after the colon"
        )))
    } else {
        Ok(value.to_string())
    }
}

impl TryFrom<String> for ActivationEvent {
    type Error = PluginError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        ActivationEvent::parse(&value)
    }
}

impl From<ActivationEvent> for String {
    fn from(value: ActivationEvent) -> Self {
        value.as_string()
    }
}

// ---------------------------------------------------------------------------
// Manifest
// ---------------------------------------------------------------------------

/// `agora-plugin.json`, as written by the author.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PluginManifest {
    /// Schema version of this file. Currently always `1`.
    pub manifest: u32,
    pub id: PluginId,
    pub name: String,
    pub version: semver::Version,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// SPDX identifier. Required: a plugin with no stated licence is one
    /// nobody can safely fork, and Agora's whole point is that they can.
    pub license: String,
    /// Where the source lives. Optional, but the manager shows it prominently
    /// and its absence is worth noticing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// Host API versions this plugin supports, e.g. `">=0.1, <0.2"`.
    pub api_range: semver::VersionReq,
    /// Package-relative path to the bundled JavaScript entrypoint. Omitted by
    /// a purely declarative plugin such as a theme.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entrypoint: Option<String>,
    #[serde(default)]
    pub activation: Vec<ActivationEvent>,
    #[serde(default)]
    pub capabilities: CapabilityRequest,
    /// Other plugins that must be installed and enabled, by version range.
    #[serde(default)]
    pub dependencies: BTreeMap<PluginId, semver::VersionReq>,
    /// Hosts this plugin may reach, if it asks for the `network` capability.
    #[serde(default)]
    pub network: NetworkDeclaration,
    #[serde(default)]
    pub contributions: Contributions,
    /// Version of the plugin's own stored data shape. Bumping it tells the
    /// host a migration is expected on upgrade, and lets the host keep a
    /// checkpoint of the old data before applying one.
    #[serde(default = "default_data_version")]
    pub data_version: u32,
}

fn default_data_version() -> u32 {
    1
}

/// The hosts a plugin says it will talk to.
///
/// Holding the `network` capability is not permission to reach the internet;
/// it is permission to reach *these* hosts. The list is shown to the user at
/// install time, which is only meaningful because it is also what the network
/// broker enforces at request time.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NetworkDeclaration {
    #[serde(default)]
    pub hosts: Vec<String>,
}

impl NetworkDeclaration {
    /// How many hosts one plugin may declare. A list long enough to hide
    /// something in is not a list the user can meaningfully consent to.
    pub const MAX_HOSTS: usize = 10;

    pub fn is_empty(&self) -> bool {
        self.hosts.is_empty()
    }
}

/// Reject anything that is not a plain public hostname.
///
/// Wildcards are refused because `*.example.com` is not something a user can
/// evaluate, and loopback and private ranges are refused because a plugin
/// reaching `127.0.0.1` is reaching the user's own machine — including
/// Agora's own MCP server — rather than the internet.
fn validate_declared_host(host: &str) -> PluginResult<()> {
    if host.is_empty() || host.len() > 253 {
        return Err(PluginError::invalid_manifest(format!(
            "`{host}` is not a hostname"
        )));
    }
    if host != host.to_ascii_lowercase() {
        return Err(PluginError::invalid_manifest(format!(
            "host `{host}` must be lowercase"
        )));
    }
    if host.contains("://") || host.contains('/') || host.contains('?') || host.contains('@') {
        return Err(PluginError::invalid_manifest(format!(
            "`{host}` must be a bare hostname, not a URL"
        )));
    }
    if host.contains('*') {
        return Err(PluginError::invalid_manifest(format!(
            "`{host}` may not use a wildcard; list each host you need"
        )));
    }
    if host.contains(':') {
        return Err(PluginError::invalid_manifest(format!(
            "`{host}` must not include a port"
        )));
    }
    if host == "localhost" || host.ends_with(".localhost") || host.ends_with(".local") {
        return Err(PluginError::invalid_manifest(format!(
            "`{host}` is on this machine; plugins may not reach local services"
        )));
    }
    // An IP literal sidesteps DNS, and with it the private-range rejection
    // that protects the user's own network.
    if host.parse::<std::net::IpAddr>().is_ok() {
        return Err(PluginError::invalid_manifest(format!(
            "`{host}` is an IP address; declare a hostname"
        )));
    }
    if !host.contains('.') {
        return Err(PluginError::invalid_manifest(format!(
            "`{host}` is not a fully qualified hostname"
        )));
    }
    let valid = host.split('.').all(|label| {
        !label.is_empty()
            && label.len() <= 63
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    });
    if !valid {
        return Err(PluginError::invalid_manifest(format!(
            "`{host}` is not a valid hostname"
        )));
    }
    Ok(())
}

impl PluginManifest {
    /// Parse and validate in one step. Prefer this over `serde_json` directly:
    /// deserialisation alone does not enforce the cross-field rules.
    pub fn parse(json: &str) -> PluginResult<Self> {
        let manifest: PluginManifest = serde_json::from_str(json).map_err(|e| {
            PluginError::invalid_manifest(format!("{MANIFEST_FILENAME} is not valid: {e}"))
        })?;
        manifest.validate()?;
        Ok(manifest)
    }

    /// Enforce every rule that spans more than one field.
    pub fn validate(&self) -> PluginResult<()> {
        if self.manifest != MANIFEST_SCHEMA_VERSION {
            return Err(PluginError::invalid_manifest(format!(
                "manifest schema {} is not supported; this Agora reads schema {}",
                self.manifest, MANIFEST_SCHEMA_VERSION
            )));
        }
        if self.name.trim().is_empty() {
            return Err(PluginError::invalid_manifest("`name` must not be empty"));
        }
        if self.license.trim().is_empty() {
            return Err(PluginError::invalid_manifest(
                "`license` must name an SPDX identifier, for example `MIT` or `GPL-3.0-only`",
            ));
        }
        if let Some(entrypoint) = &self.entrypoint {
            validate_package_path("entrypoint", entrypoint)?;
            if !entrypoint.ends_with(".js") && !entrypoint.ends_with(".mjs") {
                return Err(PluginError::invalid_manifest(format!(
                    "entrypoint `{entrypoint}` must be bundled JavaScript (.js or .mjs); \
                     TypeScript is compiled at packaging time, not by the launcher"
                )));
            }
        }
        if let Some(source) = &self.source {
            if !source.starts_with("https://") {
                return Err(PluginError::invalid_manifest(format!(
                    "`source` must be an https URL, got `{source}`"
                )));
            }
        }
        if self.dependencies.contains_key(&self.id) {
            return Err(PluginError::new(
                PluginErrorCode::DependencyCycle,
                format!("`{}` depends on itself", self.id),
            ));
        }
        self.validate_contributions()?;
        self.validate_activation()?;
        self.validate_network()?;
        Ok(())
    }

    fn validate_network(&self) -> PluginResult<()> {
        let wants_network = self
            .capabilities
            .required
            .iter()
            .chain(self.capabilities.optional.iter())
            .any(|name| name == "network");

        if self.network.hosts.len() > NetworkDeclaration::MAX_HOSTS {
            return Err(PluginError::invalid_manifest(format!(
                "this plugin declares {} hosts; the limit is {}",
                self.network.hosts.len(),
                NetworkDeclaration::MAX_HOSTS
            )));
        }
        for host in &self.network.hosts {
            validate_declared_host(host)?;
        }
        // Both halves have to agree, in both directions: a capability with no
        // hosts can never do anything, and hosts with no capability is a
        // permission the user was never asked about.
        if wants_network && self.network.hosts.is_empty() {
            return Err(PluginError::invalid_manifest(
                "the `network` capability was requested but no hosts were declared;                  list the hosts this plugin talks to under `network.hosts`",
            ));
        }
        if !wants_network && !self.network.hosts.is_empty() {
            return Err(PluginError::invalid_manifest(
                "`network.hosts` was declared without requesting the `network` capability",
            ));
        }
        Ok(())
    }

    fn validate_contributions(&self) -> PluginResult<()> {
        // A plugin colliding with itself is an author mistake worth naming
        // precisely; two plugins colliding is the host's problem, later.
        let mut seen: BTreeMap<(crate::contributions::ContributionKind, &str), ()> =
            BTreeMap::new();
        for (kind, id) in self.contributions.local_ids() {
            validate_local_id(kind, id)?;
            if seen.insert((kind, id), ()).is_some() {
                return Err(PluginError::new(
                    PluginErrorCode::DuplicateContribution,
                    format!("`{id}` is declared twice as a {kind}"),
                ));
            }
        }

        // Every script-backed contribution needs somewhere to call into.
        let needs_script = !self.contributions.commands.is_empty()
            || !self.contributions.diagnostics.is_empty()
            || !self.contributions.launch_checks.is_empty()
            || self
                .contributions
                .pages
                .iter()
                .any(|p| matches!(p.view, crate::contributions::ViewSource::Host { .. }))
            || self
                .contributions
                .instance_panels
                .iter()
                .any(|p| matches!(p.view, crate::contributions::ViewSource::Host { .. }))
            || !self.contributions.replacements.is_empty();
        if needs_script && self.entrypoint.is_none() {
            return Err(PluginError::invalid_manifest(
                "this plugin contributes something that has to call into script, \
                 but declares no `entrypoint`",
            ));
        }

        for page in &self.contributions.pages {
            if let crate::contributions::ViewSource::Custom { html } = &page.view {
                validate_package_path("page view html", html)?;
            }
        }
        for panel in &self.contributions.instance_panels {
            if let crate::contributions::ViewSource::Custom { html } = &panel.view {
                validate_package_path("instance panel view html", html)?;
            }
        }
        for replacement in &self.contributions.replacements {
            // A replacement is the whole screen. The host-rendered path is the
            // one that is themed, accessible and controller-navigable by
            // construction, and a custom frame is still a prototype — putting
            // an unproven one where the home page used to be is precisely
            // "building a product on it".
            if !matches!(
                replacement.view,
                crate::contributions::ViewSource::Host { .. }
            ) {
                return Err(PluginError::invalid_manifest(format!(
                    "replacement `{}` must use a host-rendered view; a custom frame may add a                      page but may not stand in for a built-in surface",
                    replacement.id
                )));
            }
        }

        // Two offers for the same surface from one plugin is an author
        // mistake: the picker would show the same plugin twice with no way to
        // tell the entries apart from their origin.
        let mut surfaces = BTreeMap::new();
        for replacement in &self.contributions.replacements {
            if surfaces
                .insert(replacement.surface, replacement.id.as_str())
                .is_some()
            {
                return Err(PluginError::new(
                    PluginErrorCode::DuplicateContribution,
                    format!(
                        "this plugin offers to replace `{}` more than once",
                        replacement.surface
                    ),
                ));
            }
        }

        for check in &self.contributions.launch_checks {
            if check.timeout_ms == 0 {
                return Err(PluginError::invalid_manifest(format!(
                    "launch check `{}` has a zero timeout",
                    check.id
                )));
            }
        }
        Ok(())
    }

    fn validate_activation(&self) -> PluginResult<()> {
        // An activation event pointing at a contribution the plugin does not
        // have will simply never fire, which reads to the author as "my plugin
        // does nothing" with no clue why. Catch it here instead.
        for event in &self.activation {
            match event {
                ActivationEvent::View(id) => {
                    // Replacements count: opening a surface the user chose this
                    // plugin for is opening one of its views, and a replacement
                    // that never started would render as the built-in with no
                    // explanation.
                    let known = self.contributions.pages.iter().any(|p| &p.id == id)
                        || self
                            .contributions
                            .instance_panels
                            .iter()
                            .any(|p| &p.id == id)
                        || self.contributions.replacements.iter().any(|p| &p.id == id);
                    if !known {
                        return Err(PluginError::invalid_manifest(format!(
                            "activation `onView:{id}` names no page, instance panel or                              replacement in this manifest"
                        )));
                    }
                }
                ActivationEvent::Command(id) => {
                    if !self.contributions.commands.iter().any(|c| &c.id == id) {
                        return Err(PluginError::invalid_manifest(format!(
                            "activation `onCommand:{id}` names no command in this manifest"
                        )));
                    }
                }
                ActivationEvent::Startup
                | ActivationEvent::InstanceOpened
                | ActivationEvent::Event(_) => {}
            }
        }
        if self.entrypoint.is_none() && !self.activation.is_empty() {
            return Err(PluginError::invalid_manifest(
                "activation events were declared but there is no `entrypoint` to activate",
            ));
        }
        Ok(())
    }

    /// Whether this plugin runs any script at all.
    pub fn is_declarative_only(&self) -> bool {
        self.entrypoint.is_none()
    }

    /// Whether the plugin supports the host it is about to run on.
    pub fn supports_host(&self, host: &semver::Version) -> bool {
        // `VersionReq` treats a pre-release as out of range unless the range
        // names one. Plugin versions are ordinary releases, so the plain
        // `matches` is the behaviour authors expect.
        self.api_range.matches(host)
    }
}

/// Reject any path that could escape the package directory.
///
/// Checked here, in the contract, rather than only at extraction time: the
/// same rule has to hold for a local development folder, which is never
/// extracted from anything.
pub fn validate_package_path(label: &str, path: &str) -> PluginResult<()> {
    if path.is_empty() {
        return Err(PluginError::invalid_manifest(format!("{label} is empty")));
    }
    if path.starts_with('/') || path.starts_with('\\') {
        return Err(PluginError::invalid_manifest(format!(
            "{label} `{path}` must be relative to the package"
        )));
    }
    // `C:\...`, `\\server\share`, and every other rooted Windows form.
    if path.len() >= 2 && path.as_bytes()[1] == b':' {
        return Err(PluginError::invalid_manifest(format!(
            "{label} `{path}` must be relative to the package"
        )));
    }
    for component in path.split(['/', '\\']) {
        if component == ".." {
            return Err(PluginError::invalid_manifest(format!(
                "{label} `{path}` climbs out of the package"
            )));
        }
        if component.is_empty() || component == "." {
            return Err(PluginError::invalid_manifest(format!(
                "{label} `{path}` has an empty or `.` path segment"
            )));
        }
        // NTFS alternate data streams, and anything else with a colon.
        if component.contains(':') {
            return Err(PluginError::invalid_manifest(format!(
                "{label} `{path}` contains a `:` in a path segment"
            )));
        }
    }
    if path.contains('\0') {
        return Err(PluginError::invalid_manifest(format!(
            "{label} contains a NUL byte"
        )));
    }
    Ok(())
}

fn validate_local_id(kind: crate::contributions::ContributionKind, id: &str) -> PluginResult<()> {
    if id.is_empty() || id.len() > 64 {
        return Err(PluginError::invalid_manifest(format!(
            "{kind} id `{id}` must be 1-64 characters"
        )));
    }
    // `/` is checked first, and separately: it is the namespace separator the
    // host adds, so an author who uses one has made a specific mistake and
    // deserves to be told which one rather than a generic charset complaint.
    if id.contains('/') {
        return Err(PluginError::invalid_manifest(format!(
            "{kind} id `{id}` must not contain `/`; Agora adds the plugin namespace itself"
        )));
    }
    let ok = id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.');
    if !ok {
        return Err(PluginError::invalid_manifest(format!(
            "{kind} id `{id}` may only contain letters, digits, `-`, `_` and `.`"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_json() -> serde_json::Value {
        serde_json::json!({
            "manifest": 1,
            "id": "acme.dashboard",
            "name": "Instance Dashboard",
            "version": "1.0.0",
            "license": "MIT",
            "apiRange": ">=0.1, <0.2",
            "entrypoint": "dist/main.js",
        })
    }

    fn parse(value: serde_json::Value) -> PluginResult<PluginManifest> {
        PluginManifest::parse(&value.to_string())
    }

    #[test]
    fn parses_a_minimal_manifest() {
        let manifest = parse(minimal_json()).unwrap();
        assert_eq!(manifest.id.as_str(), "acme.dashboard");
        assert_eq!(manifest.id.publisher(), "acme");
        assert_eq!(manifest.id.name(), "dashboard");
        assert_eq!(manifest.data_version, 1);
        assert!(manifest.supports_host(&semver::Version::new(0, 1, 0)));
        assert!(!manifest.supports_host(&semver::Version::new(0, 2, 0)));
    }

    #[test]
    fn qualifies_contribution_ids_with_the_plugin_namespace() {
        let id = PluginId::parse("acme.dashboard").unwrap();
        assert_eq!(id.qualify("overview"), "acme.dashboard/overview");
    }

    #[test]
    fn rejects_an_id_without_a_publisher_segment() {
        let mut json = minimal_json();
        json["id"] = serde_json::json!("dashboard");
        let err = parse(json).unwrap_err();
        assert_eq!(err.code, PluginErrorCode::InvalidManifest);
        assert!(err.message.contains("publisher.plugin"), "{}", err.message);
    }

    #[test]
    fn rejects_an_id_with_uppercase_or_spaces() {
        for bad in [
            "Acme.dashboard",
            "acme.Dash board",
            "acme.-dash",
            "acme.dash-",
        ] {
            assert!(
                PluginId::parse(bad).is_err(),
                "`{bad}` should have been rejected"
            );
        }
    }

    #[test]
    fn rejects_an_unknown_manifest_schema_version_rather_than_guessing() {
        let mut json = minimal_json();
        json["manifest"] = serde_json::json!(2);
        let err = parse(json).unwrap_err();
        assert!(err.message.contains("schema 2"), "{}", err.message);
    }

    #[test]
    fn rejects_an_unknown_field_so_a_typo_is_not_silently_ignored() {
        let mut json = minimal_json();
        json["entryPoint"] = serde_json::json!("dist/main.js");
        assert!(parse(json).is_err());
    }

    #[test]
    fn rejects_an_entrypoint_that_climbs_out_of_the_package() {
        for bad in [
            "../evil.js",
            "dist/../../evil.js",
            "/etc/evil.js",
            "C:/evil.js",
        ] {
            let mut json = minimal_json();
            json["entrypoint"] = serde_json::json!(bad);
            let err = parse(json).unwrap_err();
            assert_eq!(err.code, PluginErrorCode::InvalidManifest, "`{bad}`");
        }
    }

    #[test]
    fn rejects_an_entrypoint_that_is_not_bundled_javascript() {
        let mut json = minimal_json();
        json["entrypoint"] = serde_json::json!("src/main.ts");
        let err = parse(json).unwrap_err();
        assert!(
            err.message.contains("bundled JavaScript"),
            "{}",
            err.message
        );
    }

    #[test]
    fn rejects_a_non_https_source_url() {
        let mut json = minimal_json();
        json["source"] = serde_json::json!("http://example.com/plugin");
        assert!(parse(json).is_err());
    }

    #[test]
    fn rejects_a_plugin_that_depends_on_itself() {
        let mut json = minimal_json();
        json["dependencies"] = serde_json::json!({ "acme.dashboard": ">=1.0" });
        let err = parse(json).unwrap_err();
        assert_eq!(err.code, PluginErrorCode::DependencyCycle);
    }

    #[test]
    fn rejects_two_contributions_of_the_same_kind_sharing_an_id() {
        let mut json = minimal_json();
        json["contributions"] = serde_json::json!({
            "commands": [
                { "id": "run", "title": "Run", "export": "a" },
                { "id": "run", "title": "Run again", "export": "b" },
            ]
        });
        let err = parse(json).unwrap_err();
        assert_eq!(err.code, PluginErrorCode::DuplicateContribution);
    }

    #[test]
    fn allows_the_same_id_on_two_different_surfaces() {
        let mut json = minimal_json();
        json["contributions"] = serde_json::json!({
            "commands": [{ "id": "overview", "title": "Overview", "export": "a" }],
            "pages": [{ "id": "overview", "title": "Overview", "view": { "kind": "host", "export": "b" } }],
        });
        assert!(parse(json).is_ok());
    }

    #[test]
    fn rejects_a_contribution_id_containing_the_namespace_separator() {
        let mut json = minimal_json();
        json["contributions"] = serde_json::json!({
            "commands": [{ "id": "acme.dashboard/run", "title": "Run", "export": "a" }]
        });
        let err = parse(json).unwrap_err();
        assert!(err.message.contains("must not contain"), "{}", err.message);
    }

    #[test]
    fn a_theme_needs_no_entrypoint() {
        let json = serde_json::json!({
            "manifest": 1,
            "id": "acme.dusk",
            "name": "Dusk",
            "version": "1.0.0",
            "license": "CC0-1.0",
            "apiRange": ">=0.1, <0.2",
            "contributions": {
                "theme": { "id": "dusk", "title": "Dusk", "dark": { "surface": "#14161c" } }
            }
        });
        let manifest = parse(json).unwrap();
        assert!(manifest.is_declarative_only());
    }

    #[test]
    fn a_script_backed_contribution_without_an_entrypoint_is_rejected() {
        let json = serde_json::json!({
            "manifest": 1,
            "id": "acme.broken",
            "name": "Broken",
            "version": "1.0.0",
            "license": "MIT",
            "apiRange": ">=0.1, <0.2",
            "contributions": {
                "commands": [{ "id": "run", "title": "Run", "export": "run" }]
            }
        });
        let err = parse(json).unwrap_err();
        assert!(err.message.contains("no `entrypoint`"), "{}", err.message);
    }

    #[test]
    fn activation_naming_a_missing_contribution_is_rejected_at_parse_time() {
        let mut json = minimal_json();
        json["activation"] = serde_json::json!(["onView:nowhere"]);
        let err = parse(json).unwrap_err();
        assert!(err.message.contains("names no page"), "{}", err.message);
    }

    #[test]
    fn activation_events_round_trip_through_their_wire_strings() {
        for raw in [
            "onStartup",
            "onInstanceOpened",
            "onView:overview",
            "onCommand:run",
            "onEvent:instance.installed",
        ] {
            let event = ActivationEvent::parse(raw).unwrap();
            assert_eq!(event.as_string(), raw);
        }
    }

    #[test]
    fn an_unrecognised_activation_event_names_what_was_expected() {
        let err = ActivationEvent::parse("whenever").unwrap_err();
        assert!(err.message.contains("onStartup"), "{}", err.message);
    }
}

#[cfg(test)]
mod network_tests {
    use super::*;

    fn with_network(
        capabilities: serde_json::Value,
        hosts: &[&str],
    ) -> PluginResult<PluginManifest> {
        let json = serde_json::json!({
            "manifest": 1,
            "id": "acme.fetcher",
            "name": "Fetcher",
            "version": "1.0.0",
            "license": "MIT",
            "apiRange": ">=0.1, <0.2",
            "entrypoint": "main.js",
            "capabilities": capabilities,
            "network": { "hosts": hosts },
        });
        PluginManifest::parse(&json.to_string())
    }

    #[test]
    fn a_network_plugin_declares_the_hosts_it_will_reach() {
        let manifest = with_network(
            serde_json::json!({ "required": ["network"] }),
            &["api.example.com"],
        )
        .unwrap();
        assert_eq!(manifest.network.hosts, vec!["api.example.com"]);
    }

    #[test]
    fn asking_for_network_without_naming_a_host_is_rejected() {
        let err = with_network(serde_json::json!({ "required": ["network"] }), &[]).unwrap_err();
        assert!(err.message.contains("network.hosts"), "{}", err.message);
    }

    #[test]
    fn naming_hosts_without_asking_for_network_is_rejected() {
        let err = with_network(serde_json::json!({}), &["api.example.com"]).unwrap_err();
        assert!(
            err.message.contains("without requesting"),
            "{}",
            err.message
        );
    }

    #[test]
    fn a_wildcard_host_is_refused_because_a_user_cannot_consent_to_it() {
        let err = with_network(
            serde_json::json!({ "required": ["network"] }),
            &["*.example.com"],
        )
        .unwrap_err();
        assert!(err.message.contains("wildcard"), "{}", err.message);
    }

    #[test]
    fn loopback_and_local_hosts_are_refused() {
        for host in ["localhost", "agora.localhost", "printer.local"] {
            let err =
                with_network(serde_json::json!({ "required": ["network"] }), &[host]).unwrap_err();
            assert!(
                err.message.contains("on this machine"),
                "`{host}`: {}",
                err.message
            );
        }
    }

    #[test]
    fn an_ip_literal_is_refused_so_dns_range_checks_cannot_be_bypassed() {
        for host in ["127.0.0.1", "10.0.0.5", "192.168.1.1"] {
            let err =
                with_network(serde_json::json!({ "required": ["network"] }), &[host]).unwrap_err();
            assert!(
                err.message.contains("IP address"),
                "`{host}`: {}",
                err.message
            );
        }
    }

    #[test]
    fn a_url_where_a_hostname_belongs_is_refused() {
        let err = with_network(
            serde_json::json!({ "required": ["network"] }),
            &["https://api.example.com/v1"],
        )
        .unwrap_err();
        assert!(err.message.contains("bare hostname"), "{}", err.message);
    }

    #[test]
    fn a_host_with_a_port_is_refused() {
        let err = with_network(
            serde_json::json!({ "required": ["network"] }),
            &["api.example.com:8443"],
        )
        .unwrap_err();
        assert!(err.message.contains("port"), "{}", err.message);
    }

    #[test]
    fn too_many_declared_hosts_are_refused() {
        let hosts: Vec<String> = (0..NetworkDeclaration::MAX_HOSTS + 1)
            .map(|i| format!("h{i}.example.com"))
            .collect();
        let refs: Vec<&str> = hosts.iter().map(|h| h.as_str()).collect();
        let err = with_network(serde_json::json!({ "required": ["network"] }), &refs).unwrap_err();
        assert!(err.message.contains("the limit is"), "{}", err.message);
    }

    #[test]
    fn a_plugin_that_wants_no_network_needs_no_declaration() {
        assert!(with_network(serde_json::json!({ "required": ["instance:read"] }), &[]).is_ok());
    }

    fn with_replacements(replacements: serde_json::Value) -> serde_json::Value {
        serde_json::json!({
            "manifest": 1,
            "id": "acme.dashboard",
            "name": "Dashboard",
            "version": "1.0.0",
            "license": "MIT",
            "apiRange": ">=0.1, <0.2",
            "entrypoint": "main.js",
            "contributions": { "replacements": replacements }
        })
    }

    #[test]
    fn a_host_rendered_replacement_is_accepted() {
        let manifest = PluginManifest::parse(
            &with_replacements(serde_json::json!([{
                "id": "home",
                "title": "Compact home",
                "surface": "home",
                "view": { "kind": "host", "export": "home" }
            }]))
            .to_string(),
        )
        .unwrap();
        assert_eq!(manifest.contributions.replacements.len(), 1);
        assert_eq!(
            manifest.contributions.replacements[0].surface,
            crate::contributions::ReplaceableSurface::Home
        );
    }

    /// A replacement is the whole screen, so it has to be the path that is
    /// themed and controller-navigable by construction. A custom frame may
    /// still add a page; it may not stand in for a built-in surface.
    #[test]
    fn a_custom_frame_may_not_replace_a_built_in_surface() {
        let error = PluginManifest::parse(
            &with_replacements(serde_json::json!([{
                "id": "home",
                "title": "Compact home",
                "surface": "home",
                "view": { "kind": "custom", "html": "home.html" }
            }]))
            .to_string(),
        )
        .unwrap_err();
        assert!(error.message.contains("host-rendered"), "{}", error.message);
    }

    #[test]
    fn a_surface_this_build_does_not_know_is_refused_rather_than_ignored() {
        let error = PluginManifest::parse(
            &with_replacements(serde_json::json!([{
                "id": "home",
                "title": "Whatever",
                "surface": "the-entire-launcher",
                "view": { "kind": "host", "export": "home" }
            }]))
            .to_string(),
        )
        .unwrap_err();
        assert_eq!(error.code, PluginErrorCode::InvalidManifest);
    }

    /// One plugin offering the same surface twice would appear in the picker
    /// as two entries the user cannot tell apart by origin.
    #[test]
    fn one_plugin_cannot_offer_the_same_surface_twice() {
        let error = PluginManifest::parse(
            &with_replacements(serde_json::json!([
                {
                    "id": "compact",
                    "title": "Compact",
                    "surface": "home",
                    "view": { "kind": "host", "export": "a" }
                },
                {
                    "id": "roomy",
                    "title": "Roomy",
                    "surface": "home",
                    "view": { "kind": "host", "export": "b" }
                }
            ]))
            .to_string(),
        )
        .unwrap_err();
        assert_eq!(error.code, PluginErrorCode::DuplicateContribution);
    }

    #[test]
    fn a_replacement_needs_an_entrypoint_to_call_into() {
        let mut value = with_replacements(serde_json::json!([{
            "id": "home",
            "title": "Compact home",
            "surface": "home",
            "view": { "kind": "host", "export": "home" }
        }]));
        value.as_object_mut().unwrap().remove("entrypoint");
        let error = PluginManifest::parse(&value.to_string()).unwrap_err();
        assert!(error.message.contains("entrypoint"), "{}", error.message);
    }

    /// Opening a surface the user chose this plugin for is opening one of its
    /// views, so `onView:` has to accept a replacement id — otherwise the
    /// plugin never starts and the surface silently renders as the built-in.
    #[test]
    fn activation_may_name_a_replacement() {
        let mut value = with_replacements(serde_json::json!([{
            "id": "home",
            "title": "Compact home",
            "surface": "home",
            "view": { "kind": "host", "export": "home" }
        }]));
        value["activation"] = serde_json::json!(["onView:home"]);
        assert!(PluginManifest::parse(&value.to_string()).is_ok());

        value["activation"] = serde_json::json!(["onView:nothing-here"]);
        assert!(PluginManifest::parse(&value.to_string()).is_err());
    }
}
