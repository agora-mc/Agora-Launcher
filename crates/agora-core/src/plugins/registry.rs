//! Deciding which plugins may run, and in what order.
//!
//! Resolution is a pure function of the install records and the host API
//! version. It does no I/O, touches no filesystem and runs no plugin code,
//! which is what lets the plugin manager show an accurate picture of a broken
//! set-up *without* loading any of it — and what lets "disable everything"
//! work when the thing that is broken is a plugin that crashes on activation.
//!
//! Four questions get answered, in this order, because each depends on the
//! previous one:
//!
//! 1. Is this plugin's `apiRange` satisfied by the running host?
//! 2. Are its dependencies installed, enabled, and of an acceptable version?
//! 3. Is the dependency graph acyclic?
//! 4. Given all that, what order should they activate in?

use super::store::{self, PluginRecord};
use agora_plugin_api::contributions::ContributionKind;
use agora_plugin_api::manifest::PluginId;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Why a plugin is, or is not, going to run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "state", content = "detail")]
pub enum PluginStatus {
    /// Enabled, resolvable, and ready to activate.
    Ready,
    /// Installed but switched off by the user.
    Disabled,
    /// The plugin does not support this version of Agora.
    IncompatibleApi { required: String, host: String },
    /// A dependency is missing, disabled, or the wrong version.
    UnresolvedDependency { dependency: String, reason: String },
    /// This plugin is part of a dependency cycle.
    DependencyCycle { cycle: Vec<String> },
    /// Activation failed the last time it was tried.
    Failed { message: String },
}

impl PluginStatus {
    /// Whether the host should try to activate this plugin.
    pub fn is_runnable(&self) -> bool {
        matches!(self, PluginStatus::Ready)
    }

    /// A sentence for the plugin manager, phrased for whoever has to fix it.
    pub fn explain(&self) -> String {
        match self {
            PluginStatus::Ready => "Ready".to_string(),
            PluginStatus::Disabled => "Turned off".to_string(),
            PluginStatus::IncompatibleApi { required, host } => format!(
                "Needs Agora plugin API {required}; this version provides {host}. \
                 Check for a plugin update."
            ),
            PluginStatus::UnresolvedDependency { dependency, reason } => {
                format!("Needs `{dependency}`: {reason}")
            }
            PluginStatus::DependencyCycle { cycle } => {
                format!("Dependency cycle: {}", cycle.join(" \u{2192} "))
            }
            PluginStatus::Failed { message } => format!("Failed to start: {message}"),
        }
    }
}

/// One contribution, with the plugin namespace already applied.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NamespacedContribution {
    /// `publisher.plugin/local`.
    pub id: String,
    pub plugin_id: String,
    pub local_id: String,
    pub kind: ContributionKind,
    pub title: String,
}

/// A plugin plus everything decided about it.
#[derive(Debug, Clone)]
pub struct ResolvedPlugin {
    pub record: PluginRecord,
    pub status: PluginStatus,
}

impl ResolvedPlugin {
    pub fn id(&self) -> &PluginId {
        self.record.id()
    }
}

/// The whole picture, after resolution.
#[derive(Debug, Clone, Default)]
pub struct Resolution {
    /// Every installed plugin, in id order, with its status.
    pub plugins: Vec<ResolvedPlugin>,
    /// Runnable plugins in the order they should be activated: a plugin never
    /// appears before something it depends on.
    pub activation_order: Vec<PluginId>,
    /// Contributions from runnable plugins, ready for the UI.
    pub contributions: Vec<NamespacedContribution>,
    /// Problems worth telling the user about that are not attached to a single
    /// plugin — for example two sources claiming the same plugin id.
    pub warnings: Vec<String>,
}

impl Resolution {
    pub fn get(&self, plugin_id: &PluginId) -> Option<&ResolvedPlugin> {
        self.plugins.iter().find(|p| p.id() == plugin_id)
    }

    pub fn runnable(&self) -> impl Iterator<Item = &ResolvedPlugin> {
        self.plugins.iter().filter(|p| p.status.is_runnable())
    }
}

