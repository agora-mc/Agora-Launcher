//! Reviewed pack-file import (`.mrpack` and `.agora-pack.json`).
//!
//! Every front end follows the same two steps: [`plan_pack_file`] shows what an import
//! would do and records the file's fingerprint, then [`import_pack_file`] imports exactly
//! the bytes that were reviewed. The desktop and the CLI both call these.

use crate::agora_pack_import::{self, PackImportPlan, PackImportReport};
use crate::import_service::{ImportRequest, ImportService, ImportSource};
use crate::instance_service::{CreateInstanceRequest, InstanceService};
use agora_core::ctx::Ctx;
use agora_core::error::{LauncherError, LauncherResult};
use std::path::Path;

/// Builds the sink that receives instance-creation progress for the instance id
/// the import chose. The desktop returns an emitter for its Create dialog; the
/// CLI returns the context's own sink.
pub type CreateProgressFactory<'a> =
    &'a (dyn Fn(&str) -> std::sync::Arc<dyn agora_core::event_sink::ProgressSink> + Send + Sync);

/// What an import produced: the new instance, and for an Agora pack the
/// per-mod report (installed / failed / needs action) a front end must show.
#[derive(Debug, Clone, serde::Serialize)]
pub struct PackImportOutcome {
    pub instance_id: String,
    pub report: Option<PackImportReport>,
}

/// What importing a pack file would do. Reads the file and this install's
/// settings; contacts no source. An `.mrpack` is staged for review (see
/// [`ImportService::take_reviewed_copy`]).
pub fn plan_pack_file(ctx: &Ctx, source_path: &str) -> LauncherResult<PackImportPlan> {
    let lower = source_path.to_ascii_lowercase();
    if lower.ends_with(".mrpack") {
        return ImportService::new(ctx.clone()).plan_mrpack_import(Path::new(source_path));
    }
    if !lower.ends_with(".json") {
        return Err(LauncherError::Generic {
            code: "ERR_INVALID_FORMAT".into(),
            message: "Only .agora-pack.json and .mrpack files have a review step.".into(),
        });
    }
    let bytes = read_agora_pack_bytes(source_path)?;
    agora_pack_import::plan_agora_pack_bytes(ctx, &bytes)
}

/// The bytes of a `.agora-pack.json`, refusing one too large to read. A file
/// whose size cannot be read is refused too, rather than read unchecked. The
/// pack parser re-checks the size of the bytes it is given.
pub fn read_agora_pack_bytes(source_path: &str) -> LauncherResult<Vec<u8>> {
    let unreadable = || LauncherError::Generic {
        code: "ERR_PACK_READ".into(),
        message: format!("Cannot read pack file: {source_path}"),
    };
    let too_big = std::fs::metadata(source_path)
        .map_err(|_| unreadable())?
        .len()
        > agora_pack_import::MAX_PACK_FILE_BYTES;
    if too_big {
        return Err(agora_pack_import::pack_too_big_error());
    }
    std::fs::read(source_path).map_err(|_| unreadable())
}

/// Import a pack file. `.mrpack` goes through [`ImportService`]; `.agora-pack.json`
/// creates the instance and installs each entry. `reviewed_sha256` is the
/// fingerprint the review recorded; bytes that no longer match it are refused.
pub async fn import_pack_file(
    ctx: &Ctx,
    source_path: &str,
    reviewed_sha256: Option<&str>,
    name: Option<&str>,
    skip_unfetchable: bool,
    create_progress: CreateProgressFactory<'_>,
) -> LauncherResult<PackImportOutcome> {
    let lower = source_path.to_ascii_lowercase();
    if lower.ends_with(".mrpack") {
        Ok(PackImportOutcome {
            instance_id: import_mrpack(ctx, reviewed_sha256, name, skip_unfetchable).await?,
            report: None,
        })
    } else if lower.ends_with(".json") || lower.ends_with(".agora-pack.json") {
        import_agora_json(ctx, source_path, reviewed_sha256, name, create_progress).await
    } else {
        Err(LauncherError::Generic {
            code: "ERR_INVALID_FORMAT".into(),
            message: "Unsupported pack file extension. Use .mrpack or .agora-pack.json.".into(),
        })
    }
}

/// Imports the staged copy the review made (not the original file, which may
/// have changed since), then removes that copy.
async fn import_mrpack(
    ctx: &Ctx,
    reviewed_sha256: Option<&str>,
    name: Option<&str>,
    skip_unfetchable: bool,
) -> LauncherResult<String> {
    let svc = ImportService::new(ctx.clone());
    let staged = svc.take_reviewed_copy(reviewed_sha256)?;
    let request = ImportRequest {
        source: ImportSource::Mrpack {
            path: staged,
            origin_url: None,
            skip_unfetchable,
        },
        symlink_saves: false,
    };
    let result = svc
        .run_import_named(
            request,
            name.map(str::to_string),
            ctx.progress_sink.clone(),
            agora_core::event_sink::CancellationToken::new(),
        )
        .await;
    if let Some(sha) = reviewed_sha256 {
        svc.discard_reviewed_copy(sha);
    }
    Ok(result?.instance_id)
}

