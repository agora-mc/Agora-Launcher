//! Content providers: every source of browsable, installable content behind
//! one interface.
//!
//! Agora's own catalog is not a provider — it is the signed registry, and it
//! stays the thing Agora *is*. Everything else is: Modrinth, Technic, and any
//! community plugin that contributes a `contentProviders` entry. Browse, the
//! detail page and the install flow talk to [`ContentProvider`], never to a
//! named source.
//!
//! # The rule this module exists to keep
//!
//! > Providers decide what content is available. Agora decides how that
//! > content is safely installed.
//!
//! A provider returns data — search pages, projects, versions, and an
//! [`InstallPlan`] naming files and their digests. It never downloads, writes,
//! snapshots or records anything. [`authorize_plan`] judges every plan by the
//! same rule whether it came from Agora's own Modrinth code or from a plugin:
//!
//! - a file over HTTPS from a host the provider declared, with SHA-256 or
//!   SHA-512, needs no comment;
//! - an undeclared host, plain HTTP, or only MD5/SHA-1 is **reduced
//!   assurance**: the user is warned and may continue;
//! - a file with no digest at all is **low security** and installs only with
//!   *Allow low security downloads* on.
//!
//! That is Technic's long-standing model (Solder warns, bare zips need the
//! toggle), generalised. Agora warns and asks; it does not decide for the
//! user. Enabling a provider is the user's statement of trust in it, and the
//! provider id is stamped on everything it installs so that statement stays
//! visible.
//!
//! # Official providers are not privileged
//!
//! [`modrinth::ModrinthProvider`] and [`technic::TechnicProvider`] are compiled
//! in, and implement exactly the trait [`plugin::PluginProvider`] implements.
//! There is no method on [`ContentProvider`] a plugin cannot answer. Where the
//! official integrations still have code paths of their own (Modrinth's
//! single-file install, Technic's consent tiers, mrpack import) they are
//! listed in `docs/plugins/providers.md` as migration debt, not hidden.

pub mod plugin;

use crate::ctx::Ctx;
use crate::error::{LauncherError, LauncherResult};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

pub use agora_plugin_api::provider::{
    CategoryDefinition, FilterDefinition, InstallPlan, Integrity, ProjectDetail, ProjectSummary,
    ProviderSort, RankingProfile, ResolveRequest, SearchRequest, SearchResponse, VersionsRequest,
    VersionsResponse,
};

/// Prefix of a Browse/detail item id that belongs to a provider.
pub const ITEM_PREFIX: &str = "provider:";

/// **Allow low security downloads**: content with no integrity information
/// at all. Shared with Technic's bare-zip tier, because it is one question
/// ("install things Agora cannot check at all?") and asking it twice would let
/// the answers drift. The key predates the label and is kept so existing
/// choices carry over.
pub const LOW_SECURITY_SETTING: &str = "allow_unverified_packs";

/// Who ships a provider.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum ProviderOrigin {
    /// Built into Agora and maintained with it.
    Official,
    /// Contributed by an installed plugin.
    Plugin { plugin_id: String },
}

/// Everything the UI needs to know about a provider without calling it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderDescriptor {
    /// `modrinth`, `technic`, or `<plugin-id>/<provider-id>`.
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub origin: ProviderOrigin,
    pub content_types: Vec<String>,
    pub filters: Vec<FilterDefinition>,
    pub sorts: Vec<ProviderSort>,
    pub paginates: bool,
    /// How this provider's popularity numbers compare with other sources'.
    pub ranking: RankingProfile,
    /// Hosts downloads may come from without a warning.
    pub download_hosts: Vec<String>,
    /// Whether the user has this provider switched on.
    pub enabled: bool,
    /// Why an enabled provider cannot be used right now (Lockdown, plugin
    /// network access off). `None` when it can.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unavailable_reason: Option<String>,
}

impl ProviderDescriptor {
    pub fn is_usable(&self) -> bool {
        self.enabled && self.unavailable_reason.is_none()
    }
}

