//! Loader compatibility evaluation (Work Packages 3/4).
//!
//! Given the dependency declarations of a mod and the signed loader catalog,
//! this module decides whether the currently installed loader is compatible
//! and, when it is not, which signed catalog candidates would work and which
//! one should be recommended.
//!
//! Only *loader requirements* participate:
//!
//! - Declarations targeting a known framework capability (`fabricloader`,
//!   `quilt_loader`, `forge`, `neoforge`), and
//! - Built-in Forge/NeoForge language-loader declarations (`javafml` and
//!   `lowcodefml`). Third-party language providers such as KotlinForForge are
//!   ordinary installed mod dependencies, not distribution capabilities.
//!
//! Minecraft, Java, and general mod dependencies are never treated as loader
//! requirements. Unknown language capabilities are `Unsupported` and thereby
//! prevent automatic recommendation.
//!
//! Every candidate and recommendation is a tuple from the signed catalog —
//! no version outside the catalog is ever suggested.

use crate::dependency_ops::{DependencyDecl, DependencyImportance, DependencySource};
use crate::loader_manifests::{
    LoaderCapabilities, LoaderCatalog, LoaderEntry, LoaderReleaseChannel,
};
use crate::version_match::{compare_versions, evaluate_requirement, RequirementEvaluation};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;

/// Known framework capabilities that count as loader requirements.
pub const KNOWN_FRAMEWORK_TARGETS: [&str; 4] =
    ["fabricloader", "quilt_loader", "forge", "neoforge"];

/// Language providers supplied by Forge/NeoForge itself. Other `modLoader`
/// values are provided by installed mod JARs and must use ordinary dependency
/// checks instead of loader-catalog compatibility logic.
pub const BUILTIN_FML_LANGUAGE_PROVIDERS: [&str; 2] = ["javafml", "lowcodefml"];

// ---------------------------------------------------------------------------
// Public request / report types
// ---------------------------------------------------------------------------

/// Input to a loader compatibility evaluation.
///
/// The caller supplies the loader family, Minecraft version, the currently
/// installed loader version (when known), every dependency declaration of the
/// mod under evaluation, and the signed catalog to evaluate against.
#[derive(Debug, Clone, Serialize)]
pub struct LoaderCompatibilityRequest<'a> {
    /// Loader family, e.g. `fabric`, `quilt`, `forge`, `neoforge`.
    pub loader: &'a str,
    pub minecraft_version: &'a str,
    /// Currently installed loader version, when known.
    pub current_loader_version: Option<&'a str>,
    /// All dependency declarations of the mod (only loader requirements are
    /// evaluated; see the module docs).
    pub requirements: &'a [DependencyDecl],
    /// The signed loader catalog. Recommendations never reference versions
    /// outside this catalog.
    pub catalog: &'a LoaderCatalog,
}

/// Status of the current loader (and of the request as a whole).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CurrentLoaderStatus {
    /// The current loader satisfies every hard requirement.
    Compatible,
    /// The current loader is verifiable (present in the signed catalog) but
    /// fails at least one hard requirement.
    Incompatible,
    /// The current loader cannot be verified: the current tuple is absent
    /// from the signed catalog, or a hard requirement is `Unsupported`
    /// (unknown capability or uninterpretable range). No recommendation is
    /// made from a presumed invisible version.
    Indeterminate,
    /// No signed catalog candidate satisfies all hard requirements, so there
    /// is nothing to recommend (and no verifiable current loader to bless).
    NoCompatibleCandidates,
}

/// Why a declaration counts as a loader requirement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoaderRequirementKind {
    /// Targets a known framework capability (`fabricloader`, `quilt_loader`,
    /// `forge`, `neoforge`).
    Framework,
    /// A built-in Forge/NeoForge `modLoader`/`loaderVersion` declaration
    /// (`javafml` or `lowcodefml`).
    LanguageLoader,
}

/// Serializable mirror of [`RequirementEvaluation`] for report consumers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequirementVerdict {
    Satisfied,
    Unsatisfied,
    Unsupported { reason: String },
}

impl From<RequirementEvaluation> for RequirementVerdict {
    fn from(evaluation: RequirementEvaluation) -> Self {
        match evaluation {
            RequirementEvaluation::Satisfied => RequirementVerdict::Satisfied,
            RequirementEvaluation::Unsatisfied => RequirementVerdict::Unsatisfied,
            RequirementEvaluation::Unsupported { reason } => {
                RequirementVerdict::Unsupported { reason }
            }
        }
    }
}

/// Result of evaluating one loader requirement against one candidate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LoaderRequirementResult {
    /// The original requirement declaration.
    pub declaration: DependencyDecl,
    /// Every declaring mod represented by this semantically identical
    /// requirement. Grouping keeps one file-level language-loader predicate
    /// from producing a separate solver/UI row for every affected mod.
    #[serde(default)]
    pub declaring_mod_ids: Vec<String>,
    /// The normalized (lowercase) capability name that was evaluated.
    pub capability: String,
    pub kind: LoaderRequirementKind,
    /// The version the candidate provides for the capability, when one exists.
    pub candidate_provided_version: Option<String>,
    /// The authoritative strict evaluation. Not serialized; the JSON surface
    /// carries [`Self::verdict`] instead.
    #[serde(skip)]
    pub evaluation: RequirementEvaluation,
    /// Serializable form of [`Self::evaluation`].
    pub verdict: RequirementVerdict,
}

/// A signed catalog candidate that satisfies every hard requirement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CompatibleLoaderCandidate {
    pub loader_version: String,
    pub release_channel: LoaderReleaseChannel,
    /// Curator-set priority; a lower explicit rank is preferred over missing.
    pub recommendation_rank: Option<u32>,
    /// Capabilities this candidate provides.
    pub capabilities: LoaderCapabilities,
    /// Per-requirement evaluations for this candidate (hard and soft).
    pub requirement_results: Vec<LoaderRequirementResult>,
}

impl CompatibleLoaderCandidate {
    fn from_entry(
        entry: &LoaderEntry,
        family: &str,
        requirement_results: Vec<LoaderRequirementResult>,
    ) -> Self {
        CompatibleLoaderCandidate {
            loader_version: entry.loader_version.clone(),
            release_channel: entry.release_channel,
            recommendation_rank: entry.recommendation_rank,
            capabilities: entry.capabilities(family),
            requirement_results,
        }
    }
}

/// An actionable conflict between two hard requirements.
///
/// Carries structured evidence (declaring mod id, target, version ranges) for
/// both sides rather than opaque text. Only produced when both requirements
/// are individually satisfiable by signed candidates but no single candidate
/// satisfies both, and never when unknown syntax or a missing capability
/// prevents proving the conflict.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoaderConflict {
    pub declaring_mod_id: Option<String>,
    pub target_id: String,
    pub version_ranges: Vec<String>,
    pub with_declaring_mod_id: Option<String>,
    pub with_target_id: String,
    pub with_version_ranges: Vec<String>,
    /// Human-readable summary built from the structured evidence.
    pub message: String,
}

