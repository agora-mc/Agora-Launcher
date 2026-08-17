//! Core-owned install plan resolver.
//!
//! Transforms an [`InstallIntent`] into a [`PreparedPlan`] with fully-resolved
//! artifacts, dependency dispositions, and conflicts — without requiring Tauri
//! or desktop types.
//!
//! The resolver handles:
//! - Curated (GitHub Release / Modrinth-id) artifact resolution
//! - Raw Modrinth (uncurated) artifact resolution
//! - Manual local file resolution
//! - Dependency BFS traversal for both curated and Modrinth dep graphs
//! - Registry known-conflict checking
//! - Artifact construction with hash specs

use crate::ctx::Ctx;
use crate::dependency_ops::{AliasMap, DepSource, Requirement};
use crate::download;
use crate::error::{LauncherError, LauncherResult};
use crate::github_ratelimit;
use crate::http_client::{self, ClientCategory, HttpClients};
use crate::install_pipeline::{
    ArtifactMetadata, ArtifactSource, ConflictKind, ConflictResolution, DepConflict,
    DepDisposition, HashAlgorithm, HashSpec, HashedValue, InstallAction, InstallIntent,
    PreparedPlan, ResolvedArtifact, ResolvedDep, ResolvedDownload, ResolvedLocal,
    ResolvedOperation, SourceType,
};
use crate::models::{InstalledMod, InstanceManifest, ModVersionCandidate};
use crate::registry::{self, ManifestDeps};
use serde::Deserialize;
use sha2::Digest;
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::path::Path;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A candidate version returned from raw Modrinth API (with dependency info).
#[derive(Debug, Clone)]
pub struct RawModrinthVersionCandidate {
    pub version: String,
    pub version_id: String,
    pub name: String,
    pub filename: String,
    pub download_url: String,
    pub sha1: Option<String>,
    pub sha512: Option<String>,
    pub size: Option<u64>,
    pub mc_versions: Vec<String>,
    pub loaders: Vec<String>,
    pub release_date: Option<String>,
    pub primary: bool,
    pub changelog: Option<String>,
    pub dependencies: Vec<RawModrinthDep>,
}

/// A dependency declared in a raw Modrinth version.
#[derive(Debug, Clone)]
pub struct RawModrinthDep {
    pub project_id: Option<String>,
    pub version_id: Option<String>,
    pub dependency_type: String,
}

/// Identities supplied by the items of a batch install.
///
/// Dependency resolution consults this set so a dependency that is also a
/// root item of the same batch is treated as satisfied by the batch itself:
/// no duplicate artifact is resolved and a stale version pin cannot turn the
/// dependency into a blocking error.
#[derive(Debug, Default)]
struct BatchContext {
    /// Lowercased item ids, registry ids, and Modrinth project ids of every
    /// root artifact in the batch.
    identities: HashSet<String>,
    /// Lowercased identity -> target filename of the batch artifact that
    /// provides it.
    target_filenames: HashMap<String, String>,
    /// Native loader identity -> Modrinth project id for a batch root. Native
    /// metadata can call TerraBlender `terrablender` while the batch item is
    /// identified by its opaque Modrinth project id.
    loader_project_ids: HashMap<String, String>,
    /// Lowercased Modrinth project identity -> original project id for batch
    /// roots. Modrinth opaque ids are case-sensitive on the API, so the
    /// original spelling must be preserved for network requests.
    project_ids: HashMap<String, String>,
}

impl BatchContext {
    fn from_artifacts<'a>(artifacts: impl IntoIterator<Item = &'a ResolvedArtifact>) -> Self {
        let mut ctx = Self::default();
        for artifact in artifacts {
            let (item_id, filename, metadata) = match artifact {
                ResolvedArtifact::Download(download) => (
                    download.item_id.as_str(),
                    download.filename.as_str(),
                    &download.metadata,
                ),
                ResolvedArtifact::LocalFile(local) => (
                    local.item_id.as_str(),
                    local.filename.as_str(),
                    &local.metadata,
                ),
            };
            let mut identities = vec![item_id.to_ascii_lowercase()];
            if let Some(registry_id) = &metadata.registry_id {
                identities.push(registry_id.to_ascii_lowercase());
            }
            if let Some(modrinth_id) = &metadata.modrinth_id {
                let key = modrinth_id.to_ascii_lowercase();
                identities.push(key.clone());
                ctx.project_ids
                    .entry(key)
                    .or_insert_with(|| modrinth_id.clone());
            }
            for identity in identities {
                ctx.identities.insert(identity.clone());
                ctx.target_filenames
                    .entry(identity)
                    .or_insert_with(|| filename.to_string());
            }
        }
        ctx
    }

    fn add_native_metadata(
        &mut self,
        artifact: &ResolvedArtifact,
        metadata: &crate::dependency_ops::JarDeps,
    ) {
        let (filename, project_id) = match artifact {
            ResolvedArtifact::Download(download) => (
                download.filename.as_str(),
                download.metadata.modrinth_id.as_deref(),
            ),
            ResolvedArtifact::LocalFile(local) => (
                local.filename.as_str(),
                local.metadata.modrinth_id.as_deref(),
            ),
        };

        for loader_id in metadata.all_mod_ids() {
            let identity = loader_id.to_ascii_lowercase();
            self.identities.insert(identity.clone());
            self.target_filenames
                .entry(identity.clone())
                .or_insert_with(|| filename.to_string());
            if let Some(project_id) = project_id {
                self.loader_project_ids
                    .entry(identity)
                    .or_insert_with(|| project_id.to_ascii_lowercase());
            }
        }
    }

    fn target_filename_for(&self, identity: &str) -> Option<&str> {
        self.target_filenames
            .get(&identity.to_ascii_lowercase())
            .map(String::as_str)
    }

    fn project_id_for_loader(&self, identity: &str) -> Option<&str> {
        self.loader_project_ids
            .get(&identity.to_ascii_lowercase())
            .map(String::as_str)
    }
}

/// Minimal project fields from the Modrinth batch projects endpoint, used to
/// hydrate dependency display names and page links.
#[derive(Deserialize)]
struct ModrinthBatchProject {
    id: String,
    title: String,
    slug: Option<String>,
    project_type: String,
}

#[derive(Deserialize)]
struct ModrinthSearchResponse {
    #[serde(default)]
    hits: Vec<ModrinthSearchHit>,
}

#[derive(Deserialize)]
struct ModrinthSearchHit {
    project_id: String,
    title: String,
    slug: String,
    project_type: String,
}

// ---------------------------------------------------------------------------
// Private Modrinth API response types (with dependency info)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct ModrinthApiVersion {
    id: String,
    name: Option<String>,
    version_number: String,
    date_published: Option<String>,
    game_versions: Option<Vec<String>>,
    loaders: Option<Vec<String>>,
    files: Vec<ModrinthApiFile>,
    #[serde(default)]
    dependencies: Vec<ModrinthApiDep>,
    #[serde(default)]
    changelog: Option<String>,
}

#[derive(Deserialize)]
struct ModrinthApiFile {
    url: String,
    filename: String,
    primary: bool,
    hashes: Option<ModrinthApiHashes>,
    #[serde(default)]
    size: Option<u64>,
}

#[derive(Deserialize)]
struct ModrinthApiHashes {
    sha1: Option<String>,
    sha512: Option<String>,
}

#[derive(Deserialize)]
struct ModrinthApiDep {
    project_id: Option<String>,
    version_id: Option<String>,
    dependency_type: Option<String>,
}

// ---------------------------------------------------------------------------
// Private GitHub Release API types
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct GitHubRelease {
    tag_name: String,
    published_at: Option<String>,
    assets: Vec<GitHubReleaseAsset>,
}

#[derive(Deserialize)]
struct GitHubReleaseAsset {
    name: String,
    #[allow(dead_code)]
    browser_download_url: String,
    #[serde(default)]
    size: Option<u64>,
    #[serde(default)]
    digest: Option<String>,
}

// ---------------------------------------------------------------------------
// Resolver
// ---------------------------------------------------------------------------

/// Core-owned resolver that prepares [`PreparedPlan`] from an [`InstallIntent`].
#[derive(Clone)]
pub struct Resolver {
    ctx: Ctx,
    github_token: Option<String>,
    clear_stored_github_token_on_unauthorized: bool,
}

impl Resolver {
    pub fn new(ctx: Ctx) -> Self {
        Self {
            ctx,
            github_token: None,
            clear_stored_github_token_on_unauthorized: false,
        }
    }

    pub fn with_github_token(mut self, token: String) -> Self {
        self.github_token = Some(token);
        self
    }

    /// Attach a token read from Agora's secure credential store. If GitHub
    /// rejects it, clear the stored credential before retrying anonymously.
    pub fn with_stored_github_token(mut self, token: String) -> Self {
        self.github_token = Some(token);
        self.clear_stored_github_token_on_unauthorized = true;
        self
    }

    // ------------------------------------------------------------------
    // Top-level dispatch
    // ------------------------------------------------------------------

    /// Resolve an intent into a [`PreparedPlan`].
    pub async fn resolve(
        &self,
        intent: &InstallIntent,
        manifest: &InstanceManifest,
    ) -> LauncherResult<PreparedPlan> {
        let revision = self.compute_registry_revision()?;

        match &intent.action {
            InstallAction::Install {
                source_type,
                item_id,
                candidate_version,
            } => match source_type {
                SourceType::Curated => {
                    self.resolve_curated_install(
                        manifest,
                        item_id,
                        candidate_version.as_deref(),
                        revision,
                        false,
                        intent.overrides.allow_closest_version,
                    )
                    .await
                }
                SourceType::Modrinth => {
                    self.resolve_raw_modrinth_install(
                        manifest,
                        item_id,
                        candidate_version.as_deref(),
                        revision,
                        false,
                        intent.overrides.allow_closest_version,
                    )
                    .await
                }
                SourceType::Manual => {
                    resolve_manual_install(item_id, candidate_version.as_deref(), revision)
                }
            },
            InstallAction::Update {
                item_id,
                target_version,
            } => {
                let installed = find_installed_by_identity(manifest, item_id).ok_or_else(|| {
                    LauncherError::Generic {
                        code: "ERR_UPDATE_TARGET_MISSING".into(),
                        message: format!("{item_id} is not installed in this instance."),
                    }
                })?;
                if installed.source == "modrinth_raw" {
                    let project_id = installed.modrinth_id.as_deref().unwrap_or(item_id);
                    self.resolve_raw_modrinth_install(
                        manifest,
                        project_id,
                        normalize_requested_version(Some(target_version)),
                        revision,
                        true,
                        intent.overrides.allow_closest_version,
                    )
                    .await
                } else {
                    let registry_id = installed.registry_id.as_deref().unwrap_or(item_id);
                    self.resolve_curated_install(
                        manifest,
                        registry_id,
                        normalize_requested_version(Some(target_version)),
                        revision,
                        true,
                        intent.overrides.allow_closest_version,
                    )
                    .await
                }
            }
            InstallAction::Remove { filename } => {
                Ok(crate::install_service::InstallService::prepare_removal(
                    manifest, filename, revision,
                ))
            }
            InstallAction::BatchRemove { filenames } => {
                let operations = filenames
                    .iter()
                    .map(|filename| {
                        crate::install_service::InstallService::prepare_removal(
                            manifest,
                            filename,
                            revision.clone(),
                        )
                        .operation
                    })
                    .collect();
                Ok(PreparedPlan {
                    operation: ResolvedOperation::BatchRemove { operations },
                    dependencies: Vec::new(),
                    conflicts: Vec::new(),
                    registry_revision: revision,
                })
            }
            InstallAction::BatchUpdate { items } => {
                self.resolve_batch_update(manifest, items, revision).await
            }
            InstallAction::BatchInstall { items } => {
                self.resolve_batch_install(
                    manifest,
                    items,
                    revision,
                    intent.overrides.allow_closest_version,
                    &intent.overrides.skip_items,
                )
                .await
            }
            InstallAction::RepairLockfile { .. } => Err(LauncherError::Generic {
                code: "ERR_LOCKFILE_COMMAND".into(),
                message: "Lockfile repair must be prepared by the verified lockfile command."
                    .into(),
            }),
        }
    }

    // ------------------------------------------------------------------
    // Registry revision
    // ------------------------------------------------------------------

    fn compute_registry_revision(&self) -> LauncherResult<String> {
        let path = self.ctx.paths.registry_db();
        if !path.is_file() {
            return Ok("registry-unavailable".into());
        }
        let bytes = std::fs::read(&path).map_err(|e| LauncherError::Generic {
            code: "ERR_REGISTRY_READ".into(),
            message: format!("Could not read registry: {e}"),
        })?;
        let mut hasher = sha2::Sha256::new();
        hasher.update(bytes);
        Ok(format!("{:x}", hasher.finalize()))
    }

    // ------------------------------------------------------------------
    // Curated install resolution
    // ------------------------------------------------------------------

    /// Resolve the artifact for a curated item at the requested version
    /// (or the best compatible version when none is requested).
    async fn resolve_curated_artifact(
        &self,
        manifest: &InstanceManifest,
        item_id: &str,
        requested_version: Option<&str>,
        allow_closest_version: bool,
    ) -> LauncherResult<ResolvedArtifact> {
        let item = {
            let conn = open_registry_db(&self.ctx.paths.registry_db())?;
            registry::get_item_by_id(&conn, item_id)?.ok_or_else(|| LauncherError::Generic {
                code: "ERR_ITEM_NOT_FOUND".into(),
                message: format!("Registry item '{item_id}' not found."),
            })?
        };

        let candidates = self
            .list_curated_versions(&item, &manifest.minecraft_version, &manifest.loader)
            .await?;
        let candidate =
            select_curated_candidate(&candidates, requested_version).or_else(|error| {
                if allow_closest_version {
                    select_closest_curated_candidate(&candidates, &manifest.minecraft_version)
                        .ok_or(error)
                } else {
                    Err(error)
                }
            })?;
        curated_artifact(&item, candidate)
    }

    async fn resolve_curated_install(
        &self,
        manifest: &InstanceManifest,
        item_id: &str,
        requested_version: Option<&str>,
        registry_revision: String,
        update: bool,
        allow_closest_version: bool,
    ) -> LauncherResult<PreparedPlan> {
        let artifact = self
            .resolve_curated_artifact(manifest, item_id, requested_version, allow_closest_version)
            .await?;
        let (dependencies, conflicts) = self
            .resolve_curated_dependencies(manifest, item_id, None)
            .await?;

        let operation = if update {
            let installed = find_installed_by_identity(manifest, item_id).ok_or_else(|| {
                LauncherError::Generic {
                    code: "ERR_UPDATE_TARGET_MISSING".into(),
                    message: format!("{item_id} is not installed."),
                }
            })?;
            ResolvedOperation::Update {
                old_version_id: installed
                    .version
                    .clone()
                    .unwrap_or_else(|| "unknown".into()),
                new_artifact: artifact,
            }
        } else {
            ResolvedOperation::Install { artifact }
        };

        Ok(PreparedPlan {
            operation,
            dependencies,
            conflicts,
            registry_revision,
        })
    }

