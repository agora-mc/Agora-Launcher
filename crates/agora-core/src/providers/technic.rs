//! Technic, as an official provider.
//!
//! Technic is the reason the provider vocabulary has a pack plan at all. It
//! distributes only modpacks, cannot paginate, publishes MD5 at best, and
//! serves files from whatever host a pack author chose. If the same interface
//! carries both Technic and Modrinth, it is genuinely provider-neutral rather
//! than "Modrinth's API with the names changed".
//!
//! Technic is also where the shared security tiers come from: a Solder pack
//! publishes MD5 and installs after a warning; a bare zip publishes nothing
//! and needs *Allow low security downloads*. [`super::judge_plan`] applies
//! exactly that to every provider.

use super::{
    network_unavailable_reason, ContentProvider, NativeHit, ProviderDescriptor, ProviderHit,
    ProviderOrigin, ProviderPage,
};
use crate::ctx::Ctx;
use crate::error::{LauncherError, LauncherResult};
use crate::technic::{TechnicPackDetail, TechnicSearchResult, TechnicTier};
use agora_plugin_api::provider::{
    FileHashes, InstallPlan, PackFile, PackPlan, PlannedDownload, ProjectDetail, ProjectLink,
    ProjectSummary, ProjectVersion, ProviderSort, ReleaseChannel, ResolveRequest, SearchRequest,
    VersionsRequest, VersionsResponse,
};
use async_trait::async_trait;

pub const PROVIDER_ID: &str = "technic";

/// Technic's search ignores `offset`, so one fetch serves the whole query.
const SEARCH_LIMIT: u32 = 30;

/// Technic's popularity scale.
///
/// Technic ratings are far denser relative to installs than Modrinth follows
/// (~0.11% vs ~0.02%), so the endorsement ceiling is proportionally lower.
/// Without this a Technic pack's ratings would barely register beside
/// Modrinth projects. Technic has no library category.
pub fn ranking_profile() -> agora_plugin_api::provider::RankingProfile {
    agora_plugin_api::provider::RankingProfile {
        downloads_ceiling: 250_000_000,
        endorsements_ceiling: 2_000,
        library_categories: Vec::new(),
    }
}

pub struct TechnicProvider {
    ctx: Ctx,
}

impl TechnicProvider {
    pub fn new(ctx: Ctx) -> Self {
        Self { ctx }
    }

    fn setting(&self, key: &str) -> bool {
        crate::settings::SettingsService::new(self.ctx.clone())
            .get_bool(key)
            .unwrap_or(false)
    }

    fn require_usable(&self) -> LauncherResult<()> {
        let conn = crate::db::local_state_connection(&self.ctx.paths.local_state_db()).map_err(
            |error| LauncherError::Generic {
                code: "ERR_LOCAL_STATE_FAILED".into(),
                message: error.to_string(),
            },
        )?;
        crate::technic::consent_for_tier(&conn, TechnicTier::Solder)
    }
}

pub fn summary_from_search(hit: &TechnicSearchResult) -> ProjectSummary {
    ProjectSummary {
        id: hit.slug.clone(),
        title: hit.title.clone(),
        description: Some(hit.description.clone()).filter(|d| !d.is_empty()),
        author: hit.author.clone(),
        icon_url: hit.icon_url.clone(),
        content_type: "pack".into(),
        categories: hit.tags.clone(),
        downloads: Some(hit.installs),
        follows: Some(hit.likes),
        page_url: Some(hit.page_url.clone()),
        minecraft_versions: Vec::new(),
        loaders: Vec::new(),
        hero_image_url: None,
        // A bare zip carries no digest of any kind.
        low_security: hit.tier == TechnicTier::Zip,
    }
}

