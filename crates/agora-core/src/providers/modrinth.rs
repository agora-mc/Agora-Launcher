//! Modrinth, as an official provider.
//!
//! A translation layer and nothing more: Modrinth's API shapes in, the
//! provider vocabulary out. The HTTP, facet and parsing code it calls is the
//! same `crate::modrinth` code Agora has always used, so moving Browse onto
//! the provider interface changes no request Modrinth receives.

use super::{
    network_unavailable_reason, ContentProvider, NativeHit, ProviderDescriptor, ProviderHit,
    ProviderOrigin, ProviderPage,
};
use crate::ctx::Ctx;
use crate::error::{LauncherError, LauncherResult};
use crate::http_client::{self, ClientCategory};
use crate::modrinth::{ModrinthSearchParams, ModrinthSearchResult, ModrinthSort, ModrinthVersion};
use agora_plugin_api::provider::{
    DependencyKind, FileHashes, FilePlan, InstallPlan, PlannedDownload, ProjectDetail, ProjectLink,
    ProjectSummary, ProjectVersion, ProviderDependency, ProviderSort, ReleaseChannel,
    ResolveRequest, SearchRequest, VersionsRequest, VersionsResponse,
};
use async_trait::async_trait;

pub const PROVIDER_ID: &str = "modrinth";

/// Where Modrinth serves files from. Everything else is out of scope.
const DOWNLOAD_HOSTS: &[&str] = &["cdn.modrinth.com"];

pub struct ModrinthProvider {
    ctx: Ctx,
}

impl ModrinthProvider {
    pub fn new(ctx: Ctx) -> Self {
        Self { ctx }
    }

    fn enabled(&self) -> bool {
        crate::settings::SettingsService::new(self.ctx.clone())
            .get_bool("modrinth_enabled")
            .unwrap_or(false)
    }

    fn require_usable(&self) -> LauncherResult<()> {
        crate::modrinth::ModrinthService::new(self.ctx.clone()).check_enabled()
    }
}

/// Agora's content type to Modrinth's `project_type`.
pub fn modrinth_project_type(content_type: &str) -> &str {
    match content_type {
        "pack" => "modpack",
        "server" => "minecraft_java_server",
        other => other,
    }
}

fn modrinth_sort(sort: ProviderSort) -> ModrinthSort {
    match sort {
        ProviderSort::Relevance => ModrinthSort::Relevance,
        ProviderSort::Downloads => ModrinthSort::Downloads,
        ProviderSort::Follows => ModrinthSort::Follows,
        ProviderSort::Newest => ModrinthSort::Newest,
        ProviderSort::Updated => ModrinthSort::Updated,
    }
}

pub fn summary_from_search(hit: &ModrinthSearchResult) -> ProjectSummary {
    ProjectSummary {
        id: hit.project_id.clone(),
        title: hit.title.clone(),
        description: Some(hit.description.clone()).filter(|d| !d.is_empty()),
        author: Some(hit.author.clone()).filter(|a| !a.is_empty()),
        icon_url: hit.icon_url.clone(),
        content_type: crate::browse_cache::normalize_modrinth_content_type(&hit.project_type)
            .to_string(),
        categories: hit.categories.clone(),
        downloads: u64::try_from(hit.downloads).ok(),
        follows: u64::try_from(hit.follows).ok(),
        page_url: (!hit.slug.is_empty())
            .then(|| format!("https://modrinth.com/{}/{}", hit.project_type, hit.slug)),
        minecraft_versions: hit.versions.clone(),
        loaders: Vec::new(),
        hero_image_url: hit.featured_gallery.clone(),
    }
}

fn channel(version_type: Option<&str>) -> ReleaseChannel {
    match version_type.map(str::to_ascii_lowercase).as_deref() {
        Some("beta") => ReleaseChannel::Beta,
        Some("alpha") => ReleaseChannel::Alpha,
        _ => ReleaseChannel::Release,
    }
}

fn dependency_kind(raw: &str) -> Option<DependencyKind> {
    match raw {
        "required" => Some(DependencyKind::Required),
        "optional" => Some(DependencyKind::Optional),
        "incompatible" => Some(DependencyKind::Incompatible),
        "embedded" => Some(DependencyKind::Embedded),
        _ => None,
    }
}

fn to_version(version: &ModrinthVersion) -> ProjectVersion {
    ProjectVersion {
        id: version.id.clone(),
        name: version
            .name
            .clone()
            .unwrap_or_else(|| version.version_number.clone()),
        version_number: version.version_number.clone(),
        channel: channel(version.version_type.as_deref()),
        minecraft_versions: version.game_versions.clone().unwrap_or_default(),
        loaders: version
            .loaders
            .clone()
            .unwrap_or_default()
            .into_iter()
            .map(|l| l.to_ascii_lowercase())
            .collect(),
        published: version.date_published.clone(),
        dependencies: version
            .dependencies
            .iter()
            .filter_map(|dep| {
                Some(ProviderDependency {
                    // A version-only dependency names no project; the
                    // provider vocabulary needs one, so it is skipped rather
                    // than guessed.
                    project_id: dep.project_id.clone()?,
                    version_id: dep.version_id.clone(),
                    kind: dependency_kind(&dep.dependency_type)?,
                })
            })
            .collect(),
        changelog: version.changelog.clone(),
    }
}