/// A search hit, plus the source's own payload where Agora's older screens
/// still read it.
///
/// `native` is presentation-only and transitional: the legacy Modrinth and
/// Technic cards read their original shapes. Nothing decides *behaviour* from
/// it, and a plugin provider leaves it empty with no loss of function.
#[derive(Debug, Clone)]
pub struct ProviderHit {
    pub summary: ProjectSummary,
    pub native: Option<NativeHit>,
}

#[derive(Clone)]
pub struct NativeHit(pub std::sync::Arc<dyn std::any::Any + Send + Sync>);

impl NativeHit {
    pub fn new<T: std::any::Any + Send + Sync>(value: T) -> Self {
        Self(std::sync::Arc::new(value))
    }

    /// The provider's own result, if it is a `T`.
    pub fn downcast_ref<T: std::any::Any>(&self) -> Option<&T> {
        self.0.downcast_ref::<T>()
    }
}

impl std::fmt::Debug for NativeHit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NativeHit(..)")
    }
}

#[derive(Debug, Clone, Default)]
pub struct ProviderPage {
    pub hits: Vec<ProviderHit>,
    pub total: Option<u64>,
    pub has_more: bool,
}

/// One content source.
#[async_trait]
pub trait ContentProvider: Send + Sync {
    fn descriptor(&self) -> ProviderDescriptor;

    /// Categories for Browse's picker. Most providers declare a fixed list;
    /// one whose categories live on its own site may fetch them.
    async fn categories(&self) -> LauncherResult<Vec<CategoryDefinition>> {
        Ok(Vec::new())
    }

    async fn search(&self, request: SearchRequest) -> LauncherResult<ProviderPage>;

    async fn project(&self, project_id: &str) -> LauncherResult<ProjectDetail>;

    async fn versions(&self, request: VersionsRequest) -> LauncherResult<VersionsResponse>;

    async fn resolve(&self, request: ResolveRequest) -> LauncherResult<InstallPlan>;
}

/// The providers this session knows about.
#[derive(Clone, Default)]
pub struct ProviderRegistry {
    providers: Vec<Arc<dyn ContentProvider>>,
}

impl ProviderRegistry {
    /// Agora's official providers plus every provider an enabled, granted
    /// plugin contributes.
    pub fn new(ctx: &Ctx, plugins: Option<&crate::plugins::PluginService>) -> Self {
        let mut providers: Vec<Arc<dyn ContentProvider>> = ctx.games.builtin_providers(ctx);
        if let Some(service) = plugins {
            providers.extend(plugin::PluginProvider::discover(ctx, service));
        }
        Self { providers }
    }

    /// A registry of exactly these providers. For tests and for adapters that
    /// assemble their own.
    pub fn from_providers(providers: Vec<Arc<dyn ContentProvider>>) -> Self {
        Self { providers }
    }

    pub fn descriptors(&self) -> Vec<ProviderDescriptor> {
        self.providers.iter().map(|p| p.descriptor()).collect()
    }

    pub fn get(&self, provider_id: &str) -> Option<Arc<dyn ContentProvider>> {
        self.providers
            .iter()
            .find(|p| p.descriptor().id == provider_id)
            .cloned()
    }

    /// A provider that exists *and* may be used now, or the reason it may not.
    pub fn usable(&self, provider_id: &str) -> LauncherResult<Arc<dyn ContentProvider>> {
        let provider = self
            .get(provider_id)
            .ok_or_else(|| LauncherError::Generic {
                code: "ERR_PROVIDER_UNKNOWN".into(),
                message: format!("No content provider named `{provider_id}` is installed."),
            })?;
        let descriptor = provider.descriptor();
        if !descriptor.enabled {
            return Err(LauncherError::Generic {
                code: "ERR_PROVIDER_DISABLED".into(),
                message: format!("{} is turned off in Settings.", descriptor.title),
            });
        }
        if let Some(reason) = descriptor.unavailable_reason {
            return Err(LauncherError::Generic {
                code: "ERR_PROVIDER_UNAVAILABLE".into(),
                message: reason,
            });
        }
        Ok(provider)
    }

    /// Every provider that may be searched right now.
    pub fn usable_providers(&self) -> Vec<(ProviderDescriptor, Arc<dyn ContentProvider>)> {
        self.providers
            .iter()
            .map(|p| (p.descriptor(), p.clone()))
            .filter(|(descriptor, _)| descriptor.is_usable())
            .collect()
    }
}

