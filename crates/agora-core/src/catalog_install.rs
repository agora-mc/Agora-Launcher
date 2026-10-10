//! Installing a curated catalog entry for another game into an instance (MASTER_SPEC §26.8).
//!
//! The steps, in order, and where each one can stop:
//!
//! 1. The entry must be for the instance's game. Its compatibility entry is the first one whose
//!    `stores` include the instance's store and whose `game_versions` include its version.
//! 2. Each framework the entry requires must be installed, and at least `min_version`. A framework
//!    whose version cannot be read is a warning, and the launch check still guards it.
//! 3. The file is chosen. `github_release` takes the newest non-draft, non-prerelease release with
//!    an asset matching the compatibility entry's `asset` glob. `direct_hash` is the entry's URL.
//! 4. An entry this instance already has (any release) is reported and nothing changes.
//! 5. Under `--dry-run` the plan is returned here. Otherwise the caller is told what is about to
//!    be downloaded, and the bytes are fetched through the existing host policy.
//! 6. The bytes are checked through [`crate::artifact_hash::verify_download`], which is the hash
//!    policy in code. They are imported with a `ContentSource::Catalog`, then placed through
//!    [`crate::game_deploy::decide_placement`], the same rule `games instance content add` uses.
//!
//! Network access goes through [`CatalogTransport`], so the steps above can be tested with a fake.

use std::path::PathBuf;

use agora_game_api::{BaseReference, FrameworkId, GameDefinition, LayerSource, RuntimeIdentity};
use async_trait::async_trait;
use serde::Serialize;

use crate::artifact_hash::{first_unmet, verify_download, ConfirmableHash, HashOrigin};
use crate::content_store::{self, ContentSource};
use crate::ctx::Ctx;
use crate::error::{LauncherError, LauncherResult};
use crate::game_deploy::{self, PlacementDecision};
use crate::game_frameworks::{self, FrameworkStatus};
use crate::game_instance::{self, GameInstanceManifest};
use crate::github_release::{self, GitHubAuth, GitHubRelease};
use crate::http_client::HttpClients;
use crate::registry::{DownloadSource, GameCatalogItem, GameCompatibility};

/// The network side of an install: a page of a repository's releases, and the bytes of a file.
#[async_trait]
pub trait CatalogTransport: Send + Sync {
    /// One page of `owner/repo`'s releases, and the total number of pages.
    async fn releases_page(
        &self,
        repo: &str,
        page: u32,
    ) -> LauncherResult<(Vec<GitHubRelease>, u32)>;
    /// The bytes at `url`. `pinned_host` is the host of the entry's signed source, for a
    /// `direct_hash` file; `None` for a GitHub release file.
    async fn fetch(&self, url: &str, pinned_host: Option<&str>) -> LauncherResult<Vec<u8>>;
}

/// The real transport: GitHub's API and downloads through the checked HTTP clients.
pub struct HttpTransport<'a> {
    pub clients: &'a HttpClients,
    pub auth: GitHubAuth,
}

#[async_trait]
impl CatalogTransport for HttpTransport<'_> {
    async fn releases_page(
        &self,
        repo: &str,
        page: u32,
    ) -> LauncherResult<(Vec<GitHubRelease>, u32)> {
        github_release::list_releases_page(self.clients, repo, page, &self.auth).await
    }

    async fn fetch(&self, url: &str, pinned_host: Option<&str>) -> LauncherResult<Vec<u8>> {
        match pinned_host {
            Some(host) => crate::download::download_pinned_bytes(self.clients, url, host).await,
            None => crate::download::download_mod_bytes(self.clients, url).await,
        }
    }
}

/// What the caller asked for.
pub struct InstallRequest<'a> {
    pub instance_id: &'a str,
    pub item: &'a GameCatalogItem,
    /// Accept a hash that differs from a curator pin or an earlier install. Never accepts a
    /// published hash that differs, and never overrides a framework refusal.
    pub install_anyway: bool,
    /// Plan only: nothing is downloaded or stored.
    pub dry_run: bool,
}

/// The compatibility entry that was chosen, the frameworks checked, and the file to fetch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CatalogPlan {
    pub item_id: String,
    pub item_name: String,
    pub instance_id: String,
    /// Index of the chosen compatibility entry in the manifest's order.
    pub compatibility_index: usize,
    pub compatibility: GameCompatibility,
    pub frameworks: Vec<FrameworkCheck>,
    pub source: PlannedSource,
    pub warnings: Vec<String>,
}