/// The version a resolve should install: the one asked for, or the newest
/// release (falling back to any channel) that matches the target.
fn pick_version<'a>(
    versions: &'a [ModrinthVersion],
    requested: Option<&str>,
) -> Option<&'a ModrinthVersion> {
    if let Some(requested) = requested {
        return versions
            .iter()
            .find(|v| v.id == requested || v.version_number == requested);
    }
    let newest = |candidates: Vec<&'a ModrinthVersion>| {
        candidates
            .into_iter()
            .max_by(|a, b| a.date_published.cmp(&b.date_published))
    };
    newest(
        versions
            .iter()
            .filter(|v| channel(v.version_type.as_deref()) == ReleaseChannel::Release)
            .collect(),
    )
    .or_else(|| newest(versions.iter().collect()))
}

async fn fetch_versions(
    ctx: &Ctx,
    project_id: &str,
    minecraft_version: Option<&str>,
    loader: Option<&str>,
) -> LauncherResult<Vec<ModrinthVersion>> {
    let mut query = Vec::new();
    if let Some(version) = minecraft_version.filter(|v| !v.is_empty()) {
        let json = serde_json::to_string(&[version]).unwrap_or_default();
        query.push(format!("game_versions={}", urlencoding::encode(&json)));
    }
    if let Some(loader) = loader.filter(|l| !l.is_empty()) {
        let json = serde_json::to_string(&[loader]).unwrap_or_default();
        query.push(format!("loaders={}", urlencoding::encode(&json)));
    }
    let mut url = format!(
        "https://api.modrinth.com/v2/project/{}/version",
        urlencoding::encode(project_id)
    );
    if !query.is_empty() {
        url.push('?');
        url.push_str(&query.join("&"));
    }
    http_client::checked_get_json(&ctx.http_clients, ClientCategory::Modrinth, &url).await
}

#[async_trait]
impl ContentProvider for ModrinthProvider {
    fn descriptor(&self) -> ProviderDescriptor {
        ProviderDescriptor {
            id: PROVIDER_ID.into(),
            title: "Modrinth".into(),
            description: Some("Mods, packs, shaders and resource packs from modrinth.com".into()),
            origin: ProviderOrigin::Official,
            content_types: agora_plugin_api::provider::CONTENT_TYPES
                .iter()
                .map(|t| t.to_string())
                .collect(),
            filters: Vec::new(),
            sorts: vec![
                ProviderSort::Relevance,
                ProviderSort::Downloads,
                ProviderSort::Follows,
                ProviderSort::Newest,
                ProviderSort::Updated,
            ],
            paginates: true,
            download_hosts: DOWNLOAD_HOSTS.iter().map(|h| h.to_string()).collect(),
            enabled: self.enabled(),
            unavailable_reason: network_unavailable_reason(
                &self.ctx,
                Some("network_modrinth_enabled"),
            ),
        }
    }

    async fn search(&self, request: SearchRequest) -> LauncherResult<ProviderPage> {
        self.require_usable()?;
        let params = ModrinthSearchParams {
            query: Some(request.query.clone()).filter(|q| !q.is_empty()),
            categories: request.category.clone().map(|c| vec![c]),
            loaders: request.loader.clone().map(|l| vec![l]),
            game_versions: request.minecraft_version.clone().map(|v| vec![v]),
            sort: Some(modrinth_sort(request.sort)),
            limit: Some(request.limit),
            offset: Some(request.offset),
            project_type: request
                .content_type
                .as_deref()
                .map(modrinth_project_type)
                .map(str::to_string),
        };
        let page = crate::modrinth::search_modrinth_http(&params).await?;
        let has_more = page.total_hits > u64::from(request.offset) + page.results.len() as u64;
        Ok(ProviderPage {
            total: Some(page.total_hits),
            has_more,
            hits: page
                .results
                .into_iter()
                .map(|hit| ProviderHit {
                    summary: summary_from_search(&hit),
                    native: Some(NativeHit::Modrinth(hit)),
                })
                .collect(),
        })
    }