/// Switch a provider on or off.
///
/// The existing Modrinth and Technic toggles stay the user-facing switch for
/// the official providers, stored under `<id>_enabled` as they always were.
/// A plugin provider's switch *is* its plugin's: disabling it disables the
/// plugin rather than hiding the provider behind a second setting that could
/// disagree with the first. Uninstalling stays in plugin management.
pub fn set_enabled(
    ctx: &Ctx,
    registry: &ProviderRegistry,
    plugins: Option<&crate::plugins::PluginService>,
    provider_id: &str,
    enabled: bool,
) -> LauncherResult<()> {
    let descriptor = registry
        .get(provider_id)
        .map(|p| p.descriptor())
        .ok_or_else(|| LauncherError::Generic {
            code: "ERR_PROVIDER_UNKNOWN".into(),
            message: format!("No content provider named `{provider_id}` is installed."),
        })?;
    match descriptor.origin {
        ProviderOrigin::Official => crate::settings::SettingsService::new(ctx.clone()).set(
            &format!("{}_enabled", descriptor.id),
            &serde_json::Value::Bool(enabled),
        ),
        ProviderOrigin::Plugin { plugin_id } => {
            let service = plugins.ok_or_else(|| LauncherError::Generic {
                code: "ERR_PLUGINS_UNAVAILABLE".into(),
                message: "The plugin service is not available.".into(),
            })?;
            let id = agora_plugin_api::manifest::PluginId::parse(&plugin_id).map_err(|e| {
                LauncherError::Generic {
                    code: "ERR_PLUGIN_ID_INVALID".into(),
                    message: e.message,
                }
            })?;
            service.set_enabled(&id, enabled)
        }
    }
}

/// Every usable provider's categories, for Browse's category picker.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderCategories {
    pub provider_id: String,
    pub provider_title: String,
    pub categories: Vec<CategoryDefinition>,
}

/// Ask every usable provider for its categories. A provider that fails is
/// left out rather than failing the picker.
pub async fn categories(registry: &ProviderRegistry) -> Vec<ProviderCategories> {
    let mut all = Vec::new();
    for (descriptor, provider) in registry.usable_providers() {
        if let Ok(categories) = provider.categories().await {
            // An empty `contentTypes` means every type the provider offers;
            // expanded here so the frontend has one rule to apply.
            let categories = categories
                .into_iter()
                .map(|mut category| {
                    if category.content_types.is_empty() {
                        category.content_types = descriptor.content_types.clone();
                    }
                    category
                })
                .collect();
            all.push(ProviderCategories {
                provider_id: descriptor.id,
                provider_title: descriptor.title,
                categories,
            });
        }
    }
    all
}

// ---------------------------------------------------------------------------
// Item ids
// ---------------------------------------------------------------------------

/// `provider:<provider-id>:<project-id>`.
///
/// Provider ids never contain `:` — official ids are fixed words and plugin
/// ids are `[a-z0-9.-]` segments joined by `/` — so the first `:` after the
/// prefix always ends the provider id, and a project id may contain anything.
pub fn item_id(provider_id: &str, project_id: &str) -> String {
    format!("{ITEM_PREFIX}{provider_id}:{project_id}")
}

pub fn parse_item_id(item_id: &str) -> Option<(&str, &str)> {
    let rest = item_id.strip_prefix(ITEM_PREFIX)?;
    let (provider, project) = rest.split_once(':')?;
    (!provider.is_empty() && !project.is_empty()).then_some((provider, project))
}

// ---------------------------------------------------------------------------
// Plan authorization — one rule for every provider
// ---------------------------------------------------------------------------

/// One thing about a plan the user should know before installing it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SecurityNote {
    pub url_host: String,
    pub reason: String,
}

