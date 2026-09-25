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
//! - a file fetched over HTTPS from a host the provider declared, with a
//!   SHA-256 or SHA-512 digest, is **in scope**;
//! - anything else — an undeclared host, plain HTTP, only MD5/SHA-1, no digest
//!   at all — is **unverified content**, and needs the same
//!   `allow_unverified_packs` consent Technic's bare zips have always needed.
//!
//! Enabling a provider is the user's statement of trust in it; the provider id
//! is stamped on everything it installs so that statement stays visible.
//!
//! # Official providers are not privileged
//!
//! [`modrinth::ModrinthProvider`] and [`technic::TechnicProvider`] are compiled
//! in, and implement exactly the trait [`plugin::PluginProvider`] implements.
//! There is no method on [`ContentProvider`] a plugin cannot answer. Where the
//! official integrations still have code paths of their own (Modrinth's
//! single-file install, Technic's consent tiers, mrpack import) they are
//! listed in `docs/plugins/providers.md` as migration debt, not hidden.

pub mod browse;
pub mod install;
pub mod modrinth;
pub mod plugin;
pub mod technic;

use crate::ctx::Ctx;
use crate::error::{LauncherError, LauncherResult};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

pub use agora_plugin_api::provider::{
    FilterDefinition, InstallPlan, Integrity, ProjectDetail, ProjectSummary, ProviderSort,
    ResolveRequest, SearchRequest, SearchResponse, VersionsRequest, VersionsResponse,
};

/// Prefix of a Browse/detail item id that belongs to a provider.
pub const ITEM_PREFIX: &str = "provider:";

/// Setting that permits content Agora cannot verify. Shared with Technic's
/// bare-zip tier: it is one question ("install things Agora cannot check?")
/// and asking it twice would let the answers drift.
pub const UNVERIFIED_SETTING: &str = "allow_unverified_packs";

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
    /// Hosts downloads may come from without counting as unverified.
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

#[derive(Debug, Clone)]
pub enum NativeHit {
    Modrinth(crate::modrinth::ModrinthSearchResult),
    Technic(crate::technic::TechnicSearchResult),
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
        let mut providers: Vec<Arc<dyn ContentProvider>> = vec![
            Arc::new(modrinth::ModrinthProvider::new(ctx.clone())),
            Arc::new(technic::TechnicProvider::new(ctx.clone())),
        ];
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

/// Why part of a plan counts as unverified.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnverifiedReason {
    pub url_host: String,
    pub reason: String,
}

/// Core's verdict on an install plan.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanAuthorization {
    /// Empty when every file is in scope.
    pub unverified: Vec<UnverifiedReason>,
}

impl PlanAuthorization {
    pub fn is_verified(&self) -> bool {
        self.unverified.is_empty()
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
    hosts.iter().any(|allowed| {
        host == *allowed
            || (host.len() > allowed.len()
                && host.ends_with(allowed.as_str())
                && host.as_bytes()[host.len() - allowed.len() - 1] == b'.')
    })
}

/// Validate a plan and decide whether the user's settings permit it.
///
/// Refuses outright when the plan is malformed. Refuses with
/// `ERR_UNVERIFIED_CONTENT_DISABLED` when it contains unverified content and
/// the user has not allowed that. Otherwise returns the verdict, which the
/// install path carries so the download uses the matching host policy.
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
    if !authorization.is_verified() && !unverified_allowed(ctx) {
        let first = &authorization.unverified[0];
        return Err(LauncherError::Generic {
            code: "ERR_UNVERIFIED_CONTENT_DISABLED".into(),
            message: format!(
                "This install includes content Agora cannot verify ({}: {}). Allow unverified \
                 content in Settings to install it anyway.",
                first.url_host, first.reason
            ),
        });
    }
    Ok(authorization)
}

