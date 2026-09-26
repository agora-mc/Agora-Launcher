//! Turning a provider's plan into something Agora installs.
//!
//! The provider has already said *what*. Everything here is Agora deciding
//! *whether* and *how*: validating and authorizing the plan, expressing a file
//! plan as an ordinary pipeline artifact (so it gets the same staging,
//! verification, snapshot, rollback and manifest recording as curated
//! content), resolving its declared dependencies through the same provider,
//! and handing a pack plan to the import service.

use super::{authorize_plan, parse_item_id, ProviderRegistry, ResolveRequest};
use crate::ctx::Ctx;
use crate::dependency_ops::{DepSource, Requirement};
use crate::error::{LauncherError, LauncherResult};
use crate::install_pipeline::{
    ArtifactMetadata, ArtifactSource, ConflictKind, ConflictResolution, DepConflict,
    DepDisposition, HashAlgorithm, HashSpec, HashedValue, ProviderArtifact, ResolvedArtifact,
    ResolvedDep, ResolvedDownload, SourceType,
};
use crate::models::{InstalledMod, InstanceManifest};
use agora_plugin_api::provider::{DependencyKind, FilePlan, InstallPlan};
use std::collections::{BTreeMap, BTreeSet};

/// How deep required dependencies are followed. Real mod graphs are shallow;
/// the bound exists so a provider cannot make one click fan out forever.
const MAX_DEPENDENCY_DEPTH: usize = 4;
/// Most dependencies one install may pull in.
const MAX_DEPENDENCIES: usize = 48;

fn not_a_provider_item(item_id: &str) -> LauncherError {
    LauncherError::Generic {
        code: "ERR_PROVIDER_ITEM".into(),
        message: format!("`{item_id}` is not a content-provider item."),
    }
}

fn is_installed_from(item: &InstalledMod, provider_id: &str, project_id: &str) -> bool {
    item.provider
        .as_ref()
        .is_some_and(|origin| origin.provider_id == provider_id && origin.project_id == project_id)
}

fn installed<'a>(
    manifest: &'a InstanceManifest,
    provider_id: &str,
    project_id: &str,
) -> Option<&'a InstalledMod> {
    manifest
        .mods
        .iter()
        .chain(manifest.resourcepacks.iter())
        .chain(manifest.shaders.iter())
        .chain(manifest.datapacks.iter())
        .find(|item| is_installed_from(item, provider_id, project_id))
}

/// A file plan as a pipeline artifact, carrying every digest the provider
/// published for the executor to re-check.
fn artifact_from(
    provider_id: &str,
    project_id: &str,
    plan: &FilePlan,
    download_hosts: &[String],
    low_security: bool,
) -> LauncherResult<ResolvedArtifact> {
    let hashes = &plan.file.hashes;
    let values: Vec<HashedValue> = [
        (HashAlgorithm::Sha512, &hashes.sha512),
        (HashAlgorithm::Sha256, &hashes.sha256),
        (HashAlgorithm::Sha1, &hashes.sha1),
        (HashAlgorithm::Md5, &hashes.md5),
    ]
    .into_iter()
    .filter_map(|(algorithm, value)| {
        value.as_ref().map(|value| HashedValue {
            algorithm,
            value: value.to_ascii_lowercase(),
        })
    })
    .collect();
    Ok(ResolvedArtifact::Download(ResolvedDownload {
        item_id: super::item_id(provider_id, project_id),
        version_id: plan.version_id.clone(),
        source: ArtifactSource::Download {
            url: plan.file.url.clone(),
        },
        hashes: HashSpec { values },
        size: plan.file.size.unwrap_or(0),
        filename: plan.file.filename.clone(),
        metadata: ArtifactMetadata {
            provider: Some(ProviderArtifact {
                provider_id: provider_id.to_string(),
                project_id: project_id.to_string(),
                version_id: plan.version_id.clone(),
                download_hosts: download_hosts.to_vec(),
                low_security,
            }),
            source_type: SourceType::Provider,
            registry_id: None,
            modrinth_id: None,
            content_type: plan.content_type.clone(),
            version: Some(plan.version_number.clone()),
            download_strategy: None,
            pinned_host: None,
        },
    }))
}

/// Resolve one project into a file artifact, authorized under the shared rule.
async fn resolve_file(
    ctx: &Ctx,
    registry: &ProviderRegistry,
    manifest: &InstanceManifest,
    provider_id: &str,
    project_id: &str,
    version_id: Option<&str>,
) -> LauncherResult<(ResolvedArtifact, FilePlan)> {
    let provider = registry.usable(provider_id)?;
    let descriptor = provider.descriptor();
    let plan = provider
        .resolve(ResolveRequest {
            project_id: project_id.to_string(),
            version_id: version_id.map(str::to_string),
            minecraft_version: manifest.minecraft_version.clone(),
            loader: manifest.loader.clone(),
        })
        .await?;
    let authorization = authorize_plan(ctx, &plan, &descriptor.download_hosts)?;
    let InstallPlan::File(file) = plan else {
        return Err(LauncherError::Generic {
            code: "ERR_USE_PACK_INSTALL".into(),
            message: format!(
                "{} describes this as a modpack; install it as a new instance.",
                descriptor.title
            ),
        });
    };
    let artifact = artifact_from(
        provider_id,
        project_id,
        &file,
        &descriptor.download_hosts,
        authorization.is_low_security(),
    )?;
    Ok((artifact, file))
}