    async fn resolve_curated_dependencies(
        &self,
        manifest: &InstanceManifest,
        root_item_id: &str,
        batch: Option<&BatchContext>,
    ) -> LauncherResult<(Vec<ResolvedDep>, Vec<DepConflict>)> {
        let (dependency_map, aliases, known_conflicts, dep_display) = {
            let conn = open_registry_db(&self.ctx.paths.registry_db())?;
            let dependency_map = registry::get_all_manifest_dependencies(&conn)?;
            let alias_pairs = registry::get_all_mod_aliases(&conn)?;
            let known_conflicts = registry::get_known_conflicts(&conn)?;
            // Prefetch every manifest-declared item so dependency rows can
            // show real project names and page links instead of raw ids.
            let mut item_ids: Vec<String> = dependency_map
                .values()
                .flat_map(|deps| {
                    deps.required
                        .iter()
                        .chain(deps.optional.iter())
                        .chain(deps.incompatible.iter())
                        .cloned()
                })
                .collect();
            item_ids.push(root_item_id.to_string());
            let items = registry::get_items_by_ids(&conn, &item_ids).unwrap_or_default();
            let dep_display: HashMap<String, (String, Option<String>)> = items
                .into_iter()
                .map(|(id, item)| (id.to_ascii_lowercase(), (item.name, item.page_url)))
                .collect();
            (
                dependency_map,
                AliasMap::from_pairs(&alias_pairs),
                known_conflicts,
                dep_display,
            )
        };

        let installed: Vec<&InstalledMod> = all_installed(manifest).collect();
        let installed_ids: BTreeMap<String, &&InstalledMod> = installed
            .iter()
            .flat_map(|item| {
                let ids: [Option<&str>; 3] = [
                    item.registry_id.as_deref(),
                    item.modrinth_id.as_deref(),
                    item.mod_jar_id.as_deref(),
                ];
                ids.into_iter()
                    .flatten()
                    .map(|id| (aliases.resolve_or_self(id).to_ascii_lowercase(), item))
                    .collect::<Vec<_>>()
            })
            .collect();

        let mut queue = VecDeque::new();
        if let Some(root) = dependency_map.get(root_item_id) {
            enqueue_manifest_deps(&mut queue, root);
        }
        let mut resolved = BTreeMap::<String, ResolvedDep>::new();
        let mut expanded = HashSet::new();

        while let Some((raw_id, requirement)) = queue.pop_front() {
            let canonical = aliases.resolve_or_self(&raw_id);
            let key = canonical.to_ascii_lowercase();
            if let Some(existing) = resolved.get_mut(&key) {
                if requirement == Requirement::Required {
                    existing.requirement = Requirement::Required;
                }
                continue;
            }
            let display = dep_display.get(&key).cloned();
            let display_name = display.as_ref().map(|(name, _)| name.clone());
            let page_url = display.as_ref().and_then(|(_, url)| url.clone());
            if let Some(installed) = installed_ids.get(&key) {
                resolved.insert(
                    key,
                    ResolvedDep {
                        mod_jar_id: canonical,
                        requirement,
                        source: DepSource::Manifest,
                        display_name: display_name
                            .or_else(|| Some(effective_installed_filename(installed).to_string())),
                        page_url,
                        disposition: DepDisposition::ReuseExisting {
                            mod_jar_id: installed
                                .mod_jar_id
                                .clone()
                                .unwrap_or_else(|| raw_id.clone()),
                            installed_filename: effective_installed_filename(installed),
                        },
                    },
                );
                continue;
            }
            if is_platform_dependency(&key, &manifest.loader) {
                resolved.insert(
                    key,
                    ResolvedDep {
                        mod_jar_id: canonical,
                        requirement,
                        source: DepSource::Manifest,
                        display_name: None,
                        page_url: None,
                        disposition: DepDisposition::ReuseExisting {
                            mod_jar_id: raw_id,
                            installed_filename: format!("provided by {} loader", manifest.loader),
                        },
                    },
                );
                continue;
            }
            if let Some(batch) = batch {
                if let Some(target_filename) = batch.target_filename_for(&key) {
                    // Another root item of this batch provides the dependency:
                    // its own operation installs the artifact, so mark the
                    // dependency satisfied without adding a duplicate file.
                    resolved.insert(
                        key,
                        ResolvedDep {
                            mod_jar_id: canonical,
                            requirement,
                            source: DepSource::Manifest,
                            display_name,
                            page_url,
                            disposition: DepDisposition::IncludedInBatch {
                                target_filename: target_filename.to_string(),
                            },
                        },
                    );
                    continue;
                }
            }

            let disposition = self.load_curated_dep(&canonical, manifest).await;
            resolved.insert(
                key.clone(),
                ResolvedDep {
                    mod_jar_id: canonical.clone(),
                    requirement,
                    source: DepSource::Manifest,
                    display_name,
                    page_url,
                    disposition,
                },
            );
            if expanded.insert(key) {
                if let Some(child) = dependency_map.get(&canonical) {
                    enqueue_manifest_deps(&mut queue, child);
                }
            }
        }

        let incoming: HashSet<String> = std::iter::once(root_item_id.to_ascii_lowercase())
            .chain(resolved.keys().cloned())
            .collect();
        let installed_set: HashSet<String> = installed_ids.keys().cloned().collect();
        let conflicts =
            build_known_conflicts(&known_conflicts, &aliases, &incoming, &installed_set);

        Ok((resolved.into_values().collect(), conflicts))
    }

    async fn load_curated_dep(&self, item_id: &str, manifest: &InstanceManifest) -> DepDisposition {
        let item = {
            let conn = match open_registry_db(&self.ctx.paths.registry_db()) {
                Ok(c) => c,
                Err(e) => {
                    return DepDisposition::Unresolved {
                        reason: e.to_string(),
                    }
                }
            };
            match registry::get_item_by_id(&conn, item_id) {
                Ok(Some(item)) => item,
                Ok(None) => {
                    return DepDisposition::Unresolved {
                        reason: format!("Registry item '{item_id}' not found."),
                    }
                }
                Err(e) => {
                    return DepDisposition::Unresolved {
                        reason: e.to_string(),
                    }
                }
            }
        };

        let candidates = match self
            .list_curated_versions(&item, &manifest.minecraft_version, &manifest.loader)
            .await
        {
            Ok(c) => c,
            Err(e) => {
                return DepDisposition::Unresolved {
                    reason: e.to_string(),
                }
            }
        };

        match select_curated_candidate(&candidates, None) {
            Ok(candidate) => match curated_artifact(&item, candidate) {
                Ok(artifact) => DepDisposition::InstallCandidate { artifact },
                Err(e) => DepDisposition::Unresolved {
                    reason: e.to_string(),
                },
            },
            Err(e) => DepDisposition::Unresolved {
                reason: e.to_string(),
            },
        }
    }

    // ------------------------------------------------------------------
    // GitHub Releases version list
    // ------------------------------------------------------------------

    /// Compute which tail pages to fetch after page 1 for the bi-directional
    /// initial fetch heuristic.
    ///
    /// When page 1 has no compatible candidates and there are multiple pages,
    /// returns up to 3 oldest pages (highest page numbers) that are most
    /// likely to contain versions matching an older MC version.
    pub fn compute_tail_pages(total_pages: u32, page1_has_compatible: bool) -> Vec<u32> {
        if total_pages <= 1 || page1_has_compatible {
            return vec![];
        }
        let mut pages: Vec<u32> = (2..=total_pages).rev().collect();
        pages.truncate(3);
        pages
    }

    /// Bi-directional initial fetch: page 1 + tail pages via core Resolver.
    ///
    /// Fetches the first page (newest releases). If no compatible candidate
    /// is found and more pages exist, also fetches the last few pages
    /// (oldest releases) which are most likely to match older MC versions.
    /// Results are sorted by compatibility then release date.
    pub async fn fetch_github_releases_initial(
        &self,
        source: &str,
        mc_version: &str,
        loader: &str,
    ) -> LauncherResult<(Vec<ModVersionCandidate>, u32, Vec<u32>)> {
        let (page1, total_pages) = self
            .fetch_github_releases_page(source, mc_version, loader, 1)
            .await?;
        let mut all = page1;
        let mut pages_fetched = vec![1u32];
        let page1_has_compatible = all.iter().any(|c| c.is_compatible);
        let tail = Self::compute_tail_pages(total_pages, page1_has_compatible);
        for &p in &tail {
            if let Ok((cands, _)) = self
                .fetch_github_releases_page(source, mc_version, loader, p)
                .await
            {
                pages_fetched.push(p);
                all.extend(cands);
            }
        }
        sort_versions_by_compatibility(&mut all);
        Ok((all, total_pages, pages_fetched))
    }

    /// Batch-fetch specific GitHub pages concurrently.
    ///
    /// Results preserve page order from the input slice. Individual page
    /// failures are tolerated and skipped (only success responses are
    /// returned).
    pub async fn fetch_github_versions_batch(
        &self,
        source: &str,
        mc_version: &str,
        loader: &str,
        pages: &[u32],
    ) -> LauncherResult<Vec<(u32, Vec<ModVersionCandidate>)>> {
        let mut handles = Vec::new();
        for &p in pages {
            let mc = mc_version.to_owned();
            let ld = loader.to_owned();
            let src = source.to_owned();
            let resolver = self.clone();
            handles.push(tokio::spawn(async move {
                resolver
                    .fetch_github_releases_page(&src, &mc, &ld, p)
                    .await
                    .map(|(c, _)| (p, c))
                    .map_err(|e| e.to_string())
            }));
        }
        let mut results = Vec::new();
        for handle in handles {
            match handle.await {
                Ok(Ok((p, cands))) => results.push((p, cands)),
                Ok(Err(e)) => {
                    eprintln!("fetch_github_versions_batch: page failed: {e}");
                }
                Err(e) => {
                    eprintln!("fetch_github_versions_batch: task join failed: {e}");
                }
            }
        }
        Ok(results)
    }

    /// List curated versions for a registry item, filtered by MC version and loader.
    pub async fn list_curated_versions(
        &self,
        item: &crate::registry::RegistryItem,
        mc_version: &str,
        loader: &str,
    ) -> LauncherResult<Vec<ModVersionCandidate>> {
        let has_modrinth = item.modrinth_id.as_deref().is_some_and(|id| !id.is_empty());

        match item.download_strategy.as_str() {
            "github_release" => {
                let primary = self
                    .fetch_all_github_releases(&item.source_identifier, mc_version, loader)
                    .await;
                let candidates = match primary {
                    Ok(c) if !c.is_empty() => return Ok(c),
                    Ok(_) => Vec::new(),
                    Err(_) => Vec::new(),
                };
                if has_modrinth {
                    let alt = fetch_modrinth_versions_for_item(
                        &self.ctx.http_clients,
                        &item.source_identifier,
                        item.modrinth_id.as_deref(),
                        mc_version,
                        loader,
                    )
                    .await?;
                    if !alt.is_empty() {
                        return Ok(alt);
                    }
                }
                Ok(candidates)
            }
            "modrinth_id" => {
                fetch_modrinth_versions_for_item(
                    &self.ctx.http_clients,
                    &item.source_identifier,
                    None,
                    mc_version,
                    loader,
                )
                .await
            }
            // Fully hand-curated: no API call, the signed manifest is the
            // source of truth. A Modrinth mirror, when the curator declared
            // one, is still an acceptable fallback if the pinned host is down —
            // the artifact is SHA-256-verified either way.
            "direct_hash" => match direct_hash_versions_for_item(item, mc_version, loader) {
                Ok(candidates) if !candidates.is_empty() => Ok(candidates),
                Ok(_) | Err(_) if has_modrinth => {
                    fetch_modrinth_versions_for_item(
                        &self.ctx.http_clients,
                        &item.source_identifier,
                        item.modrinth_id.as_deref(),
                        mc_version,
                        loader,
                    )
                    .await
                }
                other => other,
            },
            _ => Err(LauncherError::Generic {
                code: "ERR_UNSUPPORTED_STRATEGY".into(),
                message: format!(
                    "Download strategy '{}' is not supported for version resolution.",
                    item.download_strategy
                ),
            }),
        }
    }

    /// Resolve candidates for an automatic update check without walking the
    /// complete GitHub release history. The interactive Versions tab can load
    /// older pages on demand; background checks use page one plus up to three
    /// tail pages instead.
    pub async fn list_curated_versions_for_update(
        &self,
        item: &crate::registry::RegistryItem,
        mc_version: &str,
        loader: &str,
    ) -> LauncherResult<Vec<ModVersionCandidate>> {
        if item.download_strategy != "github_release" {
            return self.list_curated_versions(item, mc_version, loader).await;
        }

        let has_modrinth = item.modrinth_id.as_deref().is_some_and(|id| !id.is_empty());
        let primary = match self
            .fetch_github_releases_initial(&item.source_identifier, mc_version, loader)
            .await
        {
            Ok((candidates, _, _)) => candidates,
            Err(_) => Vec::new(),
        };
        if !primary.is_empty() {
            return Ok(primary);
        }
        if has_modrinth {
            return fetch_modrinth_versions_for_item(
                &self.ctx.http_clients,
                &item.source_identifier,
                item.modrinth_id.as_deref(),
                mc_version,
                loader,
            )
            .await;
        }
        Ok(primary)
    }

    /// Fetch all GitHub release pages for a source, returning candidates filtered
    /// and sorted by compatibility.
    pub async fn fetch_all_github_releases(
        &self,
        source: &str,
        mc_version: &str,
        loader: &str,
    ) -> LauncherResult<Vec<ModVersionCandidate>> {
        let mut all = Vec::new();
        let mut page: u32 = 1;
        let max_pages = 50;
        let mut total_pages = 1;

        loop {
            if page > max_pages || page > total_pages {
                break;
            }
            match self
                .fetch_github_releases_page(source, mc_version, loader, page)
                .await
            {
                Ok((candidates, reported_total_pages)) => {
                    all.extend(candidates);
                    total_pages = reported_total_pages.max(page);
                    if page >= total_pages {
                        break;
                    }
                }
                Err(_) => {
                    break;
                }
            }
            page += 1;
        }

        sort_versions_by_compatibility(&mut all);
        Ok(all)
    }

    /// Fetch a single page of GitHub releases.
    pub async fn fetch_github_releases_page(
        &self,
        source: &str,
        mc_version: &str,
        loader: &str,
        page: u32,
    ) -> LauncherResult<(Vec<ModVersionCandidate>, u32)> {
        let url =
            format!("https://api.github.com/repos/{source}/releases?per_page=100&page={page}");

        let headers = github_auth_headers(self.github_token.as_deref());
        let mut response = self.send_github_releases_request(&url, &headers).await?;

        // Release listings are public. A stale or malformed stored token must
        // not turn a public request into a hard failure, and must not be
        // retried with the same invalid Authorization header.
        if response.status() == reqwest::StatusCode::UNAUTHORIZED && self.github_token.is_some() {
            if self.clear_stored_github_token_on_unauthorized {
                // Attempt a single token refresh before falling back to anonymous.
                if crate::auth::try_refresh_after_401_with_token(
                    self.github_token.as_deref().unwrap_or_default(),
                )
                .await
                .is_ok()
                {
                    if let Some(new_token) = crate::auth::get_valid_access_token().await {
                        let new_headers = github_auth_headers(Some(&new_token));
                        response = self
                            .send_github_releases_request(&url, &new_headers)
                            .await?;
                    } else {
                        response = self.send_github_releases_request(&url, &[]).await?;
                    }
                } else {
                    let _ = crate::auth::clear_token();
                    response = self.send_github_releases_request(&url, &[]).await?;
                }
            } else {
                response = self.send_github_releases_request(&url, &[]).await?;
            }
        }

        if github_ratelimit::is_rate_limit_response(&response) {
            let retry = github_ratelimit::parse_retry_after(&response);
            github_ratelimit::report_rate_limit(retry).await;
            return Err(LauncherError::Generic {
                code: "ERR_RATE_LIMITED".into(),
                message: format!("GitHub rate limit hit while fetching releases for {source}."),
            });
        }

        let link_value = response
            .headers()
            .get("link")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());

        let releases: Vec<GitHubRelease> = response
            .error_for_status()
            .map_err(|e| LauncherError::Generic {
                code: "ERR_NETWORK".into(),
                message: format!("GitHub API request failed: {e}"),
            })?
            .json()
            .await
            .map_err(|_| LauncherError::Generic {
                code: "ERR_NETWORK".into(),
                message: "Failed to parse GitHub releases response.".into(),
            })?;

        let total_pages = parse_link_total_pages(link_value.as_deref());
        let mut candidates: Vec<ModVersionCandidate> = Vec::new();

        for release in &releases {
            for asset in &release.assets {
                if !is_installable_github_asset(&asset.name) {
                    continue;
                }
                let (found_mc, loader_str, compat) = parse_version_from_github_asset(
                    &asset.name,
                    &release.tag_name,
                    mc_version,
                    loader,
                );

                let download_url = format!(
                    "https://github.com/{source}/releases/download/{tag}/{asset_name}",
                    tag = urlencoding::encode(&release.tag_name),
                    asset_name = asset.name,
                );

                candidates.push(ModVersionCandidate {
                    version: release.tag_name.clone(),
                    filename: asset.name.clone(),
                    download_url,
                    mc_version: found_mc,
                    loader: loader_str,
                    release_date: release.published_at.clone(),
                    is_compatible: compat == "compatible",
                    version_compat: compat.to_string(),
                    sha1: None,
                    sha256: asset
                        .digest
                        .as_deref()
                        .and_then(|digest| digest.strip_prefix("sha256:"))
                        .map(str::to_string),
                    sha512: None,
                    size: asset.size,
                });
            }
        }