/// Work out what can run.
///
/// `records` may include tombstones from an uninstall that kept data; they are
/// dropped here rather than by every caller.
pub fn resolve(records: Vec<PluginRecord>, host_api: &semver::Version) -> Resolution {
    let mut records: Vec<PluginRecord> = records
        .into_iter()
        .filter(|record| !store::is_tombstone(record))
        .collect();
    // Sorted here rather than relying on the caller. `store::list` happens to
    // return id order today, but the determinism the activation order promises
    // must not depend on that staying true — or on a caller that assembled the
    // list some other way.
    records.sort_by(|a, b| a.id().cmp(b.id()));

    let by_id: BTreeMap<PluginId, &PluginRecord> = records
        .iter()
        .map(|record| (record.id().clone(), record))
        .collect();

    let mut statuses: BTreeMap<PluginId, PluginStatus> = BTreeMap::new();
    let mut warnings = Vec::new();

    // 1. Compatibility and the user's own switch. A disabled plugin is not
    //    checked any further: the user turning something off should not
    //    produce a wall of complaints about it.
    for record in &records {
        let status = if !record.enabled {
            PluginStatus::Disabled
        } else if let Some(message) = &record.last_error {
            PluginStatus::Failed {
                message: message.clone(),
            }
        } else if !record.manifest.supports_host(host_api) {
            PluginStatus::IncompatibleApi {
                required: record.manifest.api_range.to_string(),
                host: host_api.to_string(),
            }
        } else {
            PluginStatus::Ready
        };
        statuses.insert(record.id().clone(), status);
    }

    // 2. Dependencies. Iterated to a fixed point because a plugin whose
    //    dependency turns out to be unresolvable is itself unresolvable, and
    //    that has to propagate up the chain rather than one level.
    loop {
        let mut changed = false;
        for record in &records {
            if !matches!(statuses.get(record.id()), Some(PluginStatus::Ready)) {
                continue;
            }
            for (dependency, requirement) in &record.manifest.dependencies {
                let reason = match by_id.get(dependency) {
                    None => Some("not installed".to_string()),
                    Some(dep) if !dep.enabled => Some("installed but turned off".to_string()),
                    Some(dep) if !requirement.matches(&dep.manifest.version) => Some(format!(
                        "version {} is installed, but `{requirement}` is required",
                        dep.manifest.version
                    )),
                    Some(dep) => match statuses.get(dep.id()) {
                        Some(PluginStatus::Ready) | None => None,
                        Some(other) => Some(other.explain()),
                    },
                };
                if let Some(reason) = reason {
                    statuses.insert(
                        record.id().clone(),
                        PluginStatus::UnresolvedDependency {
                            dependency: dependency.to_string(),
                            reason,
                        },
                    );
                    changed = true;
                    break;
                }
            }
        }
        if !changed {
            break;
        }
    }

    // 3. Cycles, among what is still standing.
    let ready: Vec<&PluginRecord> = records
        .iter()
        .filter(|record| matches!(statuses.get(record.id()), Some(PluginStatus::Ready)))
        .collect();
    for cycle in find_cycles(&ready) {
        for member in &cycle {
            statuses.insert(
                member.clone(),
                PluginStatus::DependencyCycle {
                    cycle: cycle.iter().map(|id| id.to_string()).collect(),
                },
            );
        }
    }

    // 4. Order. Deterministic: ties break on plugin id, so the same set of
    //    plugins always activates in the same order and a bug that depends on
    //    ordering is reproducible rather than intermittent.
    let runnable: Vec<&PluginRecord> = records
        .iter()
        .filter(|record| matches!(statuses.get(record.id()), Some(PluginStatus::Ready)))
        .collect();
    let activation_order = topological_order(&runnable);

    // Contributions, namespaced. Collisions are impossible by construction
    // once namespaced, but the check stays: it is the thing that would catch a
    // future change that stopped namespacing them.
    let mut contributions = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for plugin_id in &activation_order {
        let Some(record) = by_id.get(plugin_id) else {
            continue;
        };
        for (kind, local_id) in record.manifest.contributions.local_ids() {
            let id = record.manifest.id.qualify(local_id);
            if !seen.insert(id.clone()) {
                warnings.push(format!(
                    "two plugins contributed `{id}`; the later one was ignored"
                ));
                continue;
            }
            contributions.push(NamespacedContribution {
                title: contribution_title(record, kind, local_id),
                id,
                plugin_id: plugin_id.to_string(),
                local_id: local_id.to_string(),
                kind,
            });
        }
    }

    let plugins = records
        .into_iter()
        .map(|record| {
            let status = statuses
                .get(record.id())
                .cloned()
                .unwrap_or(PluginStatus::Disabled);
            ResolvedPlugin { record, status }
        })
        .collect();

    Resolution {
        plugins,
        activation_order,
        contributions,
        warnings,
    }
}