impl LoaderConflict {
    fn new(first: &DependencyDecl, second: &DependencyDecl) -> Self {
        let (first, second) = order_decls(first, second);
        let first_label = decl_label(first);
        let second_label = decl_label(second);
        let message = format!(
            "conflict between {first_label} and {second_label}: no signed loader candidate satisfies both"
        );
        LoaderConflict {
            declaring_mod_id: first.declaring_mod_id.clone(),
            target_id: first.target_id.clone(),
            version_ranges: first.version_ranges.clone(),
            with_declaring_mod_id: second.declaring_mod_id.clone(),
            with_target_id: second.target_id.clone(),
            with_version_ranges: second.version_ranges.clone(),
            message,
        }
    }
}

/// The full compatibility report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LoaderCompatibilityReport {
    pub current_status: CurrentLoaderStatus,
    /// Evaluations of every loader requirement against the current loader,
    /// when the current tuple is present in the signed catalog. Empty when
    /// the current tuple is unknown or absent from the catalog.
    pub requirements: Vec<LoaderRequirementResult>,
    /// Signed catalog candidates satisfying every hard requirement, ordered
    /// deterministically (stable before prerelease, explicit rank before
    /// missing rank, newest version last-resort).
    pub compatible_versions: Vec<CompatibleLoaderCandidate>,
    /// Signed catalog candidates for which every hard requirement is either
    /// satisfied or indeterminate, with at least one indeterminate result.
    /// These require explicit manual confirmation and are never automatic
    /// recommendations.
    pub indeterminate_versions: Vec<CompatibleLoaderCandidate>,
    /// The recommended signed candidate, when one can be proven.
    pub recommended_version: Option<CompatibleLoaderCandidate>,
    /// Minimal pair conflicts between hard requirements.
    pub conflicts: Vec<LoaderConflict>,
}

// ---------------------------------------------------------------------------
// Evaluation
// ---------------------------------------------------------------------------

/// Whether a declaration is a loader requirement, and what kind.
pub fn is_loader_requirement(decl: &DependencyDecl) -> Option<LoaderRequirementKind> {
    let target = decl.target_id.to_lowercase();
    if KNOWN_FRAMEWORK_TARGETS.contains(&target.as_str()) {
        Some(LoaderRequirementKind::Framework)
    } else if BUILTIN_FML_LANGUAGE_PROVIDERS.contains(&target.as_str())
        && matches!(
            decl.source,
            DependencySource::ForgeLanguageLoader | DependencySource::NeoForgeLanguageLoader
        )
    {
        Some(LoaderRequirementKind::LanguageLoader)
    } else {
        None
    }
}

/// Evaluate loader compatibility for a request. See the module docs for the
/// full semantics.
pub fn evaluate_loader_compatibility(
    request: &LoaderCompatibilityRequest,
) -> LoaderCompatibilityReport {
    let family = request.loader.to_lowercase();

    let specs = grouped_loader_requirement_specs(request.requirements);

    let candidates: Vec<&LoaderEntry> = request
        .catalog
        .loaders
        .get(&family)
        .map(|entries| {
            entries
                .iter()
                .filter(|entry| entry.mc_version == request.minecraft_version)
                .collect()
        })
        .unwrap_or_default();

    let evaluated: Vec<EvaluatedCandidate> = candidates
        .iter()
        .map(|entry| EvaluatedCandidate::new(entry, &family, &specs))
        .collect();

    // The current tuple, when it is a signed catalog tuple.
    let current_entry = request.current_loader_version.and_then(|version| {
        request
            .catalog
            .find_entry(&family, request.minecraft_version, version)
    });

    let current_results: Vec<LoaderRequirementResult> = current_entry
        .map(|entry| {
            specs
                .iter()
                .map(|spec| evaluate_spec_against(entry, &family, spec))
                .collect()
        })
        .unwrap_or_default();

    // Keep raw requirement evidence even when the installed tuple is absent
    // from the catalog. Prefer the best deterministic catalog candidate as
    // evidence; if no candidate exists, retain each declaration as an
    // explicitly unsupported manual-review result instead of dropping it.
    let report_requirements = if !current_results.is_empty() {
        current_results.clone()
    } else if let Some(candidate) = evaluated
        .iter()
        .min_by(|left, right| entry_preference(left.entry, right.entry))
    {
        candidate.results.clone()
    } else {
        specs
            .iter()
            .map(|spec| {
                let evaluation = RequirementEvaluation::Unsupported {
                    reason: "current loader tuple is absent from the signed catalog".into(),
                };
                LoaderRequirementResult {
                    declaration: spec.decl.clone(),
                    declaring_mod_ids: spec.declaring_mod_ids.clone(),
                    capability: spec.capability.clone(),
                    kind: spec.kind,
                    candidate_provided_version: None,
                    evaluation: evaluation.clone(),
                    verdict: evaluation.into(),
                }
            })
            .collect()
    };

    let hard_positions: Vec<usize> = specs
        .iter()
        .enumerate()
        .filter(|(_, spec)| is_hard(&spec.decl))
        .map(|(index, _)| index)
        .collect();

    let current_hard_unsupported = current_results.iter().enumerate().any(|(index, result)| {
        hard_positions.contains(&index)
            && matches!(result.evaluation, RequirementEvaluation::Unsupported { .. })
    });
    let current_hard_satisfied = current_results.iter().enumerate().all(|(index, result)| {
        !hard_positions.contains(&index)
            || matches!(result.evaluation, RequirementEvaluation::Satisfied)
    });

    let usable: Vec<&EvaluatedCandidate> = evaluated.iter().filter(|c| c.usable).collect();
    let mut indeterminate: Vec<&EvaluatedCandidate> = evaluated
        .iter()
        .filter(|candidate| {
            let mut has_unsupported = false;
            for (result, spec) in candidate.results.iter().zip(&specs) {
                if !is_hard(&spec.decl) {
                    continue;
                }
                match &result.evaluation {
                    RequirementEvaluation::Satisfied => {}
                    RequirementEvaluation::Unsupported { .. } => has_unsupported = true,
                    RequirementEvaluation::Unsatisfied => return false,
                }
            }
            has_unsupported
        })
        .collect();
    indeterminate.sort_by(|a, b| entry_preference(a.entry, b.entry));

    let current_status = match current_entry {
        Some(_) if current_hard_unsupported => CurrentLoaderStatus::Indeterminate,
        Some(_) if current_hard_satisfied => CurrentLoaderStatus::Compatible,
        Some(_) => CurrentLoaderStatus::Incompatible,
        None if !indeterminate.is_empty() => CurrentLoaderStatus::Indeterminate,
        None if usable.is_empty() => CurrentLoaderStatus::NoCompatibleCandidates,
        None => CurrentLoaderStatus::Indeterminate,
    };

    let mut compatible_versions: Vec<CompatibleLoaderCandidate> = usable
        .iter()
        .map(|candidate| {
            CompatibleLoaderCandidate::from_entry(
                candidate.entry,
                &family,
                candidate.results.clone(),
            )
        })
        .collect();
    compatible_versions.sort_by(candidate_preference);

    let recommended_version = if current_hard_unsupported
        || (!indeterminate.is_empty() && !current_entry.is_some_and(|_| current_hard_satisfied))
    {
        None
    } else if let Some(entry) = current_entry.filter(|_| current_hard_satisfied) {
        Some(CompatibleLoaderCandidate::from_entry(
            entry,
            &family,
            current_results.clone(),
        ))
    } else {
        let allow_prerelease = current_entry
            .map(|entry| entry.release_channel == LoaderReleaseChannel::Prerelease)
            .unwrap_or(false);
        let mut pool: Vec<&EvaluatedCandidate> = usable;
        pool.sort_by(|a, b| entry_preference(a.entry, b.entry));
        pool.into_iter()
            .find(|candidate| {
                candidate.entry.release_channel == LoaderReleaseChannel::Stable || allow_prerelease
            })
            .map(|candidate| {
                CompatibleLoaderCandidate::from_entry(
                    candidate.entry,
                    &family,
                    candidate.results.clone(),
                )
            })
    };

    let indeterminate_versions = indeterminate
        .into_iter()
        .map(|candidate| {
            CompatibleLoaderCandidate::from_entry(
                candidate.entry,
                &family,
                candidate.results.clone(),
            )
        })
        .collect();

    let conflicts = pair_conflicts(&evaluated, &specs, &hard_positions);

    LoaderCompatibilityReport {
        current_status,
        requirements: report_requirements,
        compatible_versions,
        indeterminate_versions,
        recommended_version,
        conflicts,
    }
}