        Ok((candidates, total_pages))
    }

    async fn send_github_releases_request(
        &self,
        url: &str,
        headers: &[(String, String)],
    ) -> LauncherResult<reqwest::Response> {
        let _permit = crate::github_ratelimit::acquire_github_permit().await;
        crate::http_client::checked_send(
            &self.ctx.http_clients,
            ClientCategory::GitHub,
            reqwest::Method::GET,
            url,
            headers,
            None,
            None,
        )
        .await
    }

    // ------------------------------------------------------------------
    // Raw Modrinth install resolution
    // ------------------------------------------------------------------

    /// Resolve the artifact for a raw Modrinth project at the requested
    /// version (or the newest compatible version when none is requested).
    /// Returns the selected candidate alongside the artifact so callers can
    /// run native-metadata and dependency traversal without re-listing.
    async fn resolve_raw_modrinth_artifact(
        &self,
        manifest: &InstanceManifest,
        project_id: &str,
        requested_version: Option<&str>,
        allow_closest_version: bool,
    ) -> LauncherResult<(RawModrinthVersionCandidate, ResolvedArtifact)> {
        let mut candidates = self
            .list_raw_modrinth_versions(manifest, project_id)
            .await?;
        let mut used_closest_candidates = false;
        if allow_closest_version {
            let requested_found =
                normalize_requested_version(requested_version).is_some_and(|requested| {
                    candidates.iter().any(|candidate| {
                        candidate.version_id == requested
                            || candidate.version == requested
                            || candidate.filename == requested
                    })
                });
            if candidates.is_empty() || (requested_version.is_some() && !requested_found) {
                if let Ok(fallback) = self
                    .list_raw_modrinth_versions_closest(manifest, project_id)
                    .await
                {
                    if !fallback.is_empty() {
                        candidates = fallback;
                        used_closest_candidates = true;
                    }
                }
            }
        }
        let candidate =
            select_raw_modrinth_candidate(&candidates, requested_version).or_else(|error| {
                if allow_closest_version && used_closest_candidates {
                    select_closest_raw_modrinth_candidate(&candidates, &manifest.minecraft_version)
                        .ok_or(error)
                } else if allow_closest_version {
                    candidates.first().ok_or(error)
                } else {
                    Err(error)
                }
            })?;
        let artifact = raw_modrinth_artifact(project_id, candidate)?;
        Ok((candidate.clone(), artifact))
    }

    async fn resolve_raw_modrinth_install(
        &self,
        manifest: &InstanceManifest,
        project_id: &str,
        requested_version: Option<&str>,
        registry_revision: String,
        update: bool,
        allow_closest_version: bool,
    ) -> LauncherResult<PreparedPlan> {
        let (candidate, artifact) = self
            .resolve_raw_modrinth_artifact(
                manifest,
                project_id,
                requested_version,
                allow_closest_version,
            )
            .await?;
        // A multi-loader Modrinth version may advertise compatibility-route
        // dependencies (for example Connector for NeoForge) at the version
        // level. Once the verified JAR is available, its active-loader-native
        // metadata is the authoritative dependency source.
        let native_metadata = self.native_loader_metadata(manifest, &candidate).await;
        let dependencies = self
            .resolve_raw_modrinth_deps(manifest, &candidate, native_metadata.as_ref(), None, false)
            .await;

        let operation = if update {
            let installed = find_installed_by_identity(manifest, project_id).ok_or_else(|| {
                LauncherError::Generic {
                    code: "ERR_UPDATE_TARGET_MISSING".into(),
                    message: format!("{project_id} is not installed."),
                }
            })?;
            ResolvedOperation::Update {
                old_version_id: installed
                    .version
                    .clone()
                    .unwrap_or_else(|| "unknown".into()),
                new_artifact: artifact,
            }
        } else {
            ResolvedOperation::Install { artifact }
        };

        Ok(PreparedPlan {
            operation,
            dependencies,
            conflicts: Vec::new(),
            registry_revision,
        })
    }

    /// List raw Modrinth versions for a project, filtered by MC version and loader.
    pub async fn list_raw_modrinth_versions(
        &self,
        manifest: &InstanceManifest,
        project_id: &str,
    ) -> LauncherResult<Vec<RawModrinthVersionCandidate>> {
        let url = format!(
            "https://api.modrinth.com/v2/project/{pid}/version?game_versions=[\"{gv}\"]&loaders=[\"{ld}\"]",
            pid = urlencoding::encode(project_id),
            gv = urlencoding::encode(&manifest.minecraft_version),
            ld = urlencoding::encode(&manifest.loader),
        );
        self.fetch_raw_modrinth_versions_url(&url).await
    }

    /// Fetch loader-compatible versions without restricting Minecraft version.
    /// This is only used after an explicit closest-version opt-in so a normal
    /// resolution remains strict about the target instance version.
    async fn list_raw_modrinth_versions_closest(
        &self,
        manifest: &InstanceManifest,
        project_id: &str,
    ) -> LauncherResult<Vec<RawModrinthVersionCandidate>> {
        let url = format!(
            "https://api.modrinth.com/v2/project/{pid}/version?loaders=[\"{ld}\"]",
            pid = urlencoding::encode(project_id),
            ld = urlencoding::encode(&manifest.loader),
        );
        self.fetch_raw_modrinth_versions_url(&url).await
    }

    async fn fetch_raw_modrinth_versions_url(
        &self,
        url: &str,
    ) -> LauncherResult<Vec<RawModrinthVersionCandidate>> {
        let versions: Vec<ModrinthApiVersion> =
            http_client::checked_get_json(&self.ctx.http_clients, ClientCategory::Modrinth, url)
                .await?;

        Ok(versions
            .into_iter()
            .map(|v| {
                let primary_file = v
                    .files
                    .iter()
                    .find(|f| f.primary)
                    .or_else(|| v.files.first());
                let (filename, download_url, sha1, file_size) = match primary_file {
                    Some(f) => (
                        f.filename.clone(),
                        f.url.clone(),
                        f.hashes.as_ref().and_then(|h| h.sha1.clone()),
                        f.size,
                    ),
                    None => (String::new(), String::new(), None, None),
                };
                RawModrinthVersionCandidate {
                    version: v.version_number,
                    version_id: v.id,
                    name: v.name.unwrap_or_default(),
                    filename,
                    download_url,
                    sha1,
                    sha512: primary_file
                        .and_then(|f| f.hashes.as_ref())
                        .and_then(|h| h.sha512.clone()),
                    size: file_size,
                    mc_versions: v.game_versions.unwrap_or_default(),
                    loaders: v
                        .loaders
                        .unwrap_or_default()
                        .into_iter()
                        .map(|l| l.to_lowercase())
                        .collect(),
                    release_date: v.date_published,
                    primary: primary_file.map(|f| f.primary).unwrap_or(false),
                    changelog: v.changelog,
                    dependencies: v
                        .dependencies
                        .into_iter()
                        .filter_map(|d| {
                            d.dependency_type.map(|dt| RawModrinthDep {
                                project_id: d.project_id,
                                version_id: d.version_id,
                                dependency_type: dt,
                            })
                        })
                        .collect(),
                }
            })
            .filter(|c| !c.download_url.is_empty())
            .collect())
    }

    /// Download and verify a selected Modrinth artifact solely to inspect its
    /// active-loader metadata during dependency resolution. A failed download
    /// or missing/mismatched API hash deliberately falls back to the Modrinth
    /// version dependency list rather than inventing a dependency result.
    async fn native_loader_metadata(
        &self,
        manifest: &InstanceManifest,
        candidate: &RawModrinthVersionCandidate,
    ) -> Option<crate::dependency_ops::JarDeps> {
        let expected_sha1 = candidate.sha1.as_deref()?.trim();
        if expected_sha1.is_empty() {
            return None;
        }
        let bytes =
            crate::download::download_mod_bytes(&self.ctx.http_clients, &candidate.download_url)
                .await
                .ok()?;
        if !crate::download::sha1_hex(&bytes).eq_ignore_ascii_case(expected_sha1) {
            return None;
        }

        let parsed =
            crate::jar_metadata::parse_jar_metadata_bytes_for_loader(&bytes, &manifest.loader);
        parsed.has_native_metadata.then_some(parsed.metadata)
    }

    fn native_dependency_project_mappings(
        &self,
        metadata: &crate::dependency_ops::JarDeps,
    ) -> std::collections::HashMap<String, String> {
        let mut mappings = std::collections::HashMap::new();
        let Ok(connection) = open_registry_db(&self.ctx.paths.registry_db()) else {
            return mappings;
        };
        let aliases = registry::get_all_mod_aliases(&connection)
            .map(|pairs| AliasMap::from_pairs(&pairs))
            .unwrap_or_else(|_| AliasMap::from_pairs(&[]));
        for loader_id in metadata
            .depends_on
            .iter()
            .chain(metadata.optional_deps.iter())
        {
            let registry_id = aliases.resolve_or_self(loader_id);
            let Some(project_id) = registry::get_item_by_id(&connection, &registry_id)
                .ok()
                .flatten()
                .and_then(|item| item.modrinth_id)
                .filter(|id| !id.trim().is_empty())
            else {
                continue;
            };
            mappings.insert(loader_id.to_ascii_lowercase(), project_id);
        }
        mappings
    }

    async fn resolve_raw_modrinth_deps(
        &self,
        manifest: &InstanceManifest,
        root: &RawModrinthVersionCandidate,
        root_native_metadata: Option<&crate::dependency_ops::JarDeps>,
        batch: Option<&BatchContext>,
        allow_closest_version: bool,
    ) -> Vec<ResolvedDep> {
        let installed_ids: HashSet<String> = all_installed(manifest)
            .filter_map(|item| item.modrinth_id.as_ref())
            .map(|id| id.to_ascii_lowercase())
            .collect();
        let installed_loader_ids: HashSet<String> = all_installed(manifest)
            .flat_map(|item| {
                item.mod_jar_id
                    .iter()
                    .chain(item.provided_mod_ids.iter())
                    .map(|id| id.to_ascii_lowercase())
                    .collect::<Vec<_>>()
            })
            .collect();

        let mut native_project_mappings = root_native_metadata
            .map(|metadata| self.native_dependency_project_mappings(metadata))
            .unwrap_or_default();
        if let Some(batch) = batch {
            for (loader_id, project_id) in &batch.loader_project_ids {
                native_project_mappings
                    .entry(loader_id.clone())
                    .or_insert_with(|| project_id.clone());
            }
        }
        let mut native_project_lookup_attempted = HashSet::new();
        let mut queue = VecDeque::new();
        for dep in effective_raw_modrinth_dependencies(
            root,
            root_native_metadata,
            &native_project_mappings,
        ) {
            let requirement = match dep.dependency_type.as_str() {
                "required" => Requirement::Required,
                "optional" => Requirement::Optional,
                _ => continue,
            };
            queue.push_back((dep.project_id.clone(), dep.version_id.clone(), requirement));
        }

        let mut expanded = BTreeMap::<String, Requirement>::new();
        let mut resolved = BTreeMap::<String, ResolvedDep>::new();
        // Every project id encountered during the traversal, used to hydrate
        // display names and page links in one batched Modrinth request.
        let mut project_ids = HashMap::<String, String>::new();

        while let Some((project_id, version_id, requirement)) = queue.pop_front() {
            let Some(pid) = project_id else {
                let identity = version_id.unwrap_or_else(|| "unknown-version".into());
                if let Some(loader_id) = identity.strip_prefix("loader-id:") {
                    let loader_key = loader_id.to_ascii_lowercase();
                    if installed_loader_ids.contains(&loader_key) {
                        let installed = all_installed(manifest).find(|item| {
                            item.mod_jar_id
                                .iter()
                                .chain(item.provided_mod_ids.iter())
                                .any(|id| id.eq_ignore_ascii_case(loader_id))
                        });
                        resolved.insert(
                            loader_key,
                            ResolvedDep {
                                mod_jar_id: loader_id.to_string(),
                                requirement,
                                source: DepSource::Jar,
                                display_name: None,
                                page_url: None,
                                disposition: DepDisposition::ReuseExisting {
                                    mod_jar_id: loader_id.to_string(),
                                    installed_filename: installed
                                        .map(effective_installed_filename)
                                        .unwrap_or_else(|| "installed".into()),
                                },
                            },
                        );
                        continue;
                    } else if let Some(batch) = batch {
                        if let Some(target_filename) = batch.target_filename_for(loader_id) {
                            // Native metadata identifies a sibling by its
                            // loader id (for example `terrablender`), while
                            // the batch root is usually identified by an
                            // opaque Modrinth project id.
                            resolved.insert(
                                loader_key,
                                ResolvedDep {
                                    mod_jar_id: loader_id.to_string(),
                                    requirement,
                                    source: DepSource::Jar,
                                    display_name: None,
                                    page_url: None,
                                    disposition: DepDisposition::IncludedInBatch {
                                        target_filename: target_filename.to_string(),
                                    },
                                },
                            );
                            continue;
                        }
                    }
                    if let Some(project_id) = native_project_mappings.get(&loader_key).cloned() {
                        queue.push_front((Some(project_id), None, requirement));
                        continue;
                    }
                    if native_project_lookup_attempted.insert(loader_key.clone()) {
                        if let Some(project_id) =
                            self.lookup_modrinth_project_for_loader(loader_id).await
                        {
                            native_project_mappings.insert(loader_key.clone(), project_id.clone());
                            queue.push_front((Some(project_id), None, requirement));
                            continue;
                        }
                    }
                    resolved.insert(
                        loader_key,
                        ResolvedDep {
                            mod_jar_id: loader_id.to_string(),
                            requirement,
                            source: DepSource::Jar,
                            display_name: None,
                            page_url: None,
                            disposition: DepDisposition::Unresolved {
                                reason: format!(
                                    "No Modrinth project or enabled artifact provides loader capability '{loader_id}'."
                                ),
                            },
                        },
                    );
                    continue;
                }
                resolved.insert(
                    identity.clone(),
                    ResolvedDep {
                        mod_jar_id: identity,
                        requirement,
                        source: DepSource::Manifest,
                        display_name: None,
                        page_url: None,
                        disposition: DepDisposition::Unresolved {
                            reason: "Modrinth dependency omitted its project ID.".into(),
                        },
                    },
                );
                continue;
            };
            let key = pid.to_ascii_lowercase();
            project_ids
                .entry(key.clone())
                .or_insert_with(|| pid.clone());
            let should_expand = match expanded.get(&key) {
                Some(Requirement::Required) => false,
                Some(Requirement::Optional) if requirement == Requirement::Optional => false,
                _ => true,
            };
            if !should_expand {
                if requirement == Requirement::Required {
                    if let Some(existing) = resolved.get_mut(&key) {
                        existing.requirement = Requirement::Required;
                    }
                }
                continue;
            }
            expanded.insert(key.clone(), requirement);

            if installed_ids.contains(&key) {
                let installed = all_installed(manifest).find(|item| {
                    item.modrinth_id
                        .as_deref()
                        .map(str::to_ascii_lowercase)
                        .as_deref()
                        == Some(key.as_str())
                });
                resolved.insert(
                    key,
                    ResolvedDep {
                        mod_jar_id: pid.clone(),
                        requirement,
                        source: DepSource::Manifest,
                        display_name: None,
                        page_url: None,
                        disposition: DepDisposition::ReuseExisting {
                            mod_jar_id: pid,
                            installed_filename: installed
                                .map(effective_installed_filename)
                                .unwrap_or_else(|| "installed".into()),
                        },
                    },
                );
                continue;
            }
            if let Some(batch) = batch {
                if let Some(target_filename) = batch.target_filename_for(&key) {
                    // Another root item of this batch installs the dependency,
                    // so the batch's own artifact satisfies it. Skipping the
                    // pin lookup also prevents a stale pinned version from
                    // blocking the whole install.
                    resolved.insert(
                        key,
                        ResolvedDep {
                            mod_jar_id: pid.clone(),
                            requirement,
                            source: DepSource::Manifest,
                            display_name: None,
                            page_url: None,
                            disposition: DepDisposition::IncludedInBatch {
                                target_filename: target_filename.to_string(),
                            },
                        },
                    );
                    continue;
                }
            }

            let candidates = match self.list_raw_modrinth_versions(manifest, &pid).await {
                Ok(mut candidates) => {
                    if allow_closest_version && candidates.is_empty() {
                        if let Ok(fallback) = self
                            .list_raw_modrinth_versions_closest(manifest, &pid)
                            .await
                        {
                            candidates = fallback;
                        }
                    }
                    Some(candidates)
                }
                Err(_) => None,
            };
            let (disposition, child_deps) = match candidates {
                Some(candidates) => {
                    // A pinned dependency version may be filtered out by the
                    // instance's MC/loader filters even though the project has
                    // compatible releases; fall back to the best available
                    // candidate so a stale pin cannot block the install.
                    let selected =
                        select_raw_modrinth_candidate(&candidates, version_id.as_deref()).or_else(
                            |pinned_error| {
                                if version_id.is_some() {
                                    select_raw_modrinth_candidate(&candidates, None)
                                        .map_err(|_| pinned_error)
                                } else {
                                    Err(pinned_error)
                                }
                            },
                        );
                    match selected {
                        Ok(candidate) => {
                            let native_metadata =
                                self.native_loader_metadata(manifest, candidate).await;
                            if let Some(native_metadata) = native_metadata.as_ref() {
                                for provided_id in native_metadata.all_mod_ids() {
                                    native_project_mappings
                                        .entry(provided_id.to_ascii_lowercase())
                                        .or_insert_with(|| pid.clone());
                                }
                            }
                            let child_mappings = native_metadata
                                .as_ref()
                                .map(|metadata| self.native_dependency_project_mappings(metadata))
                                .map(|mut mappings| {
                                    for (loader_id, project_id) in &native_project_mappings {
                                        mappings
                                            .entry(loader_id.clone())
                                            .or_insert_with(|| project_id.clone());
                                    }
                                    mappings
                                })
                                .unwrap_or_else(|| native_project_mappings.clone());
                            let children = effective_raw_modrinth_dependencies(
                                candidate,
                                native_metadata.as_ref(),
                                &child_mappings,
                            );
                            match raw_modrinth_artifact(&pid, candidate) {
                                Ok(artifact) => {
                                    (DepDisposition::InstallCandidate { artifact }, children)
                                }
                                Err(e) => (
                                    DepDisposition::Unresolved {
                                        reason: e.to_string(),
                                    },
                                    Vec::new(),
                                ),
                            }
                        }
                        Err(e) => (
                            DepDisposition::Unresolved {
                                reason: e.to_string(),
                            },
                            Vec::new(),
                        ),
                    }
                }
                None => (
                    DepDisposition::Unresolved {
                        reason: "Failed to list Modrinth versions".into(),
                    },
                    Vec::new(),
                ),
            };
            resolved.insert(
                key,
                ResolvedDep {
                    mod_jar_id: pid.clone(),
                    requirement,
                    source: DepSource::Manifest,
                    display_name: None,
                    page_url: None,
                    disposition,
                },
            );
            for child in child_deps {
                let child_requirement = match child.dependency_type.as_str() {
                    "required" if requirement == Requirement::Required => Requirement::Required,
                    "required" | "optional" => Requirement::Optional,
                    _ => continue,
                };
                queue.push_back((child.project_id, child.version_id, child_requirement));
            }
        }

        // Hydrate display names and page links for every dependency project
        // in one batched Modrinth request. Failures are non-fatal: the UI
        // falls back to the raw id when the metadata is unavailable.
        if let Some(batch) = batch {
            for (key, project_id) in &batch.project_ids {
                project_ids
                    .entry(key.clone())
                    .or_insert_with(|| project_id.clone());
            }
        }
        let project_info = self.fetch_batch_project_info(&project_ids).await;

        // A loader's native id and its Modrinth dependency edge often use
        // different identities. For example, the JAR declares `glitchcore`
        // while Modrinth declares project `s3dmwKy5`. Collapse the native
        // evidence into the real dependency row instead of showing a
        // duplicate unresolved error.
        collapse_native_project_dependencies(
            &mut resolved,
            &project_info,
            &native_project_mappings,
        );

        for dep in resolved.values_mut() {
            let project_info_entry = project_info
                .get(&dep.mod_jar_id.to_ascii_lowercase())
                .or_else(|| {
                    batch
                        .and_then(|batch| batch.project_id_for_loader(&dep.mod_jar_id))
                        .and_then(|project_id| project_info.get(project_id))
                });
            if let Some((name, url)) = project_info_entry {
                dep.display_name = Some(name.clone());
                dep.page_url = url.clone();
            }
        }

        resolved.into_values().collect()
    }

    /// Resolve a native loader id to a Modrinth project when the loader
    /// metadata did not carry a project id and the curated registry has no
    /// alias for it. Queries are bounded and results are accepted only when
    /// the project identity is a close match to the native id.
    async fn lookup_modrinth_project_for_loader(&self, loader_id: &str) -> Option<String> {
        let normalized = normalize_project_identity(loader_id);
        let base = native_loader_search_identity(loader_id);
        let queries = if base != normalized && base.len() >= 4 {
            vec![loader_id.replace('_', " "), base.clone()]
        } else {
            vec![loader_id.replace('_', " ")]
        };

        for query in queries {
            let url = format!(
                "https://api.modrinth.com/v2/search?query={}&limit=5",
                urlencoding::encode(&query)
            );
            let Ok(response) = http_client::checked_get_json::<ModrinthSearchResponse>(
                &self.ctx.http_clients,
                ClientCategory::Modrinth,
                &url,
            )
            .await
            else {
                continue;
            };
            if let Some(hit) = response.hits.into_iter().find(|hit| {
                hit.project_type == "mod"
                    && project_search_identity_matches(&base, &hit.title, &hit.slug)
            }) {
                return Some(hit.project_id);
            }
        }
        None
    }

    /// Batched Modrinth project lookup for dependency display names and page
    /// URLs. Tolerates per-chunk failures (missing projects are just absent).
    async fn fetch_batch_project_info(
        &self,
        project_ids: &HashMap<String, String>,
    ) -> HashMap<String, (String, Option<String>)> {
        let mut info = HashMap::new();
        let ids: Vec<&String> = project_ids.values().collect();
        for chunk in ids.chunks(100) {
            let encoded = serde_json::to_string(chunk).unwrap_or_else(|_| "[]".to_string());
            let url = format!(
                "https://api.modrinth.com/v2/projects?ids={}",
                urlencoding::encode(&encoded)
            );
            let projects = match http_client::checked_get_json::<Vec<ModrinthBatchProject>>(
                &self.ctx.http_clients,
                ClientCategory::Modrinth,
                &url,
            )
            .await
            {
                Ok(projects) => projects,
                Err(_) => {
                    // Keep metadata enrichment resilient when the bulk
                    // endpoint is unavailable or one opaque id is rejected.
                    // The individual endpoint preserves the same allowlisted
                    // Modrinth client policy and is only used as a fallback.
                    let mut fallback = Vec::new();
                    for project_id in chunk {
                        let single_url = format!(
                            "https://api.modrinth.com/v2/project/{}",
                            urlencoding::encode(project_id)
                        );
                        if let Ok(project) = http_client::checked_get_json::<ModrinthBatchProject>(
                            &self.ctx.http_clients,
                            ClientCategory::Modrinth,
                            &single_url,
                        )
                        .await
                        {
                            fallback.push(project);
                        }
                    }
                    fallback
                }
            };
            for project in projects {
                let page_url = project
                    .slug
                    .map(|slug| format!("https://modrinth.com/{}/{}", project.project_type, slug))
                    .or_else(|| Some(format!("https://modrinth.com/project/{}", project.id)));
                info.insert(project.id.to_ascii_lowercase(), (project.title, page_url));
            }
        }
        info
    }

    // ------------------------------------------------------------------
    // Batch resolution
    // ------------------------------------------------------------------

    async fn resolve_batch_install(
        &self,
        manifest: &InstanceManifest,
        items: &[crate::install_pipeline::BatchInstallItem],
        registry_revision: String,
        allow_closest_version: bool,
        skip_items: &[String],
    ) -> LauncherResult<PreparedPlan> {
        // Phase 1: resolve every root artifact before any dependency work so
        // the batch can treat its own items as satisfied dependencies.
        let mut roots: Vec<(
            crate::install_pipeline::BatchInstallItem,
            ResolvedArtifact,
            Option<RawModrinthVersionCandidate>,
            Option<crate::dependency_ops::JarDeps>,
        )> = Vec::new();
        let active_items: Vec<_> = items
            .iter()
            .filter(|item| {
                !skip_items
                    .iter()
                    .any(|skipped| skipped.eq_ignore_ascii_case(&item.item_id))
            })
            .collect();
        if active_items.is_empty() {
            return Err(LauncherError::Generic {
                code: "ERR_BATCH_EMPTY".into(),
                message: "No batch items remain after skipping incompatible items.".into(),
            });
        }
        for item in active_items {
            let root_result = async {
                Ok::<_, LauncherError>(match item.source_type {
                    SourceType::Curated => (
                        self.resolve_curated_artifact(
                            manifest,
                            &item.item_id,
                            item.candidate_version.as_deref(),
                            allow_closest_version,
                        )
                        .await?,
                        None,
                    ),
                    SourceType::Modrinth => {
                        let (candidate, artifact) = self
                            .resolve_raw_modrinth_artifact(
                                manifest,
                                &item.item_id,
                                item.candidate_version.as_deref(),
                                allow_closest_version,
                            )
                            .await?;
                        (artifact, Some(candidate))
                    }
                    SourceType::Manual => {
                        let prepared = resolve_manual_install(
                            &item.item_id,
                            item.candidate_version.as_deref(),
                            registry_revision.clone(),
                        )?;
                        let artifact = match prepared.operation {
                            ResolvedOperation::Install { artifact } => artifact,
                            _ => unreachable!(
                                "manual install always resolves to an install operation"
                            ),
                        };
                        (artifact, None)
                    }
                })
            }
            .await;
            let (artifact, candidate) = match root_result {
                Ok(root) => root,
                Err(LauncherError::VersionNotFound) => {
                    let source = match item.source_type {
                        SourceType::Curated => "curated",
                        SourceType::Modrinth => "Modrinth",
                        SourceType::Manual => "manual",
                    };
                    let closest = self
                        .closest_version_summary(manifest, item.source_type.clone(), &item.item_id)
                        .await
                        .map(|summary| format!(" Closest available: {summary}."))
                        .unwrap_or_default();
                    return Err(LauncherError::Generic {
                        code: "ERR_VERSION_NOT_FOUND".into(),
                        message: format!(
                            "No compatible version found for {source} item '{}' on Minecraft {} / {}.{} Try the closest available version or skip this item from the batch.",
                            item.item_id, manifest.minecraft_version, manifest.loader, closest
                        ),
                    });
                }
                Err(error) => return Err(error),
            };
            let native_metadata = match candidate.as_ref() {
                Some(candidate) => self.native_loader_metadata(manifest, candidate).await,
                None => None,
            };
            roots.push((item.clone(), artifact, candidate, native_metadata));
        }

        let mut batch_ctx =
            BatchContext::from_artifacts(roots.iter().map(|(_, artifact, _, _)| artifact));
        for (_, artifact, _, native_metadata) in &roots {
            if let Some(native_metadata) = native_metadata {
                batch_ctx.add_native_metadata(artifact, native_metadata);
            }
        }

        // Phase 2: build the operations and resolve dependencies with the
        // batch context so an item's dependency on a sibling batch item is
        // satisfied by the sibling's own artifact.
        let mut operations = Vec::new();
        let mut deps_map = BTreeMap::<String, ResolvedDep>::new();
        let mut conflicts_map = BTreeMap::<String, DepConflict>::new();

        for (item, artifact, candidate, native_metadata) in roots {
            let (dependencies, conflicts) = match item.source_type {
                SourceType::Curated => {
                    self.resolve_curated_dependencies(manifest, &item.item_id, Some(&batch_ctx))
                        .await?
                }
                SourceType::Modrinth => {
                    let candidate = candidate.expect("modrinth roots always carry a candidate");
                    let dependencies = self
                        .resolve_raw_modrinth_deps(
                            manifest,
                            &candidate,
                            native_metadata.as_ref(),
                            Some(&batch_ctx),
                            allow_closest_version,
                        )
                        .await;
                    (dependencies, Vec::new())
                }
                SourceType::Manual => (Vec::new(), Vec::new()),
            };
            operations.push(ResolvedOperation::Install { artifact });
            merge_deps(&mut deps_map, dependencies);
            for conflict in conflicts {
                conflicts_map.insert(conflict.conflict_id.clone(), conflict);
            }
        }

        // Batch-level known-conflict pass: conflicts between two batch roots
        // or between different roots' dependency closures are invisible to
        // the per-root passes (each only sees its own closure), so check the
        // union here. Duplicates against per-root conflicts collapse on
        // conflict_id.
        {
            let (known_conflicts, alias_pairs) = {
                let conn = open_registry_db(&self.ctx.paths.registry_db())?;
                (
                    registry::get_known_conflicts(&conn)?,
                    registry::get_all_mod_aliases(&conn)?,
                )
            };
            let aliases = AliasMap::from_pairs(&alias_pairs);
            let installed_set: HashSet<String> = all_installed(manifest)
                .flat_map(|item| {
                    let ids: [Option<&str>; 3] = [
                        item.registry_id.as_deref(),
                        item.modrinth_id.as_deref(),
                        item.mod_jar_id.as_deref(),
                    ];
                    ids.into_iter()
                        .flatten()
                        .map(|id| aliases.resolve_or_self(id).to_ascii_lowercase())
                        .collect::<Vec<_>>()
                })
                .collect();
            let incoming: HashSet<String> = batch_ctx
                .identities
                .iter()
                .map(|identity| aliases.resolve_or_self(identity).to_ascii_lowercase())
                .chain(deps_map.keys().cloned())
                .collect();
            for conflict in
                build_known_conflicts(&known_conflicts, &aliases, &incoming, &installed_set)
            {
                conflicts_map.insert(conflict.conflict_id.clone(), conflict);
            }
        }

        Ok(PreparedPlan {
            operation: ResolvedOperation::BatchInstall { operations },
            dependencies: deps_map.into_values().collect(),
            conflicts: conflicts_map.into_values().collect(),
            registry_revision,
        })
    }

    async fn closest_version_summary(
        &self,
        manifest: &InstanceManifest,
        source_type: SourceType,
        item_id: &str,
    ) -> Option<String> {
        match source_type {
            SourceType::Modrinth => {
                let candidates = self
                    .list_raw_modrinth_versions_closest(manifest, item_id)
                    .await
                    .ok()?;
                let candidate = select_closest_raw_modrinth_candidate(
                    &candidates,
                    &manifest.minecraft_version,
                )?;
                Some(format!(
                    "{} ({}) for {}",
                    candidate.version,
                    candidate.filename,
                    candidate.mc_versions.join(", ")
                ))
            }
            SourceType::Curated => {
                let item = {
                    let conn = open_registry_db(&self.ctx.paths.registry_db()).ok()?;
                    registry::get_item_by_id(&conn, item_id).ok()??
                };
                let candidates = self
                    .list_curated_versions(&item, &manifest.minecraft_version, &manifest.loader)
                    .await
                    .ok()?;
                let candidate =
                    select_closest_curated_candidate(&candidates, &manifest.minecraft_version)?;
                Some(format!(
                    "{} ({}){}",
                    candidate.version,
                    candidate.filename,
                    candidate
                        .mc_version
                        .as_deref()
                        .map(|version| format!(" for {version}"))
                        .unwrap_or_default()
                ))
            }
            SourceType::Manual => None,
        }
    }

    async fn resolve_batch_update(
        &self,
        manifest: &InstanceManifest,
        items: &[crate::install_pipeline::BatchUpdateItem],
        registry_revision: String,
    ) -> LauncherResult<PreparedPlan> {
        let mut operations = Vec::new();
        let mut deps_map = BTreeMap::<String, ResolvedDep>::new();
        let mut conflicts_map = BTreeMap::<String, DepConflict>::new();

        for item in items {
            let installed =
                find_installed_by_identity(manifest, &item.item_id).ok_or_else(|| {
                    LauncherError::Generic {
                        code: "ERR_UPDATE_TARGET_MISSING".into(),
                        message: format!("{} is not installed.", item.item_id),
                    }
                })?;
            let prepared = if installed.source == "modrinth_raw" {
                let project_id = installed.modrinth_id.as_deref().unwrap_or(&item.item_id);
                self.resolve_raw_modrinth_install(
                    manifest,
                    project_id,
                    normalize_requested_version(Some(&item.target_version)),
                    registry_revision.clone(),
                    true,
                    false,
                )
                .await?
            } else {
                let registry_id = installed.registry_id.as_deref().unwrap_or(&item.item_id);
                self.resolve_curated_install(
                    manifest,
                    registry_id,
                    normalize_requested_version(Some(&item.target_version)),
                    registry_revision.clone(),
                    true,
                    false,
                )
                .await?
            };
            operations.push(prepared.operation);
            merge_deps(&mut deps_map, prepared.dependencies);
            for conflict in prepared.conflicts {
                conflicts_map.insert(conflict.conflict_id.clone(), conflict);
            }
        }

        Ok(PreparedPlan {
            operation: ResolvedOperation::BatchUpdate { operations },
            dependencies: deps_map.into_values().collect(),
            conflicts: conflicts_map.into_values().collect(),
            registry_revision,
        })
    }
}