/// One required framework, as the instance has it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FrameworkCheck {
    pub framework: String,
    pub name: String,
    pub min_version: Option<String>,
    /// The version found, or `None` when the framework is present but its version is unreadable.
    pub found_version: Option<String>,
}

/// The file the plan will fetch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "strategy", rename_all = "snake_case")]
pub enum PlannedSource {
    GithubRelease {
        repo: String,
        tag: String,
        asset: String,
        download_url: String,
        /// The SHA-256 GitHub published for the asset, when it published one.
        published_sha256: Option<String>,
        size: Option<u64>,
        /// The curator's pin for this exact release file, when there is one.
        pin: Option<ConfirmableHash>,
    },
    DirectHash {
        url: String,
        file: String,
        /// The entry's manifest hash, which names this one file.
        sha256: String,
        /// The host of the entry's signed source, which the download is pinned to.
        pinned_host: String,
        size: Option<u64>,
    },
}

impl PlannedSource {
    /// The file name the bytes are stored under.
    pub fn file_name(&self) -> &str {
        match self {
            PlannedSource::GithubRelease { asset, .. } => asset,
            PlannedSource::DirectHash { file, .. } => file,
        }
    }

    /// The release tag, for a GitHub file. `None` for a `direct_hash` file.
    pub fn release(&self) -> Option<&str> {
        match self {
            PlannedSource::GithubRelease { tag, .. } => Some(tag),
            PlannedSource::DirectHash { .. } => None,
        }
    }

    /// Where the file comes from, as a person reads it.
    pub fn describe(&self) -> String {
        match self {
            PlannedSource::GithubRelease { repo, tag, .. } => {
                format!("GitHub {repo} release {tag}")
            }
            PlannedSource::DirectHash { url, .. } => format!("direct download {url}"),
        }
    }
}

/// What the hash check found, in words a person can read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HashReport {
    /// The lowercase SHA-256 of the downloaded bytes, which the install records.
    pub sha256: String,
    /// Whether a hash the source published, or the curator's manifest, checked the bytes.
    pub verified: bool,
    /// How the bytes were checked, or why they were not.
    pub basis: String,
    /// When an expectation did not match and `--install-anyway` let the install go on: whether it
    /// was the curator's pin or an earlier install.
    pub confirmed_past: Option<HashOrigin>,
}

/// What the install did, after the plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum InstallState {
    /// `--dry-run`: the plan is all there is.
    DryRun,
    /// The instance already has an item from this entry. Nothing changed.
    AlreadyInstalled {
        installed_release: Option<String>,
        /// Whether the release it has is the release that would be installed.
        same_release: bool,
    },
    /// The file is in the content store and placed in the instance.
    Placed {
        content_item_id: String,
        mount_path: String,
        source_path: String,
        size: u64,
        hash: HashReport,
    },
    /// The archive has a FOMOD installer. It is in the content store but not in the instance.
    NeedsInstaller {
        content_item_id: String,
        size: u64,
        hash: HashReport,
        reason: String,
        top_level: Vec<String>,
        /// The command that runs the installer, as `content add` words it.
        installer_command: String,
    },
}

/// The whole result: the plan, what happened, and any warnings the install raised.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InstallReport {
    pub plan: CatalogPlan,
    pub state: InstallState,
}