// ---------------------------------------------------------------------------
// Internals
// ---------------------------------------------------------------------------

struct LoaderRequirementSpec {
    decl: DependencyDecl,
    declaring_mod_ids: Vec<String>,
    kind: LoaderRequirementKind,
    capability: String,
}

fn grouped_loader_requirement_specs(requirements: &[DependencyDecl]) -> Vec<LoaderRequirementSpec> {
    let mut specs: Vec<LoaderRequirementSpec> = Vec::new();
    for decl in requirements {
        let Some(kind) = is_loader_requirement(decl) else {
            continue;
        };
        let capability = decl.target_id.to_lowercase();
        if let Some(existing) = specs.iter_mut().find(|existing| {
            existing.kind == kind
                && existing.capability == capability
                && existing.decl.version_ranges == decl.version_ranges
                && existing.decl.importance == decl.importance
                && existing.decl.grammar == decl.grammar
        }) {
            if let Some(mod_id) = &decl.declaring_mod_id {
                if !existing.declaring_mod_ids.contains(mod_id) {
                    existing.declaring_mod_ids.push(mod_id.clone());
                    existing.declaring_mod_ids.sort();
                }
            }
            continue;
        }
        specs.push(LoaderRequirementSpec {
            decl: decl.clone(),
            declaring_mod_ids: decl.declaring_mod_id.iter().cloned().collect(),
            kind,
            capability,
        });
    }
    specs
}

struct EvaluatedCandidate<'a> {
    entry: &'a LoaderEntry,
    results: Vec<LoaderRequirementResult>,
    usable: bool,
}

impl<'a> EvaluatedCandidate<'a> {
    fn new(entry: &'a LoaderEntry, family: &str, specs: &[LoaderRequirementSpec]) -> Self {
        let results: Vec<LoaderRequirementResult> = specs
            .iter()
            .map(|spec| evaluate_spec_against(entry, family, spec))
            .collect();
        let usable = results.iter().zip(specs).all(|(result, spec)| {
            !is_hard(&spec.decl) || matches!(result.evaluation, RequirementEvaluation::Satisfied)
        });
        EvaluatedCandidate {
            entry,
            results,
            usable,
        }
    }
}

fn is_hard(decl: &DependencyDecl) -> bool {
    decl.importance == DependencyImportance::Required
}

fn evaluate_spec_against(
    entry: &LoaderEntry,
    family: &str,
    spec: &LoaderRequirementSpec,
) -> LoaderRequirementResult {
    let provided = entry.capability_version(family, &spec.capability);
    let evaluation = match provided {
        Some(version) => evaluate_requirement(&spec.decl, version),
        None => match spec.kind {
            // A known framework capability the candidate does not provide can
            // be proven unmet — the distribution simply does not carry it.
            LoaderRequirementKind::Framework => RequirementEvaluation::Unsatisfied,
            // An unknown language capability cannot be proven one way or the
            // other; fail closed and never recommend on a guess.
            LoaderRequirementKind::LanguageLoader => RequirementEvaluation::Unsupported {
                reason: format!(
                    "candidate does not provide language loader capability '{}'",
                    spec.capability
                ),
            },
        },
    };
    LoaderRequirementResult {
        declaration: spec.decl.clone(),
        declaring_mod_ids: spec.declaring_mod_ids.clone(),
        capability: spec.capability.clone(),
        kind: spec.kind,
        candidate_provided_version: provided.map(str::to_string),
        evaluation: evaluation.clone(),
        verdict: evaluation.into(),
    }
}

/// Deterministic preference order for recommendation and listing:
/// stable before prerelease, explicit lower rank before missing rank, then
/// newest version as the deterministic fallback.
fn entry_preference(a: &LoaderEntry, b: &LoaderEntry) -> Ordering {
    channel_priority(a.release_channel)
        .cmp(&channel_priority(b.release_channel))
        .then_with(|| {
            a.recommendation_rank
                .is_none()
                .cmp(&b.recommendation_rank.is_none())
        })
        .then_with(|| a.recommendation_rank.cmp(&b.recommendation_rank))
        .then_with(|| compare_versions(&b.loader_version, &a.loader_version))
}

fn candidate_preference(a: &CompatibleLoaderCandidate, b: &CompatibleLoaderCandidate) -> Ordering {
    channel_priority(a.release_channel)
        .cmp(&channel_priority(b.release_channel))
        .then_with(|| {
            a.recommendation_rank
                .is_none()
                .cmp(&b.recommendation_rank.is_none())
        })
        .then_with(|| a.recommendation_rank.cmp(&b.recommendation_rank))
        .then_with(|| compare_versions(&b.loader_version, &a.loader_version))
}

fn channel_priority(channel: LoaderReleaseChannel) -> u8 {
    match channel {
        LoaderReleaseChannel::Stable => 0,
        LoaderReleaseChannel::Prerelease => 1,
    }
}

