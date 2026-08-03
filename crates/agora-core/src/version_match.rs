//! Version comparison and range matching for JAR-declared incompatibilities.
//!
//! Fabric mods use predicate strings like `">=0.40.0"`, `"<2.0"`, `"*"`, with
//! space-separated AND-joined sub-predicates. Arrays of predicate strings are
//! OR-joined. Forge/NeoForge uses Maven-style ranges like `"[1.0,2.0)"`.
//!
//! Minecraft mod versions are frequently non-SemVer (e.g. `MC1.20.1-3.2.1-build.42`,
//! `0.5.3+build.2`), so the comparator is intentionally lenient: it splits on
//! `.`, `-`, `+`, `_` and compares segment-by-segment, numerically when both
//! segments are numeric, lexicographically otherwise.

use crate::dependency_ops::{DependencyDecl, VersionGrammar};
use std::cmp::Ordering;

/// Result of evaluating a version-range declaration against an installed version.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VersionMatch {
    /// The installed version falls within the declared incompatible range.
    Matched,
    /// The installed version is outside the declared range.
    NotMatched,
    /// The declaration has no version constraint (unconditional — matches any).
    Unconditional,
}

// ---------------------------------------------------------------------------
// Version comparison
// ---------------------------------------------------------------------------

/// Compare two version strings leniently. Splits on `.`, `-`, and `_` and
/// compares segment-by-segment. Numeric segments compare numerically; when
/// one segment is numeric and the other is not, the numeric segment is
/// considered "greater" (newer). Non-numeric segments compare lexicographically.
/// When one version is a prefix of the other, the longer one is "greater"
/// (unless all remaining segments are zero/empty, in which case they're equal).
/// SemVer build metadata after `+` is ignored for precedence.
pub fn compare_versions(a: &str, b: &str) -> Ordering {
    let a = strip_build_metadata(a);
    let b = strip_build_metadata(b);
    if let (Some((a_prefix, a_suffix)), Some((b_prefix, b_suffix))) =
        (a.split_once('-'), b.split_once('-'))
    {
        let prefix_order = compare_version_segments(a_prefix, b_prefix);
        if prefix_order != Ordering::Equal {
            return prefix_order;
        }
        return compare_version_segments(a_suffix, b_suffix);
    }
    compare_version_segments(a, b)
}

fn compare_version_segments(a: &str, b: &str) -> Ordering {
    let seg_a = split_version_segments(a);
    let seg_b = split_version_segments(b);
    let max = seg_a.len().max(seg_b.len());
    for i in 0..max {
        let sa = *seg_a.get(i).unwrap_or(&"");
        let sb = *seg_b.get(i).unwrap_or(&"");
        let ord = compare_segments(sa, sb);
        if ord != Ordering::Equal {
            return ord;
        }
    }
    Ordering::Equal
}

/// Remove SemVer build metadata, which must not affect version precedence.
fn strip_build_metadata(v: &str) -> &str {
    v.split_once('+')
        .map(|(precedence, _)| precedence)
        .unwrap_or(v)
}

/// Split a version string into segments on `.`, `-`, `_`.
fn split_version_segments(v: &str) -> Vec<&str> {
    v.split(['.', '-', '_']).collect()
}

/// Compare two version segments. If both parse as integers, compare numerically.
/// If one is numeric and the other isn't, numeric > non-numeric (so `2` > `beta`).
/// If neither is numeric, compare lexicographically (case-insensitive).
/// An empty segment is treated as `0` so that `1.0.0` equals `1.0`.
fn compare_segments(a: &str, b: &str) -> Ordering {
    let na = if a.is_empty() {
        Some(0)
    } else {
        a.parse::<i64>().ok()
    };
    let nb = if b.is_empty() {
        Some(0)
    } else {
        b.parse::<i64>().ok()
    };
    match (na, nb) {
        (Some(x), Some(y)) => x.cmp(&y),
        (Some(_), None) => Ordering::Greater,
        (None, Some(_)) => Ordering::Less,
        (None, None) => a.to_lowercase().cmp(&b.to_lowercase()),
    }
}

// ---------------------------------------------------------------------------
// Fabric predicate matching
// ---------------------------------------------------------------------------

/// A single comparison operator parsed from a Fabric predicate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CmpOp {
    Greater,
    GreaterEqual,
    Less,
    LessEqual,
    Equal,
    Any,
}

/// Evaluate a Fabric predicate string against an installed version.
///
/// A predicate string like `">=1.0 <2.0"` contains space-separated
/// sub-predicates that are AND-joined. Returns `true` only if ALL
/// sub-predicates match.
///
/// Recognized operators: `>=`, `<=`, `>`, `<`, `=`, `~` (approximate —
/// treats as `>=` for now), and `*` (any version). A bare version with no
/// operator is treated as exact match.
pub fn fabric_predicate_matches(predicate: &str, version: &str) -> bool {
    let trimmed = predicate.trim();
    if trimmed.is_empty() || trimmed == "*" {
        return true;
    }
    for sub in trimmed.split_whitespace() {
        if !fabric_single_matches(sub, version) {
            return false;
        }
    }
    true
}

fn fabric_single_matches(sub: &str, version: &str) -> bool {
    if sub == "*" {
        return true;
    }
    let (op, ver) = parse_predicate_operator(sub);
    match op {
        CmpOp::Any => {
            // No operator — treat as exact match (Fabric spec: bare version = exact).
            compare_versions(version, ver) == Ordering::Equal
        }
        CmpOp::Greater => compare_versions(version, ver) == Ordering::Greater,
        CmpOp::GreaterEqual => {
            matches!(
                compare_versions(version, ver),
                Ordering::Greater | Ordering::Equal
            )
        }
        CmpOp::Less => compare_versions(version, ver) == Ordering::Less,
        CmpOp::LessEqual => {
            matches!(
                compare_versions(version, ver),
                Ordering::Less | Ordering::Equal
            )
        }
        CmpOp::Equal => compare_versions(version, ver) == Ordering::Equal,
    }
}

/// Parse the operator prefix from a predicate like `">=1.0"` → `(GreaterEqual, "1.0")`.
fn parse_predicate_operator(s: &str) -> (CmpOp, &str) {
    for (prefix, op) in [
        (">=", CmpOp::GreaterEqual),
        ("<=", CmpOp::LessEqual),
        (">", CmpOp::Greater),
        ("<", CmpOp::Less),
        ("=", CmpOp::Equal),
        ("~", CmpOp::GreaterEqual), // approximate → treat as >=
    ] {
        if let Some(rest) = s.strip_prefix(prefix) {
            return (op, rest.trim());
        }
    }
    (CmpOp::Any, s)
}

/// Evaluate a list of Fabric predicate strings against an installed version.
///
/// The list has OR semantics: if ANY entry matches, the whole declaration
/// matches. Each entry is itself an AND of space-separated sub-predicates.
/// An empty list means unconditional (any version matches).
pub fn fabric_ranges_match(ranges: &[String], version: &str) -> bool {
    if ranges.is_empty() {
        return true;
    }
    ranges.iter().any(|r| fabric_predicate_matches(r, version))
}

// ---------------------------------------------------------------------------
// Forge Maven range matching
// ---------------------------------------------------------------------------