/// Import an Agora plain-JSON pack (`.agora-pack.json`): create the instance,
/// then install each entry's pinned file, checked against the pack's hash.
async fn import_agora_json(
    ctx: &Ctx,
    source_path: &str,
    reviewed_sha256: Option<&str>,
    name_override: Option<&str>,
    create_progress: CreateProgressFactory<'_>,
) -> LauncherResult<PackImportOutcome> {
    // One read: the bytes checked against the reviewed fingerprint are the
    // bytes parsed and installed.
    let bytes = read_agora_pack_bytes(source_path)?;
    let pack = agora_pack_import::parse_reviewed_agora_pack(&bytes, reviewed_sha256)?;
    let inst = &pack.instance;
    // The name the user chose in the review wins, and its folder follows it;
    // without one the pack's own id and name are used.
    let chosen = name_override.map(str::trim).filter(|n| !n.is_empty());
    let name = chosen
        .map(str::to_string)
        .unwrap_or_else(|| agora_pack_import::pack_display_name(inst));
    let instance_id = agora_core::paths::sanitize_id(match chosen {
        Some(chosen) => chosen,
        None => inst.id.as_deref().unwrap_or(&name),
    });
    let instance_id = if instance_id.is_empty() {
        "imported-pack".into()
    } else {
        instance_id
    };
    let req = CreateInstanceRequest {
        name: name.clone(),
        instance_id: instance_id.clone(),
        minecraft_version: inst.minecraft_version.clone(),
        loader: inst.loader.clone(),
        loader_version: inst.loader_version.clone(),
        jvm_memory_mb: Some(4096),
        jvm_memory_mode: Some("auto".into()),
        jvm_gc: None,
        jvm_custom_args: None,
        jvm_always_pre_touch: None,
        is_modpack: None,
        pack_icon_url: None,
        template_id: None,
    };
    let creating = InstanceService::new(
        ctx.clone()
            .with_progress_sink(create_progress(&instance_id)),
    );
    creating.create(req).await?;
    let instances = InstanceService::new(ctx.clone());
    let mut report =
        agora_pack_import::install_agora_pack_mods(ctx, &instance_id, &name, pack).await;
    if agora_pack_import::nothing_installed_for_lack_of_connection(&report) {
        // Nothing usable was made, so do not leave an empty instance behind.
        // It goes to the trash, and a later import starts clean.
        match instances.delete(&instance_id) {
            Ok(()) => {
                let subject = if report.total == 1 {
                    "This mod needs".to_string()
                } else {
                    format!("These {} mods need", report.total)
                };
                return Err(LauncherError::Generic {
                    code: "ERR_PACK_OFFLINE".into(),
                    message: format!(
                        "{subject} to be downloaded. Couldn't connect: no internet, or the \
                         server can't be reached right now. Nothing was installed, and the new \
                         instance was removed. Try again when you're online."
                    ),
                });
            }
            Err(error) => report.warnings.push(format!(
                "The mods could not be downloaded for lack of a connection, and the new \
                 instance could not be removed ({error}). It was kept; try again when you're online."
            )),
        }
    }
    Ok(PackImportOutcome {
        instance_id,
        report: Some(report),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use agora_core::ctx::CoreContext;
    use agora_core::event_sink::{CollectingProgressSink, ProgressSink};
    use std::sync::{Arc, Mutex};

    #[tokio::test]
    async fn agora_pack_import_reports_create_progress_for_the_chosen_instance() {
        let root =
            std::env::temp_dir().join(format!("agora-pack-file-import-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let ctx = CoreContext::for_testing(root.clone());
        let _ = agora_core::db::init_local_state_db(&ctx.paths.local_state_db());
        let file = root.join("p.agora-pack.json");
        std::fs::write(
            &file,
            r#"{"format":"agora-pack/v1","instance":{"id":"prog-pack","name":"Prog","minecraft_version":"1.21","loader":"fabric","loader_version":"0.16.0"},"mods":[]}"#,
        )
        .unwrap();
        let path = file.to_string_lossy().into_owned();
        let plan = plan_pack_file(&ctx, &path).unwrap();

        let sink = Arc::new(CollectingProgressSink::new());
        let seen_ids = Arc::new(Mutex::new(Vec::<String>::new()));
        let (sink_for_factory, ids) = (sink.clone(), seen_ids.clone());
        let factory = move |id: &str| -> Arc<dyn ProgressSink> {
            ids.lock().unwrap().push(id.to_string());
            sink_for_factory.clone()
        };
        // The loader download needs a network, so the import may fail; the
        // create step still reports before that.
        let _ = import_pack_file(&ctx, &path, Some(&plan.file_sha256), None, false, &factory).await;

        assert_eq!(*seen_ids.lock().unwrap(), vec!["prog-pack".to_string()]);
        assert!(
            !sink.events().is_empty(),
            "instance creation must report progress to the sink the front end gave"
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