    async fn project(&self, project_id: &str) -> LauncherResult<ProjectDetail> {
        let full = crate::modrinth::ModrinthService::new(self.ctx.clone())
            .fetch_project_full(project_id)
            .await?;
        Ok(ProjectDetail {
            project: ProjectSummary {
                id: full.id.clone(),
                title: full.title.clone(),
                description: Some(full.description.clone()).filter(|d| !d.is_empty()),
                author: None,
                icon_url: full.icon_url.clone(),
                content_type: crate::browse_cache::normalize_modrinth_content_type(
                    &full.project_type,
                )
                .to_string(),
                categories: full.categories.clone(),
                downloads: u64::try_from(full.downloads).ok(),
                follows: u64::try_from(full.followers).ok(),
                page_url: full.page_url.clone(),
                minecraft_versions: Vec::new(),
                loaders: Vec::new(),
                hero_image_url: full.gallery_urls.first().cloned(),
            },
            body: full.body,
            gallery: full.gallery_urls,
            license: full.license_id,
            updated: full.source_updated_at,
            links: full
                .page_url
                .map(|url| {
                    vec![ProjectLink {
                        label: "Modrinth page".into(),
                        url,
                    }]
                })
                .unwrap_or_default(),
        })
    }

    async fn versions(&self, request: VersionsRequest) -> LauncherResult<VersionsResponse> {
        self.require_usable()?;
        let raw = fetch_versions(
            &self.ctx,
            &request.project_id,
            request.minecraft_version.as_deref(),
            request.loader.as_deref(),
        )
        .await?;
        Ok(VersionsResponse {
            versions: raw.iter().map(to_version).collect(),
        })
    }

    async fn resolve(&self, request: ResolveRequest) -> LauncherResult<InstallPlan> {
        self.require_usable()?;
        let raw = fetch_versions(
            &self.ctx,
            &request.project_id,
            Some(request.minecraft_version.as_str()),
            Some(request.loader.as_str()),
        )
        .await?;
        let version = pick_version(&raw, request.version_id.as_deref()).ok_or_else(|| {
            LauncherError::Generic {
                code: "ERR_NO_COMPATIBLE_VERSION".into(),
                message: format!(
                    "Modrinth has no version of this project for Minecraft {} with {}.",
                    request.minecraft_version, request.loader
                ),
            }
        })?;
        let file = version
            .files
            .iter()
            .find(|f| f.primary)
            .or_else(|| version.files.first())
            .ok_or_else(|| LauncherError::Generic {
                code: "ERR_NO_FILE".into(),
                message: "That Modrinth version has no downloadable file.".into(),
            })?;
        // Modrinth modpacks are `.mrpack` archives whose file list is inside
        // the archive, so a plan cannot be written without downloading it
        // first — which is exactly what a provider must not do. They install
        // through Agora's mrpack import instead. Listed as migration debt.
        let loaders = version.loaders.clone().unwrap_or_default();
        let content_type = if file.filename.ends_with(".mrpack") {
            return Err(LauncherError::Generic {
                code: "ERR_USE_PACK_IMPORT".into(),
                message: "Modrinth modpacks install through the modpack import.".into(),
            });
        } else if loaders.iter().any(|l| l == "iris" || l == "optifine") {
            "shader"
        } else if loaders.iter().any(|l| l == "minecraft") {
            "resourcepack"
        } else if loaders.iter().any(|l| l == "datapack") {
            "datapack"
        } else {
            "mod"
        };
        Ok(InstallPlan::File(FilePlan {
            version_id: version.id.clone(),
            version_number: version.version_number.clone(),
            content_type: content_type.into(),
            file: PlannedDownload {
                url: file.url.clone(),
                filename: file.filename.clone(),
                size: file.size,
                hashes: FileHashes {
                    sha512: file.hashes.as_ref().and_then(|h| h.sha512.clone()),
                    sha1: file.hashes.as_ref().and_then(|h| h.sha1.clone()),
                    ..Default::default()
                },
            },
            dependencies: to_version(version).dependencies,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn version(id: &str, published: &str, kind: &str) -> ModrinthVersion {
        serde_json::from_value(serde_json::json!({
            "id": id,
            "version_number": id,
            "date_published": published,
            "version_type": kind,
            "files": [],
            "dependencies": [
                { "project_id": "P7dR8mSH", "dependency_type": "required" },
                { "version_id": "orphan", "dependency_type": "required" },
                { "project_id": "x", "dependency_type": "something-new" }
            ]
        }))
        .unwrap()
    }

    #[test]
    fn picks_the_newest_release_before_any_prerelease() {
        let versions = vec![
            version("1.0", "2024-01-01", "release"),
            version("2.0-beta", "2024-06-01", "beta"),
            version("1.1", "2024-03-01", "release"),
        ];
        assert_eq!(pick_version(&versions, None).unwrap().id, "1.1");
        assert_eq!(
            pick_version(&versions, Some("2.0-beta")).unwrap().id,
            "2.0-beta"
        );
        assert!(pick_version(&versions, Some("9.9")).is_none());
    }

    #[test]
    fn dependencies_without_a_project_or_a_known_kind_are_dropped_not_guessed() {
        let converted = to_version(&version("1.0", "2024-01-01", "release"));
        assert_eq!(converted.dependencies.len(), 1);
        assert_eq!(converted.dependencies[0].project_id, "P7dR8mSH");
        assert_eq!(converted.dependencies[0].kind, DependencyKind::Required);
    }
}