/// Core's verdict on an install plan, in the two tiers the user sees.
///
/// This is the same model Technic has always used, applied to every
/// provider: Solder packs publish MD5 and install after a warning; bare zips
/// publish nothing and appear only with **Allow low security downloads** on.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanAuthorization {
    /// Reduced assurance the user is warned about and may continue past: a
    /// host the provider did not declare, plain HTTP, only MD5/SHA-1.
    pub warnings: Vec<SecurityNote>,
    /// Files with no integrity information at all. These install only with
    /// low security downloads allowed.
    pub low_security: Vec<SecurityNote>,
    /// Pack files outside the usual content folders (`mods/`, `config/`, …),
    /// or `.jar` files outside `mods/`. These install only with **Reduced
    /// security mode** on.
    #[serde(default)]
    pub outside_content_folders: Vec<String>,
}

impl PlanAuthorization {
    /// Nothing to warn about.
    pub fn is_clean(&self) -> bool {
        self.warnings.is_empty()
            && self.low_security.is_empty()
            && self.outside_content_folders.is_empty()
    }

    pub fn is_low_security(&self) -> bool {
        !self.low_security.is_empty()
    }
}

/// Whether `url` is HTTPS on one of `hosts` (or a subdomain of one).
pub fn url_in_scope(url: &str, hosts: &[String]) -> bool {
    let Ok(parsed) = reqwest::Url::parse(url) else {
        return false;
    };
    if parsed.scheme() != "https" || parsed.port().is_some_and(|p| p != 443) {
        return false;
    }
    let Some(host) = parsed.host_str() else {
        return false;
    };
    let host = host.to_ascii_lowercase();
    // `*` reaches anywhere, but it declares nowhere in particular: a provider
    // that can fetch from any host has not told the user where its files
    // live, so those downloads still carry a warning.
    hosts.iter().filter(|h| h.as_str() != "*").any(|allowed| {
        let allowed = allowed.strip_prefix("*.").unwrap_or(allowed);
        host == allowed
            || (host.len() > allowed.len()
                && host.ends_with(allowed)
                && host.as_bytes()[host.len() - allowed.len() - 1] == b'.')
    })
}

/// Validate a plan and decide whether the user's settings permit it.
///
/// Refuses a malformed plan. Refuses a plan containing files with no
/// integrity information unless the user has allowed low security downloads.
/// Everything else is permitted, with any warnings returned for the review
/// screen: reduced assurance is the user's call, not Agora's.
pub fn authorize_plan(
    ctx: &Ctx,
    plan: &InstallPlan,
    declared_hosts: &[String],
) -> LauncherResult<PlanAuthorization> {
    plan.validate().map_err(|e| LauncherError::Generic {
        code: "ERR_PROVIDER_PLAN_INVALID".into(),
        message: e.message,
    })?;
    let authorization = judge_plan(plan, declared_hosts);
    if authorization.is_low_security() && !low_security_allowed(ctx) {
        let first = &authorization.low_security[0];
        return Err(LauncherError::Generic {
            code: "ERR_LOW_SECURITY_DISABLED".into(),
            message: format!(
                "This install includes a file with no integrity information ({}: {}). Turn on \
                 Allow low security downloads in Settings to install it anyway.",
                first.url_host, first.reason
            ),
        });
    }
    if let Some(first) = authorization.outside_content_folders.first() {
        if !crate::settings::reduced_security_enabled(ctx) {
            return Err(LauncherError::Generic {
                code: "ERR_REDUCED_SECURITY_REQUIRED".into(),
                message: format!(
                    "This pack places files outside the usual content folders (for example \
                     '{first}'). Turn on Reduced security mode in Settings to install it anyway."
                ),
            });
        }
    }
    Ok(authorization)
}