/// Evaluate a Forge/NeoForge Maven version range against an installed version.
///
/// Maven range grammar:
/// - `[a,b)` — `>= a, < b` (inclusive lower, exclusive upper)
/// - `[a,b]` — `>= a, <= b` (inclusive both)
/// - `(a,b)` — `> a, < b` (exclusive both)
/// - `(a,b]` — `> a, <= b` (exclusive lower, inclusive upper)
/// - `[a,]` or `[a,)` — `>= a` (no upper bound)
/// - `(,b]` or `(,b)` — `< b` or `<= b` (no lower bound)
/// - `[a]` — exact match `== a`
/// - bare `a` (no brackets) — `>= a` (Maven treats bare version as minimum)
pub fn maven_range_matches(range: &str, version: &str) -> bool {
    let trimmed = range.trim();
    if trimmed.is_empty() || trimmed == "*" {
        return true;
    }

    // Bracketed range: [a,b) / (a,b] / [a,b] / (a,b) / [a,] / (,b] / [a] etc.
    if let Some(inner) = parse_maven_brackets(trimmed) {
        return inner.matches(version);
    }

    // Bare version (no brackets): Maven treats as minimum (>= version).
    compare_versions(version, trimmed) != Ordering::Less
}

struct MavenBracketRange {
    lower_inclusive: bool,
    upper_inclusive: bool,
    lower: Option<String>,
    upper: Option<String>,
}

impl MavenBracketRange {
    fn matches(&self, version: &str) -> bool {
        if let Some(ref lower) = self.lower {
            let cmp = compare_versions(version, lower);
            if self.lower_inclusive {
                if cmp == Ordering::Less {
                    return false;
                }
            } else if cmp != Ordering::Greater {
                return false;
            }
        }
        if let Some(ref upper) = self.upper {
            let cmp = compare_versions(version, upper);
            if self.upper_inclusive {
                if cmp == Ordering::Greater {
                    return false;
                }
            } else if cmp != Ordering::Less {
                return false;
            }
        }
        true
    }
}