/// Minimal pair conflicts: two hard requirements are in conflict when each is
/// individually satisfiable by signed candidates but no single candidate
/// satisfies both. Requirements whose evaluation is `Unsupported` for any
/// candidate (unknown syntax or capability) never produce a conflict claim.
fn pair_conflicts<'a>(
    evaluated: &[EvaluatedCandidate<'a>],
    specs: &[LoaderRequirementSpec],
    hard_positions: &[usize],
) -> Vec<LoaderConflict> {
    let mut satisfying: Vec<Vec<usize>> = Vec::with_capacity(hard_positions.len());
    let mut unprovable: Vec<bool> = Vec::with_capacity(hard_positions.len());
    for &position in hard_positions {
        let mut candidates = Vec::new();
        let mut unknown = false;
        for (index, candidate) in evaluated.iter().enumerate() {
            match &candidate.results[position].evaluation {
                RequirementEvaluation::Satisfied => candidates.push(index),
                RequirementEvaluation::Unsupported { .. } => unknown = true,
                RequirementEvaluation::Unsatisfied => {}
            }
        }
        satisfying.push(candidates);
        unprovable.push(unknown);
    }

    let mut conflicts = Vec::new();
    for first in 0..hard_positions.len() {
        for second in (first + 1)..hard_positions.len() {
            if unprovable[first] || unprovable[second] {
                continue;
            }
            if satisfying[first].is_empty() || satisfying[second].is_empty() {
                continue;
            }
            let disjoint = satisfying[first]
                .iter()
                .all(|index| !satisfying[second].contains(index));
            if disjoint {
                conflicts.push(LoaderConflict::new(
                    &specs[hard_positions[first]].decl,
                    &specs[hard_positions[second]].decl,
                ));
            }
        }
    }
    conflicts
}

/// Deterministic decl ordering for conflict evidence: declaring mod id, then
/// target id, then joined ranges.
fn decl_sort_key(decl: &DependencyDecl) -> (&str, &str, String) {
    (
        decl.declaring_mod_id.as_deref().unwrap_or(""),
        decl.target_id.as_str(),
        decl.version_ranges.join(","),
    )
}

fn order_decls<'a>(
    a: &'a DependencyDecl,
    b: &'a DependencyDecl,
) -> (&'a DependencyDecl, &'a DependencyDecl) {
    if decl_sort_key(a) <= decl_sort_key(b) {
        (a, b)
    } else {
        (b, a)
    }
}