/// The pure half of [`authorize_plan`].
pub fn judge_plan(plan: &InstallPlan, declared_hosts: &[String]) -> PlanAuthorization {
    use agora_plugin_api::provider::{InstallPlan as P, PlannedDownload};
    let mut verdict = PlanAuthorization::default();
    let mut judge = |download: &PlannedDownload| {
        let host = reqwest::Url::parse(&download.url)
            .ok()
            .and_then(|u| u.host_str().map(str::to_string))
            .unwrap_or_default();
        let note = |reason: &str| SecurityNote {
            url_host: host.clone(),
            reason: reason.into(),
        };
        if !url_in_scope(&download.url, declared_hosts) {
            verdict
                .warnings
                .push(note("not an HTTPS host this provider declared"));
        }
        match download.hashes.integrity() {
            Integrity::Strong => {}
            Integrity::Weak => verdict
                .warnings
                .push(note("only an MD5 or SHA-1 digest was published")),
            Integrity::None => verdict.low_security.push(note("no digest was published")),
        }
    };
    match plan {
        P::File(file) => judge(&file.file),
        P::Pack(pack) => {
            pack.files.iter().for_each(|f| judge(&f.download));
            verdict.outside_content_folders = pack
                .files
                .iter()
                .filter(|f| !agora_plugin_api::provider::pack_path_in_content_roots(&f.path))
                .map(|f| f.path.clone())
                .collect();
            if let Some(overrides) = &pack.overrides {
                judge(overrides);
            }
        }
    }
    verdict
}

/// Whether the user allows content with no integrity information.
pub fn low_security_allowed(ctx: &Ctx) -> bool {
    crate::settings::SettingsService::new(ctx.clone())
        .get_bool(LOW_SECURITY_SETTING)
        .unwrap_or(false)
}

/// A digest's name, the value the provider published, and how to compute it.
type DigestCheck<'a> = (&'static str, &'a Option<String>, fn(&[u8]) -> String);

/// Check size and every published digest.
pub fn verify_planned(
    bytes: &[u8],
    download: &agora_plugin_api::provider::PlannedDownload,
) -> LauncherResult<()> {
    use sha2::Digest as _;
    if let Some(size) = download.size {
        if size != bytes.len() as u64 {
            return Err(LauncherError::Generic {
                code: "ERR_SIZE_MISMATCH".into(),
                message: format!(
                    "{} is {} bytes; the provider said {size}.",
                    download.filename,
                    bytes.len()
                ),
            });
        }
    }
    let hashes = &download.hashes;
    let checks: [DigestCheck<'_>; 4] = [
        ("SHA-512", &hashes.sha512, |b| {
            format!("{:x}", sha2::Sha512::digest(b))
        }),
        ("SHA-256", &hashes.sha256, |b| {
            format!("{:x}", sha2::Sha256::digest(b))
        }),
        ("SHA-1", &hashes.sha1, crate::download::sha1_hex),
        ("MD5", &hashes.md5, crate::download::md5_hex),
    ];
    for (name, expected, compute) in checks {
        if let Some(expected) = expected {
            if !compute(bytes).eq_ignore_ascii_case(expected) {
                return Err(LauncherError::Generic {
                    code: "ERR_HASH_MISMATCH".into(),
                    message: format!(
                        "{} failed its {name} check: the file is not the one the provider \
                         described.",
                        download.filename
                    ),
                });
            }
        }
    }
    Ok(())
}