/// The pure half of [`authorize_plan`].
pub fn judge_plan(plan: &InstallPlan, declared_hosts: &[String]) -> PlanAuthorization {
    use agora_plugin_api::provider::{InstallPlan as P, PlannedDownload};
    let mut unverified = Vec::new();
    let mut judge = |download: &PlannedDownload| {
        let host = reqwest::Url::parse(&download.url)
            .ok()
            .and_then(|u| u.host_str().map(str::to_string))
            .unwrap_or_default();
        if !url_in_scope(&download.url, declared_hosts) {
            unverified.push(UnverifiedReason {
                url_host: host.clone(),
                reason: "not an HTTPS host this provider declared".into(),
            });
        }
        match download.hashes.integrity() {
            Integrity::Strong => {}
            Integrity::Weak => unverified.push(UnverifiedReason {
                url_host: host,
                reason: "only an MD5 or SHA-1 digest was published".into(),
            }),
            Integrity::None => unverified.push(UnverifiedReason {
                url_host: host,
                reason: "no digest was published".into(),
            }),
        }
    };
    match plan {
        P::File(file) => judge(&file.file),
        P::Pack(pack) => {
            pack.files.iter().for_each(|f| judge(&f.download));
            if let Some(overrides) = &pack.overrides {
                judge(overrides);
            }
        }
    }
    PlanAuthorization { unverified }
}

fn unverified_allowed(ctx: &Ctx) -> bool {
    crate::settings::SettingsService::new(ctx.clone())
        .get_bool(UNVERIFIED_SETTING)
        .unwrap_or(false)
}

/// Fetch one planned file and check it against every digest the provider
/// published.
///
/// In-scope files go through [`HostPolicy::ProviderDeclared`], which keeps
/// every redirect hop on the declared hosts. Anything else was already
/// accepted as unverified by [`authorize_plan`] and goes through the
/// consented-content policy — still Lockdown-gated, still refusing private and
/// loopback addresses — after re-checking that consent, because a plan can
/// outlive a settings change.
///
/// [`HostPolicy::ProviderDeclared`]: crate::http_client::HostPolicy::ProviderDeclared
pub async fn download_planned(
    ctx: &Ctx,
    download: &agora_plugin_api::provider::PlannedDownload,
    declared_hosts: &[String],
) -> LauncherResult<Vec<u8>> {
    use crate::http_client::{self, ClientCategory, HostPolicy};
    let bytes = if url_in_scope(&download.url, declared_hosts) {
        http_client::checked_get_bytes_with_policy(
            &ctx.http_clients,
            ClientCategory::ConsentedContent,
            &download.url,
            HostPolicy::ProviderDeclared(declared_hosts),
        )
        .await?
    } else {
        if !unverified_allowed(ctx) {
            return Err(LauncherError::Generic {
                code: "ERR_UNVERIFIED_CONTENT_DISABLED".into(),
                message: "Unverified content was turned off after this install was planned.".into(),
            });
        }
        http_client::checked_get_bytes_with_policy(
            &ctx.http_clients,
            ClientCategory::ConsentedContent,
            &download.url,
            HostPolicy::UserConsented,
        )
        .await?
    };
    verify_planned(&bytes, download)?;
    Ok(bytes)
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
pub(crate) fn network_unavailable_reason(ctx: &Ctx, setting: Option<&str>) -> Option<String> {
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
        assert!(verdict.is_verified());
        // Subdomains of a declared host are in scope, as for plugin fetches.
        let verdict = judge_plan(
            &file_plan("https://eu.cdn.example.com/a.jar", strong()),
            &hosts,
        );
        assert!(verdict.is_verified());
    }

    #[test]
    fn anything_outside_the_declaration_is_unverified() {
        let hosts = vec!["cdn.example.com".to_string()];
        for url in [
            "http://cdn.example.com/a.jar",
            "https://evil.example.org/a.jar",
            "https://cdn.example.com.evil.org/a.jar",
            "https://notcdn.example.com/a.jar",
            "https://cdn.example.com:8443/a.jar",
        ] {
            assert!(
                !judge_plan(&file_plan(url, strong()), &hosts).is_verified(),
                "{url} must not count as in scope"
            );
        }
    }

    #[test]
    fn weak_or_missing_digests_are_unverified_even_on_a_declared_host() {
        let hosts = vec!["cdn.example.com".to_string()];
        let weak = FileHashes {
            sha1: Some("b".repeat(40)),
            ..Default::default()
        };
        assert!(
            !judge_plan(&file_plan("https://cdn.example.com/a.jar", weak), &hosts).is_verified()
        );
        assert!(!judge_plan(
            &file_plan("https://cdn.example.com/a.jar", FileHashes::default()),
            &hosts
        )
        .is_verified());
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