fn parse_maven_brackets(s: &str) -> Option<MavenBracketRange> {
    let bytes = s.as_bytes();
    if bytes.is_empty() || (bytes[0] != b'[' && bytes[0] != b'(') {
        return None;
    }
    let lower_inclusive = bytes[0] == b'[';
    let last = *bytes.last()?;
    if last != b']' && last != b')' {
        return None;
    }
    let upper_inclusive = last == b']';
    let inner = &s[1..s.len() - 1];
    let parts: Vec<&str> = inner.splitn(2, ',').collect();

    match parts.as_slice() {
        [lower, upper] => {
            let lower = lower.trim();
            let upper = upper.trim();
            let lower = if lower.is_empty() {
                None
            } else {
                Some(lower.to_string())
            };
            let upper = if upper.is_empty() {
                None
            } else {
                Some(upper.to_string())
            };
            Some(MavenBracketRange {
                lower_inclusive,
                upper_inclusive,
                lower,
                upper,
            })
        }
        [single] => {
            // [a] — exact match
            let v = single.trim();
            if v.is_empty() {
                return None;
            }
            Some(MavenBracketRange {
                lower_inclusive: true,
                upper_inclusive: true,
                lower: Some(v.to_string()),
                upper: Some(v.to_string()),
            })
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Unified evaluation
// ---------------------------------------------------------------------------

/// Evaluate whether an incompatibility declaration's version ranges match
/// the installed target version. This is the entry point used by the health
/// check.
///
/// - `version_ranges`: the predicates/ranges from the `IncompatibilityDecl`.
/// - `target_version`: the installed target mod's version string.
/// - `is_fabric_grammar`: true for Fabric/Quilt predicates, false for Forge Maven ranges.
pub fn evaluate_version_match(
    version_ranges: &[String],
    target_version: &str,
    is_fabric_grammar: bool,
) -> VersionMatch {
    if version_ranges.is_empty()
        || version_ranges
            .iter()
            .any(|r| r.trim() == "*" || r.trim().is_empty())
    {
        return VersionMatch::Unconditional;
    }
    let matched = if is_fabric_grammar {
        fabric_ranges_match(version_ranges, target_version)
    } else {
        version_ranges
            .iter()
            .any(|r| maven_range_matches(r, target_version))
    };
    if matched {
        VersionMatch::Matched
    } else {
        VersionMatch::NotMatched
    }
}

// ---------------------------------------------------------------------------
// Strict authoritative requirement evaluation (Work Package 2)
// ---------------------------------------------------------------------------
//
// This is the authoritative API for evaluating `DependencyDecl` version
// requirements. It deliberately does NOT reuse the lenient comparator above:
// it implements the CURRENT Fabric Loader semantics from upstream
// `VersionPredicateParser` / `VersionComparisonOperator` /
// `SemanticVersionImpl`, and strict Maven range grammar for Forge/NeoForge.
// Invalid or unknown predicates fail closed with `Unsupported`.

/// Authoritative result of evaluating a dependency requirement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequirementEvaluation {
    /// The provided version satisfies every declared constraint.
    Satisfied,
    /// The provided version does not satisfy the declared constraints.
    Unsatisfied,
    /// The declaration could not be interpreted (invalid/unknown predicate or
    /// range). Fail closed — the caller must not guess.
    Unsupported { reason: String },
}

/// Evaluate a dependency declaration's version requirement against a provided
/// version string.
///
/// - Empty `version_ranges` means unconstrained → `Satisfied`.
/// - Fabric grammar: OR across the range vec; AND inside each space-separated
///   predicate string; wildcards, `~`, `^`, prerelease precedence and
///   build-ignoring follow Fabric Loader's `VersionPredicateParser`.
/// - Maven grammar: bracket ranges, open bounds, exact `[a]`, bare minimums,
///   and comma-separated union inside one entry.
pub fn evaluate_requirement(
    decl: &DependencyDecl,
    provided_version: &str,
) -> RequirementEvaluation {
    if decl.version_ranges.is_empty() {
        return RequirementEvaluation::Satisfied;
    }
    match decl.grammar {
        VersionGrammar::Fabric => {
            match eval_fabric_ranges(&decl.version_ranges, provided_version) {
                Ok(true) => RequirementEvaluation::Satisfied,
                Ok(false) => RequirementEvaluation::Unsatisfied,
                Err(reason) => RequirementEvaluation::Unsupported { reason },
            }
        }
        VersionGrammar::Maven => match eval_maven_ranges(&decl.version_ranges, provided_version) {
            Ok(true) => RequirementEvaluation::Satisfied,
            Ok(false) => RequirementEvaluation::Unsatisfied,
            Err(reason) => RequirementEvaluation::Unsupported { reason },
        },
    }
}

// --- Strict version model (Fabric Loader superset of SemVer) ---------------

/// A single `<version core>` component. `Wildcard` is only produced when
/// parsing a predicate reference with `store_x` (e.g. `1.x`); provided
/// versions are parsed without wildcards.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StrictComponent {
    Num(u32),
    Wildcard,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StrictSemVer {
    components: Vec<StrictComponent>,
    prerelease: Option<String>,
    build: Option<String>,
    friendly: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum StrictVersion {
    Semantic(StrictSemVer),
    Raw(String),
}

impl StrictVersion {
    fn friendly(&self) -> &str {
        match self {
            StrictVersion::Semantic(version) => &version.friendly,
            StrictVersion::Raw(raw) => raw,
        }
    }
}

/// Parse a version string the way Fabric's `VersionParser` does: try the
/// semantic superset; on failure fall back to a plain string version that only
/// supports exact equality. Empty input is an error (upstream throws).
fn parse_strict_version(input: &str, store_x: bool) -> Result<StrictVersion, String> {
    if input.is_empty() {
        return Err("Version must be a non-empty string!".to_string());
    }
    match parse_semantic(input, store_x) {
        Ok(version) => Ok(StrictVersion::Semantic(version)),
        Err(_) => Ok(StrictVersion::Raw(input.to_string())),
    }
}

/// Parse a semantic version per `SemanticVersionImpl`:
/// `[0-9]+(\.[0-9]+)*(-prerelease)?(+build)?` with an arbitrary number of
/// components, `x`/`X`/`*` wildcards in the last position when `store_x` is
/// set, and arbitrary build contents. The prerelease must be empty or
/// dot-separated `[-0-9A-Za-z]+` identifiers.
fn parse_semantic(input: &str, store_x: bool) -> Result<StrictSemVer, String> {
    let (version_part, build) = match input.split_once('+') {
        Some((version, build)) => (version, Some(build.to_string())),
        None => (input, None),
    };
    let (core, prerelease) = match version_part.split_once('-') {
        Some((core, prerelease)) => (core, Some(prerelease.to_string())),
        None => (version_part, None),
    };
    if let Some(prerelease) = &prerelease {
        if prerelease.is_empty() || !valid_prerelease(prerelease) {
            return Err(format!("Invalid prerelease string '{prerelease}'!"));
        }
    }
    if let Some(build) = &build {
        if build.is_empty() || !valid_build(build) {
            return Err(format!("Invalid build string '{build}'!"));
        }
    }
    if core.ends_with('.') {
        return Err("Negative version number component found!".to_string());
    }
    if core.starts_with('.') {
        return Err("Missing version component!".to_string());
    }
    let component_strings: Vec<&str> = core.split('.').collect();
    if component_strings.is_empty() {
        return Err("Did not provide version numbers!".to_string());
    }
    let mut components: Vec<StrictComponent> = Vec::with_capacity(component_strings.len());
    let mut first_wildcard_idx: Option<usize> = None;
    for (i, component_str) in component_strings.iter().enumerate() {
        if store_x && (component_str.eq(&"x") || component_str.eq(&"X") || component_str.eq(&"*")) {
            if prerelease.is_some() {
                return Err("Pre-release versions are not allowed to use X-ranges!".to_string());
            }
            components.push(StrictComponent::Wildcard);
            if first_wildcard_idx.is_none() {
                first_wildcard_idx = Some(i);
            }
            continue;
        }
        if store_x && i > 0 && components[i - 1] == StrictComponent::Wildcard {
            return Err("Interjacent wildcard (1.x.2) are disallowed!".to_string());
        }
        if component_str.trim().is_empty() {
            return Err("Missing version number component!".to_string());
        }
        let parsed = parse_unsigned_component(component_str).ok_or_else(|| {
            format!("Could not parse version number component '{component_str}'!")
        })?;
        components.push(StrictComponent::Num(parsed));
    }
    if store_x && components.len() == 1 && components[0] == StrictComponent::Wildcard {
        return Err("Versions of form 'x' or 'X' not allowed!".to_string());
    }
    // Strip trailing wildcards (1.x.x -> 1.x).
    if let Some(first) = first_wildcard_idx {
        if first > 0 && components.len() > first + 1 {
            components.truncate(first + 1);
        }
    }
    let friendly = build_friendly(&components, prerelease.as_deref(), build.as_deref());
    Ok(StrictSemVer {
        components,
        prerelease,
        build,
        friendly,
    })
}

/// Java's `Integer.parseInt` accepts an optional leading `+`; replicate that
/// for component parsing. Negative components fail (as in upstream).
fn parse_unsigned_component(s: &str) -> Option<u32> {
    let s = s.strip_prefix('+').unwrap_or(s);
    if s.is_empty() {
        return None;
    }
    s.parse::<u32>().ok()
}

fn valid_prerelease(prerelease: &str) -> bool {
    prerelease
        .split('.')
        .all(|id| !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'))
}

fn valid_build(build: &str) -> bool {
    build
        .split('.')
        .all(|id| !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'))
}

fn build_friendly(
    components: &[StrictComponent],
    prerelease: Option<&str>,
    build: Option<&str>,
) -> String {
    let mut out = String::new();
    for (i, component) in components.iter().enumerate() {
        if i > 0 {
            out.push('.');
        }
        match component {
            StrictComponent::Num(number) => out.push_str(&number.to_string()),
            StrictComponent::Wildcard => out.push('x'),
        }
    }
    if let Some(prerelease) = prerelease {
        out.push('-');
        out.push_str(prerelease);
    }
    if let Some(build) = build {
        out.push('+');
        out.push_str(build);
    }
    out
}

/// Component access per `SemanticVersionImpl.getVersionComponent`: missing
/// components repeat `0`, or repeat the wildcard when the last component is a
/// wildcard.
fn component_at(version: &StrictSemVer, index: usize) -> StrictComponent {
    match version.components.get(index) {
        Some(component) => *component,
        None => {
            if version.components.last() == Some(&StrictComponent::Wildcard) {
                StrictComponent::Wildcard
            } else {
                StrictComponent::Num(0)
            }
        }
    }
}

fn has_wildcard(version: &StrictSemVer) -> bool {
    version
        .components
        .iter()
        .any(|component| matches!(component, StrictComponent::Wildcard))
}

/// Compare two semantic versions per `SemanticVersionImpl.compareTo`:
/// component-by-component (wildcards are skipped), then SemVer prerelease
/// precedence (numeric identifiers compare numerically, non-numeric
/// lexicographically, numeric < non-numeric, fewer identifiers < more, release
/// > prerelease). Build metadata is ignored.
fn semantic_compare(a: &StrictSemVer, b: &StrictSemVer) -> Ordering {
    let max = a.components.len().max(b.components.len());
    for i in 0..max {
        let ca = component_at(a, i);
        let cb = component_at(b, i);
        if matches!(ca, StrictComponent::Wildcard) || matches!(cb, StrictComponent::Wildcard) {
            continue;
        }
        let na = match ca {
            StrictComponent::Num(number) => number,
            StrictComponent::Wildcard => unreachable!(),
        };
        let nb = match cb {
            StrictComponent::Num(number) => number,
            StrictComponent::Wildcard => unreachable!(),
        };
        let ord = na.cmp(&nb);
        if ord != Ordering::Equal {
            return ord;
        }
    }
    match (&a.prerelease, &b.prerelease) {
        (Some(pa), Some(pb)) => compare_prerelease(pa, pb),
        (Some(_), None) => {
            if has_wildcard(b) {
                Ordering::Equal
            } else {
                Ordering::Less
            }
        }
        (None, Some(_)) => {
            if has_wildcard(a) {
                Ordering::Equal
            } else {
                Ordering::Greater
            }
        }
        (None, None) => Ordering::Equal,
    }
}

/// SemVer prerelease identifier comparison matching upstream's tokenizer:
/// numeric identifiers compare by length first (then lexicographically for
/// equal lengths), numeric < non-numeric, then lexicographic, then fewer
/// identifiers < more.
fn compare_prerelease(a: &str, b: &str) -> Ordering {
    let a_parts: Vec<&str> = if a.is_empty() {
        Vec::new()
    } else {
        a.split('.').collect()
    };
    let b_parts: Vec<&str> = if b.is_empty() {
        Vec::new()
    } else {
        b.split('.').collect()
    };
    for (pa, pb) in a_parts.iter().zip(b_parts.iter()) {
        let a_numeric = !pa.is_empty() && pa.chars().all(|c| c.is_ascii_digit());
        let b_numeric = !pb.is_empty() && pb.chars().all(|c| c.is_ascii_digit());
        match (a_numeric, b_numeric) {
            (true, true) => {
                let ord = pa.len().cmp(&pb.len());
                if ord != Ordering::Equal {
                    return ord;
                }
            }
            (true, false) => return Ordering::Less,
            (false, true) => return Ordering::Greater,
            (false, false) => {}
        }
        let ord = pa.cmp(pb);
        if ord != Ordering::Equal {
            return ord;
        }
    }
    a_parts.len().cmp(&b_parts.len())
}

// --- Fabric predicate semantics (upstream VersionPredicateParser) ----------

/// Comparison operators from `VersionComparisonOperator`. The serialized
/// prefixes must be probed longest-first (`>=` before `>`, `<=` before `<`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StrictOp {
    Greater,
    GreaterEqual,
    Less,
    LessEqual,
    Equal,
    SameToNextMinor,
    SameToNextMajor,
}

impl StrictOp {
    fn serialized(self) -> &'static str {
        match self {
            StrictOp::GreaterEqual => ">=",
            StrictOp::LessEqual => "<=",
            StrictOp::Greater => ">",
            StrictOp::Less => "<",
            StrictOp::Equal => "=",
            StrictOp::SameToNextMinor => "~",
            StrictOp::SameToNextMajor => "^",
        }
    }

    /// Whether the operator has at least one inclusive bound. These are the
    /// only operators Fabric lets a non-semver reference degrade to exact
    /// string equality.
    fn has_inclusive_bound(self) -> bool {
        matches!(
            self,
            StrictOp::Equal
                | StrictOp::GreaterEqual
                | StrictOp::LessEqual
                | StrictOp::SameToNextMinor
                | StrictOp::SameToNextMajor
        )
    }

    /// `VersionComparisonOperator.test`: semantic comparison when both sides
    /// are semantic; otherwise exact string equality for inclusive operators
    /// and `false` for exclusive ones.
    fn test(self, target: &StrictVersion, reference: &StrictVersion) -> bool {
        match (target, reference) {
            (StrictVersion::Semantic(target), StrictVersion::Semantic(reference)) => {
                let ord = semantic_compare(target, reference);
                match self {
                    StrictOp::Greater => ord == Ordering::Greater,
                    StrictOp::GreaterEqual => ord != Ordering::Less,
                    StrictOp::Less => ord == Ordering::Less,
                    StrictOp::LessEqual => ord != Ordering::Greater,
                    StrictOp::Equal => ord == Ordering::Equal,
                    StrictOp::SameToNextMinor => {
                        ord != Ordering::Less
                            && component_at(target, 0) == component_at(reference, 0)
                            && component_at(target, 1) == component_at(reference, 1)
                    }
                    StrictOp::SameToNextMajor => {
                        ord != Ordering::Less
                            && component_at(target, 0) == component_at(reference, 0)
                    }
                }
            }
            _ => {
                if self.has_inclusive_bound() {
                    target.friendly() == reference.friendly()
                } else {
                    false
                }
            }
        }
    }
}

#[derive(Debug, Clone)]
struct SinglePredicate {
    operator: StrictOp,
    reference: StrictVersion,
}

/// Parse one Fabric predicate string (space-separated terms are AND-joined).
/// `*`/blank terms are skipped. Errors replicate `VersionParsingException`
/// from upstream and make the whole predicate unsupported.
fn parse_fabric_predicate(predicate: &str) -> Result<Vec<SinglePredicate>, String> {
    let mut predicates = Vec::new();
    for token in predicate.split_whitespace() {
        if token == "*" {
            continue;
        }
        let mut operator = StrictOp::Equal;
        let mut rest = token;
        for candidate in [
            StrictOp::GreaterEqual,
            StrictOp::LessEqual,
            StrictOp::Greater,
            StrictOp::Less,
            StrictOp::Equal,
            StrictOp::SameToNextMinor,
            StrictOp::SameToNextMajor,
        ] {
            if let Some(remainder) = rest.strip_prefix(candidate.serialized()) {
                operator = candidate;
                rest = remainder;
                break;
            }
        }
        let reference = parse_strict_version(rest, true)?;
        if let StrictVersion::Semantic(semantic) = &reference {
            if has_wildcard(semantic) {
                if operator != StrictOp::Equal {
                    return Err(format!(
                        "Invalid predicate '{predicate}', version ranges with wildcards (.X) require using the equality operator or no operator at all!"
                    ));
                }
                // `1.x` -> same major, `1.2.x` -> same minor, per upstream.
                let comp_count = semantic.components.len();
                operator = if comp_count == 2 {
                    StrictOp::SameToNextMajor
                } else {
                    StrictOp::SameToNextMinor
                };
                let new_components: Vec<StrictComponent> =
                    semantic.components[..comp_count.saturating_sub(1)].to_vec();
                let rewritten = StrictVersion::Semantic(StrictSemVer {
                    friendly: build_friendly(&new_components, None, semantic.build.as_deref()),
                    components: new_components,
                    prerelease: None,
                    build: semantic.build.clone(),
                });
                predicates.push(SinglePredicate {
                    operator,
                    reference: rewritten,
                });
                continue;
            }
        } else {
            // Non-semver reference: exclusive-bound operators are rejected;
            // everything else degrades to exact equality.
            if !operator.has_inclusive_bound() {
                return Err(format!(
                    "Invalid predicate '{predicate}', version ranges need to be semantic version compatible to use operators that exclude the bound!"
                ));
            }
            operator = StrictOp::Equal;
        }
        predicates.push(SinglePredicate {
            operator,
            reference,
        });
    }
    Ok(predicates)
}

/// Evaluate Fabric ranges (OR across entries, AND inside each entry) against
/// a provided version. `Err` = unsupported (fail closed).
fn eval_fabric_ranges(ranges: &[String], provided_version: &str) -> Result<bool, String> {
    let target = parse_strict_version(provided_version, false)?;
    for range in ranges {
        let predicates = parse_fabric_predicate(range)?;
        if predicates.is_empty() {
            return Ok(true);
        }
        if predicates
            .iter()
            .all(|predicate| predicate.operator.test(&target, &predicate.reference))
        {
            return Ok(true);
        }
    }
    Ok(false)
}

// --- Maven range grammar (Forge/NeoForge) ----------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
struct StrictMavenRange {
    lower: Option<String>,
    lower_inclusive: bool,
    upper: Option<String>,
    upper_inclusive: bool,
}

/// Evaluate Maven range entries (OR across entries, union within an entry)
/// against a provided version. `Err` = malformed range (unsupported).
fn eval_maven_ranges(ranges: &[String], provided_version: &str) -> Result<bool, String> {
    let target = parse_strict_version(provided_version, false)?;
    for range in ranges {
        let parsed = parse_maven_range_spec(range)?;
        if parsed.iter().any(|r| maven_range_satisfied(r, &target)) {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Parse a Maven range spec: bracket ranges and/or bare minimums joined by
/// top-level commas as a union.
fn parse_maven_range_spec(spec: &str) -> Result<Vec<StrictMavenRange>, String> {
    let trimmed = spec.trim();
    if trimmed.is_empty() {
        return Err("Maven range spec must not be empty.".to_string());
    }
    let parts = split_maven_union(trimmed)?;
    let mut ranges = Vec::with_capacity(parts.len());
    for part in parts {
        ranges.push(parse_maven_single(part)?);
    }
    Ok(ranges)
}

/// Split a Maven spec on commas that are outside any bracket pair. Tracks
/// bracket depth and rejects unbalanced brackets.
fn split_maven_union(spec: &str) -> Result<Vec<&str>, String> {
    let mut parts = Vec::new();
    let mut depth: i32 = 0;
    let mut start = 0;
    for (i, c) in spec.char_indices() {
        match c {
            '[' | '(' => depth += 1,
            ']' | ')' => {
                depth -= 1;
                if depth < 0 {
                    return Err(format!(
                        "Malformed Maven range '{spec}': unbalanced bracket."
                    ));
                }
            }
            ',' if depth == 0 => {
                parts.push(spec[start..i].trim());
                start = i + 1;
            }
            _ => {}
        }
    }
    if depth != 0 {
        return Err(format!(
            "Malformed Maven range '{spec}': unbalanced bracket."
        ));
    }
    parts.push(spec[start..].trim());
    if parts.iter().any(|part| part.is_empty()) {
        return Err(format!(
            "Malformed Maven range '{spec}': empty union element."
        ));
    }
    Ok(parts)
}

fn valid_maven_bound(bound: &str) -> bool {
    !bound.is_empty()
        && !bound
            .chars()
            .any(|c| matches!(c, '[' | ']' | '(' | ')') || c.is_whitespace())
}

/// Parse one Maven range: `[a,b]`, `[a,b)`, `(a,b]`, `(a,b)`, open bounds
/// (`[a,)`, `(,b]`, …), exact `[a]`, or a bare version meaning a minimum.
fn parse_maven_single(part: &str) -> Result<StrictMavenRange, String> {
    if part == "*" {
        return Err(format!(
            "Malformed Maven range '{part}': '*' is not a Maven version range."
        ));
    }
    let first = part.chars().next().unwrap_or(' ');
    let last = part.chars().last().unwrap_or(' ');
    let bracketed = matches!(first, '[' | '(') || matches!(last, ']' | ')');
    if !bracketed {
        if part.chars().any(|c| matches!(c, '[' | ']' | '(' | ')')) {
            return Err(format!("Malformed Maven range '{part}'."));
        }
        return Ok(StrictMavenRange {
            lower: Some(part.to_string()),
            lower_inclusive: true,
            upper: None,
            upper_inclusive: true,
        });
    }
    if !matches!(first, '[' | '(') || !matches!(last, ']' | ')') || part.len() < 2 {
        return Err(format!("Malformed Maven range '{part}'."));
    }
    let lower_inclusive = first == '[';
    let upper_inclusive = last == ']';
    let inner = &part[1..part.len() - 1];
    let segments: Vec<&str> = inner.split(',').collect();
    match segments.as_slice() {
        [single] => {
            let single = single.trim();
            if single.is_empty() || !valid_maven_bound(single) {
                return Err(format!("Malformed Maven range '{part}'."));
            }
            if !lower_inclusive {
                return Err(format!(
                    "Malformed Maven range '{part}': exclusive bounds cannot be exact."
                ));
            }
            Ok(StrictMavenRange {
                lower: Some(single.to_string()),
                lower_inclusive: true,
                upper: Some(single.to_string()),
                upper_inclusive: true,
            })
        }
        [lower, upper] => {
            let lower = lower.trim();
            let upper = upper.trim();
            if lower.is_empty() && upper.is_empty() {
                return Err(format!("Malformed Maven range '{part}'."));
            }
            if !lower.is_empty() && !valid_maven_bound(lower) {
                return Err(format!("Malformed Maven range '{part}'."));
            }
            if !upper.is_empty() && !valid_maven_bound(upper) {
                return Err(format!("Malformed Maven range '{part}'."));
            }
            Ok(StrictMavenRange {
                lower: if lower.is_empty() {
                    None
                } else {
                    Some(lower.to_string())
                },
                lower_inclusive,
                upper: if upper.is_empty() {
                    None
                } else {
                    Some(upper.to_string())
                },
                upper_inclusive,
            })
        }
        _ => Err(format!("Malformed Maven range '{part}': too many commas.")),
    }
}

/// Whether a provided version falls inside a parsed Maven range. Bounds are
/// compared strictly semantically; when either operand is non-semver the bound
/// can only be satisfied through exact string equality on an inclusive bound.
fn maven_range_satisfied(range: &StrictMavenRange, target: &StrictVersion) -> bool {
    if let Some(lower) = &range.lower {
        match maven_bound_cmp(target, lower) {
            Some(Ordering::Less) => return false,
            Some(Ordering::Equal) => {
                if !range.lower_inclusive {
                    return false;
                }
            }
            Some(Ordering::Greater) => {}
            None => {
                if !(range.lower_inclusive && target.friendly() == lower.as_str()) {
                    return false;
                }
            }
        }
    }
    if let Some(upper) = &range.upper {
        match maven_bound_cmp(target, upper) {
            Some(Ordering::Greater) => return false,
            Some(Ordering::Equal) => {
                if !range.upper_inclusive {
                    return false;
                }
            }
            Some(Ordering::Less) => {}
            None => {
                if !(range.upper_inclusive && target.friendly() == upper.as_str()) {
                    return false;
                }
            }
        }
    }
    true
}

/// Compare a provided version against a Maven range bound. `None` when either
/// side is not a parseable semantic version (no ordering exists).
fn maven_bound_cmp(target: &StrictVersion, bound: &str) -> Option<Ordering> {
    let bound_version = parse_strict_version(bound, false).ok()?;
    match (target, bound_version) {
        (StrictVersion::Semantic(target), StrictVersion::Semantic(bound)) => {
            Some(semantic_compare(target, &bound))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- compare_versions ---

    #[test]
    fn compare_numeric_versions() {
        assert_eq!(compare_versions("1.0", "1.0"), Ordering::Equal);
        assert_eq!(compare_versions("2.0", "1.0"), Ordering::Greater);
        assert_eq!(compare_versions("1.0", "2.0"), Ordering::Less);
        assert_eq!(compare_versions("1.10", "1.9"), Ordering::Greater);
        assert_eq!(compare_versions("1.0.0", "1.0"), Ordering::Equal);
    }

    #[test]
    fn compare_non_semver_versions() {
        assert_eq!(
            compare_versions("MC1.20.1-3.2.1", "MC1.20.1-3.2.0"),
            Ordering::Greater
        );
        assert_eq!(
            compare_versions("0.5.3+build.2", "0.5.3+build.1"),
            Ordering::Equal
        );
        assert_eq!(compare_versions("0.6.0+mc1.21.1", "0.6.0"), Ordering::Equal);
        assert_eq!(compare_versions("1.0.0", "1.0.0"), Ordering::Equal);
        assert_eq!(
            compare_versions("1.21.1-3.15.6", "1.21-3.4.4"),
            Ordering::Greater
        );
    }

    #[test]
    fn compare_mixed_numeric_string() {
        // Numeric > non-numeric segment
        assert_eq!(compare_versions("2.0", "beta"), Ordering::Greater);
        assert_eq!(compare_versions("1.0.0", "1.0.beta"), Ordering::Greater);
    }

    // --- Fabric predicate matching ---

    #[test]
    fn fabric_predicate_exact() {
        assert!(fabric_predicate_matches("1.0", "1.0"));
        assert!(!fabric_predicate_matches("1.0", "1.1"));
    }

    #[test]
    fn fabric_predicate_less_than() {
        assert!(fabric_predicate_matches("<2.0", "1.9"));
        assert!(!fabric_predicate_matches("<2.0", "2.0"));
        assert!(!fabric_predicate_matches("<2.0", "2.1"));
    }

    #[test]
    fn fabric_predicate_greater_equal() {
        assert!(fabric_predicate_matches(">=1.0", "1.0"));
        assert!(fabric_predicate_matches(">=1.0", "2.0"));
        assert!(!fabric_predicate_matches(">=1.0", "0.9"));
    }

    #[test]
    fn fabric_predicates_ignore_build_metadata() {
        assert!(!fabric_predicate_matches("<0.6.0", "0.6.0+mc1.21.1"));
        assert!(fabric_predicate_matches(">=0.6.0", "0.6.0+mc1.21.1"));
        assert!(fabric_predicate_matches("<0.6.0", "0.5.7+mc1.21.1"));
        assert!(fabric_predicate_matches("<0.6.0", "0.6.0-beta.3+mc1.21.1"));
    }

    #[test]
    fn fabric_predicate_and() {
        // Space-separated = AND
        assert!(fabric_predicate_matches(">=1.0 <2.0", "1.5"));
        assert!(!fabric_predicate_matches(">=1.0 <2.0", "0.5")); // < lower
        assert!(!fabric_predicate_matches(">=1.0 <2.0", "2.5")); // > upper
        assert!(fabric_predicate_matches(">=1.0 <2.0", "1.0")); // boundary
    }

    #[test]
    fn fabric_predicate_wildcard() {
        assert!(fabric_predicate_matches("*", "anything"));
        assert!(fabric_predicate_matches("*", "1.0"));
    }

    // --- Fabric ranges (OR) ---

    #[test]
    fn fabric_ranges_or_semantics() {
        let ranges = vec!["<2.0".to_string(), ">=3.0".to_string()];
        assert!(fabric_ranges_match(&ranges, "1.5")); // matches <2.0
        assert!(fabric_ranges_match(&ranges, "3.5")); // matches >=3.0
        assert!(!fabric_ranges_match(&ranges, "2.5")); // matches neither
    }

    #[test]
    fn fabric_ranges_empty_is_match() {
        assert!(fabric_ranges_match(&[], "anything"));
    }

    // --- Forge Maven range matching ---

    #[test]
    fn maven_range_inclusive_exclusive() {
        assert!(maven_range_matches("[1.0,2.0)", "1.0"));
        assert!(maven_range_matches("[1.0,2.0)", "1.5"));
        assert!(!maven_range_matches("[1.0,2.0)", "2.0")); // exclusive upper
        assert!(!maven_range_matches("[1.0,2.0)", "0.9"));
    }

    #[test]
    fn maven_range_inclusive_both() {
        assert!(maven_range_matches("[1.0,2.0]", "1.0"));
        assert!(maven_range_matches("[1.0,2.0]", "2.0"));
        assert!(!maven_range_matches("[1.0,2.0]", "0.9"));
        assert!(!maven_range_matches("[1.0,2.0]", "2.1"));
    }

    #[test]
    fn maven_range_exact() {
        assert!(maven_range_matches("[1.0]", "1.0"));
        assert!(!maven_range_matches("[1.0]", "1.1"));
    }

    #[test]
    fn maven_range_no_upper() {
        assert!(maven_range_matches("[1.0,)", "1.0"));
        assert!(maven_range_matches("[1.0,)", "99.0"));
        assert!(!maven_range_matches("[1.0,)", "0.9"));
        // Also [1.0,] form
        assert!(maven_range_matches("[1.0,]", "5.0"));
    }

    #[test]
    fn maven_range_no_lower() {
        assert!(maven_range_matches("(,2.0]", "1.0"));
        assert!(maven_range_matches("(,2.0]", "2.0"));
        assert!(!maven_range_matches("(,2.0]", "2.1"));
        assert!(!maven_range_matches("(,1.21-3.4.4]", "1.21.1-3.15.6"));
    }

    #[test]
    fn maven_range_bare_version() {
        // Bare version = >= minimum
        assert!(maven_range_matches("1.0", "1.0"));
        assert!(maven_range_matches("1.0", "2.0"));
        assert!(!maven_range_matches("1.0", "0.9"));
    }

    #[test]
    fn maven_range_wildcard() {
        assert!(maven_range_matches("*", "anything"));
    }

    // --- Unified evaluate_version_match ---

    #[test]
    fn evaluate_fabric_matched() {
        let ranges = vec!["<2.0".to_string()];
        assert_eq!(
            evaluate_version_match(&ranges, "1.5", true),
            VersionMatch::Matched
        );
    }

    #[test]
    fn evaluate_fabric_not_matched() {
        let ranges = vec!["<2.0".to_string()];
        assert_eq!(
            evaluate_version_match(&ranges, "2.5", true),
            VersionMatch::NotMatched
        );
    }

    #[test]
    fn evaluate_fabric_unconditional() {
        let ranges: Vec<String> = vec![];
        assert_eq!(
            evaluate_version_match(&ranges, "anything", true),
            VersionMatch::Unconditional
        );
    }

    #[test]
    fn evaluate_forge_matched() {
        let ranges = vec!["[1.0,2.0)".to_string()];
        assert_eq!(
            evaluate_version_match(&ranges, "1.5", false),
            VersionMatch::Matched
        );
    }

    #[test]
    fn evaluate_forge_not_matched() {
        let ranges = vec!["[1.0,2.0)".to_string()];
        assert_eq!(
            evaluate_version_match(&ranges, "2.5", false),
            VersionMatch::NotMatched
        );
    }

    #[test]
    fn evaluate_non_semver_version() {
        let ranges = vec!["<2.0".to_string()];
        assert_eq!(
            evaluate_version_match(&ranges, "MC1.20.1-1.5", true),
            VersionMatch::Matched
        );
    }

    // -----------------------------------------------------------------------
    // Strict authoritative API (Work Package 2)
    // -----------------------------------------------------------------------

    use crate::dependency_ops::{DependencyDecl, DependencyImportance, DependencySource};

    fn fabric_decl(ranges: &[&str]) -> DependencyDecl {
        DependencyDecl {
            declaring_mod_id: None,
            target_id: "some_mod".into(),
            version_ranges: ranges.iter().map(|s| s.to_string()).collect(),
            importance: DependencyImportance::Required,
            grammar: crate::dependency_ops::VersionGrammar::Fabric,
            source: DependencySource::FabricDepends,
        }
    }

    fn maven_decl(ranges: &[&str]) -> DependencyDecl {
        DependencyDecl {
            declaring_mod_id: None,
            target_id: "some_mod".into(),
            version_ranges: ranges.iter().map(|s| s.to_string()).collect(),
            importance: DependencyImportance::Required,
            grammar: crate::dependency_ops::VersionGrammar::Maven,
            source: DependencySource::ForgeDependency,
        }
    }

    fn sat(decl: &DependencyDecl, version: &str) -> bool {
        evaluate_requirement(decl, version) == RequirementEvaluation::Satisfied
    }

    fn unsat(decl: &DependencyDecl, version: &str) -> bool {
        evaluate_requirement(decl, version) == RequirementEvaluation::Unsatisfied
    }

    fn unsupported(decl: &DependencyDecl, version: &str) -> bool {
        matches!(
            evaluate_requirement(decl, version),
            RequirementEvaluation::Unsupported { .. }
        )
    }

    #[test]
    fn strict_empty_ranges_are_satisfied() {
        assert!(sat(&fabric_decl(&[]), "anything"));
        assert!(sat(&maven_decl(&[]), "anything"));
        assert!(sat(&fabric_decl(&[""]), "anything"));
        assert!(sat(&fabric_decl(&["*"]), "anything"));
        assert!(sat(&fabric_decl(&["  "]), "anything"));
    }

    #[test]
    fn strict_fabric_exact() {
        assert!(sat(&fabric_decl(&["1.0"]), "1.0"));
        assert!(unsat(&fabric_decl(&["1.0"]), "1.1"));
        assert!(sat(&fabric_decl(&["=1.0"]), "1.0"));
        assert!(unsat(&fabric_decl(&["=1.0"]), "1.0.1"));
        // Missing components repeat 0.
        assert!(sat(&fabric_decl(&["1.0"]), "1.0.0"));
        assert!(sat(&fabric_decl(&["1.0.0"]), "1.0"));
        assert!(unsat(&fabric_decl(&["1.0.1"]), "1.0"));
    }

    #[test]
    fn strict_fabric_ordered_operators() {
        assert!(sat(&fabric_decl(&[">=1.0"]), "1.0"));
        assert!(sat(&fabric_decl(&[">=1.0"]), "2.0"));
        assert!(unsat(&fabric_decl(&[">=1.0"]), "0.9"));
        assert!(unsat(&fabric_decl(&[">1.0"]), "1.0"));
        assert!(sat(&fabric_decl(&[">1.0"]), "1.0.1"));
        assert!(sat(&fabric_decl(&["<=1.0"]), "1.0"));
        assert!(unsat(&fabric_decl(&["<=1.0"]), "1.1"));
        assert!(unsat(&fabric_decl(&["<1.0"]), "1.0"));
        assert!(sat(&fabric_decl(&["<1.0"]), "0.9"));
        assert!(sat(&fabric_decl(&["<1.10"]), "1.9"));
    }

    #[test]
    fn strict_fabric_approximate_and_caret() {
        assert!(sat(&fabric_decl(&["~1.2"]), "1.2"));
        assert!(sat(&fabric_decl(&["~1.2"]), "1.2.5"));
        assert!(sat(&fabric_decl(&["~1.2"]), "1.2.99"));
        assert!(unsat(&fabric_decl(&["~1.2"]), "1.3.0"));
        assert!(unsat(&fabric_decl(&["~1.2"]), "2.0"));
        assert!(unsat(&fabric_decl(&["~1.2"]), "0.9"));
        assert!(sat(&fabric_decl(&["^1.2"]), "1.2"));
        assert!(sat(&fabric_decl(&["^1.2"]), "1.9.9"));
        assert!(unsat(&fabric_decl(&["^1.2"]), "2.0"));
        assert!(unsat(&fabric_decl(&["^1.2"]), "0.9"));
        assert!(sat(&fabric_decl(&["^1"]), "1.99"));
        assert!(unsat(&fabric_decl(&["^1"]), "2.0"));
    }

    #[test]
    fn strict_fabric_wildcards() {
        assert!(sat(&fabric_decl(&["1.x"]), "1.0"));
        assert!(sat(&fabric_decl(&["1.x"]), "1.5"));
        assert!(sat(&fabric_decl(&["1.x"]), "1"));
        assert!(unsat(&fabric_decl(&["1.x"]), "2.0"));
        assert!(unsat(&fabric_decl(&["1.x"]), "0.9"));
        assert!(sat(&fabric_decl(&["1.X"]), "1.3"));
        assert!(sat(&fabric_decl(&["1.2.x"]), "1.2.3"));
        assert!(sat(&fabric_decl(&["1.2.x"]), "1.2"));
        assert!(unsat(&fabric_decl(&["1.2.x"]), "1.3.0"));
        assert!(unsat(&fabric_decl(&["1.2.x"]), "2.0"));
    }

    #[test]
    fn strict_fabric_wildcard_with_operator_unsupported() {
        assert!(unsupported(&fabric_decl(&[">=1.x"]), "1.5"));
        assert!(unsupported(&fabric_decl(&["<1.2.x"]), "1.1"));
        assert!(unsupported(&fabric_decl(&["~1.x"]), "1.5"));
    }

    #[test]
    fn strict_fabric_and_expression() {
        let decl = fabric_decl(&[">=1.0 <2.0"]);
        assert!(sat(&decl, "1.0"));
        assert!(sat(&decl, "1.5"));
        assert!(unsat(&decl, "0.5"));
        assert!(unsat(&decl, "2.0"));
        assert!(unsat(&decl, "2.5"));
    }

    #[test]
    fn strict_fabric_or_across_range_vec() {
        let decl = fabric_decl(&["<2.0", ">=3.0"]);
        assert!(sat(&decl, "1.5"));
        assert!(sat(&decl, "3.5"));
        assert!(unsat(&decl, "2.5"));
    }

    #[test]
    fn strict_fabric_prerelease_precedence() {
        assert!(sat(&fabric_decl(&["<1.0.0"]), "1.0.0-alpha"));
        assert!(unsat(&fabric_decl(&[">=1.0.0"]), "1.0.0-alpha"));
        assert!(unsat(&fabric_decl(&["=1.0.0"]), "1.0.0-alpha"));
        assert!(unsat(&fabric_decl(&["=1.0.0-alpha"]), "1.0.0-alpha.1"));
        assert!(sat(&fabric_decl(&["=1.0.0-alpha.1"]), "1.0.0-alpha.1"));
        // Numeric prerelease identifiers compare numerically (2 < 10).
        assert!(unsat(&fabric_decl(&["<1.0.0-alpha.2"]), "1.0.0-alpha.10"));
        assert!(sat(&fabric_decl(&["<1.0.0-alpha.2"]), "1.0.0-alpha.1"));
        // Numeric identifiers sort before non-numeric ones.
        assert!(unsat(&fabric_decl(&["=1.0.0-alpha"]), "1.0.0-1"));
        assert!(sat(&fabric_decl(&["<1.0.0-alpha"]), "1.0.0-1"));
    }

    #[test]
    fn strict_fabric_build_metadata_ignored() {
        assert!(sat(&fabric_decl(&["=1.0.0+build.5"]), "1.0.0+build.9"));
        assert!(sat(&fabric_decl(&["=1.0.0"]), "1.0.0+build.9"));
        assert!(sat(&fabric_decl(&["<1.0.1"]), "1.0.0+mc1.21.1"));
        assert!(unsat(&fabric_decl(&["<1.0.0+build.1"]), "1.0.0"));
    }

    #[test]
    fn strict_fabric_non_semver_exact_equality_only() {
        let exact = fabric_decl(&["=MC1.20.1"]);
        assert!(sat(&exact, "MC1.20.1"));
        assert!(unsat(&exact, "MC1.20.2"));
        assert!(sat(&fabric_decl(&["MC1.20.1"]), "MC1.20.1"));
        assert!(unsat(&fabric_decl(&["MC1.20.1"]), "mc1.20.1"));
        // Inclusive operators degrade to equality for non-semver references.
        assert!(sat(&fabric_decl(&[">=MC1.20.1"]), "MC1.20.1"));
        assert!(unsat(&fabric_decl(&[">=MC1.20.1"]), "MC1.20.2"));
        assert!(sat(&fabric_decl(&["~foo"]), "foo"));
        assert!(unsat(&fabric_decl(&["~foo"]), "bar"));
        assert!(sat(&fabric_decl(&["^foo"]), "foo"));
        // Exclusive operators on non-semver references are rejected.
        assert!(unsupported(&fabric_decl(&[">MC1.20.1"]), "MC1.20.1"));
        assert!(unsupported(&fabric_decl(&["<MC1.20.1"]), "MC1.20.1"));
        // A non-semver TARGET just never satisfies exclusive operators.
        assert!(unsat(&fabric_decl(&[">1.0"]), "MC1.20.1"));
        // Interjacent wildcards fall back to string equality (upstream).
        assert!(sat(&fabric_decl(&["1.x.2"]), "1.x.2"));
        assert!(unsat(&fabric_decl(&["1.x.2"]), "1.0.2"));
    }

    #[test]
    fn strict_fabric_invalid_predicates_unsupported() {
        assert!(unsupported(&fabric_decl(&[">=1.0 <"]), "1.5"));
        assert!(unsupported(&fabric_decl(&[">="]), "1.5"));
        assert!(unsupported(&fabric_decl(&["="]), "1.5"));
        assert!(unsupported(&fabric_decl(&["> 1.0"]), "1.5"));
        // A valid range in the vec does not rescue a genuinely invalid one.
        assert!(unsupported(&fabric_decl(&[">MC1.20.1", "1.0"]), "1.0"));
        assert!(unsupported(&fabric_decl(&[">=1.0"]), ""));
        // Malformed semantic suffixes must not be normalized into a release
        // version by build/prerelease metadata handling.
        assert!(unsat(&fabric_decl(&["=1.0.0+"]), "1.0.0"));
        assert!(unsat(&fabric_decl(&["=1.0.0-"]), "1.0.0"));
    }

    #[test]
    fn strict_fabric_unknown_strings_are_exact_equality() {
        // Arbitrary strings are valid Fabric StringVersions with equality
        // semantics — they match only identical strings.
        assert!(sat(
            &fabric_decl(&["!!not-a-predicate!!"]),
            "!!not-a-predicate!!"
        ));
        assert!(unsat(&fabric_decl(&["!!not-a-predicate!!"]), "1.0"));
    }

    #[test]
    fn strict_maven_bracket_ranges() {
        assert!(sat(&maven_decl(&["[1.0,2.0)"]), "1.0"));
        assert!(sat(&maven_decl(&["[1.0,2.0)"]), "1.5"));
        assert!(unsat(&maven_decl(&["[1.0,2.0)"]), "2.0"));
        assert!(unsat(&maven_decl(&["[1.0,2.0)"]), "0.9"));
        assert!(sat(&maven_decl(&["[1.0,2.0]"]), "2.0"));
        assert!(unsat(&maven_decl(&["[1.0,2.0]"]), "2.1"));
        assert!(unsat(&maven_decl(&["(1.0,2.0)"]), "1.0"));
        assert!(sat(&maven_decl(&["(1.0,2.0)"]), "1.5"));
        assert!(unsat(&maven_decl(&["(1.0,2.0]"]), "1.0"));
        assert!(sat(&maven_decl(&["(1.0,2.0]"]), "2.0"));
    }

    #[test]
    fn strict_maven_open_bounds() {
        assert!(sat(&maven_decl(&["[1.0,)"]), "1.0"));
        assert!(sat(&maven_decl(&["[1.0,)"]), "99.0"));
        assert!(unsat(&maven_decl(&["[1.0,)"]), "0.9"));
        assert!(sat(&maven_decl(&["[1.0,]"]), "5.0"));
        assert!(unsat(&maven_decl(&["(1.0,)"]), "1.0"));
        assert!(sat(&maven_decl(&["(1.0,)"]), "1.5"));
        assert!(sat(&maven_decl(&["(,2.0]"]), "1.0"));
        assert!(sat(&maven_decl(&["(,2.0]"]), "2.0"));
        assert!(unsat(&maven_decl(&["(,2.0]"]), "2.1"));
        assert!(sat(&maven_decl(&["(,2.0)"]), "1.9"));
        assert!(unsat(&maven_decl(&["(,2.0)"]), "2.0"));
    }

    #[test]
    fn strict_maven_exact_and_bare_minimum() {
        assert!(sat(&maven_decl(&["[1.0]"]), "1.0"));
        assert!(unsat(&maven_decl(&["[1.0]"]), "1.1"));
        assert!(sat(&maven_decl(&["1.0"]), "1.0"));
        assert!(sat(&maven_decl(&["1.0"]), "2.0"));
        assert!(unsat(&maven_decl(&["1.0"]), "0.9"));
    }

    #[test]
    fn strict_maven_comma_union() {
        let decl = maven_decl(&["[1.0,2.0),[3.0,4.0)"]);
        assert!(sat(&decl, "1.5"));
        assert!(unsat(&decl, "2.5"));
        assert!(sat(&decl, "3.5"));
        assert!(unsat(&decl, "4.0"));
        assert!(sat(&maven_decl(&["[1.0,2.0],[3.0,4.0]"]), "4.0"));
    }

    #[test]
    fn strict_maven_malformed_ranges_unsupported() {
        assert!(unsupported(&maven_decl(&["[]"]), "1.0"));
        assert!(unsupported(&maven_decl(&["()"]), "1.0"));
        assert!(unsupported(&maven_decl(&["[1.0"]), "1.0"));
        assert!(unsupported(&maven_decl(&["1.0)"]), "1.0"));
        assert!(unsupported(&maven_decl(&["(1.0)"]), "1.0"));
        assert!(unsupported(&maven_decl(&["[1,2,3]"]), "1.0"));
        assert!(unsupported(&maven_decl(&["1.0,"]), "1.0"));
        assert!(unsupported(&maven_decl(&["[1.0,2.0) [3.0,4.0)"]), "3.5"));
        assert!(unsupported(&maven_decl(&["*"]), "1.0"));
        assert!(unsupported(&maven_decl(&["[1.0, 2.0"]), "1.5"));
    }

    #[test]
    fn strict_maven_non_semver_bounds_equality_only() {
        let lower = maven_decl(&["[MC1.20.1,)"]);
        assert!(sat(&lower, "MC1.20.1"));
        assert!(unsat(&lower, "MC1.20.2"));
        assert!(unsat(&maven_decl(&["(MC1.20.1,)"]), "MC1.20.1"));
        assert!(unsat(&maven_decl(&["[1.0,2.0)"]), "MC1.20.1"));
    }

    #[test]
    fn strict_requirement_uses_grammar_of_decl() {
        // Identical range strings mean different things per grammar.
        let fabric = fabric_decl(&["1.0"]);
        let maven = maven_decl(&["1.0"]);
        assert!(unsat(&fabric, "1.5"));
        assert!(sat(&maven, "1.5"));
        assert!(sat(&fabric, "1.0"));
        assert!(sat(&maven, "1.0"));
    }

    #[test]
    fn strict_fabric_matches_upstream_parse_case_list() {
        // Cases from the user plan exercised end-to-end through the public API.
        let cases: &[(&str, &str, bool)] = &[
            ("1.0", "1.0", true),
            ("=1.0", "1.0", true),
            ("1.0", "1.0.1", false),
            (">=1.0", "1.0", true),
            (">=1.0", "0.5", false),
            ("<=1.0", "1.0", true),
            ("<=1.0", "1.5", false),
            (">1.0", "1.0", false),
            ("<1.0", "1.0", false),
            ("~1.2", "1.2.1", true),
            ("~1.2", "1.3.0", false),
            ("^1.2", "1.8", true),
            ("^1.2", "2.0", false),
            ("1.x", "1.4", true),
            ("1.x", "2.0", false),
            ("1.2.x", "1.2.9", true),
            ("1.2.x", "1.3.0", false),
            ("*", "9.9", true),
            (">=1.0 <2.0", "1.5", true),
            (">=1.0 <2.0", "0.5", false),
            ("1.0.0-alpha", "1.0.0-alpha", true),
            ("1.0.0-alpha", "1.0.0-beta", false),
            ("<1.0.0", "1.0.0-alpha", true),
            ("1.0.0+build.1", "1.0.0+build.2", true),
            ("MC1.20.1", "MC1.20.1", true),
            ("MC1.20.1", "MC1.20.2", false),
        ];
        for (range, provided, expected) in cases {
            let decl = fabric_decl(&[range]);
            assert_eq!(
                sat(&decl, provided),
                *expected,
                "range '{range}' vs '{provided}'"
            );
        }
    }
}