fn decl_label(decl: &DependencyDecl) -> String {
    match &decl.declaring_mod_id {
        Some(mod_id) => {
            let ranges = if decl.version_ranges.is_empty() {
                "any version".to_string()
            } else {
                decl.version_ranges.join(" or ")
            };
            format!("'{}' requiring {} {ranges}", mod_id, decl.target_id)
        }
        None => format!(
            "an unowned requirement for {} ({})",
            decl.target_id,
            decl.version_ranges.join(" or ")
        ),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dependency_ops::{DependencyImportance, DependencySource, VersionGrammar};
    use std::collections::BTreeMap;

    fn entry(
        loader_version: &str,
        channel: LoaderReleaseChannel,
        rank: Option<u32>,
        provided: &[(&str, &str)],
    ) -> LoaderEntry {
        LoaderEntry {
            mc_version: "1.21".into(),
            loader_version: loader_version.into(),
            source_url: format!("https://example.com/pin/{loader_version}"),
            sha256: loader_version.repeat(8),
            file_name: format!("loader-{loader_version}.jar"),
            file_type: "installer_jar".into(),
            version_json_sha256: None,
            installer_spec: None,
            provided_versions: provided
                .iter()
                .map(|(capability, version)| (capability.to_string(), version.to_string()))
                .collect(),
            release_channel: channel,
            recommendation_rank: rank,
        }
    }

    fn catalog(family: &str, entries: Vec<LoaderEntry>) -> LoaderCatalog {
        LoaderCatalog {
            domain_allowlist: vec!["example.com".into()],
            loaders: BTreeMap::from([(family.to_string(), entries)]),
        }
    }

    fn fabric_decl(
        declaring: &str,
        target: &str,
        ranges: &[&str],
        importance: DependencyImportance,
        source: DependencySource,
    ) -> DependencyDecl {
        DependencyDecl {
            declaring_mod_id: Some(declaring.into()),
            target_id: target.into(),
            version_ranges: ranges.iter().map(|r| r.to_string()).collect(),
            importance,
            grammar: VersionGrammar::Fabric,
            source,
        }
    }

    fn request<'a>(
        loader: &'a str,
        current: Option<&'a str>,
        requirements: &'a [DependencyDecl],
        catalog: &'a LoaderCatalog,
    ) -> LoaderCompatibilityRequest<'a> {
        LoaderCompatibilityRequest {
            loader,
            minecraft_version: "1.21",
            current_loader_version: current,
            requirements,
            catalog,
        }
    }

    fn result_version(report: &LoaderCompatibilityReport) -> Option<&str> {
        report
            .recommended_version
            .as_ref()
            .map(|candidate| candidate.loader_version.as_str())
    }

    // -----------------------------------------------------------------------
    // Fabric / Quilt default capabilities
    // -----------------------------------------------------------------------

    #[test]
    fn fabric_default_capability_satisfies_loader_requirement() {
        let catalog = catalog(
            "fabric",
            vec![
                entry("0.18.6", LoaderReleaseChannel::Stable, None, &[]),
                entry("0.19.0", LoaderReleaseChannel::Stable, None, &[]),
            ],
        );
        let decl = fabric_decl(
            "moda",
            "fabricloader",
            &[">=0.19.0"],
            DependencyImportance::Required,
            DependencySource::FabricDepends,
        );
        let report =
            evaluate_loader_compatibility(&request("fabric", Some("0.19.0"), &[decl], &catalog));
        assert_eq!(report.current_status, CurrentLoaderStatus::Compatible);
        assert_eq!(result_version(&report), Some("0.19.0"));
        assert_eq!(report.compatible_versions.len(), 1);
        assert_eq!(
            report.compatible_versions[0].capabilities.distribution_id,
            "fabric"
        );
        assert_eq!(
            report.compatible_versions[0]
                .capabilities
                .provided_versions
                .get("fabricloader")
                .map(String::as_str),
            Some("0.19.0")
        );
    }

    #[test]
    fn quilt_default_capability_satisfies_loader_requirement() {
        let catalog = catalog(
            "quilt",
            vec![entry("0.27.1", LoaderReleaseChannel::Stable, None, &[])],
        );
        let decl = fabric_decl(
            "moda",
            "quilt_loader",
            &[">=0.20.0"],
            DependencyImportance::Required,
            DependencySource::QuiltDepends,
        );
        let report =
            evaluate_loader_compatibility(&request("quilt", Some("0.27.1"), &[decl], &catalog));
        assert_eq!(report.current_status, CurrentLoaderStatus::Compatible);
        assert_eq!(result_version(&report), Some("0.27.1"));
    }

    // -----------------------------------------------------------------------
    // Explicit capabilities win / augment
    // -----------------------------------------------------------------------

    #[test]
    fn explicit_capabilities_allow_cross_family_provision() {
        // Quilt provides Fabric compatibility; an explicitly declared
        // fabricloader capability lets a fabricloader requirement be met by
        // a Quilt candidate.
        let catalog = catalog(
            "quilt",
            vec![entry(
                "0.27.1",
                LoaderReleaseChannel::Stable,
                None,
                &[("fabricloader", "0.16.0")],
            )],
        );
        let decl = fabric_decl(
            "moda",
            "fabricloader",
            &[">=0.15.0"],
            DependencyImportance::Required,
            DependencySource::FabricDepends,
        );
        let report = evaluate_loader_compatibility(&request("quilt", None, &[decl], &catalog));
        assert_eq!(
            report.current_status,
            CurrentLoaderStatus::Indeterminate,
            "no current loader, but usable candidates exist"
        );
        assert_eq!(result_version(&report), Some("0.27.1"));
        assert_eq!(report.compatible_versions.len(), 1);
        assert_eq!(
            report.compatible_versions[0].requirement_results[0]
                .candidate_provided_version
                .as_deref(),
            Some("0.16.0")
        );
    }

    #[test]
    fn explicit_language_capability_allows_forge_recommendation() {
        let catalog = catalog(
            "forge",
            vec![entry(
                "51.0.0",
                LoaderReleaseChannel::Stable,
                None,
                &[("javafml", "3.0.0")],
            )],
        );
        let decl = DependencyDecl {
            declaring_mod_id: Some("moda".into()),
            target_id: "javafml".into(),
            version_ranges: vec!["[3,)".into()],
            importance: DependencyImportance::Required,
            grammar: VersionGrammar::Maven,
            source: DependencySource::ForgeLanguageLoader,
        };
        let report = evaluate_loader_compatibility(&request("forge", None, &[decl], &catalog));
        assert_eq!(result_version(&report), Some("51.0.0"));
        assert_eq!(
            report.compatible_versions[0].requirement_results[0].verdict,
            RequirementVerdict::Satisfied
        );
    }

    // -----------------------------------------------------------------------
    // Forge / NeoForge language-provider capabilities
    // -----------------------------------------------------------------------

    #[test]
    fn forge_language_loader_uses_documented_major_provider_version() {
        let catalog = catalog(
            "forge",
            vec![entry("51.0.0", LoaderReleaseChannel::Stable, None, &[])],
        );
        let decl = DependencyDecl {
            declaring_mod_id: Some("moda".into()),
            target_id: "javafml".into(),
            version_ranges: vec!["[51,)".into()],
            importance: DependencyImportance::Required,
            grammar: VersionGrammar::Maven,
            source: DependencySource::ForgeLanguageLoader,
        };
        let report =
            evaluate_loader_compatibility(&request("forge", Some("51.0.0"), &[decl], &catalog));
        assert_eq!(report.current_status, CurrentLoaderStatus::Compatible);
        assert_eq!(result_version(&report), Some("51.0.0"));
        assert_eq!(
            report.requirements[0].candidate_provided_version.as_deref(),
            Some("51")
        );
        assert_eq!(
            report.requirements[0].verdict,
            RequirementVerdict::Satisfied
        );
    }

    #[test]
    fn legacy_neoforge_1211_profile_provides_both_builtin_language_loaders() {
        let catalog = catalog(
            "neoforge",
            vec![entry("21.1.172", LoaderReleaseChannel::Stable, None, &[])],
        );
        let requirements = vec![
            DependencyDecl {
                declaring_mod_id: Some("structory".into()),
                target_id: "javafml".into(),
                version_ranges: vec!["[1,)".into()],
                importance: DependencyImportance::Required,
                grammar: VersionGrammar::Maven,
                source: DependencySource::NeoForgeLanguageLoader,
            },
            DependencyDecl {
                declaring_mod_id: Some("structory_towers".into()),
                target_id: "lowcodefml".into(),
                version_ranges: vec!["[1,)".into()],
                importance: DependencyImportance::Required,
                grammar: VersionGrammar::Maven,
                source: DependencySource::NeoForgeLanguageLoader,
            },
        ];
        let report = evaluate_loader_compatibility(&request(
            "neoforge",
            Some("21.1.172"),
            &requirements,
            &catalog,
        ));
        assert_eq!(report.current_status, CurrentLoaderStatus::Compatible);
        assert_eq!(report.requirements.len(), 2);
        assert!(report
            .requirements
            .iter()
            .all(|result| result.verdict == RequirementVerdict::Satisfied));
    }

    #[test]
    fn identical_language_requirements_group_declaring_mods() {
        let catalog = catalog(
            "forge",
            vec![entry("47.4.10", LoaderReleaseChannel::Stable, None, &[])],
        );
        let requirements = [
            DependencyDecl {
                declaring_mod_id: Some("mod_a".into()),
                target_id: "javafml".into(),
                version_ranges: vec!["[47,)".into()],
                importance: DependencyImportance::Required,
                grammar: VersionGrammar::Maven,
                source: DependencySource::ForgeLanguageLoader,
            },
            DependencyDecl {
                declaring_mod_id: Some("mod_b".into()),
                target_id: "javafml".into(),
                version_ranges: vec!["[47,)".into()],
                importance: DependencyImportance::Required,
                grammar: VersionGrammar::Maven,
                source: DependencySource::ForgeLanguageLoader,
            },
        ];
        let report = evaluate_loader_compatibility(&request(
            "forge",
            Some("47.4.10"),
            &requirements,
            &catalog,
        ));
        assert_eq!(report.requirements.len(), 1);
        assert_eq!(
            report.requirements[0].declaring_mod_ids,
            vec!["mod_a".to_string(), "mod_b".to_string()]
        );
    }

    #[test]
    fn third_party_language_provider_is_not_a_distribution_requirement() {
        let catalog = catalog(
            "forge",
            vec![entry("51.0.0", LoaderReleaseChannel::Stable, None, &[])],
        );
        let decl = DependencyDecl {
            declaring_mod_id: Some("moda".into()),
            target_id: "kotlinforforge".into(),
            version_ranges: vec!["[5,)".into()],
            importance: DependencyImportance::Required,
            grammar: VersionGrammar::Maven,
            source: DependencySource::ForgeLanguageLoader,
        };
        let report =
            evaluate_loader_compatibility(&request("forge", Some("51.0.0"), &[decl], &catalog));
        assert_eq!(report.current_status, CurrentLoaderStatus::Compatible);
        assert!(report.requirements.is_empty());
    }

    // -----------------------------------------------------------------------
    // Upgrade / downgrade / window
    // -----------------------------------------------------------------------

    #[test]
    fn upgrade_recommended_when_current_too_old() {
        let catalog = catalog(
            "fabric",
            vec![
                entry("0.18.4", LoaderReleaseChannel::Stable, None, &[]),
                entry("0.18.6", LoaderReleaseChannel::Stable, None, &[]),
                entry("0.19.0", LoaderReleaseChannel::Stable, None, &[]),
            ],
        );
        let decl = fabric_decl(
            "moda",
            "fabricloader",
            &[">=0.19.0"],
            DependencyImportance::Required,
            DependencySource::FabricDepends,
        );
        let report =
            evaluate_loader_compatibility(&request("fabric", Some("0.18.6"), &[decl], &catalog));
        assert_eq!(report.current_status, CurrentLoaderStatus::Incompatible);
        assert_eq!(result_version(&report), Some("0.19.0"));
        assert_eq!(report.compatible_versions.len(), 1);
        assert_eq!(report.compatible_versions[0].loader_version, "0.19.0");
    }

    #[test]
    fn downgrade_recommended_when_current_too_new() {
        let catalog = catalog(
            "fabric",
            vec![
                entry("0.18.6", LoaderReleaseChannel::Stable, None, &[]),
                entry("0.19.0", LoaderReleaseChannel::Stable, None, &[]),
                entry("0.20.0", LoaderReleaseChannel::Stable, None, &[]),
            ],
        );
        let decl = fabric_decl(
            "moda",
            "fabricloader",
            &["<0.19.0"],
            DependencyImportance::Required,
            DependencySource::FabricDepends,
        );
        let report =
            evaluate_loader_compatibility(&request("fabric", Some("0.20.0"), &[decl], &catalog));
        assert_eq!(report.current_status, CurrentLoaderStatus::Incompatible);
        // Newest compatible is 0.18.6; the newest incompatible 0.19.0 is not
        // "recommended merely because newer".
        assert_eq!(result_version(&report), Some("0.18.6"));
    }

    #[test]
    fn window_selection_uses_both_bounds() {
        let catalog = catalog(
            "fabric",
            vec![
                entry("0.18.6", LoaderReleaseChannel::Stable, None, &[]),
                entry("0.19.0", LoaderReleaseChannel::Stable, None, &[]),
                entry("0.20.0", LoaderReleaseChannel::Stable, None, &[]),
                entry("0.21.0", LoaderReleaseChannel::Stable, None, &[]),
            ],
        );
        let decl = fabric_decl(
            "moda",
            "fabricloader",
            &[">=0.19.0 <0.21.0"],
            DependencyImportance::Required,
            DependencySource::FabricDepends,
        );
        let report = evaluate_loader_compatibility(&request("fabric", None, &[decl], &catalog));
        assert_eq!(
            report
                .compatible_versions
                .iter()
                .map(|c| c.loader_version.as_str())
                .collect::<Vec<_>>(),
            vec!["0.20.0", "0.19.0"]
        );
        assert_eq!(result_version(&report), Some("0.20.0"));
    }

    // -----------------------------------------------------------------------
    // Current handling
    // -----------------------------------------------------------------------

    #[test]
    fn current_absent_from_catalog_is_indeterminate_and_not_recommended() {
        let catalog = catalog(
            "fabric",
            vec![entry("0.19.0", LoaderReleaseChannel::Stable, None, &[])],
        );
        let decl = fabric_decl(
            "moda",
            "fabricloader",
            &[">=0.19.0"],
            DependencyImportance::Required,
            DependencySource::FabricDepends,
        );
        let report =
            evaluate_loader_compatibility(&request("fabric", Some("0.99.0"), &[decl], &catalog));
        assert_eq!(report.current_status, CurrentLoaderStatus::Indeterminate);
        // The recommendation must come from the signed catalog, never from
        // the presumed invisible current version.
        assert_eq!(result_version(&report), Some("0.19.0"));
        assert_eq!(report.requirements.len(), 1);
        assert_eq!(
            report.requirements[0]
                .declaration
                .declaring_mod_id
                .as_deref(),
            Some("moda")
        );
        assert_eq!(
            report.requirements[0].candidate_provided_version.as_deref(),
            Some("0.19.0")
        );
    }

    #[test]
    fn no_current_and_no_candidates_is_no_compatible_candidates() {
        let catalog = catalog(
            "fabric",
            vec![entry("0.18.6", LoaderReleaseChannel::Stable, None, &[])],
        );
        let decl = fabric_decl(
            "moda",
            "fabricloader",
            &[">=0.19.0"],
            DependencyImportance::Required,
            DependencySource::FabricDepends,
        );
        let report = evaluate_loader_compatibility(&request("fabric", None, &[decl], &catalog));
        assert_eq!(
            report.current_status,
            CurrentLoaderStatus::NoCompatibleCandidates
        );
        assert!(report.recommended_version.is_none());
        assert!(report.compatible_versions.is_empty());
    }

    #[test]
    fn no_candidates_for_mc_version_is_no_compatible_candidates() {
        let catalog = catalog(
            "fabric",
            vec![entry("0.19.0", LoaderReleaseChannel::Stable, None, &[])],
        );
        let decl = fabric_decl(
            "moda",
            "fabricloader",
            &[">=0.19.0"],
            DependencyImportance::Required,
            DependencySource::FabricDepends,
        );
        let decls = [decl];
        let mut req = request("fabric", None, &decls, &catalog);
        req.minecraft_version = "1.20.4";
        let report = evaluate_loader_compatibility(&req);
        assert_eq!(
            report.current_status,
            CurrentLoaderStatus::NoCompatibleCandidates
        );
        assert!(report.recommended_version.is_none());
    }

    #[test]
    fn current_incompatible_while_candidates_exist() {
        let catalog = catalog(
            "fabric",
            vec![
                entry("0.18.6", LoaderReleaseChannel::Stable, None, &[]),
                entry("0.19.0", LoaderReleaseChannel::Stable, None, &[]),
            ],
        );
        let decl = fabric_decl(
            "moda",
            "fabricloader",
            &[">=0.19.0"],
            DependencyImportance::Required,
            DependencySource::FabricDepends,
        );
        let report =
            evaluate_loader_compatibility(&request("fabric", Some("0.18.6"), &[decl], &catalog));
        assert_eq!(report.current_status, CurrentLoaderStatus::Incompatible);
        assert_eq!(result_version(&report), Some("0.19.0"));
        assert_eq!(report.requirements.len(), 1);
        assert_eq!(
            report.requirements[0].verdict,
            RequirementVerdict::Unsatisfied
        );
        assert_eq!(
            report.requirements[0].candidate_provided_version.as_deref(),
            Some("0.18.6")
        );
    }

    // -----------------------------------------------------------------------
    // Prerelease handling
    // -----------------------------------------------------------------------

    #[test]
    fn prerelease_excluded_when_current_is_stable() {
        // The only satisfying candidate is a prerelease (a window strictly
        // below the 0.20.0 release), so a stable current loader must not get
        // a prerelease recommendation.
        let catalog = catalog(
            "fabric",
            vec![
                entry("0.19.0", LoaderReleaseChannel::Stable, None, &[]),
                entry("0.20.0", LoaderReleaseChannel::Stable, None, &[]),
                entry("0.20.0-beta.1", LoaderReleaseChannel::Prerelease, None, &[]),
            ],
        );
        let decl = fabric_decl(
            "moda",
            "fabricloader",
            &[">=0.20.0-beta.1 <0.20.0"],
            DependencyImportance::Required,
            DependencySource::FabricDepends,
        );
        let report =
            evaluate_loader_compatibility(&request("fabric", Some("0.19.0"), &[decl], &catalog));
        assert_eq!(report.current_status, CurrentLoaderStatus::Incompatible);
        // The prerelease would work but must not be recommended without a
        // prerelease current.
        assert!(report.recommended_version.is_none());
        assert_eq!(
            report
                .compatible_versions
                .iter()
                .map(|c| c.loader_version.as_str())
                .collect::<Vec<_>>(),
            vec!["0.20.0-beta.1"]
        );
    }

    #[test]
    fn prerelease_allowed_when_current_is_prerelease() {
        let catalog = catalog(
            "fabric",
            vec![
                entry("0.20.0-beta.1", LoaderReleaseChannel::Prerelease, None, &[]),
                entry("0.21.0-beta.1", LoaderReleaseChannel::Prerelease, None, &[]),
            ],
        );
        let decl = fabric_decl(
            "moda",
            "fabricloader",
            &[">=0.21.0-beta.1"],
            DependencyImportance::Required,
            DependencySource::FabricDepends,
        );
        // Current is a prerelease that fails the requirement (0.20.0-beta.1
        // < 0.21.0), so a signed prerelease becomes eligible.
        let report = evaluate_loader_compatibility(&request(
            "fabric",
            Some("0.20.0-beta.1"),
            &[decl],
            &catalog,
        ));
        assert_eq!(
            report.current_status,
            CurrentLoaderStatus::Incompatible,
            "current is prerelease but fails the hard requirement"
        );
        assert_eq!(result_version(&report), Some("0.21.0-beta.1"));
    }

    #[test]
    fn stable_preferred_over_prerelease_when_current_is_prerelease() {
        let catalog = catalog(
            "fabric",
            vec![
                entry("0.21.0", LoaderReleaseChannel::Stable, None, &[]),
                entry("0.22.0-beta.1", LoaderReleaseChannel::Prerelease, None, &[]),
            ],
        );
        let decl = fabric_decl(
            "moda",
            "fabricloader",
            &[">=0.21.0"],
            DependencyImportance::Required,
            DependencySource::FabricDepends,
        );
        let report = evaluate_loader_compatibility(&request(
            "fabric",
            Some("0.20.0-beta.1"),
            &[decl],
            &catalog,
        ));
        assert_eq!(result_version(&report), Some("0.21.0"));
    }

    // -----------------------------------------------------------------------
    // Recommendation ranking
    // -----------------------------------------------------------------------

    #[test]
    fn explicit_lower_rank_beats_missing_rank_and_newer_version() {
        let catalog = catalog(
            "fabric",
            vec![
                entry("0.19.0", LoaderReleaseChannel::Stable, None, &[]),
                entry("0.18.6", LoaderReleaseChannel::Stable, Some(1), &[]),
            ],
        );
        let decl = fabric_decl(
            "moda",
            "fabricloader",
            &[">=0.18.0"],
            DependencyImportance::Required,
            DependencySource::FabricDepends,
        );
        let report = evaluate_loader_compatibility(&request("fabric", None, &[decl], &catalog));
        assert_eq!(result_version(&report), Some("0.18.6"));
    }

    #[test]
    fn newest_stable_is_deterministic_fallback() {
        let catalog = catalog(
            "fabric",
            vec![
                entry("0.19.0", LoaderReleaseChannel::Stable, None, &[]),
                entry("0.18.10", LoaderReleaseChannel::Stable, None, &[]),
                entry("0.18.6", LoaderReleaseChannel::Stable, None, &[]),
            ],
        );
        let decl = fabric_decl(
            "moda",
            "fabricloader",
            &[">=0.18.0"],
            DependencyImportance::Required,
            DependencySource::FabricDepends,
        );
        let report = evaluate_loader_compatibility(&request("fabric", None, &[decl], &catalog));
        assert_eq!(result_version(&report), Some("0.19.0"));
    }

    // -----------------------------------------------------------------------
    // Unsupported / optional handling
    // -----------------------------------------------------------------------

    #[test]
    fn unsupported_hard_requirement_prevents_recommendation() {
        let catalog = catalog(
            "fabric",
            vec![entry("0.19.0", LoaderReleaseChannel::Stable, None, &[])],
        );
        // Fabric grammar with an uninterpretable predicate (wildcards are
        // only valid with the equality operator).
        let decl = fabric_decl(
            "moda",
            "fabricloader",
            &[">=1.x"],
            DependencyImportance::Required,
            DependencySource::FabricDepends,
        );
        let report =
            evaluate_loader_compatibility(&request("fabric", Some("0.19.0"), &[decl], &catalog));
        assert_eq!(report.current_status, CurrentLoaderStatus::Indeterminate);
        assert!(report.recommended_version.is_none());
        assert!(report.compatible_versions.is_empty());
    }

    #[test]
    fn recommended_results_are_reported_but_do_not_exclude() {
        let catalog = catalog(
            "fabric",
            vec![entry("0.18.6", LoaderReleaseChannel::Stable, None, &[])],
        );
        let hard = fabric_decl(
            "moda",
            "fabricloader",
            &[">=0.18.0"],
            DependencyImportance::Required,
            DependencySource::FabricDepends,
        );
        let soft = fabric_decl(
            "modb",
            "fabricloader",
            &[">=0.19.0"],
            DependencyImportance::Recommended,
            DependencySource::FabricRecommends,
        );
        let report = evaluate_loader_compatibility(&request(
            "fabric",
            Some("0.18.6"),
            &[hard, soft],
            &catalog,
        ));
        assert_eq!(report.current_status, CurrentLoaderStatus::Compatible);
        assert_eq!(result_version(&report), Some("0.18.6"));
        let soft_result = report
            .requirements
            .iter()
            .find(|r| r.declaration.declaring_mod_id.as_deref() == Some("modb"))
            .unwrap();
        assert_eq!(soft_result.verdict, RequirementVerdict::Unsatisfied);
    }

    // -----------------------------------------------------------------------
    // Non-loader dependencies
    // -----------------------------------------------------------------------

    #[test]
    fn minecraft_java_and_mod_dependencies_are_not_loader_requirements() {
        let catalog = catalog(
            "fabric",
            vec![entry("0.18.6", LoaderReleaseChannel::Stable, None, &[])],
        );
        let decls = [
            fabric_decl(
                "moda",
                "minecraft",
                &[">=1.21"],
                DependencyImportance::Required,
                DependencySource::FabricDepends,
            ),
            fabric_decl(
                "moda",
                "java",
                &[">=21"],
                DependencyImportance::Required,
                DependencySource::FabricDepends,
            ),
            fabric_decl(
                "moda",
                "fabric_api",
                &[">=0.100.0"],
                DependencyImportance::Required,
                DependencySource::FabricDepends,
            ),
        ];
        let report =
            evaluate_loader_compatibility(&request("fabric", Some("0.18.6"), &decls, &catalog));
        assert_eq!(report.current_status, CurrentLoaderStatus::Compatible);
        assert_eq!(result_version(&report), Some("0.18.6"));
        assert!(report.requirements.is_empty());
    }

    #[test]
    fn known_framework_target_from_another_family_excludes_candidate() {
        let catalog = catalog(
            "fabric",
            vec![entry("0.18.6", LoaderReleaseChannel::Stable, None, &[])],
        );
        let decl = fabric_decl(
            "moda",
            "quilt_loader",
            &[">=0.20.0"],
            DependencyImportance::Required,
            DependencySource::FabricDepends,
        );
        let report =
            evaluate_loader_compatibility(&request("fabric", Some("0.18.6"), &[decl], &catalog));
        assert_eq!(report.current_status, CurrentLoaderStatus::Incompatible);
        assert!(report.recommended_version.is_none());
    }

    // -----------------------------------------------------------------------
    // Conflicts
    // -----------------------------------------------------------------------

    #[test]
    fn minimal_pair_conflict_with_structured_evidence() {
        let catalog = catalog(
            "fabric",
            vec![
                entry("0.18.6", LoaderReleaseChannel::Stable, None, &[]),
                entry("0.19.0", LoaderReleaseChannel::Stable, None, &[]),
            ],
        );
        let lower = fabric_decl(
            "moda",
            "fabricloader",
            &[">=0.19.0"],
            DependencyImportance::Required,
            DependencySource::FabricDepends,
        );
        let upper = fabric_decl(
            "modb",
            "fabricloader",
            &["<0.19.0"],
            DependencyImportance::Required,
            DependencySource::FabricDepends,
        );
        let report = evaluate_loader_compatibility(&request(
            "fabric",
            Some("0.18.6"),
            &[lower, upper],
            &catalog,
        ));
        assert_eq!(report.current_status, CurrentLoaderStatus::Incompatible);
        assert!(report.recommended_version.is_none());
        assert_eq!(report.conflicts.len(), 1);
        let conflict = &report.conflicts[0];
        assert_eq!(conflict.declaring_mod_id.as_deref(), Some("moda"));
        assert_eq!(conflict.target_id, "fabricloader");
        assert_eq!(conflict.version_ranges, vec![">=0.19.0".to_string()]);
        assert_eq!(conflict.with_declaring_mod_id.as_deref(), Some("modb"));
        assert_eq!(conflict.with_target_id, "fabricloader");
        assert!(conflict.message.contains("moda"));
        assert!(conflict.message.contains("modb"));
    }

    #[test]
    fn no_conflict_claimed_when_unknown_syntax_prevents_proof() {
        let catalog = catalog(
            "fabric",
            vec![
                entry("0.18.6", LoaderReleaseChannel::Stable, None, &[]),
                entry("0.19.0", LoaderReleaseChannel::Stable, None, &[]),
            ],
        );
        let unknown = fabric_decl(
            "moda",
            "fabricloader",
            &[">=1.x"],
            DependencyImportance::Required,
            DependencySource::FabricDepends,
        );
        let upper = fabric_decl(
            "modb",
            "fabricloader",
            &["<0.19.0"],
            DependencyImportance::Required,
            DependencySource::FabricDepends,
        );
        let report =
            evaluate_loader_compatibility(&request("fabric", None, &[unknown, upper], &catalog));
        assert!(report.conflicts.is_empty());
    }

    #[test]
    fn no_conflict_when_one_requirement_is_unsatisfiable() {
        let catalog = catalog(
            "fabric",
            vec![
                entry("0.18.6", LoaderReleaseChannel::Stable, None, &[]),
                entry("0.19.0", LoaderReleaseChannel::Stable, None, &[]),
            ],
        );
        let impossible = fabric_decl(
            "moda",
            "fabricloader",
            &[">=9.0.0"],
            DependencyImportance::Required,
            DependencySource::FabricDepends,
        );
        let upper = fabric_decl(
            "modb",
            "fabricloader",
            &["<0.19.0"],
            DependencyImportance::Required,
            DependencySource::FabricDepends,
        );
        let report = evaluate_loader_compatibility(&request(
            "fabric",
            Some("0.18.6"),
            &[impossible, upper],
            &catalog,
        ));
        assert!(report.conflicts.is_empty());
        assert!(report.compatible_versions.is_empty());
    }

    #[test]
    fn overlapping_requirements_do_not_conflict() {
        let catalog = catalog(
            "fabric",
            vec![
                entry("0.18.6", LoaderReleaseChannel::Stable, None, &[]),
                entry("0.19.0", LoaderReleaseChannel::Stable, None, &[]),
            ],
        );
        let lower = fabric_decl(
            "moda",
            "fabricloader",
            &[">=0.18.0"],
            DependencyImportance::Required,
            DependencySource::FabricDepends,
        );
        let upper = fabric_decl(
            "modb",
            "fabricloader",
            &["<0.19.1"],
            DependencyImportance::Required,
            DependencySource::FabricDepends,
        );
        let report =
            evaluate_loader_compatibility(&request("fabric", None, &[lower, upper], &catalog));
        assert!(report.conflicts.is_empty());
        assert_eq!(report.compatible_versions.len(), 2);
        assert_eq!(result_version(&report), Some("0.19.0"));
    }

    // -----------------------------------------------------------------------
    // Determinism
    // -----------------------------------------------------------------------

    #[test]
    fn report_is_deterministic_independent_of_catalog_input_order() {
        let decl = fabric_decl(
            "moda",
            "fabricloader",
            &[">=0.18.0"],
            DependencyImportance::Required,
            DependencySource::FabricDepends,
        );
        let entries = vec![
            entry("0.19.0", LoaderReleaseChannel::Stable, None, &[]),
            entry("0.18.6", LoaderReleaseChannel::Stable, Some(2), &[]),
            entry("0.20.0-beta.1", LoaderReleaseChannel::Prerelease, None, &[]),
        ];
        let forward = catalog("fabric", entries.clone());
        let mut reversed_entries = entries.clone();
        reversed_entries.reverse();
        let backward = catalog("fabric", reversed_entries);

        let a = evaluate_loader_compatibility(&request(
            "fabric",
            Some("0.18.6"),
            std::slice::from_ref(&decl),
            &forward,
        ));
        let b =
            evaluate_loader_compatibility(&request("fabric", Some("0.18.6"), &[decl], &backward));
        assert_eq!(a, b);
        // Stable channel first; among stables the explicit rank (0.18.6, rank
        // 2) sorts before the unranked 0.19.0; usable prerelease always last.
        assert_eq!(
            a.compatible_versions
                .iter()
                .map(|c| c.loader_version.as_str())
                .collect::<Vec<_>>(),
            vec!["0.18.6", "0.19.0", "0.20.0-beta.1"]
        );
        // The current tuple satisfies the hard requirement, so it is
        // recommended without consulting the pool.
        assert_eq!(result_version(&a), Some("0.18.6"));
    }
}