fn summary_from_detail(detail: &TechnicPackDetail) -> ProjectSummary {
    ProjectSummary {
        id: detail.slug.clone(),
        title: detail.title.clone(),
        description: Some(detail.description.clone()).filter(|d| !d.is_empty()),
        author: detail.author.clone(),
        icon_url: detail.icon_url.clone(),
        content_type: "pack".into(),
        categories: detail.tags.clone(),
        downloads: Some(detail.installs),
        follows: Some(detail.likes),
        page_url: Some(detail.page_url.clone()),
        minecraft_versions: detail.minecraft.clone().into_iter().collect(),
        loaders: Vec::new(),
        hero_image_url: None,
        low_security: detail.tier == TechnicTier::Zip,
    }
}

/// The last URL path segment, if it is a safe file name.
fn filename_from_url(url: &str) -> Option<String> {
    let parsed = reqwest::Url::parse(url).ok()?;
    let name = parsed.path_segments()?.next_back()?.to_string();
    let name = urlencoding::decode(&name).ok()?.into_owned();
    agora_plugin_api::provider::validate_filename(&name).ok()?;
    Some(name)
}

/// A resolved Solder build as a pack plan.
pub fn plan_from_solder(pack: &crate::import::TechnicSolderPack) -> LauncherResult<InstallPlan> {
    let files = pack
        .mods
        .iter()
        .map(|entry| {
            let filename = filename_from_url(&entry.url).ok_or_else(|| LauncherError::Generic {
                code: "ERR_TECHNIC_FILENAME".into(),
                message: format!("Technic mod '{}' has no safe file name.", entry.name),
            })?;
            Ok(PackFile {
                path: format!("mods/{filename}"),
                download: PlannedDownload {
                    url: entry.url.clone(),
                    filename,
                    size: None,
                    hashes: FileHashes {
                        md5: entry.md5.clone().map(|m| m.trim().to_ascii_lowercase()),
                        ..Default::default()
                    },
                },
            })
        })
        .collect::<LauncherResult<Vec<_>>>()?;
    Ok(InstallPlan::Pack(PackPlan {
        name: pack.display_name.clone(),
        version_id: pack.build.clone(),
        version_number: Some(pack.build.clone()),
        minecraft_version: pack.minecraft_version.clone(),
        loader: pack.loader.clone(),
        loader_version: pack.loader_version.clone(),
        files,
        overrides: None,
    }))
}

#[async_trait]
impl ContentProvider for TechnicProvider {
    fn descriptor(&self) -> ProviderDescriptor {
        ProviderDescriptor {
            id: PROVIDER_ID.into(),
            title: "Technic".into(),
            description: Some("Modpacks from the Technic Platform".into()),
            origin: ProviderOrigin::Official,
            content_types: vec!["pack".into()],
            filters: Vec::new(),
            sorts: vec![ProviderSort::Relevance],
            paginates: false,
            ranking: ranking_profile(),
            // Technic serves no files itself; every download is from wherever
            // the pack author hosted it, so nothing is in declared scope.
            download_hosts: Vec::new(),
            enabled: self.setting("technic_enabled"),
            unavailable_reason: network_unavailable_reason(&self.ctx, None),
        }
    }

    async fn search(&self, request: SearchRequest) -> LauncherResult<ProviderPage> {
        self.require_usable()?;
        let results = crate::technic::search_technic_http(
            &self.ctx.http_clients,
            &request.query,
            SEARCH_LIMIT,
        )
        .await?;
        Ok(ProviderPage {
            has_more: false,
            total: None,
            hits: results
                .into_iter()
                // Bare zips are marked low security in their summary; Browse
                // decides whether to show them, the same way for every source.
                .map(|hit| ProviderHit {
                    summary: summary_from_search(&hit),
                    native: Some(NativeHit::Technic(hit)),
                })
                .collect(),
        })
    }