fn contribution_title(record: &PluginRecord, kind: ContributionKind, local_id: &str) -> String {
    let contributions = &record.manifest.contributions;
    let found = match kind {
        ContributionKind::Page => contributions
            .pages
            .iter()
            .find(|c| c.id == local_id)
            .map(|c| c.title.clone()),
        ContributionKind::InstancePanel => contributions
            .instance_panels
            .iter()
            .find(|c| c.id == local_id)
            .map(|c| c.title.clone()),
        ContributionKind::Command => contributions
            .commands
            .iter()
            .find(|c| c.id == local_id)
            .map(|c| c.title.clone()),
        ContributionKind::Setting => contributions
            .settings
            .iter()
            .find(|c| c.key == local_id)
            .map(|c| c.title.clone()),
        ContributionKind::Diagnostic => contributions
            .diagnostics
            .iter()
            .find(|c| c.id == local_id)
            .map(|c| c.title.clone()),
        ContributionKind::LaunchCheck => contributions
            .launch_checks
            .iter()
            .find(|c| c.id == local_id)
            .map(|c| c.title.clone()),
        ContributionKind::Theme => contributions.theme.as_ref().map(|c| c.title.clone()),
        ContributionKind::Replacement => contributions
            .replacements
            .iter()
            .find(|c| c.id == local_id)
            .map(|c| c.title.clone()),
    };
    found.unwrap_or_else(|| local_id.to_string())
}

/// Every cycle in the dependency graph, as lists of participating ids.
fn find_cycles(records: &[&PluginRecord]) -> Vec<Vec<PluginId>> {
    let present: BTreeSet<&PluginId> = records.iter().map(|record| record.id()).collect();
    let edges: BTreeMap<&PluginId, Vec<&PluginId>> = records
        .iter()
        .map(|record| {
            let deps = record
                .manifest
                .dependencies
                .keys()
                .filter(|dep| present.contains(dep))
                .collect();
            (record.id(), deps)
        })
        .collect();

    let mut cycles = Vec::new();
    let mut visiting: Vec<&PluginId> = Vec::new();
    let mut done: BTreeSet<&PluginId> = BTreeSet::new();

    fn walk<'a>(
        node: &'a PluginId,
        edges: &BTreeMap<&'a PluginId, Vec<&'a PluginId>>,
        visiting: &mut Vec<&'a PluginId>,
        done: &mut BTreeSet<&'a PluginId>,
        cycles: &mut Vec<Vec<PluginId>>,
    ) {
        if done.contains(node) {
            return;
        }
        if let Some(start) = visiting.iter().position(|seen| *seen == node) {
            let mut cycle: Vec<PluginId> =
                visiting[start..].iter().map(|id| (*id).clone()).collect();
            cycle.push(node.clone());
            cycles.push(cycle);
            return;
        }
        visiting.push(node);
        for next in edges.get(node).into_iter().flatten() {
            walk(next, edges, visiting, done, cycles);
        }
        visiting.pop();
        done.insert(node);
    }

    for record in records {
        walk(record.id(), &edges, &mut visiting, &mut done, &mut cycles);
    }
    cycles
}