/// Prefer active-loader-native dependencies over Modrinth's version-level
/// dependency list. The latter has no loader condition, so it can include a
/// compatibility bridge that is irrelevant to a native Fabric/Quilt build.
fn effective_raw_modrinth_dependencies(
    candidate: &RawModrinthVersionCandidate,
    native_metadata: Option<&crate::dependency_ops::JarDeps>,
    native_project_mappings: &std::collections::HashMap<String, String>,
) -> Vec<RawModrinthDep> {
    let mut merged = candidate.dependencies.clone();
    let Some(metadata) = native_metadata else {
        return merged;
    };

    for (loader_id, dependency_type) in metadata
        .depends_on
        .iter()
        .map(|id| (id, "required"))
        .chain(metadata.optional_deps.iter().map(|id| (id, "optional")))
    {
        if let Some(existing) = merged.iter_mut().find(|dependency| {
            dependency
                .project_id
                .as_deref()
                .is_some_and(|project_id| project_id.eq_ignore_ascii_case(loader_id))
        }) {
            if dependency_type == "required" {
                existing.dependency_type = "required".into();
            }
        } else if let Some(project_id) =
            native_project_mappings.get(&loader_id.to_ascii_lowercase())
        {
            merged.push(RawModrinthDep {
                project_id: Some(project_id.clone()),
                version_id: None,
                dependency_type: dependency_type.into(),
            });
        } else {
            // Native IDs are loader capabilities, not Modrinth project IDs.
            // Keep them as unresolved evidence unless a Modrinth edge already
            // supplied a real project ID for the same textual identity.
            merged.push(RawModrinthDep {
                project_id: None,
                version_id: Some(format!("loader-id:{loader_id}")),
                dependency_type: dependency_type.into(),
            });
        }
    }
    merged
}