/// Plan, and unless `dry_run` or already installed, fetch, check, import and place the entry.
///
/// `announce` is called once, with the plan, right before the download starts, so a caller can
/// show the file name, the source and the size.
pub async fn install(
    ctx: &Ctx,
    transport: &dyn CatalogTransport,
    request: InstallRequest<'_>,
    announce: &mut dyn FnMut(&CatalogPlan),
) -> LauncherResult<InstallReport> {
    let manifest = game_instance::get_manifest(ctx, request.instance_id)
        .map_err(|e| generic("ERR_INSTANCE_NOT_FOUND", e))?;
    if request.item.game != manifest.game.as_str() {
        return Err(LauncherError::Generic {
            code: "ERR_CATALOG_WRONG_GAME".into(),
            message: format!(
                "'{}' is an entry for {}, and instance '{}' is a {} instance.",
                request.item.id,
                request.item.game,
                request.instance_id,
                manifest.game.as_str()
            ),
        });
    }
    let definition = ctx.games.game(&manifest.game).ok_or_else(|| {
        generic(
            "ERR_CATALOG_NO_GAME",
            format!("No game package defines {}.", manifest.game.as_str()),
        )
    })?;
    if definition.content_layout.is_none() {
        return Err(generic(
            "ERR_CATALOG_NO_CONTENT_LAYOUT",
            format!(
                "Game definition '{}' has no content layout, so Agora cannot place catalog content for it.",
                definition.id.as_str()
            ),
        ));
    }

    let plan = plan(
        ctx,
        &manifest,
        definition,
        request.item,
        request.instance_id,
        transport,
    )
    .await?;

    if let Some(installed) = installed_release(ctx, &manifest, &request.item.id)? {
        let same_release = installed.as_deref() == plan.source.release();
        return Ok(InstallReport {
            plan,
            state: InstallState::AlreadyInstalled {
                installed_release: installed,
                same_release,
            },
        });
    }
    if request.dry_run {
        return Ok(InstallReport {
            plan,
            state: InstallState::DryRun,
        });
    }

    announce(&plan);
    let bytes = fetch_bytes(transport, &plan.source).await?;
    let hash = check_bytes(ctx, &plan, &bytes, request.install_anyway)?;

    let scratch = ScratchDir::new(ctx)?;
    let file_path = scratch.path.join(safe_file_name(plan.source.file_name()));
    std::fs::write(&file_path, &bytes).map_err(|e| generic("ERR_CATALOG_WRITE", e))?;

    let outcome = content_store::add_archive_with_source(
        ctx,
        &file_path,
        Some(&request.item.name),
        |sha256, _display_path| ContentSource::Catalog {
            item_id: request.item.id.clone(),
            game: request.item.game.clone(),
            release: plan.source.release().map(str::to_string),
            asset: plan.source.file_name().to_string(),
            sha256: sha256.to_string(),
            verified: hash.verified,
            added_at_unix_ms: now_unix_ms(),
        },
    )
    .map_err(|e| generic("ERR_CATALOG_IMPORT", e))?;
    let content_item = outcome.item().clone();
    let size = bytes.len() as u64;

    let layout = definition
        .content_layout
        .as_ref()
        .expect("checked above: the game has a content layout");
    let state = match game_deploy::decide_placement(&content_item, layout) {
        PlacementDecision::Place {
            mount_path,
            source_path,
            ..
        } => {
            let mount = (!mount_path.is_empty()).then_some(mount_path.as_str());
            let source = (!source_path.is_empty()).then_some(source_path.as_str());
            match game_deploy::add_content(ctx, request.instance_id, &content_item.item_id, mount, source)
            {
                Ok(_) => InstallState::Placed {
                    content_item_id: content_item.item_id.clone(),
                    mount_path,
                    source_path,
                    size,
                    hash: hash.clone(),
                },
                Err(game_deploy::DeployError::ContentAlreadyPresent(_)) => {
                    InstallState::AlreadyInstalled {
                        installed_release: plan.source.release().map(str::to_string),
                        same_release: true,
                    }
                }
                Err(e) => return Err(generic("ERR_CATALOG_PLACE", e)),
            }
        }
        PlacementDecision::Installer { reason, top_level } => InstallState::NeedsInstaller {
            installer_command: format!(
                "agora games content fomod install {} --instance {}",
                content_item.item_id, request.instance_id
            ),
            content_item_id: content_item.item_id.clone(),
            size,
            hash: hash.clone(),
            reason,
            top_level,
        },
        PlacementDecision::Unknown { top_level } => {
            return Err(LauncherError::Generic {
                code: "ERR_CATALOG_UNPLACEABLE".into(),
                message: format!(
                    "Cannot determine where '{}' goes; top-level entries: {}. It is in the content store as {}. Place it with `agora games instance content add {} {} --into <path>`.",
                    request.item.id,
                    top_level.join(", "),
                    content_item.item_id,
                    request.instance_id,
                    content_item.item_id
                ),
            })
        }
    };
    Ok(InstallReport { plan, state })
}