/// Dependencies first, ties broken by id.
fn topological_order(records: &[&PluginRecord]) -> Vec<PluginId> {
    let present: BTreeSet<&PluginId> = records.iter().map(|record| record.id()).collect();
    let mut order: Vec<PluginId> = Vec::new();
    let mut placed: BTreeSet<PluginId> = BTreeSet::new();

    // `records` arrives in id order, so repeatedly taking the first plugin
    // whose dependencies are already placed yields a stable result.
    let mut remaining: Vec<&PluginRecord> = records.to_vec();
    while !remaining.is_empty() {
        let next = remaining.iter().position(|record| {
            record
                .manifest
                .dependencies
                .keys()
                .filter(|dep| present.contains(dep))
                .all(|dep| placed.contains(dep))
        });
        match next {
            Some(index) => {
                let record = remaining.remove(index);
                placed.insert(record.id().clone());
                order.push(record.id().clone());
            }
            // Only reachable if a cycle slipped through; emitting the rest in
            // id order beats dropping them silently.
            None => {
                for record in remaining.drain(..) {
                    order.push(record.id().clone());
                }
            }
        }
    }
    order
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugins::store::PluginSource;
    use agora_plugin_api::capability::CapabilitySet;
    use agora_plugin_api::manifest::PluginManifest;

    fn host() -> semver::Version {
        semver::Version::new(0, 1, 0)
    }

    fn record(id: &str, deps: &[(&str, &str)], enabled: bool) -> PluginRecord {
        record_with(id, deps, enabled, ">=0.1, <0.2", "1.0.0")
    }

    fn record_with(
        id: &str,
        deps: &[(&str, &str)],
        enabled: bool,
        api_range: &str,
        version: &str,
    ) -> PluginRecord {
        let dependencies: serde_json::Map<String, serde_json::Value> = deps
            .iter()
            .map(|(k, v)| ((*k).to_string(), serde_json::json!(v)))
            .collect();
        let manifest: PluginManifest = serde_json::from_value(serde_json::json!({
            "manifest": 1,
            "id": id,
            "name": id,
            "version": version,
            "license": "MIT",
            "apiRange": api_range,
            "entrypoint": "main.js",
            "dependencies": dependencies,
            "contributions": {
                "commands": [{ "id": "run", "title": "Run", "export": "run" }]
            }
        }))
        .unwrap();
        PluginRecord {
            manifest,
            granted: CapabilitySet::new(),
            source: PluginSource::Package,
            install_dir: std::path::PathBuf::from("/plugins").join(id),
            enabled,
            installed_at: "2026-01-01T00:00:00Z".into(),
            updated_at: "2026-01-01T00:00:00Z".into(),
            data_version: 1,
            last_error: None,
        }
    }

    fn ids(order: &[PluginId]) -> Vec<String> {
        order.iter().map(|id| id.to_string()).collect()
    }

    #[test]
    fn an_enabled_compatible_plugin_is_ready() {
        let resolution = resolve(vec![record("acme.one", &[], true)], &host());
        assert_eq!(resolution.activation_order.len(), 1);
        assert!(resolution.plugins[0].status.is_runnable());
    }

    #[test]
    fn a_disabled_plugin_is_not_activated_and_is_not_complained_about() {
        let resolution = resolve(vec![record("acme.one", &[], false)], &host());
        assert!(resolution.activation_order.is_empty());
        assert_eq!(resolution.plugins[0].status, PluginStatus::Disabled);
        assert!(resolution.contributions.is_empty());
    }

    #[test]
    fn a_plugin_for_a_newer_api_is_reported_as_incompatible_not_broken() {
        let resolution = resolve(
            vec![record_with("acme.future", &[], true, ">=0.9", "1.0.0")],
            &host(),
        );
        let status = &resolution.plugins[0].status;
        assert!(matches!(status, PluginStatus::IncompatibleApi { .. }));
        assert!(status.explain().contains("Check for a plugin update"));
    }

    #[test]
    fn dependencies_activate_before_their_dependents() {
        let resolution = resolve(
            vec![
                record("z.last", &[("a.first", ">=1.0")], true),
                record("a.first", &[], true),
            ],
            &host(),
        );
        assert_eq!(ids(&resolution.activation_order), vec!["a.first", "z.last"]);
    }

    #[test]
    fn ordering_is_deterministic_when_nothing_depends_on_anything() {
        let resolution = resolve(
            vec![
                record("c.three", &[], true),
                record("a.one", &[], true),
                record("b.two", &[], true),
            ],
            &host(),
        );
        assert_eq!(
            ids(&resolution.activation_order),
            vec!["a.one", "b.two", "c.three"]
        );
    }

    #[test]
    fn a_missing_dependency_names_what_is_missing() {
        let resolution = resolve(
            vec![record("acme.needy", &[("other.thing", ">=1.0")], true)],
            &host(),
        );
        let status = &resolution.plugins[0].status;
        assert!(status.explain().contains("other.thing"), "{status:?}");
        assert!(status.explain().contains("not installed"), "{status:?}");
        assert!(resolution.activation_order.is_empty());
    }

    #[test]
    fn a_dependency_that_is_installed_but_off_says_so() {
        let resolution = resolve(
            vec![
                record("acme.needy", &[("other.thing", ">=1.0")], true),
                record("other.thing", &[], false),
            ],
            &host(),
        );
        let needy = resolution
            .get(&PluginId::parse("acme.needy").unwrap())
            .unwrap();
        assert!(needy.status.explain().contains("turned off"));
    }

    #[test]
    fn a_dependency_at_the_wrong_version_reports_both_versions() {
        let resolution = resolve(
            vec![
                record("acme.needy", &[("other.thing", ">=2.0")], true),
                record_with("other.thing", &[], true, ">=0.1, <0.2", "1.5.0"),
            ],
            &host(),
        );
        let needy = resolution
            .get(&PluginId::parse("acme.needy").unwrap())
            .unwrap();
        let explanation = needy.status.explain();
        assert!(explanation.contains("1.5.0"), "{explanation}");
        assert!(explanation.contains(">=2.0"), "{explanation}");
    }

    #[test]
    fn unresolvability_propagates_up_a_chain_rather_than_one_level() {
        // c needs b, b needs a, a is missing. All three must be held back.
        let resolution = resolve(
            vec![
                record("c.three", &[("b.two", ">=1.0")], true),
                record("b.two", &[("a.one", ">=1.0")], true),
            ],
            &host(),
        );
        assert!(resolution.activation_order.is_empty(), "{resolution:?}");
        for plugin in &resolution.plugins {
            assert!(
                matches!(plugin.status, PluginStatus::UnresolvedDependency { .. }),
                "{:?} was {:?}",
                plugin.id(),
                plugin.status
            );
        }
    }

    #[test]
    fn a_dependency_cycle_is_detected_and_names_its_members() {
        let resolution = resolve(
            vec![
                record("a.one", &[("b.two", ">=1.0")], true),
                record("b.two", &[("a.one", ">=1.0")], true),
            ],
            &host(),
        );
        assert!(resolution.activation_order.is_empty());
        for plugin in &resolution.plugins {
            let PluginStatus::DependencyCycle { cycle } = &plugin.status else {
                panic!("{:?} was {:?}", plugin.id(), plugin.status);
            };
            assert!(cycle.len() >= 2, "{cycle:?}");
        }
    }

    #[test]
    fn a_plugin_that_failed_last_time_stays_failed_until_something_clears_it() {
        let mut broken = record("acme.broken", &[], true);
        broken.last_error = Some("threw on activate".into());
        let resolution = resolve(vec![broken], &host());
        assert!(resolution.activation_order.is_empty());
        assert!(resolution.plugins[0]
            .status
            .explain()
            .contains("threw on activate"));
    }

    #[test]
    fn contributions_are_namespaced_by_plugin() {
        let resolution = resolve(vec![record("acme.one", &[], true)], &host());
        assert_eq!(resolution.contributions.len(), 1);
        assert_eq!(resolution.contributions[0].id, "acme.one/run");
        assert_eq!(resolution.contributions[0].kind, ContributionKind::Command);
        assert_eq!(resolution.contributions[0].title, "Run");
    }

    #[test]
    fn two_plugins_with_the_same_local_id_do_not_collide_once_namespaced() {
        let resolution = resolve(
            vec![
                record("acme.one", &[], true),
                record("other.two", &[], true),
            ],
            &host(),
        );
        let ids: Vec<&str> = resolution
            .contributions
            .iter()
            .map(|c| c.id.as_str())
            .collect();
        assert_eq!(ids, vec!["acme.one/run", "other.two/run"]);
        assert!(resolution.warnings.is_empty());
    }

    #[test]
    fn a_tombstone_left_by_an_uninstall_is_not_a_plugin() {
        let mut tombstone = record("acme.gone", &[], false);
        tombstone.install_dir = std::path::PathBuf::new();
        let resolution = resolve(vec![tombstone], &host());
        assert!(resolution.plugins.is_empty());
    }
}