fn project_identity_matches_loader(
    loader_id: &str,
    project_name: &str,
    page_url: Option<&str>,
) -> bool {
    let loader_identity = normalize_project_identity(loader_id);
    if loader_identity.is_empty() {
        return false;
    }
    normalize_project_identity(project_name) == loader_identity
        || page_url
            .and_then(|url| url.rsplit('/').next())
            .map(normalize_project_identity)
            .is_some_and(|identity| identity == loader_identity)
}

fn collapse_native_project_dependencies(
    resolved: &mut BTreeMap<String, ResolvedDep>,
    project_info: &HashMap<String, (String, Option<String>)>,
    native_project_mappings: &HashMap<String, String>,
) {
    let replacements: Vec<(String, String, Requirement)> = resolved
        .iter()
        .filter_map(|(loader_key, loader_dep)| {
            if loader_dep.source != DepSource::Jar
                || !matches!(loader_dep.disposition, DepDisposition::Unresolved { .. })
            {
                return None;
            }
            let mapped_project_key = native_project_mappings
                .get(loader_key)
                .map(|project_id| project_id.to_ascii_lowercase())
                .filter(|project_key| {
                    resolved
                        .get(project_key)
                        .is_some_and(|project_dep| project_dep.source == DepSource::Manifest)
                });
            let project_key = mapped_project_key.or_else(|| {
                resolved.iter().find_map(|(project_key, project_dep)| {
                    if project_key == loader_key || project_dep.source != DepSource::Manifest {
                        return None;
                    }
                    let (name, page_url) = project_info.get(project_key)?;
                    project_identity_matches_loader(loader_key, name, page_url.as_deref())
                        .then(|| project_key.clone())
                })
            })?;
            Some((loader_key.clone(), project_key, loader_dep.requirement))
        })
        .collect();
    for (loader_key, project_key, requirement) in replacements {
        if let Some(project_dep) = resolved.get_mut(&project_key) {
            if requirement == Requirement::Required {
                project_dep.requirement = Requirement::Required;
            }
        }
        resolved.remove(&loader_key);
    }
}

fn normalize_project_identity(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn native_loader_search_identity(loader_id: &str) -> String {
    let lower = loader_id.to_ascii_lowercase();
    if lower == "wm_binaries" {
        return "watermedia".into();
    }
    let without_version = lower
        .rfind("_v")
        .filter(|index| {
            lower[*index + 2..]
                .chars()
                .all(|character| character.is_ascii_digit())
        })
        .map(|index| &lower[..index])
        .unwrap_or(&lower);
    let without_module_suffix = ["_key_modifiers", "_binaries", "_api"]
        .iter()
        .find_map(|suffix| without_version.strip_suffix(suffix))
        .unwrap_or(without_version);
    normalize_project_identity(without_module_suffix)
}

fn project_search_identity_matches(base: &str, title: &str, slug: &str) -> bool {
    if base.is_empty() {
        return false;
    }
    let title = normalize_project_identity(title);
    let slug = normalize_project_identity(slug);
    title == base
        || slug == base
        || title.contains(base)
        || slug.contains(base)
        || base.contains(&title)
        || base.contains(&slug)
}

// ---------------------------------------------------------------------------
// Standalone functions: GitHub release helpers
// ---------------------------------------------------------------------------

fn github_auth_headers(token: Option<&str>) -> Vec<(String, String)> {
    token
        .map(|token| vec![("Authorization".into(), format!("Bearer {token}"))])
        .unwrap_or_default()
}

/// Parse the GitHub API `Link` response header to discover the total number of pages.
pub fn parse_link_total_pages(header_value: Option<&str>) -> u32 {
    let value = match header_value {
        Some(v) => v,
        None => return 1,
    };
    for part in value.split(',') {
        let trimmed = part.trim();
        if trimmed.contains("rel=\"last\"") {
            if let Some(close) = trimmed.rfind('>') {
                let substr = &trimmed[..close];
                if let Some(open) = substr.rfind('<') {
                    let url = &substr[open + 1..];
                    for segment in url.split(&['?', '&'][..]) {
                        if let Some(num) = segment.strip_prefix("page=") {
                            return num.parse::<u32>().unwrap_or(1);
                        }
                    }
                }
            }
        }
    }
    1
}

/// Sort version candidates by compatibility tier (compatible → major_match → other),
/// then by release date descending within each tier.
pub fn sort_versions_by_compatibility(versions: &mut [ModVersionCandidate]) {
    versions.sort_by(|a, b| {
        let tier = |c: &ModVersionCandidate| -> u8 {
            match c.version_compat.as_str() {
                "compatible" => 0,
                "major_match" => 1,
                _ => 2,
            }
        };
        let ta = tier(a);
        let tb = tier(b);
        ta.cmp(&tb).then_with(|| {
            b.release_date
                .as_deref()
                .unwrap_or("")
                .cmp(a.release_date.as_deref().unwrap_or(""))
        })
    });
}

/// Determine MC version and loader compatibility for a GitHub release asset.
pub fn parse_version_from_github_asset(
    filename: &str,
    tag_name: &str,
    mc_version: &str,
    loader: &str,
) -> (Option<String>, Option<String>, &'static str) {
    let mc = extract_mc_version(filename).or_else(|| extract_mc_version(tag_name));
    let lo = extract_loader(filename).or_else(|| extract_loader(tag_name));

    let mc_match = mc.as_deref().map(|v| {
        let target = mc_version.to_lowercase();
        let stripped_target = target.strip_prefix("1.").unwrap_or(&target);
        let stripped_found = v.strip_prefix("1.").unwrap_or(v);
        stripped_found == stripped_target
    });

    let lo_match = lo.as_deref().map(|l| l.eq_ignore_ascii_case(loader));
    let loader_ok = lo_match == Some(true);
    let loader_mismatch = lo.is_some() && !loader_ok;

    let major_matches = mc.as_deref().is_some_and(|found| {
        let target = mc_version.to_lowercase();
        let stripped_target = target.strip_prefix("1.").unwrap_or(&target);
        let stripped_found = found.strip_prefix("1.").unwrap_or(found);
        let target_major = stripped_target.split('.').next().unwrap_or("");
        let found_major = stripped_found.split('.').next().unwrap_or("");
        if target_major.is_empty() || found_major.is_empty() {
            return false;
        }
        target_major == found_major
            && (stripped_found.starts_with(stripped_target)
                || stripped_target.starts_with(stripped_found))
    });

    let compat = if mc_match == Some(true) && !loader_mismatch {
        "compatible"
    } else if major_matches && !loader_mismatch {
        "major_match"
    } else {
        ""
    };

    let matched_mc = if mc_match == Some(true) {
        Some(mc_version.to_string())
    } else {
        mc
    };

    (matched_mc, lo, compat)
}

/// Extract a Minecraft version hint from a string.
pub fn extract_mc_version(text: &str) -> Option<String> {
    let lower = text.to_lowercase();
    let bytes_lower = lower.as_bytes();

    let mut pos = 0;
    while pos < bytes_lower.len() {
        if pos + 1 < bytes_lower.len() && bytes_lower[pos] == b'm' && bytes_lower[pos + 1] == b'c' {
            let before_ok = pos == 0 || !bytes_lower[pos - 1].is_ascii_alphanumeric();
            let after_pos = pos + 2;
            if before_ok && after_pos < bytes_lower.len() && bytes_lower[after_pos].is_ascii_digit()
            {
                let rest = &text[after_pos..];
                let end = rest
                    .find(|c: char| !c.is_ascii_digit() && c != '.')
                    .unwrap_or(rest.len());
                let ver = &rest[..end];
                let ver = ver.strip_suffix('.').unwrap_or(ver);
                if !ver.is_empty() {
                    return Some(ver.to_string());
                }
            }
            pos += 2;
        } else {
            pos += 1;
        }
    }

    let mut i = 0;
    while i < bytes_lower.len() {
        if bytes_lower[i].is_ascii_digit() {
            if i > 0 && bytes_lower[i - 1].is_ascii_alphanumeric() {
                i += 1;
                continue;
            }
            let mut end = i + 1;
            while end < bytes_lower.len()
                && (bytes_lower[end].is_ascii_digit() || bytes_lower[end] == b'.')
            {
                end += 1;
            }
            let mut ver_end = end;
            while ver_end > i + 1 && bytes_lower[ver_end - 1] == b'.' {
                ver_end -= 1;
            }
            let candidate = &lower[i..ver_end];
            if candidate.contains('.') {
                if let Some(major_str) = candidate.split('.').next() {
                    if let Ok(major) = major_str.parse::<u32>() {
                        if major == 1 || major > 25 {
                            return Some(candidate.to_string());
                        }
                    }
                }
            }
            i = end;
        } else {
            i += 1;
        }
    }

    None
}

/// Extract a loader hint from a string.
pub fn extract_loader(text: &str) -> Option<String> {
    const KNOWN_LOADERS: &[&str] = &["fabric", "forge", "neoforge", "quilt"];
    let lower = text.to_lowercase();
    for loader in KNOWN_LOADERS {
        let mut idx = 0;
        while let Some(pos) = lower[idx..].find(loader) {
            let abs = idx + pos;
            let before_ok = abs == 0 || !lower.as_bytes()[abs - 1].is_ascii_alphanumeric();
            let after_pos = abs + loader.len();
            let after_ok =
                after_pos >= lower.len() || !lower.as_bytes()[after_pos].is_ascii_alphanumeric();
            if before_ok && after_ok {
                return Some(loader.to_string());
            }
            idx = abs + 1;
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Standalone functions: Modrinth version listing for curated items
// ---------------------------------------------------------------------------

async fn fetch_modrinth_versions_for_item(
    clients: &HttpClients,
    source_identifier: &str,
    modrinth_id: Option<&str>,
    mc_version: &str,
    loader: &str,
) -> LauncherResult<Vec<ModVersionCandidate>> {
    let project_id = modrinth_id.unwrap_or(source_identifier);

    let url = format!(
        "https://api.modrinth.com/v2/project/{pid}/version?game_versions=[\"{gv}\"]&loaders=[\"{ld}\"]",
        pid = urlencoding::encode(project_id),
        gv = urlencoding::encode(mc_version),
        ld = urlencoding::encode(loader),
    );

    #[derive(Deserialize)]
    struct MRFileHashes {
        sha1: Option<String>,
        sha256: Option<String>,
        sha512: Option<String>,
    }
    #[derive(Deserialize)]
    struct MRFile {
        url: String,
        filename: String,
        primary: bool,
        hashes: Option<MRFileHashes>,
        #[serde(default)]
        size: Option<u64>,
    }
    #[derive(Deserialize)]
    struct MRVersion {
        version_number: String,
        files: Vec<MRFile>,
    }

    let versions: Vec<MRVersion> =
        http_client::checked_get_json(clients, ClientCategory::Modrinth, &url).await?;

    let mut candidates: Vec<ModVersionCandidate> = Vec::new();
    for version in &versions {
        let primary_file = version
            .files
            .iter()
            .find(|f| f.primary)
            .or_else(|| version.files.first());
        let file = match primary_file {
            Some(f) => f,
            None => continue,
        };
        let (mc_ver, lo, compat) = parse_version_from_github_asset(
            &file.filename,
            &version.version_number,
            mc_version,
            loader,
        );
        candidates.push(ModVersionCandidate {
            version: version.version_number.clone(),
            filename: file.filename.clone(),
            download_url: file.url.clone(),
            mc_version: mc_ver,
            loader: lo,
            release_date: None,
            is_compatible: compat == "compatible",
            version_compat: compat.to_string(),
            sha1: file.hashes.as_ref().and_then(|h| h.sha1.clone()),
            sha256: file.hashes.as_ref().and_then(|h| h.sha256.clone()),
            sha512: file.hashes.as_ref().and_then(|h| h.sha512.clone()),
            size: file.size,
        });
    }
    Ok(candidates)
}

// ---------------------------------------------------------------------------
// Standalone functions: hand-curated `direct_hash` items
// ---------------------------------------------------------------------------

/// Derive the on-disk filename for a `direct_hash` download from its URL.
///
/// The manifest has no filename field, so the URL's last path segment is the
/// only name available. The registry is signed, but a traversal-shaped segment
/// must still never reach a path join, so anything that is not a plain
/// `name.ext` is rejected outright rather than sanitized.
fn direct_hash_filename(url: &str) -> Option<String> {
    let segment = url
        .split('#')
        .next()?
        .split('?')
        .next()?
        .rsplit('/')
        .next()?;
    let rejected = segment.is_empty()
        || segment.starts_with('.')
        || segment.contains('\\')
        || segment.contains("..")
        || !segment.contains('.');
    (!rejected).then(|| segment.to_string())
}

/// Compatibility verdict for one curator-declared `(mc_version, loader)` pair.
///
/// Mirrors the rules in [`registry::compatibility_from_registry_json`], but per
/// entry rather than collapsed across the whole array.
fn declared_version_compat(
    declared_mc: &str,
    declared_loader: &str,
    want_mc: &str,
    want_loader: &str,
) -> &'static str {
    if !declared_loader.eq_ignore_ascii_case(want_loader) {
        return "";
    }
    if declared_mc == want_mc {
        return "compatible";
    }
    let major = |version: &str| version.split('.').take(2).collect::<Vec<_>>().join(".");
    if major(declared_mc) == major(want_mc) {
        "major_match"
    } else {
        ""
    }
}

/// Build the candidate list for a fully hand-curated `direct_hash` item.
///
/// Unlike the other strategies there is no upstream API to query — the signed
/// manifest *is* the source of truth. Every candidate therefore points at the
/// same pinned URL and the same pinned SHA-256, and differs only in the
/// `(mc_version, loader, mod_version)` tuple the curator declared. One entry
/// describes one file; a curator who needs two files publishes two registry
/// entries.
///
/// This is deliberately strict. A `direct_hash` manifest has no API to correct
/// a curator's omission, so an under-specified one must fail loudly at resolve
/// time instead of resolving to something plausible but wrong.
fn direct_hash_versions_for_item(
    item: &crate::registry::RegistryItem,
    mc_version: &str,
    loader: &str,
) -> LauncherResult<Vec<ModVersionCandidate>> {
    let invalid = |message: String| LauncherError::Generic {
        code: "ERR_DIRECT_HASH_MANIFEST".into(),
        message,
    };

    let url = item.source_identifier.trim();
    if !url.starts_with("https://") {
        return Err(invalid(format!(
            "'{}' uses direct_hash but its source_identifier is not an https:// URL.",
            item.id
        )));
    }
    let filename = direct_hash_filename(url).ok_or_else(|| {
        invalid(format!(
            "'{}' uses direct_hash but its URL does not end in a filename, so the \
             downloaded file cannot be named.",
            item.id
        ))
    })?;
    let sha256 = valid_hash(Some(&item.sha256), 64).ok_or_else(|| {
        invalid(format!(
            "'{}' uses direct_hash but has no valid pinned sha256.",
            item.id
        ))
    })?;

    #[derive(Deserialize)]
    struct DeclaredVersion {
        mc_version: String,
        loader: String,
        mod_version: Option<String>,
    }

    let declared: Vec<DeclaredVersion> = item
        .compatible_versions_json
        .as_deref()
        .and_then(|json| serde_json::from_str(json).ok())
        .unwrap_or_default();
    if declared.is_empty() {
        return Err(invalid(format!(
            "'{}' uses direct_hash but declares no compatible_versions; a hand-curated \
             entry must state which Minecraft versions and loaders its file supports.",
            item.id
        )));
    }

    let mut candidates = Vec::new();
    for entry in &declared {
        // "latest" is the compiler's placeholder for an unhydrated entry. It is
        // meaningless for a pinned file and would make version selection
        // ambiguous, so it is rejected rather than accepted as a version.
        let version = entry
            .mod_version
            .as_deref()
            .map(str::trim)
            .filter(|version| !version.is_empty() && *version != "latest")
            .ok_or_else(|| {
                invalid(format!(
                    "'{}' uses direct_hash but a compatible_versions entry has no explicit \
                     mod_version.",
                    item.id
                ))
            })?;
        let compat = declared_version_compat(&entry.mc_version, &entry.loader, mc_version, loader);
        candidates.push(ModVersionCandidate {
            version: version.to_string(),
            filename: filename.clone(),
            download_url: url.to_string(),
            mc_version: Some(entry.mc_version.clone()),
            loader: Some(entry.loader.clone()),
            release_date: None,
            is_compatible: compat == "compatible",
            version_compat: compat.to_string(),
            sha1: None,
            sha256: Some(sha256.clone()),
            sha512: None,
            size: None,
        });
    }

    // `select_curated_candidate` takes the first compatible entry, so rank by
    // verdict instead of letting manifest authoring order decide.
    candidates.sort_by_key(|candidate| match candidate.version_compat.as_str() {
        "compatible" => 0,
        "major_match" => 1,
        _ => 2,
    });
    Ok(candidates)
}

// ---------------------------------------------------------------------------
// Candidate selection
// ---------------------------------------------------------------------------

/// Select the best candidate from a list of curated versions.
/// When `requested` is set, finds that exact version; otherwise finds
/// the first compatible candidate.
pub fn select_curated_candidate<'a>(
    candidates: &'a [ModVersionCandidate],
    requested: Option<&str>,
) -> LauncherResult<&'a ModVersionCandidate> {
    let requested = normalize_requested_version(requested);
    if let Some(requested) = requested {
        return candidates
            .iter()
            .find(|c| c.version == requested || c.filename == requested)
            .ok_or(LauncherError::VersionNotFound);
    }
    candidates
        .iter()
        .find(|c| c.is_compatible)
        .ok_or(LauncherError::VersionNotFound)
}

fn select_closest_curated_candidate<'a>(
    candidates: &'a [ModVersionCandidate],
    target_minecraft_version: &str,
) -> Option<&'a ModVersionCandidate> {
    candidates.iter().min_by_key(|candidate| {
        candidate
            .mc_version
            .as_deref()
            .map(|version| minecraft_version_distance(version, target_minecraft_version))
            .unwrap_or(u64::MAX)
    })
}