    async fn project(&self, project_id: &str) -> LauncherResult<ProjectDetail> {
        let detail = crate::technic::pack_detail(&self.ctx, project_id).await?;
        let mut links = vec![ProjectLink {
            label: "Technic page".into(),
            url: detail.page_url.clone(),
        }];
        if let Some(site) = detail.website.clone().filter(|s| s.starts_with("https://")) {
            links.push(ProjectLink {
                label: "Website".into(),
                url: site,
            });
        }
        Ok(ProjectDetail {
            project: summary_from_detail(&detail),
            body: None,
            gallery: Vec::new(),
            license: None,
            updated: None,
            links,
        })
    }

    async fn versions(&self, request: VersionsRequest) -> LauncherResult<VersionsResponse> {
        let detail = crate::technic::pack_detail(&self.ctx, &request.project_id).await?;
        // The platform API names only the recommended build; older builds are
        // behind Solder and are not a picker Technic itself offers.
        Ok(VersionsResponse {
            versions: detail
                .recommended_build
                .map(|build| ProjectVersion {
                    id: build.clone(),
                    name: build.clone(),
                    version_number: build,
                    channel: ReleaseChannel::Release,
                    minecraft_versions: detail.minecraft.into_iter().collect(),
                    ..Default::default()
                })
                .into_iter()
                .collect(),
        })
    }

    async fn resolve(&self, request: ResolveRequest) -> LauncherResult<InstallPlan> {
        let detail = crate::technic::pack_detail(&self.ctx, &request.project_id).await?;
        let solder = detail
            .solder
            .as_deref()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| LauncherError::Generic {
                code: "ERR_TECHNIC_ZIP_PACK".into(),
                message: "This Technic pack is a bare archive; it installs through the \
                          unverified-zip path."
                    .into(),
            })?;
        let build = request
            .version_id
            .clone()
            .or(detail.recommended_build.clone())
            .ok_or_else(|| LauncherError::Generic {
                code: "ERR_TECHNIC_BUILD".into(),
                message: "The Technic pack names no build to install.".into(),
            })?;
        let pack =
            crate::technic::resolve_solder_build(&self.ctx, solder, &detail.slug, &build).await?;
        plan_from_solder(&pack)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::import::{TechnicSolderMod, TechnicSolderPack};
    use agora_plugin_api::provider::Integrity;

    #[test]
    fn a_solder_build_becomes_a_pack_plan_the_shared_rule_warns_about() {
        let pack = TechnicSolderPack {
            display_name: "Tekkit".into(),
            minecraft_version: "1.12.2".into(),
            loader: "forge".into(),
            loader_version: "14.23.5.2860".into(),
            mods: vec![TechnicSolderMod {
                name: "buildcraft".into(),
                url: "http://mirror.example.net/mods/buildcraft-7.99.zip".into(),
                md5: Some("0123456789ABCDEF0123456789abcdef".into()),
            }],
            slug: "tekkit".into(),
            solder_url: "http://solder.example.net".into(),
            build: "1.2.3".into(),
        };
        let plan = plan_from_solder(&pack).unwrap();
        plan.validate().unwrap();
        assert_eq!(plan.integrity(), Integrity::Weak);
        let verdict = super::super::judge_plan(&plan, &[]);
        assert!(
            !verdict.warnings.is_empty(),
            "MD5 from an undeclared host warns"
        );
        assert!(
            !verdict.is_low_security(),
            "but it does not need the toggle"
        );
        let InstallPlan::Pack(pack_plan) = plan else {
            panic!("expected a pack plan");
        };
        assert_eq!(pack_plan.files[0].path, "mods/buildcraft-7.99.zip");
        assert_eq!(pack_plan.loader, "forge");
    }

    #[test]
    fn unsafe_file_names_in_a_solder_url_are_refused() {
        assert_eq!(filename_from_url("https://x.example/a/b/.hidden"), None);
        assert_eq!(filename_from_url("https://x.example/"), None);
        assert_eq!(
            filename_from_url("https://x.example/a/My%20Mod.jar").as_deref(),
            Some("My Mod.jar")
        );
    }
}
