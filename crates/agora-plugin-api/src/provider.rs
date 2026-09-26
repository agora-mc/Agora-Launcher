//! Content providers: the vocabulary every content source speaks.
//!
//! A *provider* is anything that can answer "what content exists?" — Agora's
//! built-in Modrinth and Technic integrations, and any community plugin that
//! contributes a [`ProviderContribution`]. The types here are the whole
//! conversation between a provider and the launcher: a search, a project, its
//! versions, and — when the user decides to install something — an
//! [`InstallPlan`] describing *what* to fetch.
//!
//! The dividing line is the point of the design:
//!
//! > Providers decide what content is available. Agora decides how that
//! > content is safely installed.
//!
//! A provider never touches the filesystem, an instance, or the database. It
//! returns data. Core validates that data (here, deterministically, before any
//! byte is fetched), downloads through its own network policy, verifies hashes,
//! snapshots the instance, and records where everything came from.
//!
//! # What a hash does and does not prove
//!
//! A provider supplies both the URL and the hash it expects. A matching hash
//! therefore proves the bytes are the ones the provider meant — it does **not**
//! prove the provider is trustworthy. Trust comes from the user choosing to
//! enable that provider, and every artifact it installs carries the provider's
//! id so that choice stays visible afterwards. [`Integrity`] captures the part a
//! hash *can* speak to: whether a strong digest was published at all.
//!
//! Every limit in this module applies identically to Agora's own providers and
//! to community ones. There is no field a built-in provider can set that a
//! plugin cannot.

use crate::error::{PluginError, PluginErrorCode, PluginResult};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Content types a provider may declare, in the launcher's own words.
///
/// `pack` is a modpack: installing one creates a new instance. `server` is a
/// server listing and is browse-only — no install plan may carry it. Everything
/// else installs into an existing instance.
pub const CONTENT_TYPES: &[&str] = &[
    "mod",
    "pack",
    "resourcepack",
    "shader",
    "datapack",
    "server",
];

/// Most results one search page may return.
pub const MAX_PAGE_SIZE: usize = 50;
/// Most filters one provider may declare.
pub const MAX_FILTERS: usize = 16;
/// Most options one filter may offer.
pub const MAX_FILTER_OPTIONS: usize = 256;
/// Most versions one listing may return.
pub const MAX_VERSIONS: usize = 500;
/// Most dependencies one version may declare.
pub const MAX_DEPENDENCIES: usize = 100;
/// Most files one pack plan may contain.
pub const MAX_PACK_FILES: usize = 2_000;
/// Longest free-text field (descriptions, changelogs, project bodies).
pub const MAX_TEXT: usize = 100_000;
/// Longest identifier, title or URL.
pub const MAX_SHORT: usize = 2_048;

/// Top-level folders a pack may place files in.
///
/// A whitelist, deliberately, and the same one Agora's override sanitiser
/// uses plus `mods/`. A pack that wants to write somewhere else — the instance
/// root, `saves/`, a launcher profile — is refused rather than trusted,
/// because a provider must never be able to shape files outside the content
/// an instance is expected to hold.
///
/// `kubejs/` is not inert: the KubeJS mod executes the scripts it finds there.
/// It is allowed because it is an ordinary part of how packs are built, and
/// the user's decision to trust the provider is the control for it — exactly
/// as for a Modrinth `.mrpack`.
pub const PACK_FILE_ROOTS: &[&str] = &[
    "mods",
    "config",
    "defaultconfigs",
    "resourcepacks",
    "shaderpacks",
    "datapacks",
    "kubejs",
];

/// Extensions no pack file may have, in any folder. Matches the override
/// sanitiser's list, minus `.jar`, which is permitted under `mods/` only.
pub const BANNED_EXTENSIONS: &[&str] = &[
    ".class", ".exe", ".bat", ".cmd", ".sh", ".ps1", ".dll", ".so", ".dylib", ".msi", ".dmg",
];

// ---------------------------------------------------------------------------
// Manifest contribution
// ---------------------------------------------------------------------------