/// Everything a provider install contributes to a plan.
pub struct ProviderResolution {
    pub artifact: ResolvedArtifact,
    pub dependencies: Vec<ResolvedDep>,
    pub conflicts: Vec<DepConflict>,
}

/// Resolve a `provider:<provider>:<project>` item for an instance, following
/// its declared dependencies through the same provider.
///
/// Dependencies are identified by provider project, not by jar metadata: a
/// provider's word about *its own* projects is what it can speak to. Jar-level
/// checks still run afterwards in the pipeline's health scan, as they do for
/// every other source.
pub async fn resolve_item(
    ctx: &Ctx,
    registry: &ProviderRegistry,
    manifest: &InstanceManifest,
    item_id: &str,
    version_id: Option<&str>,
) -> LauncherResult<ProviderResolution> {
    let (provider_id, project_id) =
        parse_item_id(item_id).ok_or_else(|| not_a_provider_item(item_id))?;
    let (artifact, root) =
        resolve_file(ctx, registry, manifest, provider_id, project_id, version_id).await?;

    let mut dependencies = Vec::new();
    let mut conflicts = Vec::new();
    let mut seen = BTreeSet::from([project_id.to_string()]);
    // (project, pinned version, requirement, depth)
    let mut queue: Vec<(String, Option<String>, Requirement, usize)> = Vec::new();
    let enqueue = |plan: &FilePlan,
                   depth: usize,
                   queue: &mut Vec<(String, Option<String>, Requirement, usize)>,
                   conflicts: &mut Vec<DepConflict>| {
        for dependency in &plan.dependencies {
            match dependency.kind {
                DependencyKind::Embedded => {}
                DependencyKind::Incompatible => {
                    if let Some(existing) = installed(manifest, provider_id, &dependency.project_id)
                    {
                        conflicts.push(DepConflict {
                            conflict_id: format!(
                                "provider-incompatible:{provider_id}:{}",
                                dependency.project_id
                            ),
                            kind: ConflictKind::IncompatibleMod,
                            existing_mod_jar_id: super::item_id(
                                provider_id,
                                &dependency.project_id,
                            ),
                            incoming_mod_jar_id: super::item_id(provider_id, project_id),
                            message: format!(
                                "{} is marked incompatible with {} by {provider_id}.",
                                plan.file.filename, existing.filename
                            ),
                            blocking: true,
                            resolution_options: vec![
                                ConflictResolution::DisableExisting,
                                ConflictResolution::Abort,
                            ],
                            chosen: None,
                        });
                    }
                }
                DependencyKind::Required | DependencyKind::Optional => {
                    // Optional dependencies are offered one level deep only:
                    // what an optional dependency itself needs is a question
                    // for when the user opts into it.
                    if dependency.kind == DependencyKind::Optional && depth > 0 {
                        continue;
                    }
                    let requirement = if dependency.kind == DependencyKind::Required {
                        Requirement::Required
                    } else {
                        Requirement::Optional
                    };
                    queue.push((
                        dependency.project_id.clone(),
                        dependency.version_id.clone(),
                        requirement,
                        depth + 1,
                    ));
                }
            }
        }
    };
    enqueue(&root, 0, &mut queue, &mut conflicts);

    let mut index = 0;
    while index < queue.len() {
        let (dep_project, dep_version, requirement, depth) = queue[index].clone();
        index += 1;
        if !seen.insert(dep_project.clone()) {
            continue;
        }
        let dep_item = super::item_id(provider_id, &dep_project);
        let disposition = if let Some(existing) = installed(manifest, provider_id, &dep_project) {
            DepDisposition::ReuseExisting {
                mod_jar_id: dep_item.clone(),
                installed_filename: existing.filename.clone(),
            }
        } else if dependencies.len() >= MAX_DEPENDENCIES || depth > MAX_DEPENDENCY_DEPTH {
            DepDisposition::Unresolved {
                reason: "Too many nested dependencies to resolve automatically.".into(),
            }
        } else {
            match resolve_file(
                ctx,
                registry,
                manifest,
                provider_id,
                &dep_project,
                dep_version.as_deref(),
            )
            .await
            {
                Ok((artifact, plan)) => {
                    if requirement == Requirement::Required {
                        enqueue(&plan, depth, &mut queue, &mut conflicts);
                    }
                    DepDisposition::InstallCandidate {
                        artifact: Box::new(artifact),
                    }
                }
                Err(error) => DepDisposition::Unresolved {
                    reason: error.to_string(),
                },
            }
        };
        dependencies.push(ResolvedDep {
            mod_jar_id: dep_item,
            requirement,
            source: DepSource::Manifest,
            disposition,
            display_name: None,
            page_url: None,
        });
    }

    Ok(ProviderResolution {
        artifact,
        dependencies,
        conflicts,
    })
}