/// The plan: the compatibility entry, the frameworks, and the file. Makes no change.
async fn plan(
    ctx: &Ctx,
    manifest: &GameInstanceManifest,
    definition: &GameDefinition,
    item: &GameCatalogItem,
    instance_id: &str,
    transport: &dyn CatalogTransport,
) -> LauncherResult<CatalogPlan> {
    let runtime = instance_runtime(manifest).ok_or_else(|| {
        generic(
            "ERR_CATALOG_NO_RUNTIME",
            format!("Instance '{instance_id}' has no runtime version, so no entry can be matched to it."),
        )
    })?;
    let (compatibility_index, compatibility) = choose_compatibility(item, &runtime)?;

    let mut warnings = Vec::new();
    let mut frameworks = Vec::new();
    for requirement in &compatibility.requires {
        let framework_id = FrameworkId::new(&requirement.framework).map_err(|e| {
            generic(
                "ERR_CATALOG_BAD_REQUIREMENT",
                format!("{}: {e}", requirement.framework),
            )
        })?;
        let definition_of = ctx
            .games
            .framework(&manifest.game, &framework_id)
            .ok_or_else(|| {
                generic(
                    "ERR_CATALOG_UNKNOWN_FRAMEWORK",
                    format!(
                        "'{}' requires framework '{}', which {} does not declare.",
                        item.id,
                        requirement.framework,
                        manifest.game.as_str()
                    ),
                )
            })?;
        let name = definition_of.name.clone();
        let status =
            game_frameworks::framework_status(ctx, instance_id, definition, definition_of)?;
        let found_version = match status {
            FrameworkStatus::Absent => {
                return Err(generic(
                    "ERR_CATALOG_FRAMEWORK_MISSING",
                    format!(
                        "'{}' needs {}{}, and instance '{instance_id}' does not have it. Install {} first.",
                        item.id,
                        name,
                        requirement
                            .min_version
                            .as_deref()
                            .map(|v| format!(" {v} or newer"))
                            .unwrap_or_default(),
                        name
                    ),
                ));
            }
            FrameworkStatus::NotDetectable => {
                return Err(generic(
                    "ERR_CATALOG_FRAMEWORK_UNDETECTABLE",
                    format!(
                        "'{}' needs {}, and Agora cannot tell whether the instance has it: the game definition declares no way to detect it.",
                        item.id, name
                    ),
                ));
            }
            FrameworkStatus::Present {
                version: Some(found),
                ..
            } => {
                if let Some(minimum) = requirement.min_version.as_deref() {
                    match game_frameworks::version_at_least(&found, minimum) {
                        Some(true) => {}
                        Some(false) => {
                            return Err(generic(
                                "ERR_CATALOG_FRAMEWORK_TOO_OLD",
                                format!(
                                    "'{}' needs {} {} or newer, and instance '{instance_id}' has {} {}. Update {} first.",
                                    item.id, name, minimum, name, found, name
                                ),
                            ));
                        }
                        None => warnings.push(format!(
                            "Could not compare {name} {found} with the {minimum} that '{}' needs.",
                            item.id
                        )),
                    }
                }
                Some(found)
            }
            FrameworkStatus::Present {
                version: None,
                unreadable,
            } => {
                warnings.push(format!(
                    "Could not read the version of {name} ({}), so it was not checked against {}. The launch check still guards a mismatched build.",
                    unreadable.unwrap_or_else(|| "unknown reason".into()),
                    requirement.min_version.as_deref().unwrap_or("any version")
                ));
                None
            }
        };
        frameworks.push(FrameworkCheck {
            framework: requirement.framework.clone(),
            name,
            min_version: requirement.min_version.clone(),
            found_version,
        });
    }

    let source = plan_source(item, &compatibility, transport).await?;
    Ok(CatalogPlan {
        item_id: item.id.clone(),
        item_name: item.name.clone(),
        instance_id: instance_id.to_string(),
        compatibility_index,
        compatibility,
        frameworks,
        source,
        warnings,
    })
}

/// The instance's runtime identity: the one it records, or its pinned base's.
fn instance_runtime(manifest: &GameInstanceManifest) -> Option<RuntimeIdentity> {
    manifest
        .runtime_identity
        .clone()
        .or_else(|| match &manifest.base {
            BaseReference::Pinned { runtime, .. } => Some(runtime.clone()),
            BaseReference::Unpinned { .. } => None,
        })
}