fn is_installable_github_asset(filename: &str) -> bool {
    let lower = filename.to_ascii_lowercase();
    let Some(stem) = lower.strip_suffix(".jar") else {
        return false;
    };
    ![
        "-api", "-dev", "-sources", "-source", "-javadoc", "-tests", "-test", "-slim",
    ]
    .iter()
    .any(|suffix| stem.ends_with(suffix))
}

/// Select the best candidate from a list of raw Modrinth versions.
pub fn select_raw_modrinth_candidate<'a>(
    candidates: &'a [RawModrinthVersionCandidate],
    requested: Option<&str>,
) -> LauncherResult<&'a RawModrinthVersionCandidate> {
    let requested = normalize_requested_version(requested);
    if let Some(requested) = requested {
        return candidates
            .iter()
            .find(|c| {
                c.version_id == requested || c.version == requested || c.filename == requested
            })
            .ok_or(LauncherError::VersionNotFound);
    }
    candidates.first().ok_or(LauncherError::VersionNotFound)
}

fn select_closest_raw_modrinth_candidate<'a>(
    candidates: &'a [RawModrinthVersionCandidate],
    target_minecraft_version: &str,
) -> Option<&'a RawModrinthVersionCandidate> {
    candidates.iter().min_by_key(|candidate| {
        candidate
            .mc_versions
            .iter()
            .map(|version| minecraft_version_distance(version, target_minecraft_version))
            .min()
            .unwrap_or(u64::MAX)
    })
}

fn minecraft_version_distance(left: &str, right: &str) -> u64 {
    let parse = |value: &str| {
        let mut parts = value
            .split('.')
            .map(|part| part.parse::<u64>().unwrap_or(0));
        [
            parts.next().unwrap_or(0),
            parts.next().unwrap_or(0),
            parts.next().unwrap_or(0),
        ]
    };
    let left = parse(left);
    let right = parse(right);
    left.into_iter()
        .zip(right)
        .enumerate()
        .map(|(index, (a, b))| a.abs_diff(b) * 10_u64.pow((2 - index) as u32))
        .sum()
}

/// Normalize a requested version string: trim whitespace, reject empty/"available"/"latest".
pub fn normalize_requested_version(requested: Option<&str>) -> Option<&str> {
    requested
        .map(str::trim)
        .filter(|v| !v.is_empty() && *v != "available" && *v != "latest")
}

// ---------------------------------------------------------------------------
// Artifact builders
// ---------------------------------------------------------------------------

fn curated_artifact(
    item: &crate::registry::RegistryItem,
    candidate: &ModVersionCandidate,
) -> LauncherResult<ResolvedArtifact> {
    let hashes = curated_hashes(item, candidate)?;

    Ok(ResolvedArtifact::Download(ResolvedDownload {
        item_id: item.id.clone(),
        version_id: candidate.version.clone(),
        source: ArtifactSource::Download {
            url: candidate.download_url.clone(),
        },
        hashes,
        size: candidate.size.unwrap_or(0),
        filename: candidate.filename.clone(),
        metadata: ArtifactMetadata {
            source_type: SourceType::Curated,
            registry_id: Some(item.id.clone()),
            modrinth_id: item.modrinth_id.clone(),
            content_type: item.content_type.clone(),
            version: Some(candidate.version.clone()),
            download_strategy: Some(item.download_strategy.clone()),
        },
    }))
}

fn curated_hashes(
    item: &crate::registry::RegistryItem,
    candidate: &ModVersionCandidate,
) -> LauncherResult<HashSpec> {
    let mut hashes = Vec::new();
    if let Some(sha512) = valid_hash(candidate.sha512.as_deref(), 128) {
        hashes.push(HashedValue {
            algorithm: HashAlgorithm::Sha512,
            value: sha512,
        });
    }
    if let Some(sha256) = valid_hash(candidate.sha256.as_deref(), 64) {
        hashes.push(HashedValue {
            algorithm: HashAlgorithm::Sha256,
            value: sha256,
        });
    }
    if let Some(sha1) = valid_hash(candidate.sha1.as_deref(), 40) {
        hashes.push(HashedValue {
            algorithm: HashAlgorithm::Sha1,
            value: sha1,
        });
    }

    // A registry item hash may pin one historical artifact. It is only a safe
    // fallback when the selected candidate published no hashes at all; never
    // combine it with hashes belonging to a different version.
    if hashes.is_empty() {
        let sha256 = valid_hash(Some(&item.sha256), 64).ok_or_else(|| LauncherError::Generic {
            code: "ERR_HASH_UNAVAILABLE".into(),
            message: format!(
                "No trusted hash is available for {} {}.",
                item.id, candidate.version
            ),
        })?;
        hashes.push(HashedValue {
            algorithm: HashAlgorithm::Sha256,
            value: sha256,
        });
    }
    Ok(HashSpec { values: hashes })
}

fn raw_modrinth_artifact(
    project_id: &str,
    candidate: &RawModrinthVersionCandidate,
) -> LauncherResult<ResolvedArtifact> {
    let mut hashes = Vec::new();
    if let Some(sha512) = valid_hash(candidate.sha512.as_deref(), 128) {
        hashes.push(HashedValue {
            algorithm: HashAlgorithm::Sha512,
            value: sha512,
        });
    }
    if let Some(sha1) = valid_hash(candidate.sha1.as_deref(), 40) {
        hashes.push(HashedValue {
            algorithm: HashAlgorithm::Sha1,
            value: sha1,
        });
    }
    if hashes.is_empty() {
        return Err(LauncherError::Generic {
            code: "ERR_HASH_UNAVAILABLE".into(),
            message: format!(
                "Modrinth did not publish a usable hash for {}.",
                candidate.filename
            ),
        });
    }

    Ok(ResolvedArtifact::Download(ResolvedDownload {
        item_id: project_id.to_string(),
        version_id: candidate.version_id.clone(),
        source: ArtifactSource::Download {
            url: candidate.download_url.clone(),
        },
        hashes: HashSpec { values: hashes },
        size: candidate.size.unwrap_or(0),
        filename: candidate.filename.clone(),
        metadata: ArtifactMetadata {
            source_type: SourceType::Modrinth,
            registry_id: None,
            modrinth_id: Some(project_id.to_string()),
            content_type: "mod".into(),
            version: Some(candidate.version.clone()),
            download_strategy: None,
        },
    }))
}

fn resolve_manual_install(
    item_id: &str,
    source_path: Option<&str>,
    registry_revision: String,
) -> LauncherResult<PreparedPlan> {
    let source_path = source_path
        .filter(|p| !p.trim().is_empty())
        .ok_or_else(|| LauncherError::Generic {
            code: "ERR_MANUAL_PATH".into(),
            message: "Manual install requires a local file path.".into(),
        })?;
    let path = Path::new(source_path);
    if !path.is_file() {
        return Err(LauncherError::Generic {
            code: "ERR_MANUAL_PATH".into(),
            message: format!("Manual artifact does not exist: {}", path.display()),
        });
    }
    let filename = path
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| name.to_ascii_lowercase().ends_with(".jar"))
        .ok_or_else(|| LauncherError::Generic {
            code: "ERR_MANUAL_FILE".into(),
            message: "Manual mods must be .jar files.".into(),
        })?;
    let bytes = std::fs::read(path).map_err(|e| LauncherError::Generic {
        code: "ERR_MANUAL_READ".into(),
        message: format!("Could not read manual artifact: {e}"),
    })?;
    let sha256 = download::sha256_hex(&bytes);

    Ok(PreparedPlan {
        operation: ResolvedOperation::Install {
            artifact: ResolvedArtifact::LocalFile(ResolvedLocal {
                item_id: item_id.to_string(),
                source_path: source_path.to_string(),
                hashes: HashSpec {
                    values: vec![HashedValue {
                        algorithm: HashAlgorithm::Sha256,
                        value: sha256,
                    }],
                },
                size: bytes.len() as u64,
                filename: filename.to_string(),
                metadata: ArtifactMetadata {
                    source_type: SourceType::Manual,
                    registry_id: None,
                    modrinth_id: None,
                    content_type: "mod".into(),
                    version: None,
                    download_strategy: None,
                },
            }),
        },
        dependencies: Vec::new(),
        conflicts: Vec::new(),
        registry_revision,
    })
}

// ---------------------------------------------------------------------------
// Conflict builder
// ---------------------------------------------------------------------------

fn build_known_conflicts(
    known_conflicts: &[crate::registry::KnownConflict],
    aliases: &AliasMap,
    incoming: &HashSet<String>,
    installed_set: &HashSet<String>,
) -> Vec<DepConflict> {
    let mut conflicts = Vec::new();
    for conflict in known_conflicts {
        let a = aliases
            .resolve_or_self(&conflict.mod_a_id)
            .to_ascii_lowercase();
        let b = aliases
            .resolve_or_self(&conflict.mod_b_id)
            .to_ascii_lowercase();
        if (incoming.contains(&a) && (installed_set.contains(&b) || incoming.contains(&b)))
            || (incoming.contains(&b) && (installed_set.contains(&a) || incoming.contains(&a)))
        {
            conflicts.push(DepConflict {
                conflict_id: format!("known:{a}:{b}"),
                kind: ConflictKind::IncompatibleMod,
                existing_mod_jar_id: if installed_set.contains(&a) {
                    a.clone()
                } else {
                    b.clone()
                },
                incoming_mod_jar_id: if incoming.contains(&a) {
                    a.clone()
                } else {
                    b.clone()
                },
                message: conflict.notes.clone().unwrap_or_else(|| {
                    format!("The curated registry reports a conflict between {a} and {b}.")
                }),
                blocking: conflict.severity != "info",
                resolution_options: vec![ConflictResolution::Abort, ConflictResolution::Skip],
                chosen: None,
            });
        }
    }
    conflicts
}

// ---------------------------------------------------------------------------
// General helpers
// ---------------------------------------------------------------------------

fn enqueue_manifest_deps(queue: &mut VecDeque<(String, Requirement)>, deps: &ManifestDeps) {
    queue.extend(
        deps.required
            .iter()
            .cloned()
            .map(|id| (id, Requirement::Required)),
    );
    queue.extend(
        deps.optional
            .iter()
            .cloned()
            .map(|id| (id, Requirement::Optional)),
    );
}

/// Validate that a hash hex string has the expected length.
pub fn valid_hash(value: Option<&str>, length: usize) -> Option<String> {
    value
        .map(str::trim)
        .filter(|v| v.len() == length && v.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .map(str::to_ascii_lowercase)
}

fn is_platform_dependency(dependency: &str, loader: &str) -> bool {
    matches!(
        dependency,
        "minecraft" | "java" | "fabricloader" | "fabric_loader" | "quilt_loader" | "quilt-loader"
    ) || dependency.eq_ignore_ascii_case(loader)
        || (loader == "neoforge" && dependency == "forge")
}

fn find_installed_by_identity<'a>(
    manifest: &'a InstanceManifest,
    identity: &str,
) -> Option<&'a InstalledMod> {
    all_installed(manifest).find(|item| {
        item.registry_id
            .as_deref()
            .map(|id| id.eq_ignore_ascii_case(identity))
            .unwrap_or(false)
            || item
                .modrinth_id
                .as_deref()
                .map(|id| id.eq_ignore_ascii_case(identity))
                .unwrap_or(false)
            || item
                .mod_jar_id
                .as_deref()
                .map(|id| id.eq_ignore_ascii_case(identity))
                .unwrap_or(false)
    })
}

fn all_installed(manifest: &InstanceManifest) -> impl Iterator<Item = &InstalledMod> {
    manifest
        .mods
        .iter()
        .chain(manifest.resourcepacks.iter())
        .chain(manifest.shaders.iter())
        .chain(manifest.datapacks.iter())
        .chain(manifest.worlds.iter())
}

fn effective_installed_filename(item: &InstalledMod) -> String {
    if item.enabled || item.filename.ends_with(".disabled") {
        item.filename.clone()
    } else {
        format!("{}.disabled", item.filename)
    }
}

fn merge_deps(target: &mut BTreeMap<String, ResolvedDep>, incoming: Vec<ResolvedDep>) {
    for dependency in incoming {
        let key = dependency.mod_jar_id.to_ascii_lowercase();
        target
            .entry(key)
            .and_modify(|existing| {
                if dependency.requirement == Requirement::Required {
                    existing.requirement = Requirement::Required;
                }
            })
            .or_insert(dependency);
    }
}

fn open_registry_db(path: &std::path::Path) -> LauncherResult<rusqlite::Connection> {
    if !path.is_file() {
        return Err(LauncherError::RegistryMissing);
    }
    let conn = rusqlite::Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|_| LauncherError::RegistryMissing)?;
    conn.pragma_update(None, "query_only", "ON")
        .map_err(|_| LauncherError::RegistryMissing)?;
    Ok(conn)
}

// ---------------------------------------------------------------------------
// Required API access for fetch_modrinth_versions_for_item's HttpClients::new
// ---------------------------------------------------------------------------