/// A content source a plugin contributes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderContribution {
    pub id: String,
    /// Shown in Browse and Settings. Should name the *source* ("CurseForge"),
    /// not the plugin.
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Which of [`CONTENT_TYPES`] this provider can return.
    pub content_types: Vec<String>,
    /// Provider-specific filters, rendered by the host.
    #[serde(default)]
    pub filters: Vec<FilterDefinition>,
    /// Orderings this provider can honour. Browse asks for the closest one.
    #[serde(default = "default_sorts")]
    pub sorts: Vec<ProviderSort>,
    /// Whether `search` honours `offset`. A provider that cannot paginate is
    /// asked once per query and drained locally.
    #[serde(default = "default_true")]
    pub paginates: bool,
    /// Categories offered in Browse's category picker. The chosen id is sent
    /// back as `SearchRequest::category`.
    #[serde(default)]
    pub categories: Vec<CategoryDefinition>,
    /// How this provider's popularity numbers compare with everyone else's,
    /// so Browse can rank its results fairly beside other sources.
    #[serde(default)]
    pub ranking: RankingProfile,
    pub exports: ProviderExports,
}

/// One entry in Browse's category picker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CategoryDefinition {
    pub id: String,
    pub label: String,
    /// Content types this category applies to. Empty means all of the
    /// provider's content types.
    #[serde(default)]
    pub content_types: Vec<String>,
}

/// Where a provider's popularity signals saturate.
///
/// Browse merges several sources into one ranked list. A download on one
/// site is not worth the same as a download on another — a site with a
/// thousand users would otherwise always lose to one with a million — so each
/// provider says what "as popular as it gets" looks like on its own site, and
/// the ranker scales against that. Curated content keeps its own band above
/// every provider regardless.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RankingProfile {
    /// Downloads (or installs) at which popularity is considered maximal.
    pub downloads_ceiling: u64,
    /// Follows, likes or ratings at which endorsement is considered maximal.
    pub endorsements_ceiling: u64,
    /// Category ids marking libraries and APIs, which rank lower: they are
    /// popular because other things depend on them, not because people seek
    /// them out.
    #[serde(default)]
    pub library_categories: Vec<String>,
}

impl RankingProfile {
    /// Smallest ceilings a provider may declare. Lower would saturate almost
    /// everything and give every result the maximum uncurated score.
    pub const MIN_DOWNLOADS_CEILING: u64 = 10_000;
    pub const MIN_ENDORSEMENTS_CEILING: u64 = 100;

    pub fn validate(&self) -> PluginResult<()> {
        if self.downloads_ceiling < Self::MIN_DOWNLOADS_CEILING
            || self.endorsements_ceiling < Self::MIN_ENDORSEMENTS_CEILING
        {
            return Err(PluginError::invalid_manifest(format!(
                "ranking ceilings must be at least {} downloads and {} endorsements",
                Self::MIN_DOWNLOADS_CEILING,
                Self::MIN_ENDORSEMENTS_CEILING
            )));
        }
        if self.library_categories.len() > 32 {
            return Err(PluginError::invalid_manifest(
                "ranking may name at most 32 library categories",
            ));
        }
        Ok(())
    }

    pub fn is_library(&self, categories: &[String]) -> bool {
        categories.iter().any(|category| {
            self.library_categories
                .iter()
                .any(|library| library.eq_ignore_ascii_case(category))
        })
    }
}

impl Default for RankingProfile {
    /// Calibrated against Modrinth, the largest source Agora has measured:
    /// ~250M downloads and ~50k follows for the most popular projects.
    fn default() -> Self {
        Self {
            downloads_ceiling: 250_000_000,
            endorsements_ceiling: 50_000,
            library_categories: vec!["library".into(), "api".into()],
        }
    }
}

fn default_true() -> bool {
    true
}

fn default_sorts() -> Vec<ProviderSort> {
    vec![ProviderSort::Relevance]
}

/// The exported functions the host calls.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderExports {
    /// `(SearchRequest) -> SearchResponse`
    pub search: String,
    /// `(ProjectRequest) -> ProjectDetail`. Optional: without it, the detail
    /// view is built from the search result.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    /// `(VersionsRequest) -> VersionsResponse`
    pub versions: String,
    /// `(ResolveRequest) -> InstallPlan`
    pub resolve: String,
}