/// The first compatibility entry whose stores include the runtime's store and whose game versions
/// include its version.
fn choose_compatibility(
    item: &GameCatalogItem,
    runtime: &RuntimeIdentity,
) -> LauncherResult<(usize, GameCompatibility)> {
    let store = runtime.store.as_str();
    let found = item
        .game_compatibility
        .iter()
        .enumerate()
        .find(|(_, compat)| {
            compat.stores.iter().any(|s| s == store)
                && compat
                    .game_versions
                    .iter()
                    .any(|pattern| version_matches(pattern, &runtime.version))
        });
    match found {
        Some((index, compat)) => Ok((index, compat.clone())),
        None => {
            let supported: Vec<String> = item
                .game_compatibility
                .iter()
                .map(|compat| {
                    format!(
                        "{} on {}",
                        compat.game_versions.join(", "),
                        compat.stores.join("/")
                    )
                })
                .collect();
            Err(generic(
                "ERR_CATALOG_NO_COMPATIBLE_ENTRY",
                format!(
                    "'{}' supports {}. This instance is {} {} on {}, which none of those matches.",
                    item.id,
                    supported.join("; "),
                    runtime.game.as_str(),
                    runtime.version,
                    store
                ),
            ))
        }
    }
}

/// Whether `version` matches `pattern`: the same, or a `*` on whole dot-separated components.
pub fn version_matches(pattern: &str, version: &str) -> bool {
    let pattern_parts: Vec<&str> = pattern.split('.').collect();
    let version_parts: Vec<&str> = version.split('.').collect();
    pattern_parts.len() == version_parts.len()
        && pattern_parts
            .iter()
            .zip(&version_parts)
            .all(|(p, v)| *p == "*" || p == v)
}

/// The file to fetch, from the entry's first download source that core can use.
async fn plan_source(
    item: &GameCatalogItem,
    compatibility: &GameCompatibility,
    transport: &dyn CatalogTransport,
) -> LauncherResult<PlannedSource> {
    let chosen = item
        .download_sources
        .iter()
        .find(|source| matches!(source.strategy.as_str(), "github_release" | "direct_hash"));
    let Some(source) = chosen else {
        return Err(generic(
            "ERR_CATALOG_NO_SOURCE",
            format!(
                "'{}' has no download source Agora can install from.",
                item.id
            ),
        ));
    };
    match source.strategy.as_str() {
        "github_release" => github_source(item, source, compatibility, transport).await,
        _ => direct_source(item, source),
    }
}

async fn github_source(
    item: &GameCatalogItem,
    source: &DownloadSource,
    compatibility: &GameCompatibility,
    transport: &dyn CatalogTransport,
) -> LauncherResult<PlannedSource> {
    let repo = source.identifier.clone();
    let glob = compatibility.asset.clone().ok_or_else(|| {
        generic(
            "ERR_CATALOG_NO_ASSET_PATTERN",
            format!(
                "'{}' names no asset to pick for this compatibility entry.",
                item.id
            ),
        )
    })?;
    let chosen = find_newest_asset(&repo, &glob, transport).await?;
    let Some((release, asset)) = chosen else {
        return Err(generic(
            "ERR_CATALOG_NO_ASSET",
            format!(
                "No release of {repo} has a file matching '{glob}' (drafts and prereleases are not considered)."
            ),
        ));
    };
    let published_sha256 = asset
        .digest
        .as_deref()
        .map(|digest| digest.strip_prefix("sha256:").unwrap_or(digest).to_string());
    let pin = source
        .pins
        .iter()
        .find(|pin| pin.tag == release.tag_name && pin.asset == asset.name)
        .map(|pin| ConfirmableHash {
            origin: HashOrigin::CuratorPin,
            sha256: pin.sha256.clone(),
        });
    Ok(PlannedSource::GithubRelease {
        repo,
        tag: release.tag_name.clone(),
        asset: asset.name.clone(),
        download_url: asset.browser_download_url.clone(),
        published_sha256,
        size: asset.size,
        pin,
    })
}