// The fetch_modrinth_versions_for_item function receives HttpClients from the
// Resolver's Ctx, so no standalone HttpClients::new() is needed in this module.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_requested_version() {
        assert_eq!(normalize_requested_version(None), None);
        assert_eq!(normalize_requested_version(Some("")), None);
        assert_eq!(normalize_requested_version(Some("  ")), None);
        assert_eq!(normalize_requested_version(Some("available")), None);
        assert_eq!(normalize_requested_version(Some("latest")), None);
        assert_eq!(normalize_requested_version(Some("1.0.0")), Some("1.0.0"));
        assert_eq!(
            normalize_requested_version(Some(" 1.20.1 ")),
            Some("1.20.1")
        );
    }

    #[test]
    fn test_valid_hash() {
        assert_eq!(valid_hash(Some("xyz"), 3), None);
        assert_eq!(valid_hash(Some("abc"), 2), None);
        assert_eq!(
            valid_hash(Some("abcdef0123456789abcdef0123456789"), 32),
            Some("abcdef0123456789abcdef0123456789".into())
        );
        assert_eq!(valid_hash(Some("ABCdef"), 6), Some("abcdef".into()));
        assert_eq!(valid_hash(None, 64), None);
        assert_eq!(valid_hash(Some(&"a".repeat(64)), 64), Some("a".repeat(64)));
    }

    #[test]
    fn test_extract_mc_version() {
        assert_eq!(
            extract_mc_version("fabric-api-0.91.0+1.20.1"),
            Some("1.20.1".into())
        );
        assert_eq!(extract_mc_version("mod-1.19.2.jar"), Some("1.19.2".into()));
        assert_eq!(extract_mc_version("nocontainer"), None);
        assert_eq!(extract_mc_version("lib-0.1.0"), None);
    }

    #[test]
    fn test_extract_loader() {
        assert_eq!(extract_loader("fabric-api-0.91.0"), Some("fabric".into()));
        assert_eq!(extract_loader("My-Forge-Mod"), Some("forge".into()));
        assert_eq!(extract_loader("neoforge-mod"), Some("neoforge".into()));
        assert_eq!(extract_loader("justamod"), None);
    }

    #[test]
    fn test_select_curated_candidate_no_requested_prefers_compatible() {
        let candidates = vec![
            ModVersionCandidate {
                version: "v1.0".into(),
                filename: "a.jar".into(),
                download_url: "https://example.com/a".into(),
                mc_version: None,
                loader: None,
                release_date: Some("2024-01-01".into()),
                is_compatible: false,
                sha1: None,
                sha256: None,
                sha512: None,
                size: None,
                version_compat: "".into(),
            },
            ModVersionCandidate {
                version: "v2.0".into(),
                filename: "b.jar".into(),
                download_url: "https://example.com/b".into(),
                mc_version: None,
                loader: None,
                release_date: Some("2024-02-01".into()),
                is_compatible: true,
                sha1: None,
                sha256: None,
                sha512: None,
                size: None,
                version_compat: "compatible".into(),
            },
        ];
        let selected = select_curated_candidate(&candidates, None).unwrap();
        assert_eq!(selected.version, "v2.0");
    }

    #[test]
    fn test_select_curated_candidate_without_compatible_version_fails() {
        let candidates = vec![ModVersionCandidate {
            version: "v1.0".into(),
            filename: "mod-1.17.jar".into(),
            download_url: "https://example.com/mod-1.17.jar".into(),
            mc_version: Some("1.17".into()),
            loader: Some("fabric".into()),
            release_date: None,
            is_compatible: false,
            sha1: None,
            sha256: None,
            sha512: None,
            size: None,
            version_compat: "incompatible".into(),
        }];

        assert!(matches!(
            select_curated_candidate(&candidates, None),
            Err(LauncherError::VersionNotFound)
        ));
    }

    #[test]
    fn test_github_asset_filter_rejects_non_runtime_jars() {
        assert!(is_installable_github_asset(
            "fabric-api-0.116.14+1.21.1.jar"
        ));
        assert!(is_installable_github_asset(
            "lithium-fabric-0.15.4+mc1.21.1.jar"
        ));
        assert!(!is_installable_github_asset(
            "lithium-fabric-0.15.4+mc1.21.1-api.jar"
        ));
        assert!(!is_installable_github_asset("example-sources.jar"));
        assert!(!is_installable_github_asset("example-javadoc.jar"));
        assert!(!is_installable_github_asset("README.txt"));
    }

    #[test]
    fn test_select_curated_candidate_requested_version() {
        let candidates = vec![
            ModVersionCandidate {
                version: "v1.0".into(),
                filename: "a.jar".into(),
                download_url: "https://example.com/a".into(),
                mc_version: None,
                loader: None,
                release_date: None,
                is_compatible: false,
                sha1: None,
                sha256: None,
                sha512: None,
                size: None,
                version_compat: "".into(),
            },
            ModVersionCandidate {
                version: "v2.0".into(),
                filename: "b.jar".into(),
                download_url: "https://example.com/b".into(),
                mc_version: None,
                loader: None,
                release_date: None,
                is_compatible: true,
                sha1: None,
                sha256: None,
                sha512: None,
                size: None,
                version_compat: "compatible".into(),
            },
        ];
        let selected = select_curated_candidate(&candidates, Some("v1.0")).unwrap();
        assert_eq!(selected.version, "v1.0");
    }

    #[test]
    fn test_select_curated_candidate_requested_by_filename() {
        let candidates = vec![ModVersionCandidate {
            version: "v1.0".into(),
            filename: "specific.jar".into(),
            download_url: "https://example.com/specific".into(),
            mc_version: None,
            loader: None,
            release_date: None,
            is_compatible: false,
            sha1: None,
            sha256: None,
            sha512: None,
            size: None,
            version_compat: "".into(),
        }];
        let selected = select_curated_candidate(&candidates, Some("specific.jar")).unwrap();
        assert_eq!(selected.filename, "specific.jar");
    }

    #[test]
    fn test_is_platform_dependency() {
        assert!(is_platform_dependency("minecraft", "fabric"));
        assert!(is_platform_dependency("fabricloader", "fabric"));
        assert!(is_platform_dependency("quilt_loader", "quilt"));
        assert!(is_platform_dependency("neoforge", "neoforge"));
        assert!(!is_platform_dependency("some-mod", "fabric"));
    }

    #[test]
    fn native_loader_dependencies_merge_without_project_guessing() {
        let candidate = RawModrinthVersionCandidate {
            version: "1.0".into(),
            version_id: "version".into(),
            name: "SwingThrough".into(),
            filename: "swingthrough.jar".into(),
            download_url: "https://cdn.modrinth.com/swingthrough.jar".into(),
            sha1: Some("a".repeat(40)),
            sha512: None,
            size: None,
            mc_versions: vec!["1.21.1".into()],
            loaders: vec!["fabric".into(), "neoforge".into()],
            release_date: None,
            primary: true,
            changelog: None,
            dependencies: vec![RawModrinthDep {
                project_id: Some("connector".into()),
                version_id: None,
                dependency_type: "required".into(),
            }],
        };
        let native_fabric = crate::dependency_ops::JarDeps {
            mod_jar_id: Some("swingthrough".into()),
            depends_on: vec!["native-only".into()],
            ..Default::default()
        };

        let merged = effective_raw_modrinth_dependencies(
            &candidate,
            Some(&native_fabric),
            &std::collections::HashMap::new(),
        );
        assert!(merged
            .iter()
            .any(|dependency| { dependency.project_id.as_deref() == Some("connector") }));
        let native_only = merged
            .iter()
            .find(|dependency| dependency.version_id.as_deref() == Some("loader-id:native-only"))
            .expect("native-only loader evidence");
        assert!(native_only.project_id.is_none());
    }

    #[test]
    fn raw_modrinth_artifact_keeps_display_version_separate_from_version_id() {
        let candidate = RawModrinthVersionCandidate {
            version: "0.6.10".into(),
            version_id: "01J9MODRINTHVERSION".into(),
            name: "Sodium".into(),
            filename: "sodium-0.6.10.jar".into(),
            download_url: "https://cdn.modrinth.com/sodium.jar".into(),
            sha1: Some("a".repeat(40)),
            sha512: None,
            size: Some(100),
            dependencies: Vec::new(),
            mc_versions: vec!["1.21".into()],
            loaders: vec!["fabric".into()],
            release_date: None,
            primary: true,
            changelog: None,
        };

        let ResolvedArtifact::Download(artifact) =
            raw_modrinth_artifact("sodium", &candidate).unwrap()
        else {
            panic!("expected a downloadable Modrinth artifact");
        };
        assert_eq!(artifact.version_id, "01J9MODRINTHVERSION");
        assert_eq!(artifact.metadata.version.as_deref(), Some("0.6.10"));
    }

    // -------------------------------------------------------------------
    // direct_hash: the fully hand-curated strategy. No API resolves these,
    // so the manifest must carry everything and an incomplete one must fail
    // loudly rather than resolve to something plausible but wrong.
    // -------------------------------------------------------------------

    fn direct_hash_item(
        url: &str,
        compatible_versions: Option<&str>,
    ) -> crate::registry::RegistryItem {
        crate::registry::RegistryItem {
            id: "self-hosted-mod".into(),
            name: "Self Hosted Mod".into(),
            content_type: "mod".into(),
            download_strategy: "direct_hash".into(),
            source_identifier: url.into(),
            sha256: "d".repeat(64),
            upvotes: 0,
            downvotes: 0,
            net_score: 0,
            velocity: 0.0,
            status: "active".into(),
            is_immune: false,
            immunity_reason: None,
            allow_comments: true,
            icon_url: None,
            gallery_urls_json: None,
            date_added: None,
            compatible_versions_json: compatible_versions.map(str::to_string),
            description: None,
            body_markdown: None,
            page_url: None,
            license_id: None,
            source_updated_at: None,
            modrinth_id: None,
            recommendation_reason: None,
            recommendation_overlap: None,
        }
    }

    #[test]
    fn direct_hash_resolves_from_the_manifest_alone() {
        let item = direct_hash_item(
            "https://example.com/files/self-hosted-mod-1.2.3.jar",
            Some(r#"[{"mc_version":"1.21","loader":"fabric","mod_version":"1.2.3"}]"#),
        );
        let candidates = direct_hash_versions_for_item(&item, "1.21", "fabric").unwrap();
        assert_eq!(candidates.len(), 1);
        let candidate = &candidates[0];
        assert_eq!(candidate.version, "1.2.3");
        assert_eq!(candidate.filename, "self-hosted-mod-1.2.3.jar");
        assert_eq!(
            candidate.download_url,
            "https://example.com/files/self-hosted-mod-1.2.3.jar"
        );
        assert!(candidate.is_compatible);
        // The pinned item hash must reach the candidate, or the download would
        // fall back to an unverified fetch.
        assert_eq!(candidate.sha256.as_deref(), Some("d".repeat(64).as_str()));

        let artifact = curated_artifact(&item, candidate).unwrap();
        let ResolvedArtifact::Download(download) = artifact else {
            panic!("expected a downloadable artifact");
        };
        assert!(download.hashes.values.iter().any(|hash| {
            hash.algorithm == HashAlgorithm::Sha256 && hash.value == "d".repeat(64)
        }));
    }

    #[test]
    fn direct_hash_ranks_compatible_entries_ahead_of_mismatches() {
        let item = direct_hash_item(
            "https://example.com/files/multi-1.0.0.jar",
            Some(
                r#"[{"mc_version":"1.20.1","loader":"forge","mod_version":"1.0.0"},
                    {"mc_version":"1.21.4","loader":"fabric","mod_version":"1.0.0"},
                    {"mc_version":"1.21","loader":"fabric","mod_version":"1.0.0"}]"#,
            ),
        );
        let candidates = direct_hash_versions_for_item(&item, "1.21", "fabric").unwrap();
        assert_eq!(candidates.len(), 3);
        // Exact match first, then same-major on the right loader, then the rest.
        assert_eq!(candidates[0].version_compat, "compatible");
        assert_eq!(candidates[0].mc_version.as_deref(), Some("1.21"));
        assert_eq!(candidates[1].version_compat, "major_match");
        assert_eq!(candidates[2].version_compat, "");
        assert_eq!(
            select_curated_candidate(&candidates, None)
                .unwrap()
                .mc_version
                .as_deref(),
            Some("1.21")
        );
    }

    #[test]
    fn direct_hash_rejects_incomplete_manifests() {
        let good_versions =
            Some(r#"[{"mc_version":"1.21","loader":"fabric","mod_version":"1.0.0"}]"#);
        let cases: Vec<(&str, crate::registry::RegistryItem)> = vec![
            (
                "plain http is not acceptable for a pinned artifact",
                direct_hash_item("http://example.com/files/mod-1.0.0.jar", good_versions),
            ),
            (
                "a URL with no filename cannot name the download",
                direct_hash_item("https://example.com/download?id=12", good_versions),
            ),
            (
                "a bare traversal segment must never reach a path join",
                direct_hash_item("https://example.com/files/..", good_versions),
            ),
            (
                "an encoded traversal segment is not decoded into a path",
                direct_hash_item(
                    "https://example.com/files/..%2F..%2Fevil.jar",
                    good_versions,
                ),
            ),
            (
                "no declared compatibility means nothing can be selected",
                direct_hash_item("https://example.com/files/mod-1.0.0.jar", None),
            ),
            (
                "an empty compatibility list is equally unusable",
                direct_hash_item("https://example.com/files/mod-1.0.0.jar", Some("[]")),
            ),
            (
                "the compiler's 'latest' placeholder is not a real version",
                direct_hash_item(
                    "https://example.com/files/mod-1.0.0.jar",
                    Some(r#"[{"mc_version":"1.21","loader":"fabric","mod_version":"latest"}]"#),
                ),
            ),
            (
                "a missing mod_version leaves selection ambiguous",
                direct_hash_item(
                    "https://example.com/files/mod-1.0.0.jar",
                    Some(r#"[{"mc_version":"1.21","loader":"fabric"}]"#),
                ),
            ),
        ];
        for (why, item) in cases {
            let result = direct_hash_versions_for_item(&item, "1.21", "fabric");
            assert!(result.is_err(), "{why}");
        }
    }

    #[test]
    fn direct_hash_missing_pinned_hash_is_rejected() {
        let mut item = direct_hash_item(
            "https://example.com/files/mod-1.0.0.jar",
            Some(r#"[{"mc_version":"1.21","loader":"fabric","mod_version":"1.0.0"}]"#),
        );
        item.sha256 = "not-a-hash".into();
        assert!(direct_hash_versions_for_item(&item, "1.21", "fabric").is_err());
    }

    #[test]
    fn curated_artifact_does_not_mix_stale_item_hash_with_version_hashes() {
        let item = crate::registry::RegistryItem {
            id: "xaeros-minimap".into(),
            name: "Xaero's Minimap".into(),
            content_type: "mod".into(),
            download_strategy: "modrinth_id".into(),
            source_identifier: "1bokaNcj".into(),
            sha256: "a".repeat(64),
            upvotes: 0,
            downvotes: 0,
            net_score: 0,
            velocity: 0.0,
            status: "active".into(),
            is_immune: false,
            immunity_reason: None,
            allow_comments: true,
            icon_url: None,
            gallery_urls_json: None,
            date_added: None,
            compatible_versions_json: None,
            description: None,
            body_markdown: None,
            page_url: None,
            license_id: None,
            source_updated_at: None,
            modrinth_id: Some("1bokaNcj".into()),
            recommendation_reason: None,
            recommendation_overlap: None,
        };
        let candidate = ModVersionCandidate {
            version: "fabric-26.2-26.4.2".into(),
            filename: "xaerominimap-fabric-26.2-26.4.2.jar".into(),
            download_url: "https://example.com/xaero.jar".into(),
            mc_version: Some("26.2".into()),
            loader: Some("fabric".into()),
            release_date: None,
            is_compatible: true,
            sha1: Some("b".repeat(40)),
            sha256: None,
            sha512: Some("c".repeat(128)),
            size: Some(1),
            version_compat: "compatible".into(),
        };

        let hashes = curated_hashes(&item, &candidate).unwrap();
        assert!(hashes.values.iter().any(|hash| {
            hash.algorithm == HashAlgorithm::Sha512 && hash.value == "c".repeat(128)
        }));
        assert!(!hashes
            .values
            .iter()
            .any(|hash| hash.algorithm == HashAlgorithm::Sha256));
    }

    #[test]
    fn test_parse_link_total_pages() {
        assert_eq!(parse_link_total_pages(None), 1);
        assert_eq!(
            parse_link_total_pages(Some("<https://api.github.com/repos/owner/repo/releases?page=2>; rel=\"next\", <https://api.github.com/repos/owner/repo/releases?page=5>; rel=\"last\"")),
            5
        );
    }

    #[test]
    fn test_sort_versions_by_compatibility() {
        let mut versions = vec![
            ModVersionCandidate {
                version: "v1".into(),
                filename: "a.jar".into(),
                download_url: "".into(),
                mc_version: None,
                loader: None,
                release_date: Some("2024-01-01".into()),
                is_compatible: false,
                sha1: None,
                sha256: None,
                sha512: None,
                size: None,
                version_compat: "".into(),
            },
            ModVersionCandidate {
                version: "v2".into(),
                filename: "b.jar".into(),
                download_url: "".into(),
                mc_version: None,
                loader: None,
                release_date: Some("2024-02-01".into()),
                is_compatible: true,
                sha1: None,
                sha256: None,
                sha512: None,
                size: None,
                version_compat: "compatible".into(),
            },
            ModVersionCandidate {
                version: "v3".into(),
                filename: "c.jar".into(),
                download_url: "".into(),
                mc_version: None,
                loader: None,
                release_date: Some("2024-03-01".into()),
                is_compatible: false,
                sha1: None,
                sha256: None,
                sha512: None,
                size: None,
                version_compat: "major_match".into(),
            },
        ];
        sort_versions_by_compatibility(&mut versions);
        assert_eq!(versions[0].version_compat, "compatible");
        assert_eq!(versions[1].version_compat, "major_match");
        // Within non-compatible tier, newer dates first
        assert!(versions[2].release_date.as_deref().unwrap_or("") <= "2024-01-01");
    }

    #[test]
    fn test_parse_version_from_github_asset() {
        let (mc, lo, compat) = parse_version_from_github_asset(
            "fabric-api-0.91.0+1.20.1.jar",
            "v0.91.0",
            "1.20.1",
            "fabric",
        );
        assert_eq!(mc, Some("1.20.1".into()));
        assert_eq!(lo, Some("fabric".into()));
        assert_eq!(compat, "compatible");

        let (mc2, _lo2, compat2) =
            parse_version_from_github_asset("my-mod-1.20.jar", "v1.0", "1.20.1", "fabric");
        assert_eq!(mc2, Some("1.20".into()));
        // "1.20" is a prefix of "1.20.1" → major_match (no loader mismatch)
        assert_eq!(compat2, "major_match");
    }

    #[test]
    fn test_find_installed_by_identity_empty_manifest() {
        let manifest = InstanceManifest {
            instance_id: "test".into(),
            name: "Test".into(),
            minecraft_version: "1.20.1".into(),
            loader: "fabric".into(),
            loader_version: "0.15.0".into(),
            is_locked: false,
            mods: vec![],
            resourcepacks: vec![],
            shaders: vec![],
            datapacks: vec![],
            worlds: vec![],
            created_from_pack: None,
            user_preferences: serde_json::json!({}),
        };
        assert!(find_installed_by_identity(&manifest, "anything").is_none());
    }

    #[test]
    fn test_all_installed() {
        let manifest = InstanceManifest {
            instance_id: "test".into(),
            name: "Test".into(),
            minecraft_version: "1.20.1".into(),
            loader: "fabric".into(),
            loader_version: "0.15.0".into(),
            is_locked: false,
            mods: vec![InstalledMod {
                filename: "a.jar".into(),
                source: "registry".into(),
                sha256: "aa".into(),
                installed_at: "now".into(),
                enabled: true,
                content_type: "mod".into(),
                registry_id: None,
                modrinth_id: None,
                source_url: None,
                version: None,
                java_packages: vec![],
                mod_jar_id: None,
                provided_mod_ids: vec![],
                depends_on: vec![],
                optional_deps: vec![],
                incompatible_deps: vec![],
            }],
            resourcepacks: vec![InstalledMod {
                filename: "b.zip".into(),
                source: "registry".into(),
                sha256: "bb".into(),
                installed_at: "now".into(),
                enabled: true,
                content_type: "resourcepack".into(),
                registry_id: None,
                modrinth_id: None,
                source_url: None,
                version: None,
                java_packages: vec![],
                mod_jar_id: None,
                provided_mod_ids: vec![],
                depends_on: vec![],
                optional_deps: vec![],
                incompatible_deps: vec![],
            }],
            shaders: vec![],
            datapacks: vec![],
            worlds: vec![],
            created_from_pack: None,
            user_preferences: serde_json::json!({}),
        };
        let items: Vec<&InstalledMod> = all_installed(&manifest).collect();
        assert_eq!(items.len(), 2);
    }

    // ------------------------------------------------------------------
    // fetch_github_releases_initial / compute_tail_pages / batch
    // ------------------------------------------------------------------

    #[test]
    fn github_release_auth_header_contains_one_bearer_prefix() {
        assert_eq!(
            github_auth_headers(Some("gho_test_token")),
            vec![(
                "Authorization".to_string(),
                "Bearer gho_test_token".to_string()
            )]
        );
        assert!(github_auth_headers(None).is_empty());
    }

    #[test]
    fn test_compute_tail_pages_single_page() {
        assert!(
            Resolver::compute_tail_pages(1, true).is_empty(),
            "single page with compatibles should yield no tail"
        );
        assert!(
            Resolver::compute_tail_pages(1, false).is_empty(),
            "single page without compatibles should yield no tail"
        );
    }

    #[test]
    fn test_compute_tail_pages_compatible_on_page1() {
        assert!(
            Resolver::compute_tail_pages(10, true).is_empty(),
            "compatible found on page 1: no tail needed"
        );
    }

    #[test]
    fn test_compute_tail_pages_few_pages() {
        assert_eq!(
            Resolver::compute_tail_pages(2, false),
            vec![2],
            "2 pages without compatibles: one tail page"
        );
        assert_eq!(
            Resolver::compute_tail_pages(3, false),
            vec![3, 2],
            "3 pages without compatibles: two tail pages"
        );
    }

    #[test]
    fn test_compute_tail_pages_many_pages() {
        assert_eq!(
            Resolver::compute_tail_pages(50, false),
            vec![50, 49, 48],
            "50 pages without compatibles: three oldest pages"
        );
        assert_eq!(
            Resolver::compute_tail_pages(100, false),
            vec![100, 99, 98],
            "100 pages without compatibles: three oldest pages"
        );
    }

    #[test]
    fn test_compute_tail_pages_exactly_four_pages() {
        assert_eq!(
            Resolver::compute_tail_pages(4, false),
            vec![4, 3, 2],
            "4 pages without compatibles: three oldest pages"
        );
    }

    #[tokio::test]
    async fn test_fetch_github_versions_batch_empty_pages() {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = crate::ctx::Ctx::for_testing(tmp.path().to_path_buf());
        let resolver = Resolver::new(ctx);
        let result = resolver
            .fetch_github_versions_batch("owner/repo", "1.20.1", "fabric", &[])
            .await;
        assert!(result.is_ok(), "empty pages must not fail");
        assert!(
            result.unwrap().is_empty(),
            "empty pages must return empty results"
        );
    }

    #[tokio::test]
    async fn test_fetch_github_versions_batch_single_page() {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = crate::ctx::Ctx::for_testing(tmp.path().to_path_buf());
        let resolver = Resolver::new(ctx);
        // A single page for a non-existent repo — will error but must not panic
        let result = resolver
            .fetch_github_versions_batch(
                "nonexistent/repo-does-not-exist",
                "1.20.1",
                "fabric",
                &[1],
            )
            .await;
        // The function tolerates page failures, so the outer Result is Ok
        // with whatever pages succeeded (could be empty).
        assert!(
            result.is_ok(),
            "batch must tolerate individual page failures"
        );
    }

    #[test]
    fn test_sort_merge_dedup() {
        // Verify that sorting places compatible candidates first,
        // major_match second, and nothing else third, with newer
        // dates first within each tier.
        let mut candidates = vec![
            ModVersionCandidate {
                version: "v1".into(),
                filename: "a.jar".into(),
                download_url: "".into(),
                mc_version: None,
                loader: None,
                release_date: Some("2024-01-01".into()),
                is_compatible: false,
                sha1: None,
                sha256: None,
                sha512: None,
                size: None,
                version_compat: "".into(),
            },
            ModVersionCandidate {
                version: "v2".into(),
                filename: "b.jar".into(),
                download_url: "".into(),
                mc_version: None,
                loader: None,
                release_date: Some("2024-02-01".into()),
                is_compatible: true,
                sha1: None,
                sha256: None,
                sha512: None,
                size: None,
                version_compat: "compatible".into(),
            },
            ModVersionCandidate {
                version: "v3".into(),
                filename: "c.jar".into(),
                download_url: "".into(),
                mc_version: None,
                loader: Some("fabric".into()),
                release_date: Some("2024-03-01".into()),
                is_compatible: false,
                sha1: None,
                sha256: None,
                sha512: None,
                size: None,
                version_compat: "major_match".into(),
            },
            ModVersionCandidate {
                version: "v2dup".into(),
                filename: "d.jar".into(),
                download_url: "".into(),
                mc_version: None,
                loader: None,
                release_date: Some("2024-02-15".into()),
                is_compatible: true,
                sha1: None,
                sha256: None,
                sha512: None,
                size: None,
                version_compat: "compatible".into(),
            },
        ];
        sort_versions_by_compatibility(&mut candidates);
        // First two are compatible tier (v2 and v2dup) with newer first
        assert_eq!(candidates[0].version_compat, "compatible");
        assert_eq!(candidates[1].version_compat, "compatible");
        // Compatible tier should be sorted by date descending
        assert!(
            candidates[0].release_date.as_deref().unwrap_or("")
                >= candidates[1].release_date.as_deref().unwrap_or("")
        );
        // Then major_match
        assert_eq!(candidates[2].version_compat, "major_match");
        // Then other
        assert_eq!(candidates[3].version_compat, "");
    }

    #[test]
    fn test_initial_tail_merge_correct_page_order() {
        // Simulate the merge that fetch_github_releases_initial does:
        // page 1 candidates + tail page candidates, then sorted.
        let page1 = vec![
            ModVersionCandidate {
                version: "v1".into(),
                filename: "page1_a.jar".into(),
                download_url: "".into(),
                mc_version: None,
                loader: None,
                release_date: Some("2024-01-01".into()),
                is_compatible: false,
                sha1: None,
                sha256: None,
                sha512: None,
                size: None,
                version_compat: "".into(),
            },
            ModVersionCandidate {
                version: "v2".into(),
                filename: "page1_b.jar".into(),
                download_url: "".into(),
                mc_version: None,
                loader: Some("fabric".into()),
                release_date: Some("2024-02-01".into()),
                is_compatible: false,
                sha1: None,
                sha256: None,
                sha512: None,
                size: None,
                version_compat: "major_match".into(),
            },
        ];
        let tail = vec![
            ModVersionCandidate {
                version: "v10".into(),
                filename: "tail_c.jar".into(),
                download_url: "".into(),
                mc_version: None,
                loader: None,
                release_date: Some("2023-01-01".into()),
                is_compatible: true,
                sha1: None,
                sha256: None,
                sha512: None,
                size: None,
                version_compat: "compatible".into(),
            },
            ModVersionCandidate {
                version: "v11".into(),
                filename: "tail_d.jar".into(),
                download_url: "".into(),
                mc_version: None,
                loader: None,
                release_date: Some("2023-02-01".into()),
                is_compatible: false,
                sha1: None,
                sha256: None,
                sha512: None,
                size: None,
                version_compat: "".into(),
            },
        ];
        let mut merged = page1;
        merged.extend(tail);
        sort_versions_by_compatibility(&mut merged);

        // After merge+sort: compatible first (v10), then major_match (v2), then others (v1, v11)
        assert_eq!(merged.len(), 4, "all candidates preserved after merge");
        // First: compatible
        assert_eq!(
            merged[0].filename, "tail_c.jar",
            "compatible should sort first"
        );
        assert_eq!(merged[0].version_compat, "compatible");
        // Second: major_match
        assert_eq!(
            merged[1].filename, "page1_b.jar",
            "major_match should sort second"
        );
        assert_eq!(merged[1].version_compat, "major_match");
        // Third and fourth: empty compat, sorted by date descending
        assert_eq!(merged[2].version_compat, "");
        assert_eq!(merged[3].version_compat, "");
        assert!(
            merged[2].release_date.as_deref().unwrap_or("")
                >= merged[3].release_date.as_deref().unwrap_or(""),
            "same-tier candidates sorted by date descending"
        );
    }

    #[test]
    fn test_page1_has_compatible_skips_tail_heuristic() {
        // When page 1 contains a compatible candidate, compute_tail_pages
        // should return empty. This is the decision function used by
        // fetch_github_releases_initial.
        let page1_has_compat = true;
        assert!(
            Resolver::compute_tail_pages(10, page1_has_compat).is_empty(),
            "tail pages not needed when page 1 has compatible"
        );

        let page1_has_compat = false;
        assert_eq!(
            Resolver::compute_tail_pages(10, page1_has_compat),
            vec![10, 9, 8],
            "tail pages needed when page 1 has no compatible"
        );
    }

    #[test]
    fn test_batch_context_matches_all_artifact_identities() {
        let artifact = ResolvedArtifact::Download(ResolvedDownload {
            item_id: "terrablender".into(),
            version_id: "v1".into(),
            source: ArtifactSource::Download {
                url: "https://example.com/terrablender.jar".into(),
            },
            hashes: HashSpec { values: vec![] },
            size: 100,
            filename: "TerraBlender-fabric-3.3.0.10.jar".into(),
            metadata: ArtifactMetadata {
                source_type: SourceType::Curated,
                registry_id: Some("terrablender".into()),
                modrinth_id: Some("terrablender".into()),
                content_type: "mod".into(),
                version: Some("3.3.0.10".into()),
                download_strategy: None,
            },
        });

        let ctx = BatchContext::from_artifacts([&artifact]);
        assert_eq!(
            ctx.target_filename_for("terrablender"),
            Some("TerraBlender-fabric-3.3.0.10.jar")
        );
        // Case-insensitive identity matching across item/registry/modrinth ids.
        assert_eq!(
            ctx.target_filename_for("TERRABLENDER"),
            Some("TerraBlender-fabric-3.3.0.10.jar")
        );
        assert!(ctx.target_filename_for("some-other-mod").is_none());
    }

    #[test]
    fn test_batch_context_matches_native_loader_identity_to_sibling_project() {
        let artifact = ResolvedArtifact::Download(ResolvedDownload {
            item_id: "TerraBlender-Project".into(),
            version_id: "v1".into(),
            source: ArtifactSource::Download {
                url: "https://example.com/terrablender.jar".into(),
            },
            hashes: HashSpec { values: vec![] },
            size: 100,
            filename: "TerraBlender-fabric.jar".into(),
            metadata: ArtifactMetadata {
                source_type: SourceType::Modrinth,
                registry_id: None,
                modrinth_id: Some("TerraBlender-Project".into()),
                content_type: "mod".into(),
                version: Some("1.0.0".into()),
                download_strategy: None,
            },
        });
        let native_metadata = crate::dependency_ops::JarDeps {
            mod_jar_id: Some("terrablender".into()),
            ..Default::default()
        };

        let mut ctx = BatchContext::from_artifacts([&artifact]);
        ctx.add_native_metadata(&artifact, &native_metadata);

        assert_eq!(
            ctx.target_filename_for("TerraBlender"),
            Some("TerraBlender-fabric.jar")
        );
        assert_eq!(
            ctx.project_id_for_loader("terrablender"),
            Some("terrablender-project")
        );
        assert_eq!(
            ctx.project_ids
                .get("terrablender-project")
                .map(String::as_str),
            Some("TerraBlender-Project")
        );
    }

    #[test]
    fn test_project_identity_matches_loader_name_or_slug() {
        assert!(project_identity_matches_loader(
            "glitchcore",
            "GlitchCore",
            Some("https://modrinth.com/mod/glitchcore")
        ));
        assert!(project_identity_matches_loader(
            "terra_blender",
            "A different title",
            Some("https://modrinth.com/mod/terra-blender")
        ));
        assert!(!project_identity_matches_loader(
            "another-mod",
            "GlitchCore",
            Some("https://modrinth.com/mod/glitchcore")
        ));
        assert_eq!(
            native_loader_search_identity("yet_another_config_lib_v3"),
            "yetanotherconfiglib"
        );
        assert_eq!(native_loader_search_identity("wm_binaries"), "watermedia");
        assert!(project_search_identity_matches(
            "yetanotherconfiglib",
            "YetAnotherConfigLib (YACL)",
            "yacl"
        ));
    }

    #[test]
    fn test_native_loader_dependency_collapses_into_modrinth_project_dependency() {
        let mut resolved = BTreeMap::from([
            (
                "glitchcore".into(),
                ResolvedDep {
                    mod_jar_id: "glitchcore".into(),
                    requirement: Requirement::Required,
                    source: DepSource::Jar,
                    display_name: None,
                    page_url: None,
                    disposition: DepDisposition::Unresolved {
                        reason: "native capability evidence".into(),
                    },
                },
            ),
            (
                "s3dmwky5".into(),
                ResolvedDep {
                    mod_jar_id: "s3dmwKy5".into(),
                    requirement: Requirement::Optional,
                    source: DepSource::Manifest,
                    display_name: Some("GlitchCore".into()),
                    page_url: Some("https://modrinth.com/mod/glitchcore".into()),
                    disposition: DepDisposition::InstallCandidate {
                        artifact: ResolvedArtifact::Download(ResolvedDownload {
                            item_id: "s3dmwKy5".into(),
                            version_id: "v1".into(),
                            source: ArtifactSource::Download {
                                url: "https://example.com/glitchcore.jar".into(),
                            },
                            hashes: HashSpec { values: vec![] },
                            size: 1,
                            filename: "GlitchCore.jar".into(),
                            metadata: ArtifactMetadata {
                                source_type: SourceType::Modrinth,
                                registry_id: None,
                                modrinth_id: Some("s3dmwKy5".into()),
                                content_type: "mod".into(),
                                version: Some("1.0.0".into()),
                                download_strategy: None,
                            },
                        }),
                    },
                },
            ),
        ]);
        let project_info = HashMap::from([(
            "s3dmwky5".into(),
            (
                "GlitchCore".into(),
                Some("https://modrinth.com/mod/glitchcore".into()),
            ),
        )]);

        collapse_native_project_dependencies(&mut resolved, &project_info, &HashMap::new());

        assert!(!resolved.contains_key("glitchcore"));
        assert_eq!(
            resolved["s3dmwky5"].requirement,
            Requirement::Required,
            "native required evidence must promote the real project edge"
        );
    }

    #[test]
    fn test_pinned_modrinth_dep_falls_back_to_best_candidate() {
        let candidates = vec![
            RawModrinthVersionCandidate {
                version: "2.0.0".into(),
                version_id: "best-available".into(),
                name: "current".into(),
                filename: "dep-2.0.0.jar".into(),
                download_url: "https://example.com/dep-2.0.0.jar".into(),
                sha1: Some("b".repeat(40)),
                sha512: None,
                size: Some(10),
                mc_versions: vec!["1.21".into()],
                loaders: vec!["fabric".into()],
                release_date: None,
                primary: false,
                changelog: None,
                dependencies: vec![],
            },
            RawModrinthVersionCandidate {
                version: "1.0.0".into(),
                version_id: "old-pinned".into(),
                name: "old".into(),
                filename: "dep-1.0.0.jar".into(),
                download_url: "https://example.com/dep-1.0.0.jar".into(),
                sha1: Some("a".repeat(40)),
                sha512: None,
                size: Some(10),
                mc_versions: vec!["1.20.1".into()],
                loaders: vec!["fabric".into()],
                release_date: None,
                primary: false,
                changelog: None,
                dependencies: vec![],
            },
        ];

        // The pinned version is not present in the filtered list...
        assert!(matches!(
            select_raw_modrinth_candidate(&candidates, Some("pinned-elsewhere")),
            Err(LauncherError::VersionNotFound)
        ));
        // ...so dependency resolution must fall back to the best available.
        let selected = select_raw_modrinth_candidate(&candidates, None).unwrap();
        assert_eq!(selected.version_id, "best-available");
        let closest = select_closest_raw_modrinth_candidate(&candidates, "1.20.1").unwrap();
        assert_eq!(closest.version_id, "old-pinned");
    }
}