impl ProviderContribution {
    pub fn validate(&self) -> PluginResult<()> {
        short("provider title", &self.title)?;
        if self.title.trim().is_empty() {
            return Err(PluginError::invalid_manifest(format!(
                "provider `{}` needs a title",
                self.id
            )));
        }
        if let Some(description) = &self.description {
            text("provider description", description)?;
        }
        if self.content_types.is_empty() {
            return Err(PluginError::invalid_manifest(format!(
                "provider `{}` declares no content types",
                self.id
            )));
        }
        for content_type in &self.content_types {
            validate_content_type(content_type).map_err(|_| {
                PluginError::invalid_manifest(format!(
                    "provider `{}` declares unknown content type `{content_type}`; \
                     expected one of {}",
                    self.id,
                    CONTENT_TYPES.join(", ")
                ))
            })?;
        }
        if self.filters.len() > MAX_FILTERS {
            return Err(PluginError::invalid_manifest(format!(
                "provider `{}` declares {} filters; the limit is {MAX_FILTERS}",
                self.id,
                self.filters.len()
            )));
        }
        let mut seen = std::collections::BTreeSet::new();
        for filter in &self.filters {
            filter.validate()?;
            if !seen.insert(filter.id.as_str()) {
                return Err(PluginError::new(
                    PluginErrorCode::DuplicateContribution,
                    format!(
                        "provider `{}` declares filter `{}` twice",
                        self.id, filter.id
                    ),
                ));
            }
        }
        if self.categories.len() > MAX_FILTER_OPTIONS {
            return Err(PluginError::invalid_manifest(format!(
                "provider `{}` declares {} categories; the limit is {MAX_FILTER_OPTIONS}",
                self.id,
                self.categories.len()
            )));
        }
        for category in &self.categories {
            short("category id", &category.id).map_err(to_manifest)?;
            short("category label", &category.label).map_err(to_manifest)?;
            for content_type in &category.content_types {
                validate_content_type(content_type).map_err(to_manifest)?;
            }
        }
        self.ranking.validate()?;
        if self.sorts.is_empty() {
            return Err(PluginError::invalid_manifest(format!(
                "provider `{}` must support at least one sort",
                self.id
            )));
        }
        for export in [
            Some(&self.exports.search),
            self.exports.project.as_ref(),
            Some(&self.exports.versions),
            Some(&self.exports.resolve),
        ]
        .into_iter()
        .flatten()
        {
            validate_export_name(export)?;
        }
        Ok(())
    }
}

/// Orderings a search can ask for. The host maps its own sort onto the
/// closest one the provider declared.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(rename_all = "kebab-case")]
pub enum ProviderSort {
    #[default]
    Relevance,
    Downloads,
    Follows,
    Newest,
    Updated,
}

/// A provider-specific filter, rendered by the host as a picker.
///
/// This is how a source exposes something only it has — Modrinth's
/// environment tags, a Technic-style "Solder only" switch, a CurseForge class —
/// without Agora growing an API named after that source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FilterDefinition {
    pub id: String,
    pub title: String,
    /// Whether more than one option may be chosen at once.
    #[serde(default)]
    pub multiple: bool,
    pub options: Vec<FilterOption>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FilterOption {
    pub value: String,
    pub label: String,
}

impl FilterDefinition {
    pub fn validate(&self) -> PluginResult<()> {
        validate_identifier("filter id", &self.id)?;
        short("filter title", &self.title)?;
        if self.options.is_empty() || self.options.len() > MAX_FILTER_OPTIONS {
            return Err(PluginError::invalid_manifest(format!(
                "filter `{}` must offer between 1 and {MAX_FILTER_OPTIONS} options",
                self.id
            )));
        }
        for option in &self.options {
            short("filter option value", &option.value)?;
            short("filter option label", &option.label)?;
        }
        Ok(())
    }

    /// Whether a chosen set of values is acceptable for this filter.
    pub fn accepts(&self, values: &[String]) -> bool {
        (self.multiple || values.len() <= 1)
            && values
                .iter()
                .all(|value| self.options.iter().any(|option| &option.value == value))
    }
}