fn direct_source(item: &GameCatalogItem, source: &DownloadSource) -> LauncherResult<PlannedSource> {
    let url = source.identifier.clone();
    let parsed = reqwest::Url::parse(&url).map_err(|_| {
        generic(
            "ERR_CATALOG_BAD_URL",
            format!("'{}' has a download URL that is not valid.", item.id),
        )
    })?;
    let file = parsed
        .path_segments()
        .and_then(|mut segments| segments.next_back())
        .filter(|segment| !segment.is_empty())
        .ok_or_else(|| {
            generic(
                "ERR_CATALOG_BAD_URL",
                format!("'{}' has a download URL with no file name.", item.id),
            )
        })?
        .to_string();
    let sha256 = item.sha256.clone().ok_or_else(|| {
        generic(
            "ERR_CATALOG_NO_HASH",
            format!(
                "'{}' is a direct download with no sha256 in its manifest.",
                item.id
            ),
        )
    })?;
    let pinned_host = parsed.host_str().unwrap_or_default().to_string();
    Ok(PlannedSource::DirectHash {
        url,
        file,
        sha256,
        pinned_host,
        size: None,
    })
}

/// The newest release, page by page, with a non-draft, non-prerelease asset matching `glob`
/// (case-insensitive). Within a page, releases are taken by `published_at`, newest first.
async fn find_newest_asset(
    repo: &str,
    glob: &str,
    transport: &dyn CatalogTransport,
) -> LauncherResult<Option<(GitHubRelease, crate::github_release::GitHubReleaseAsset)>> {
    let pattern = glob.to_ascii_lowercase();
    let mut page = 1;
    loop {
        let (mut releases, total_pages) = transport.releases_page(repo, page).await?;
        releases.sort_by(|a, b| b.published_at.cmp(&a.published_at));
        for release in releases {
            if release.draft || release.prerelease {
                continue;
            }
            let found = release
                .assets
                .iter()
                .find(|asset| {
                    agora_game_api::glob_match(&pattern, &asset.name.to_ascii_lowercase())
                })
                .cloned();
            if let Some(asset) = found {
                return Ok(Some((release, asset)));
            }
        }
        if page >= total_pages {
            return Ok(None);
        }
        page += 1;
    }
}

/// The bytes of the planned file.
async fn fetch_bytes(
    transport: &dyn CatalogTransport,
    source: &PlannedSource,
) -> LauncherResult<Vec<u8>> {
    match source {
        PlannedSource::GithubRelease { download_url, .. } => {
            transport.fetch(download_url, None).await
        }
        PlannedSource::DirectHash {
            url, pinned_host, ..
        } => transport.fetch(url, Some(pinned_host)).await,
    }
}

/// Check the bytes against what their source published, the curator's pin, and an earlier install.
fn check_bytes(
    ctx: &Ctx,
    plan: &CatalogPlan,
    bytes: &[u8],
    install_anyway: bool,
) -> LauncherResult<HashReport> {
    let published = match &plan.source {
        PlannedSource::GithubRelease {
            published_sha256, ..
        } => published_sha256.clone(),
        PlannedSource::DirectHash { sha256, .. } => Some(sha256.clone()),
    };
    let mut expectations = Vec::new();
    if let PlannedSource::GithubRelease { pin: Some(pin), .. } = &plan.source {
        expectations.push(pin.clone());
    }
    if let Some(remembered) = remembered_hash(
        ctx,
        &plan.item_id,
        plan.source.release(),
        plan.source.file_name(),
    )? {
        expectations.push(ConfirmableHash {
            origin: HashOrigin::PreviousInstall,
            sha256: remembered,
        });
    }

    let actual = crate::download::sha256_hex(bytes);
    let unmet = first_unmet(&actual, &expectations).map(|expected| expected.origin);
    let confirmed_past = if install_anyway { unmet } else { None };
    let check = verify_download(
        bytes,
        None,
        published.as_deref(),
        &expectations,
        install_anyway,
        plan.source.file_name(),
        plan.source.release(),
    )?;

    let basis = if check.verified {
        match &plan.source {
            PlannedSource::GithubRelease { .. } => {
                "verified by the SHA-256 GitHub published for this file".to_string()
            }
            PlannedSource::DirectHash { .. } => {
                "verified by the SHA-256 in the curator's manifest".to_string()
            }
        }
    } else {
        let mut basis = format!(
            "not verified: GitHub published no checksum for {}, so Agora could not verify it",
            plan.source.file_name()
        );
        if let (None, Some(origin)) = (confirmed_past, expectations.first().map(|e| e.origin)) {
            basis.push_str(&format!("; the bytes match {}", describe_origin(origin)));
        }
        basis
    };
    let basis = match confirmed_past {
        Some(origin) => format!(
            "{basis}; installed with --install-anyway although they differ from {}",
            describe_origin(origin)
        ),
        None => basis,
    };
    Ok(HashReport {
        sha256: check.sha256,
        verified: check.verified,
        basis,
        confirmed_past,
    })
}