/// Install a provider's pack as a new instance.
pub async fn install_pack(
    ctx: &Ctx,
    registry: &ProviderRegistry,
    item_id: &str,
    version_id: Option<&str>,
) -> LauncherResult<crate::import::ImportResult> {
    let (provider_id, project_id) =
        parse_item_id(item_id).ok_or_else(|| not_a_provider_item(item_id))?;
    let provider = registry.usable(provider_id)?;
    let descriptor = provider.descriptor();
    let plan = provider
        .resolve(ResolveRequest {
            project_id: project_id.to_string(),
            version_id: version_id.map(str::to_string),
            minecraft_version: String::new(),
            loader: String::new(),
        })
        .await?;
    let authorization = authorize_plan(ctx, &plan, &descriptor.download_hosts)?;
    let InstallPlan::Pack(pack) = plan else {
        return Err(LauncherError::Generic {
            code: "ERR_NOT_A_PACK".into(),
            message: format!(
                "{} describes this as a single file; install it into an instance.",
                descriptor.title
            ),
        });
    };
    crate::import_service::ImportService::new(ctx.clone())
        .run_import(crate::import_service::ImportRequest {
            source: crate::import_service::ImportSource::ProviderPack(
                crate::import::ProviderPackImport {
                    provider_id: provider_id.to_string(),
                    provider_title: descriptor.title,
                    project_id: project_id.to_string(),
                    plan: pack,
                    download_hosts: descriptor.download_hosts,
                    low_security_accepted: authorization.is_low_security(),
                    override_policy: crate::override_sanitizer::OverridePolicy::from_settings(ctx),
                },
            ),
            symlink_saves: false,
        })
        .await
}

/// A dry run of what installing an item would involve, for the review
/// prompt: the verdict the user is being asked to accept, before anything is
/// downloaded.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanPreview {
    pub provider_id: String,
    pub provider_title: String,
    pub kind: &'static str,
    pub name: String,
    pub version: String,
    pub file_count: usize,
    /// Reduced assurance: shown as a warning the user can continue past.
    pub warnings: Vec<super::SecurityNote>,
    /// No integrity information at all: needs low security downloads on.
    pub low_security: Vec<super::SecurityNote>,
    /// Whether the user currently allows low security downloads.
    pub low_security_allowed: bool,
    /// Pack files outside the usual content folders: need reduced security mode.
    pub outside_content_folders: Vec<String>,
    /// Whether the user currently has reduced security mode on.
    pub reduced_security_enabled: bool,
    /// Download hosts by number of files, so the prompt can say where things
    /// come from rather than listing every URL.
    pub hosts: BTreeMap<String, usize>,
}

pub async fn preview(
    ctx: &Ctx,
    registry: &ProviderRegistry,
    item_id: &str,
    version_id: Option<&str>,
    minecraft_version: &str,
    loader: &str,
) -> LauncherResult<PlanPreview> {
    let (provider_id, project_id) =
        parse_item_id(item_id).ok_or_else(|| not_a_provider_item(item_id))?;
    let provider = registry.usable(provider_id)?;
    let descriptor = provider.descriptor();
    let plan = provider
        .resolve(ResolveRequest {
            project_id: project_id.to_string(),
            version_id: version_id.map(str::to_string),
            minecraft_version: minecraft_version.to_string(),
            loader: loader.to_string(),
        })
        .await?;
    plan.validate().map_err(|e| LauncherError::Generic {
        code: "ERR_PROVIDER_PLAN_INVALID".into(),
        message: e.message,
    })?;
    let verdict = super::judge_plan(&plan, &descriptor.download_hosts);
    let low_security_allowed = super::low_security_allowed(ctx);
    let mut hosts = BTreeMap::new();
    for url in plan.urls() {
        let host = reqwest::Url::parse(url)
            .ok()
            .and_then(|u| u.host_str().map(str::to_string))
            .unwrap_or_default();
        *hosts.entry(host).or_insert(0) += 1;
    }
    let (kind, name, version, file_count) = match &plan {
        InstallPlan::File(file) => (
            "file",
            file.file.filename.clone(),
            file.version_number.clone(),
            1,
        ),
        InstallPlan::Pack(pack) => (
            "pack",
            pack.name.clone(),
            pack.version_number
                .clone()
                .unwrap_or_else(|| pack.version_id.clone()),
            pack.files.len() + usize::from(pack.overrides.is_some()),
        ),
    };
    Ok(PlanPreview {
        provider_id: descriptor.id,
        provider_title: descriptor.title,
        kind,
        name,
        version,
        file_count,
        warnings: verdict.warnings,
        low_security: verdict.low_security,
        low_security_allowed,
        outside_content_folders: verdict.outside_content_folders,
        reduced_security_enabled: crate::settings::reduced_security_enabled(ctx),
        hosts,
    })
}