/// Lockdown and plugin-network state, as a reason a provider cannot be used.
pub fn network_unavailable_reason(ctx: &Ctx, setting: Option<&str>) -> Option<String> {
    let conn = crate::db::local_state_connection(&ctx.paths.local_state_db()).ok()?;
    if crate::db::is_lockdown_enabled(&conn) {
        return Some("Privacy Lockdown Mode is on.".into());
    }
    if let Some(setting) = setting {
        if !crate::db::is_network_enabled(&conn, setting) {
            return Some("Network access for this source is off in Privacy settings.".into());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use agora_plugin_api::provider::{FileHashes, FilePlan, PlannedDownload};

    fn file_plan(url: &str, hashes: FileHashes) -> InstallPlan {
        InstallPlan::File(FilePlan {
            version_id: "v1".into(),
            version_number: "1.0.0".into(),
            content_type: "mod".into(),
            file: PlannedDownload {
                url: url.into(),
                filename: "a.jar".into(),
                size: None,
                hashes,
            },
            dependencies: vec![],
        })
    }

    fn strong() -> FileHashes {
        FileHashes {
            sha512: Some("a".repeat(128)),
            ..Default::default()
        }
    }

    #[test]
    fn item_ids_round_trip_even_when_the_project_id_has_colons() {
        let id = item_id("acme.cf/curseforge", "mod:123");
        assert_eq!(parse_item_id(&id), Some(("acme.cf/curseforge", "mod:123")));
        assert_eq!(parse_item_id("technic:foo"), None);
        assert_eq!(parse_item_id("provider::x"), None);
    }

    #[test]
    fn declared_https_with_a_strong_digest_is_in_scope() {
        let hosts = vec!["cdn.example.com".to_string()];
        let verdict = judge_plan(
            &file_plan("https://cdn.example.com/a.jar", strong()),
            &hosts,
        );
        assert!(verdict.is_clean());
        // Subdomains of a declared host are in scope, as for plugin fetches.
        let verdict = judge_plan(
            &file_plan("https://eu.cdn.example.com/a.jar", strong()),
            &hosts,
        );
        assert!(verdict.is_clean());
    }

    #[test]
    fn anything_outside_the_declaration_is_a_warning_not_a_block() {
        let hosts = vec!["cdn.example.com".to_string()];
        for url in [
            "http://cdn.example.com/a.jar",
            "https://evil.example.org/a.jar",
            "https://cdn.example.com.evil.org/a.jar",
            "https://notcdn.example.com/a.jar",
            "https://cdn.example.com:8443/a.jar",
        ] {
            let verdict = judge_plan(&file_plan(url, strong()), &hosts);
            assert!(!verdict.is_clean(), "{url} must not count as in scope");
            assert!(!verdict.is_low_security(), "{url} has a strong digest");
        }
    }

    #[test]
    fn weak_digests_warn_and_missing_digests_are_low_security() {
        let hosts = vec!["cdn.example.com".to_string()];
        let weak = FileHashes {
            sha1: Some("b".repeat(40)),
            ..Default::default()
        };
        let verdict = judge_plan(&file_plan("https://cdn.example.com/a.jar", weak), &hosts);
        assert_eq!(verdict.warnings.len(), 1);
        assert!(!verdict.is_low_security());

        let verdict = judge_plan(
            &file_plan("https://cdn.example.com/a.jar", FileHashes::default()),
            &hosts,
        );
        assert!(verdict.is_low_security());
    }

    #[test]
    fn pack_files_outside_the_content_folders_are_listed_for_reduced_security() {
        use agora_plugin_api::provider::{PackFile, PackPlan};
        let file = |path: &str| PackFile {
            path: path.into(),
            download: PlannedDownload {
                url: "https://cdn.example.com/f".into(),
                filename: "f".into(),
                size: None,
                hashes: strong(),
            },
        };
        let plan = InstallPlan::Pack(PackPlan {
            name: "P".into(),
            version_id: "1".into(),
            version_number: None,
            minecraft_version: "1.20.1".into(),
            loader: "fabric".into(),
            loader_version: String::new(),
            files: vec![
                file("mods/a.jar"),
                file("scripts/r.zs"),
                file("options.txt"),
                file("config/x.jar"),
            ],
            overrides: None,
        });
        assert_eq!(plan.validate(), Ok(()));
        let verdict = judge_plan(&plan, &["cdn.example.com".to_string()]);
        assert_eq!(
            verdict.outside_content_folders,
            ["options.txt", "config/x.jar"]
        );
        assert!(!verdict.is_clean());
    }

    #[test]
    fn verification_checks_every_published_digest() {
        use sha2::Digest as _;
        let bytes = b"hello";
        let good = PlannedDownload {
            url: "https://cdn.example.com/a.jar".into(),
            filename: "a.jar".into(),
            size: Some(5),
            hashes: FileHashes {
                sha256: Some(format!("{:x}", sha2::Sha256::digest(bytes))),
                sha1: Some(crate::download::sha1_hex(bytes)),
                ..Default::default()
            },
        };
        verify_planned(bytes, &good).unwrap();

        // A correct SHA-256 does not excuse a wrong SHA-1: a provider that
        // published both is making two claims, and both are checked.
        let mut bad = good.clone();
        bad.hashes.sha1 = Some("0".repeat(40));
        assert!(verify_planned(bytes, &bad).is_err());

        let mut wrong_size = good;
        wrong_size.size = Some(6);
        assert!(verify_planned(bytes, &wrong_size).is_err());
    }
}