fn describe_origin(origin: HashOrigin) -> &'static str {
    match origin {
        HashOrigin::CuratorPin => "the curator's pin for this release file",
        HashOrigin::PreviousInstall => "the hash from an earlier install of this release file",
    }
}

/// The SHA-256 recorded when this release file was installed before without a published hash.
/// An item imported from a verified source never gives an expectation.
fn remembered_hash(
    ctx: &Ctx,
    item_id: &str,
    release: Option<&str>,
    asset: &str,
) -> LauncherResult<Option<String>> {
    let items = content_store::list_items(ctx).map_err(|e| generic("ERR_CONTENT_STORE", e))?;
    for item in items {
        for source in &item.sources {
            if let ContentSource::Catalog {
                item_id: id,
                release: r,
                asset: a,
                sha256,
                verified: false,
                ..
            } = source
            {
                if id == item_id && r.as_deref() == release && a == asset {
                    return Ok(Some(sha256.clone()));
                }
            }
        }
    }
    Ok(None)
}

/// The release of an item from this entry that the instance already has, if it has one.
fn installed_release(
    ctx: &Ctx,
    manifest: &GameInstanceManifest,
    item_id: &str,
) -> LauncherResult<Option<Option<String>>> {
    for layer in manifest.layers.layers() {
        let LayerSource::Content { content } = &layer.source else {
            continue;
        };
        let Ok(content_item) = content_store::get_item(ctx, content) else {
            continue;
        };
        for source in &content_item.sources {
            if let ContentSource::Catalog {
                item_id: id,
                release,
                ..
            } = source
            {
                if id == item_id {
                    return Ok(Some(release.clone()));
                }
            }
        }
    }
    Ok(None)
}

/// A file name the download can be stored under: the original name with anything unusual
/// replaced, so a hostile name cannot leave the scratch folder.
fn safe_file_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '_'
            }
        })
        .collect();
    if cleaned.is_empty() || cleaned.starts_with('.') {
        format!("download{cleaned}")
    } else {
        cleaned
    }
}

/// A scratch folder under the content store for one download, removed when dropped.
struct ScratchDir {
    path: PathBuf,
}

impl ScratchDir {
    fn new(ctx: &Ctx) -> LauncherResult<Self> {
        let path = ctx
            .paths
            .content_root()
            .join("catalog-downloads")
            .join(format!("{}-{}", std::process::id(), now_unix_ms()));
        std::fs::create_dir_all(&path).map_err(|e| generic("ERR_CATALOG_WRITE", e))?;
        Ok(Self { path })
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn now_unix_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn generic(code: &str, error: impl std::fmt::Display) -> LauncherError {
    LauncherError::Generic {
        code: code.to_string(),
        message: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_version_pattern_matches_exactly_or_by_whole_components() {
        assert!(version_matches("1.6.1179.0", "1.6.1179.0"));
        assert!(version_matches("1.6.1170.*", "1.6.1170.0"));
        assert!(!version_matches("1.6.1170.*", "1.6.1171.0"));
        assert!(!version_matches("1.6.1170.*", "1.6.1170"));
        assert!(!version_matches("1.6.11", "1.6.1170.0"));
        assert!(!version_matches("1.6.1179.0", "1.6.1179.0.1"));
    }

    #[test]
    fn a_hostile_file_name_is_stored_under_a_safe_name() {
        assert_eq!(safe_file_name("CrashLogger-1.2.7z"), "CrashLogger-1.2.7z");
        assert_eq!(safe_file_name("../../evil.7z"), "download.._.._evil.7z");
        assert_eq!(safe_file_name(""), "download");
        assert!(!safe_file_name("a\\b:c").contains('\\'));
    }
}