// ---------------------------------------------------------------------------
// Host → provider requests
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchRequest {
    pub query: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minecraft_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loader: Option<String>,
    /// Category chosen in Browse's shared category picker, if any. A provider
    /// that does not recognise it should ignore it rather than return nothing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    pub sort: ProviderSort,
    pub offset: u32,
    pub limit: u32,
    /// Values for the provider's own [`FilterDefinition`]s, by filter id.
    #[serde(default)]
    pub filters: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectRequest {
    pub project_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionsRequest {
    pub project_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minecraft_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loader: Option<String>,
}

/// Asks a provider to turn a project (and optionally a specific version) into
/// something installable for a given target.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolveRequest {
    pub project_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version_id: Option<String>,
    /// The instance's Minecraft version. Empty when installing a pack, which
    /// creates its own instance.
    #[serde(default)]
    pub minecraft_version: String,
    /// The instance's loader (`fabric`, `forge`, ...). Empty for a pack.
    #[serde(default)]
    pub loader: String,
}

// ---------------------------------------------------------------------------
// Provider → host responses
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResponse {
    pub items: Vec<ProjectSummary>,
    /// Total hits, when the provider knows it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total: Option<u64>,
    /// Whether asking again with a larger offset would return more.
    #[serde(default)]
    pub has_more: bool,
}

/// One project as it appears in a result list.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSummary {
    /// Stable id *within this provider*. The host namespaces it.
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_url: Option<String>,
    pub content_type: String,
    #[serde(default)]
    pub categories: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub downloads: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub follows: Option<u64>,
    /// The project's page on the provider's own site.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_url: Option<String>,
    #[serde(default)]
    pub minecraft_versions: Vec<String>,
    #[serde(default)]
    pub loaders: Vec<String>,
    /// A wide image for the card, when the provider has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hero_image_url: Option<String>,
    /// Set when installing this would involve files with no integrity
    /// information at all (a bare archive with no digest, say). Agora then
    /// shows it only to users who allowed low security downloads, and a
    /// provider need not filter for them itself.
    #[serde(default, skip_serializing_if = "is_false")]
    pub low_security: bool,
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectDetail {
    pub project: ProjectSummary,
    /// Long description. Rendered as plain text or Markdown by the host —
    /// never as HTML.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    #[serde(default)]
    pub gallery: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated: Option<String>,
    #[serde(default)]
    pub links: Vec<ProjectLink>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectLink {
    pub label: String,
    pub url: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionsResponse {
    pub versions: Vec<ProjectVersion>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ReleaseChannel {
    #[default]
    Release,
    Beta,
    Alpha,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectVersion {
    pub id: String,
    pub name: String,
    pub version_number: String,
    #[serde(default)]
    pub channel: ReleaseChannel,
    #[serde(default)]
    pub minecraft_versions: Vec<String>,
    #[serde(default)]
    pub loaders: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub published: Option<String>,
    #[serde(default)]
    pub dependencies: Vec<ProviderDependency>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub changelog: Option<String>,
}

/// How one project relates to another. Ids refer to the *same* provider.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderDependency {
    pub project_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version_id: Option<String>,
    pub kind: DependencyKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DependencyKind {
    Required,
    Optional,
    Incompatible,
    /// Shipped inside the dependent's own file; nothing to install.
    Embedded,
}

// ---------------------------------------------------------------------------
// Install plans
// ---------------------------------------------------------------------------

/// What a provider wants installed. Data only — core performs every step.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum InstallPlan {
    /// One file into an existing instance, plus the dependencies it declares.
    File(FilePlan),
    /// A whole modpack, which becomes a new instance.
    Pack(PackPlan),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FilePlan {
    pub version_id: String,
    pub version_number: String,
    /// One of [`CONTENT_TYPES`] except `pack`.
    pub content_type: String,
    pub file: PlannedDownload,
    #[serde(default)]
    pub dependencies: Vec<ProviderDependency>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PackPlan {
    pub name: String,
    pub version_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version_number: Option<String>,
    pub minecraft_version: String,
    /// Empty for vanilla.
    #[serde(default)]
    pub loader: String,
    #[serde(default)]
    pub loader_version: String,
    #[serde(default)]
    pub files: Vec<PackFile>,
    /// An archive whose contents are laid over the instance after `files` —
    /// configs, scripts and the like. Extracted by the host's sanitiser under
    /// the same [`PACK_FILE_ROOTS`] rule as individual files.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overrides: Option<PlannedDownload>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PackFile {
    /// Instance-relative, `/`-separated, under one of [`PACK_FILE_ROOTS`].
    pub path: String,
    pub download: PlannedDownload,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlannedDownload {
    pub url: String,
    /// A single path segment.
    pub filename: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    #[serde(default)]
    pub hashes: FileHashes,
}

/// Digests the provider published for a file. Every one supplied is checked.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FileHashes {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha512: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha1: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub md5: Option<String>,
}

/// How much a published digest can say about a file.
///
/// Ordered weakest first so that "the weakest file in a plan" is `min()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Integrity {
    /// No digest at all. Agora cannot tell a corrupted or swapped file apart
    /// from the real one.
    None,
    /// Only MD5 or SHA-1, both of which can be forged deliberately. Fine for
    /// catching a truncated download; not a guarantee against tampering.
    Weak,
    /// SHA-256 or SHA-512.
    Strong,
}

impl FileHashes {
    pub fn integrity(&self) -> Integrity {
        if self.sha512.is_some() || self.sha256.is_some() {
            Integrity::Strong
        } else if self.sha1.is_some() || self.md5.is_some() {
            Integrity::Weak
        } else {
            Integrity::None
        }
    }

    fn validate(&self, label: &str) -> PluginResult<()> {
        for (name, value, len) in [
            ("sha512", &self.sha512, 128),
            ("sha256", &self.sha256, 64),
            ("sha1", &self.sha1, 40),
            ("md5", &self.md5, 32),
        ] {
            if let Some(value) = value {
                if value.len() != len || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
                    return Err(invalid_response(format!(
                        "{label}: `{name}` must be {len} hex characters"
                    )));
                }
            }
        }
        Ok(())
    }
}

impl InstallPlan {
    /// The weakest integrity of any file the plan would fetch.
    pub fn integrity(&self) -> Integrity {
        match self {
            InstallPlan::File(plan) => plan.file.hashes.integrity(),
            InstallPlan::Pack(plan) => plan
                .files
                .iter()
                .map(|file| file.download.hashes.integrity())
                .chain(plan.overrides.iter().map(|o| o.hashes.integrity()))
                .min()
                .unwrap_or(Integrity::Strong),
        }
    }

    /// Every URL the plan would fetch.
    pub fn urls(&self) -> Vec<&str> {
        match self {
            InstallPlan::File(plan) => vec![plan.file.url.as_str()],
            InstallPlan::Pack(plan) => plan
                .files
                .iter()
                .map(|file| file.download.url.as_str())
                .chain(plan.overrides.iter().map(|o| o.url.as_str()))
                .collect(),
        }
    }

    /// Structural validation. Does not decide whether the plan is *permitted*
    /// — hosts and integrity are judged by core against the user's settings.
    pub fn validate(&self) -> PluginResult<()> {
        match self {
            InstallPlan::File(plan) => {
                short("versionId", &plan.version_id)?;
                short("versionNumber", &plan.version_number)?;
                validate_content_type(&plan.content_type)?;
                if plan.content_type == "pack" {
                    return Err(invalid_response(
                        "a `file` plan cannot install a pack; return a `pack` plan",
                    ));
                }
                if plan.content_type == "server" {
                    return Err(invalid_response(
                        "servers are browse-only; there is nothing to install",
                    ));
                }
                plan.file.validate("file")?;
                validate_dependencies(&plan.dependencies)
            }
            InstallPlan::Pack(plan) => {
                short("pack name", &plan.name)?;
                if plan.name.trim().is_empty() {
                    return Err(invalid_response("a pack plan needs a name"));
                }
                short("versionId", &plan.version_id)?;
                short("minecraftVersion", &plan.minecraft_version)?;
                if plan.minecraft_version.trim().is_empty() {
                    return Err(invalid_response(
                        "a pack plan must name a Minecraft version",
                    ));
                }
                short("loader", &plan.loader)?;
                short("loaderVersion", &plan.loader_version)?;
                if plan.files.len() > MAX_PACK_FILES {
                    return Err(invalid_response(format!(
                        "a pack plan may list at most {MAX_PACK_FILES} files"
                    )));
                }
                let mut seen = std::collections::BTreeSet::new();
                for file in &plan.files {
                    validate_pack_path(&file.path)?;
                    file.download.validate(&file.path)?;
                    if !seen.insert(file.path.to_ascii_lowercase()) {
                        return Err(invalid_response(format!(
                            "pack file `{}` is listed twice",
                            file.path
                        )));
                    }
                }
                if let Some(overrides) = &plan.overrides {
                    overrides.validate("overrides")?;
                }
                Ok(())
            }
        }
    }
}

impl PlannedDownload {
    fn validate(&self, label: &str) -> PluginResult<()> {
        short("url", &self.url)?;
        if !(self.url.starts_with("https://") || self.url.starts_with("http://")) {
            return Err(invalid_response(format!(
                "{label}: download URL must be http(s)"
            )));
        }
        validate_filename(&self.filename)?;
        self.hashes.validate(label)
    }
}

// ---------------------------------------------------------------------------
// Response validation
// ---------------------------------------------------------------------------

impl SearchResponse {
    pub fn validate(&self) -> PluginResult<()> {
        if self.items.len() > MAX_PAGE_SIZE {
            return Err(invalid_response(format!(
                "a search page may return at most {MAX_PAGE_SIZE} items"
            )));
        }
        for item in &self.items {
            item.validate()?;
        }
        Ok(())
    }
}

impl ProjectSummary {
    pub fn validate(&self) -> PluginResult<()> {
        validate_project_id(&self.id)?;
        short("title", &self.title)?;
        if let Some(description) = &self.description {
            text("description", description)?;
        }
        for (label, value) in [
            ("author", &self.author),
            ("iconUrl", &self.icon_url),
            ("pageUrl", &self.page_url),
            ("heroImageUrl", &self.hero_image_url),
        ] {
            if let Some(value) = value {
                short(label, value)?;
            }
        }
        validate_content_type(&self.content_type)?;
        bounded_list("categories", &self.categories)?;
        bounded_list("minecraftVersions", &self.minecraft_versions)?;
        bounded_list("loaders", &self.loaders)
    }
}

impl ProjectDetail {
    pub fn validate(&self) -> PluginResult<()> {
        self.project.validate()?;
        if let Some(body) = &self.body {
            text("body", body)?;
        }
        bounded_list("gallery", &self.gallery)?;
        if self.links.len() > 32 {
            return Err(invalid_response("a project may list at most 32 links"));
        }
        for link in &self.links {
            short("link label", &link.label)?;
            short("link url", &link.url)?;
        }
        Ok(())
    }
}

impl VersionsResponse {
    pub fn validate(&self) -> PluginResult<()> {
        if self.versions.len() > MAX_VERSIONS {
            return Err(invalid_response(format!(
                "a version listing may return at most {MAX_VERSIONS} versions"
            )));
        }
        for version in &self.versions {
            short("version id", &version.id)?;
            short("version name", &version.name)?;
            short("versionNumber", &version.version_number)?;
            bounded_list("minecraftVersions", &version.minecraft_versions)?;
            bounded_list("loaders", &version.loaders)?;
            if let Some(changelog) = &version.changelog {
                text("changelog", changelog)?;
            }
            validate_dependencies(&version.dependencies)?;
        }
        Ok(())
    }
}

fn validate_dependencies(dependencies: &[ProviderDependency]) -> PluginResult<()> {
    if dependencies.len() > MAX_DEPENDENCIES {
        return Err(invalid_response(format!(
            "a version may declare at most {MAX_DEPENDENCIES} dependencies"
        )));
    }
    for dependency in dependencies {
        validate_project_id(&dependency.project_id)?;
        if let Some(version) = &dependency.version_id {
            short("dependency versionId", version)?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Primitive rules
// ---------------------------------------------------------------------------

/// A contract rule broken in a manifest rather than a response.
fn to_manifest(error: PluginError) -> PluginError {
    PluginError::invalid_manifest(error.message)
}

fn invalid_response(message: impl Into<String>) -> PluginError {
    PluginError::new(PluginErrorCode::InvalidResponse, message)
}

fn short(label: &str, value: &str) -> PluginResult<()> {
    if value.len() > MAX_SHORT || value.chars().any(char::is_control) {
        return Err(invalid_response(format!(
            "`{label}` must be at most {MAX_SHORT} bytes with no control characters"
        )));
    }
    Ok(())
}

fn text(label: &str, value: &str) -> PluginResult<()> {
    if value.len() > MAX_TEXT {
        return Err(invalid_response(format!(
            "`{label}` must be at most {MAX_TEXT} bytes"
        )));
    }
    Ok(())
}

fn bounded_list(label: &str, values: &[String]) -> PluginResult<()> {
    if values.len() > 256 {
        return Err(invalid_response(format!(
            "`{label}` may hold at most 256 entries"
        )));
    }
    values.iter().try_for_each(|value| short(label, value))
}

fn validate_content_type(value: &str) -> PluginResult<()> {
    if CONTENT_TYPES.contains(&value) {
        Ok(())
    } else {
        Err(invalid_response(format!(
            "`{value}` is not a content type; expected one of {}",
            CONTENT_TYPES.join(", ")
        )))
    }
}

/// A provider's own project id. Anything printable and bounded; the host
/// never builds a path or a URL from it.
pub fn validate_project_id(value: &str) -> PluginResult<()> {
    if value.trim().is_empty() || value.len() > 256 || value.chars().any(char::is_control) {
        return Err(invalid_response(format!(
            "`{value}` is not a usable project id"
        )));
    }
    Ok(())
}

fn validate_identifier(label: &str, value: &str) -> PluginResult<()> {
    let ok = !value.is_empty()
        && value.len() <= 64
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if ok {
        Ok(())
    } else {
        Err(PluginError::invalid_manifest(format!(
            "{label} `{value}` may use only letters, digits, `-` and `_`"
        )))
    }
}

fn validate_export_name(value: &str) -> PluginResult<()> {
    let ok = !value.is_empty()
        && value.len() <= 128
        && value
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_' || c == '$')
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$');
    if ok {
        Ok(())
    } else {
        Err(PluginError::invalid_manifest(format!(
            "`{value}` is not a JavaScript export name"
        )))
    }
}

/// A single, ordinary file name.
pub fn validate_filename(value: &str) -> PluginResult<()> {
    let ok = !value.is_empty()
        && value.len() <= 255
        && value != "."
        && value != ".."
        && !value.starts_with('.')
        && !value.chars().any(|c| {
            c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
        });
    if ok {
        Ok(())
    } else {
        Err(invalid_response(format!(
            "`{value}` is not a safe file name"
        )))
    }
}

/// An instance-relative path a pack may write to.
pub fn validate_pack_path(value: &str) -> PluginResult<()> {
    if value.len() > 512 || value.contains('\\') || value.starts_with('/') {
        return Err(invalid_response(format!(
            "pack path `{value}` must be relative and `/`-separated"
        )));
    }
    let segments: Vec<&str> = value.split('/').collect();
    if segments.len() < 2 {
        return Err(invalid_response(format!(
            "pack path `{value}` must be inside a folder such as `mods/`"
        )));
    }
    if !PACK_FILE_ROOTS.contains(&segments[0]) {
        return Err(invalid_response(format!(
            "pack path `{value}` is outside the folders a pack may write to ({})",
            PACK_FILE_ROOTS.join(", ")
        )));
    }
    for segment in &segments[1..segments.len() - 1] {
        if segment.is_empty() || *segment == "." || *segment == ".." || segment.contains(':') {
            return Err(invalid_response(format!("pack path `{value}` is unsafe")));
        }
    }
    let name = segments[segments.len() - 1];
    validate_filename(name)?;
    let lower = name.to_ascii_lowercase();
    if BANNED_EXTENSIONS.iter().any(|ext| lower.ends_with(ext)) {
        return Err(invalid_response(format!(
            "pack path `{value}` has a file type a pack may not install"
        )));
    }
    if lower.ends_with(".jar") && segments[0] != "mods" {
        return Err(invalid_response(format!(
            "pack path `{value}`: `.jar` files belong in `mods/`"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn download(hashes: FileHashes) -> PlannedDownload {
        PlannedDownload {
            url: "https://cdn.example.com/a.jar".into(),
            filename: "a.jar".into(),
            size: Some(10),
            hashes,
        }
    }

    fn sha256() -> FileHashes {
        FileHashes {
            sha256: Some("a".repeat(64)),
            ..Default::default()
        }
    }

    #[test]
    fn contribution_parses_with_defaults() {
        let contribution: ProviderContribution = serde_json::from_value(json!({
            "id": "example",
            "title": "Example",
            "contentTypes": ["mod"],
            "exports": { "search": "search", "versions": "versions", "resolve": "resolve" }
        }))
        .unwrap();
        contribution.validate().unwrap();
        assert!(contribution.paginates);
        assert_eq!(contribution.ranking, RankingProfile::default());
        assert_eq!(contribution.sorts, vec![ProviderSort::Relevance]);
    }

    #[test]
    fn contribution_rejects_an_unknown_content_type() {
        let contribution: ProviderContribution = serde_json::from_value(json!({
            "id": "example",
            "title": "Example",
            "contentTypes": ["skin"],
            "exports": { "search": "search", "versions": "versions", "resolve": "resolve" }
        }))
        .unwrap();
        assert!(contribution.validate().is_err());
    }

    #[test]
    fn a_ranking_that_would_saturate_everything_is_refused() {
        let ranking = RankingProfile {
            downloads_ceiling: 10,
            endorsements_ceiling: 1,
            library_categories: vec![],
        };
        assert!(ranking.validate().is_err());
        assert!(RankingProfile::default().is_library(&["Library".to_string()]));
    }

    #[test]
    fn filters_accept_only_declared_options() {
        let filter = FilterDefinition {
            id: "side".into(),
            title: "Side".into(),
            multiple: false,
            options: vec![
                FilterOption {
                    value: "client".into(),
                    label: "Client".into(),
                },
                FilterOption {
                    value: "server".into(),
                    label: "Server".into(),
                },
            ],
        };
        assert!(filter.accepts(&["client".into()]));
        assert!(!filter.accepts(&["client".into(), "server".into()]));
        assert!(!filter.accepts(&["both".into()]));
    }

    #[test]
    fn integrity_is_the_weakest_file_in_the_plan() {
        let plan = InstallPlan::Pack(PackPlan {
            name: "Pack".into(),
            version_id: "1".into(),
            version_number: None,
            minecraft_version: "1.20.1".into(),
            loader: "fabric".into(),
            loader_version: "0.15.0".into(),
            files: vec![
                PackFile {
                    path: "mods/a.jar".into(),
                    download: download(sha256()),
                },
                PackFile {
                    path: "mods/b.jar".into(),
                    download: download(FileHashes {
                        md5: Some("b".repeat(32)),
                        ..Default::default()
                    }),
                },
            ],
            overrides: None,
        });
        plan.validate().unwrap();
        assert_eq!(plan.integrity(), Integrity::Weak);
    }

    #[test]
    fn pack_paths_are_confined_to_content_folders() {
        for bad in [
            "mods",
            "../mods/a.jar",
            "mods/../../a.jar",
            "/mods/a.jar",
            "mods\\a.jar",
            "saves/world/level.dat",
            "options.txt",
            "mods/.hidden",
            "C:/mods/a.jar",
            "config/evil.jar",
            "mods/run.exe",
            "kubejs/x.sh",
            "scripts/a.zs",
        ] {
            assert!(validate_pack_path(bad).is_err(), "{bad} must be refused");
        }
        for good in [
            "mods/a.jar",
            "config/sub/dir/x.toml",
            "kubejs/server_scripts/a.js",
        ] {
            validate_pack_path(good).unwrap();
        }
    }

    #[test]
    fn a_file_plan_cannot_smuggle_a_pack() {
        let plan = InstallPlan::File(FilePlan {
            version_id: "1".into(),
            version_number: "1.0".into(),
            content_type: "pack".into(),
            file: download(sha256()),
            dependencies: vec![],
        });
        assert!(plan.validate().is_err());
    }

    #[test]
    fn malformed_hashes_are_refused_not_ignored() {
        let plan = InstallPlan::File(FilePlan {
            version_id: "1".into(),
            version_number: "1.0".into(),
            content_type: "mod".into(),
            file: download(FileHashes {
                sha256: Some("not-hex".into()),
                ..Default::default()
            }),
            dependencies: vec![],
        });
        assert!(plan.validate().is_err());
    }

    #[test]
    fn install_plans_round_trip_with_their_kind_tag() {
        let json = json!({
            "kind": "file",
            "versionId": "abc",
            "versionNumber": "1.0.0",
            "contentType": "mod",
            "file": {
                "url": "https://cdn.example.com/a.jar",
                "filename": "a.jar",
                "hashes": { "sha512": "c".repeat(128) }
            }
        });
        let plan: InstallPlan = serde_json::from_value(json).unwrap();
        plan.validate().unwrap();
        assert_eq!(plan.integrity(), Integrity::Strong);
    }
}
