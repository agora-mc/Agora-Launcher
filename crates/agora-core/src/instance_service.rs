//! Core-owned instance lifecycle and CRUD operations.

use crate::clone::ClonePrefs;
use crate::ctx::Ctx;
use crate::error::{LauncherError, LauncherResult};
use crate::event_sink::{CoreEvent, EventStatus, ProgressEvent, ProgressPhase};
use crate::loader_service::LoaderService;
use crate::models::{InstanceManifest, InstanceRow};
use std::path::Path;
use std::sync::Arc;

/// Request used by every frontend to create an instance.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct CreateInstanceRequest {
    pub name: String,
    pub instance_id: String,
    pub minecraft_version: String,
    pub loader: String,
    pub loader_version: String,
    #[serde(default)]
    pub jvm_memory_mb: Option<i64>,
    #[serde(default)]
    pub jvm_memory_mode: Option<String>,
    #[serde(default)]
    pub jvm_gc: Option<String>,
    #[serde(default)]
    pub jvm_custom_args: Option<String>,
    #[serde(default)]
    pub jvm_always_pre_touch: Option<bool>,
    #[serde(default)]
    pub is_modpack: Option<bool>,
    #[serde(default)]
    pub pack_icon_url: Option<String>,
    /// Optional instance template to seed this instance from.
    ///
    /// The template supplies config files and JVM settings; anything the
    /// request states explicitly wins, so a user who typed a heap size in the
    /// create dialog keeps it even when the template also carries one.
    #[serde(default)]
    pub template_id: Option<String>,
}

/// Settings key holding the id of the template applied when a create request
/// names none.
pub const DEFAULT_TEMPLATE_SETTING_KEY: &str = "default_instance_template";

/// Request used to clone an existing instance.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct CloneRequest {
    pub source_instance_id: String,
    pub new_name: String,
    #[serde(default)]
    pub prefs: ClonePrefs,
}

/// Combined database and manifest view.
#[derive(Debug, Clone, serde::Serialize)]
pub struct InstanceDetail {
    pub row: InstanceRow,
    pub manifest: Option<InstanceManifest>,
    pub snapshot_readiness: String,
    pub snapshot_error: Option<String>,
    /// Pack entries still missing from the instance; empty when none are known missing.
    pub pack_gaps: Vec<crate::db::InstancePackGap>,
}

/// What applying a template to an existing instance actually did.
///
/// Structured rather than a file count, because a count of zero has three
/// different meanings — a JVM-only profile that applied cleanly, a template
/// with nothing in it, and a damaged template whose files are missing — and a
/// UI reading only the number told the user "no config files to apply" after a
/// successful apply.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TemplateApplyOutcome {
    pub jvm_applied: bool,
    pub files_applied: usize,
    /// Files the template lists but no longer has on disk.
    pub files_missing: usize,
    /// The undo point taken before any file was written, when there were files
    /// to write.
    pub undo_snapshot_id: Option<String>,
}

/// Core-owned preparation result for an official-launcher handoff.
#[derive(Debug, Clone, serde::Serialize)]
pub struct DelegatedLaunchPreparation {
    pub profile_id: String,
    pub launcher_path: Option<String>,
    pub mod_ids: Vec<String>,
    /// The official launcher was already running when this handoff was
    /// prepared. It reads `launcher_profiles.json` and its saved installation
    /// selection once, at startup, so a handoff into a running launcher only
    /// focuses its window — whatever it already had selected stays selected.
    /// Adapters should offer to restart it rather than pretend the handoff
    /// picked the pack.
    pub launcher_running: bool,
    /// Agora managed to point the launcher's saved selection at this profile.
    /// False when the launcher was running, or when its UI-state file was
    /// absent or in a shape Agora declines to rewrite.
    pub selection_applied: bool,
}

/// Narrow adapter type: a synchronous callback that moves a quarantined
/// directory to the OS trash. The desktop adapter supplies the real
/// `trash::delete` implementation; core never depends on the `trash` crate.
pub type TrashFn = Arc<dyn Fn(&Path) -> LauncherResult<()> + Send + Sync>;

/// Core instance lifecycle service.
#[derive(Clone)]
pub struct InstanceService {
    ctx: Ctx,
}

impl InstanceService {
    pub fn new(ctx: Ctx) -> Self {
        Self { ctx }
    }

    pub fn list(&self) -> LauncherResult<Vec<InstanceRow>> {
        let conn = self.connection()?;
        crate::db::list_instances(&conn).map_err(|error| LauncherError::Generic {
            code: "ERR_LOCAL_STATE_FAILED".into(),
            message: error.to_string(),
        })
    }

    pub fn get(&self, instance_id: &str) -> LauncherResult<Option<InstanceDetail>> {
        let instance_id = self.validate_id(instance_id)?;
        let conn = self.connection()?;
        let Some(row) = crate::db::get_instance(&conn, &instance_id).map_err(|error| {
            LauncherError::Generic {
                code: "ERR_LOCAL_STATE_FAILED".into(),
                message: error.to_string(),
            }
        })?
        else {
            return Ok(None);
        };
        let manifest = read_manifest(&self.ctx.paths.instance_manifest(&instance_id)?)?;
        let instance_dir = self.ctx.paths.instance_dir(&instance_id)?;
        let readiness = crate::snapshot::snapshot_readiness(&instance_dir);
        let pack_gaps =
            crate::db::list_instance_pack_gaps(&conn, &instance_id).map_err(|error| {
                LauncherError::Generic {
                    code: "ERR_LOCAL_STATE_FAILED".into(),
                    message: error.to_string(),
                }
            })?;
        // A gap is over once the file is in the instance by any route (a retry,
        // or the user adding the same jar by hand). Without a manifest we cannot
        // tell, so every recorded gap stays.
        let pack_gaps = match &manifest {
            Some(manifest) => pack_gaps
                .into_iter()
                .filter(|gap| !manifest_has_file(manifest, &gap.filename))
                .collect(),
            None => pack_gaps,
        };
        Ok(Some(InstanceDetail {
            row,
            manifest,
            snapshot_readiness: readiness.as_str().into(),
            snapshot_error: crate::snapshot::snapshot_readiness_error(&instance_dir),
            pack_gaps,
        }))
    }

    /// Resolve the effective launch mode after applying an instance override.
    pub fn resolve_direct_launch(
        &self,
        instance_id: &str,
        requested_direct: bool,
    ) -> LauncherResult<bool> {
        let instance_id = self.validate_id(instance_id)?;
        let conn = self.connection()?;
        let row = crate::db::get_instance(&conn, &instance_id)
            .map_err(|error| LauncherError::Generic {
                code: "ERR_LOCAL_STATE_FAILED".into(),
                message: error.to_string(),
            })?
            .ok_or_else(|| LauncherError::Generic {
                code: "ERR_INSTANCE_NOT_FOUND".into(),
                message: format!("Instance '{instance_id}' not found"),
            })?;
        Ok(match row.launch_mode_override.as_str() {
            "direct" => true,
            "delegated" => false,
            _ => requested_direct,
        })
    }

    /// Lock the instance and take the lock snapshot Revert returns to.
    ///
    /// The snapshot is taken before the lock is set, so it records the
    /// unlocked state, and a failed snapshot leaves the instance unlocked:
    /// locking without one would leave Revert pointing at an older state.
    /// The new snapshot replaces the previous lock snapshot.
    pub fn lock(&self, instance_id: &str) -> LauncherResult<()> {
        let instance_id = self.validate_id(instance_id)?;
        let instance_dir = self.ctx.paths.instance_dir(&instance_id)?;
        let _guard = self.ctx.lock_manager.acquire(
            crate::lock_manager::LockResource::Instance(instance_id.clone()),
            "lock_instance",
        )?;
        let label = format!("Locked {}", chrono::Local::now().format("%Y-%m-%d %H:%M"));
        let snapshot = crate::snapshot::create_snapshot_scoped_with_origin(
            &instance_dir,
            Some(&label),
            crate::snapshot::prelaunch_tracked_entries(),
            crate::snapshot::SnapshotOrigin::Lock,
        )
        .map_err(|error| LauncherError::Generic {
            code: "ERR_LOCK_SNAPSHOT".into(),
            message: format!(
                "Could not take the lock snapshot, so the instance was not locked: {error}"
            ),
        })?;
        self.set_locked(&instance_id, true)?;
        // Revert picks the newest lock snapshot, so a failed cleanup of the old
        // ones leaves clutter, not a wrong revert. Say so rather than hide it.
        match crate::snapshot::lock_snapshots(&instance_dir) {
            Ok(old) => {
                for previous in old.into_iter().filter(|s| s.id != snapshot.id) {
                    if let Err(error) =
                        crate::snapshot::delete_snapshot(&instance_dir, &previous.id)
                    {
                        eprintln!(
                            "[lock] could not remove the previous lock snapshot {}: {error}",
                            previous.id
                        );
                    }
                }
            }
            Err(error) => eprintln!("[lock] could not list lock snapshots to replace: {error}"),
        }
        Ok(())
    }

    pub fn unlock(&self, instance_id: &str) -> LauncherResult<()> {
        self.set_locked(instance_id, false)
    }

    pub fn update_java(
        &self,
        instance_id: &str,
        java_path: Option<&str>,
        allow_incompatible: bool,
        custom_args: Option<&str>,
    ) -> LauncherResult<()> {
        let instance_id = self.validate_id(instance_id)?;
        let conn = self.connection()?;
        let _row = crate::db::get_instance(&conn, &instance_id)
            .map_err(|error| LauncherError::Generic {
                code: "ERR_LOCAL_STATE_FAILED".into(),
                message: error.to_string(),
            })?
            .ok_or_else(|| LauncherError::Generic {
                code: "ERR_INSTANCE_NOT_FOUND".into(),
                message: format!("Instance '{instance_id}' not found"),
            })?;
        crate::db::update_instance_java(
            &conn,
            &instance_id,
            java_path,
            allow_incompatible,
            custom_args,
        )
        .map_err(|error| LauncherError::Generic {
            code: "ERR_LOCAL_STATE_FAILED".into(),
            message: error.to_string(),
        })?;
        self.record_change(&instance_id, "update_java");
        Ok(())
    }

    /// Apply a template's JVM settings to an existing instance.
    ///
    /// This is what makes a JVM-only template a *named Java profile*: without
    /// it, applying such a template to an instance you already have does
    /// nothing at all, because file application is the only thing that runs.
    ///
    /// A `None` field means "leave the instance's own value alone", so a
    /// profile that only pins heap size does not silently reset a GC choice
    /// the user made per instance. Returns whether anything changed.
    pub fn apply_template_jvm(
        &self,
        instance_id: &str,
        jvm: &crate::template_service::TemplateJvm,
    ) -> LauncherResult<bool> {
        if jvm.is_empty() {
            return Ok(false);
        }
        let instance_id = self.validate_id(instance_id)?;
        let conn = self.connection()?;
        let row = crate::db::get_instance(&conn, &instance_id)
            .map_err(|error| LauncherError::Generic {
                code: "ERR_LOCAL_STATE_FAILED".into(),
                message: error.to_string(),
            })?
            .ok_or_else(|| LauncherError::Generic {
                code: "ERR_INSTANCE_NOT_FOUND".into(),
                message: format!("Instance '{instance_id}' not found"),
            })?;
        drop(conn);

        let memory_mb = jvm.jvm_memory_mb.unwrap_or(row.jvm_memory_mb);
        let memory_mode = jvm
            .jvm_memory_mode
            .clone()
            .unwrap_or_else(|| row.jvm_memory_mode.clone());
        let gc = jvm.jvm_gc.clone().unwrap_or_else(|| row.jvm_gc.clone());
        let custom_args = jvm
            .jvm_custom_args
            .clone()
            .unwrap_or_else(|| row.jvm_custom_args.clone());
        let always_pre_touch = jvm.jvm_always_pre_touch.unwrap_or(row.jvm_always_pre_touch);

        self.update_jvm(
            &instance_id,
            memory_mb,
            &gc,
            always_pre_touch,
            &custom_args,
            &memory_mode,
        )?;

        // The Java binary lives on a different column and a different setter,
        // and is only touched when the template actually names one.
        if let Some(java_path) = jvm.java_path.as_deref() {
            self.update_java(
                &instance_id,
                Some(java_path),
                row.java_incompatible_override,
                None,
            )?;
        }
        Ok(true)
    }

    /// Apply a whole template — JVM settings *and* captured files — to an
    /// existing instance, as one undoable operation.
    ///
    /// The two halves used to run unguarded and in the wrong order: JVM
    /// settings were committed first, then files were copied over the instance
    /// with no lock, no snapshot and no rollback, so a failure partway left
    /// settings changed and configs half-replaced with nothing to undo.
    ///
    /// Here the files go first under an undo snapshot; the database write only
    /// happens once they have all landed, and any failure restores the
    /// snapshot so the instance is exactly as it was.
    pub fn apply_template(
        &self,
        instance_id: &str,
        templates_root: &std::path::Path,
        template_id: &str,
    ) -> LauncherResult<TemplateApplyOutcome> {
        let instance_id = self.validate_id(instance_id)?;
        let template = crate::template_service::get_template(templates_root, template_id).map_err(
            |message| LauncherError::Generic {
                code: "ERR_TEMPLATE".into(),
                message,
            },
        )?;
        let instance_dir = self.ctx.paths.instance_dir(&instance_id)?;

        let _lock = self.ctx.lock_manager.acquire(
            crate::lock_manager::LockResource::Instance(instance_id.clone()),
            "apply_template",
        )?;

        // Nothing to do is not a failure, and must not cost a snapshot.
        if template.jvm.is_empty() && template.files.is_empty() {
            return Ok(TemplateApplyOutcome {
                jvm_applied: false,
                files_applied: 0,
                files_missing: 0,
                undo_snapshot_id: None,
            });
        }

        let template_error = |message: String| LauncherError::Generic {
            code: "ERR_TEMPLATE".into(),
            message,
        };

        // Only the file half can leave the instance visibly half-changed, so
        // that is what the snapshot has to cover.
        let undo_snapshot_id = if template.files.is_empty() {
            None
        } else {
            Some(
                crate::snapshot::create_snapshot(&instance_dir, Some("pre-template"))
                    .map_err(template_error)?
                    .id,
            )
        };

        let files_applied = match crate::template_service::apply_template_files(
            templates_root,
            template_id,
            &instance_dir,
        ) {
            Ok(applied) => applied,
            Err(message) => {
                if let Some(snapshot_id) = undo_snapshot_id.as_deref() {
                    let _ = crate::snapshot::restore_snapshot(&instance_dir, snapshot_id);
                }
                return Err(template_error(message));
            }
        };

        let jvm_applied = match self.apply_template_jvm(&instance_id, &template.jvm) {
            Ok(applied) => applied,
            Err(error) => {
                if let Some(snapshot_id) = undo_snapshot_id.as_deref() {
                    let _ = crate::snapshot::restore_snapshot(&instance_dir, snapshot_id);
                }
                return Err(error);
            }
        };

        // Config changes are exactly the kind of drift the pre-launch snapshot
        // reuse check must notice.
        let _ = crate::snapshot::mark_instance_mutated(&instance_dir);

        Ok(TemplateApplyOutcome {
            jvm_applied,
            files_applied,
            // A file recorded in the template but absent on disk is skipped by
            // `apply_template_files`. Counting the difference is what lets the
            // UI say the template is damaged instead of quietly under-applying.
            files_missing: template.files.len().saturating_sub(files_applied),
            undo_snapshot_id,
        })
    }

    pub fn update_jvm(
        &self,
        instance_id: &str,
        memory_mb: i64,
        gc: &str,
        always_pre_touch: bool,
        custom_args: &str,
        memory_mode: &str,
    ) -> LauncherResult<()> {
        let instance_id = self.validate_id(instance_id)?;
        let memory_mb = memory_mb.clamp(2048, 32768);
        let gc_selection = crate::gc::GcSelection::from_input(gc).ok_or_else(|| {
            LauncherError::Generic {
                code: "ERR_INVALID_JVM_GC".into(),
                message: format!(
                    "Unsupported JVM GC selection '{}'. Expected auto, low_latency, high_efficiency, or manual.",
                    gc.trim()
                ),
            }
        })?;
        let gc = gc_selection.as_str();
        let conn = self.connection()?;
        let _row = crate::db::get_instance(&conn, &instance_id)
            .map_err(|error| LauncherError::Generic {
                code: "ERR_LOCAL_STATE_FAILED".into(),
                message: error.to_string(),
            })?
            .ok_or_else(|| LauncherError::Generic {
                code: "ERR_INSTANCE_NOT_FOUND".into(),
                message: format!("Instance '{instance_id}' not found"),
            })?;
        crate::db::update_instance_jvm(
            &conn,
            &instance_id,
            memory_mb,
            gc,
            always_pre_touch,
            custom_args,
            memory_mode,
        )
        .map_err(|error| LauncherError::Generic {
            code: "ERR_LOCAL_STATE_FAILED".into(),
            message: error.to_string(),
        })?;
        self.record_change(&instance_id, "update_jvm");
        Ok(())
    }

    pub fn memory_recommendation(
        &self,
        instance_id: &str,
    ) -> LauncherResult<crate::memory_recommendation::MemoryRecommendation> {
        let instance_id = self.validate_id(instance_id)?;
        let manifest_path = self.ctx.paths.instance_manifest(&instance_id)?;
        let manifest = read_manifest(&manifest_path)?.ok_or_else(|| LauncherError::Generic {
            code: "ERR_INSTANCE_MANIFEST_MISSING".into(),
            message: format!("Instance manifest not found for '{instance_id}'"),
        })?;
        let instance_dir = self.ctx.paths.instance_dir(&instance_id)?;
        let summary = crate::memory_recommendation::summarize_instance(&instance_dir, &manifest);
        Ok(crate::memory_recommendation::detect_and_recommend(&summary))
    }

    pub fn rename(&self, instance_id: &str, name: &str) -> LauncherResult<()> {
        let instance_id = self.validate_id(instance_id)?;
        if name.trim().is_empty() {
            return Err(LauncherError::Generic {
                code: "ERR_INVALID_INSTANCE_NAME".into(),
                message: "Instance name must not be empty.".into(),
            });
        }
        let conn = self.connection()?;
        crate::db::rename_instance(&conn, &instance_id, name.trim()).map_err(|error| {
            LauncherError::Generic {
                code: "ERR_LOCAL_STATE_FAILED".into(),
                message: error.to_string(),
            }
        })?;
        self.record_change(&instance_id, "rename");
        Ok(())
    }

    /// Record the trusted remote icon associated with a downloaded pack.
    pub fn set_pack_icon_url(
        &self,
        instance_id: &str,
        icon_url: Option<&str>,
    ) -> LauncherResult<()> {
        let instance_id = self.validate_id(instance_id)?;
        if let Some(icon_url) = icon_url {
            if !icon_url.starts_with("https://") {
                return Err(LauncherError::Generic {
                    code: "ERR_ICON_URL".into(),
                    message: "Pack icons must use HTTPS.".into(),
                });
            }
        }
        let manifest_path = self.ctx.paths.instance_manifest(&instance_id)?;
        let mut manifest = crate::helpers::read_manifest(&manifest_path)?;
        if !manifest.user_preferences.is_object() {
            manifest.user_preferences = serde_json::json!({});
        }
        let preferences = manifest
            .user_preferences
            .as_object_mut()
            .ok_or(LauncherError::InstanceCreateFailed)?;
        match icon_url {
            Some(icon_url) => {
                preferences.insert(
                    "agora_pack_icon_url".into(),
                    serde_json::Value::String(icon_url.to_owned()),
                );
            }
            None => {
                preferences.remove("agora_pack_icon_url");
            }
        }
        crate::helpers::atomic_write_manifest(&manifest_path, &manifest)
    }

    /// Reconcile the official launcher profile before a desktop adapter starts
    /// the native Mojang launcher. Launch history is recorded after handoff.
    pub fn prepare_delegated_launch(
        &self,
        instance_id: &str,
    ) -> LauncherResult<DelegatedLaunchPreparation> {
        let instance_id = self.validate_id(instance_id)?;
        let conn = self.connection()?;
        let row = crate::db::get_instance(&conn, &instance_id)
            .map_err(|error| LauncherError::Generic {
                code: "ERR_LOCAL_STATE_FAILED".into(),
                message: error.to_string(),
            })?
            .ok_or(LauncherError::LaunchFailed)?;
        let user_override = crate::db::get_setting(&conn, "jvm_always_pre_touch")
            .ok()
            .flatten()
            .and_then(|value| value.as_bool());
        let manifest = read_manifest(&self.ctx.paths.instance_manifest(&instance_id)?)?;
        let memory_mb = if row.jvm_memory_mode == "auto" {
            if let Some(manifest) = manifest.as_ref() {
                let instance_dir = self.ctx.paths.instance_dir(&instance_id)?;
                let summary =
                    crate::memory_recommendation::summarize_instance(&instance_dir, manifest);
                crate::memory_recommendation::detect_and_recommend(&summary).recommended_mb
            } else {
                row.jvm_memory_mb
            }
        } else {
            row.jvm_memory_mb
        };
        let jvm = crate::models::JvmConfig {
            memory_mb,
            gc: row.jvm_gc.clone(),
            custom_args: row.jvm_custom_args.clone(),
            always_pre_touch: row.jvm_always_pre_touch && user_override.unwrap_or(true),
        };
        let profile_id = crate::launcher_profiles::profile_id_for(&row.instance_id);
        let profile = crate::launcher_profiles::LauncherProfileEntry {
            profile_id: profile_id.clone(),
            name: format!("{} (Agora)", row.name),
            last_version_id: loader_version_id(&row),
            game_dir: self.ctx.paths.instance_dir(&instance_id)?,
            java_args: jvm.to_args_for_java(crate::models::recommended_java_version_for_minecraft(
                &row.minecraft_version,
            )),
            select: true,
        };
        let launcher_path = crate::db::get_setting(&conn, "mojang_launcher_path")
            .ok()
            .flatten()
            .and_then(|value| value.as_str().map(str::to_owned));
        let launcher_running =
            crate::official_launcher::is_running(launcher_path.as_ref().map(Path::new));
        let mut selection_applied = false;
        if let Some(profiles_path) = &self.ctx.launcher_profiles_path {
            let official_root = profiles_path
                .parent()
                .ok_or(LauncherError::MojangNotFound)?;
            let minecraft_root = self.ctx.paths.minecraft_runtime_root();
            let profile_version_id = loader_version_id(&row);
            crate::launcher_profiles::materialize_version_json(
                &minecraft_root,
                official_root,
                &row.minecraft_version,
                false,
            )?;
            if profile_version_id != row.minecraft_version {
                crate::launcher_profiles::materialize_version_json(
                    &minecraft_root,
                    official_root,
                    &profile_version_id,
                    true,
                )?;
            }
            // Sweep first: the launcher lists every profile in this file, so a
            // handoff is the right moment to drop the ones whose instance is
            // gone. Failure here must never block a launch.
            let _ = self.prune_launcher_profiles(&conn, profiles_path);
            crate::launcher_profiles::upsert_profile(&profile, profiles_path)?;
            if !launcher_running {
                // Writing while the launcher is open is pointless — it reloads
                // nothing and rewrites this file from memory on its next
                // startup, undoing the selection.
                selection_applied = !crate::launcher_ui_state::select_installation(
                    official_root,
                    &profile,
                    &chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
                )
                .unwrap_or_default()
                .is_empty();
            }
        }
        let mod_ids = manifest
            .map(|manifest| {
                manifest
                    .mods
                    .into_iter()
                    .map(|mod_entry| mod_entry.registry_id.unwrap_or(mod_entry.filename))
                    .collect()
            })
            .unwrap_or_default();
        Ok(DelegatedLaunchPreparation {
            profile_id,
            launcher_path,
            mod_ids,
            launcher_running,
            selection_applied,
        })
    }

    /// Drop `agora-*` launcher profiles whose instance no longer exists.
    ///
    /// Returns the removed profile ids. Called on delete and before every
    /// delegated handoff; safe to call when no launcher is installed.
    pub fn reconcile_launcher_profiles(&self) -> LauncherResult<Vec<String>> {
        let Some(profiles_path) = &self.ctx.launcher_profiles_path else {
            return Ok(Vec::new());
        };
        let conn = self.connection()?;
        self.prune_launcher_profiles(&conn, profiles_path)
    }

    fn prune_launcher_profiles(
        &self,
        conn: &rusqlite::Connection,
        profiles_path: &Path,
    ) -> LauncherResult<Vec<String>> {
        let live: std::collections::HashSet<String> = crate::db::list_instances(conn)
            .map_err(|error| LauncherError::Generic {
                code: "ERR_LOCAL_STATE_FAILED".into(),
                message: error.to_string(),
            })?
            .into_iter()
            .map(|row| row.instance_id)
            .collect();
        crate::launcher_profiles::prune_orphan_profiles(
            profiles_path,
            &self.ctx.paths.instances_root(),
            &|instance_id| live.contains(instance_id),
        )
    }

    /// Delete an instance by moving it into Agora's "Recently deleted" area
    /// (`recently_deleted.rs`), from where it can be restored for
    /// [`crate::recently_deleted::RETENTION_DAYS`] days.
    ///
    /// Fails closed: any error leaves the instance exactly where it was.
    /// A pack-locked instance is refused (unlock it first), and so is one
    /// whose lock is held by another operation such as a launch.
    pub fn delete(&self, instance_id: &str) -> LauncherResult<()> {
        let instance_id = self.validate_id(instance_id)?;
        let op = self
            .ctx
            .operation_manager
            .register_for_instance("Delete instance", &instance_id);
        let _lock = match self.ctx.lock_manager.acquire(
            crate::lock_manager::LockResource::Instance(instance_id.clone()),
            "delete",
        ) {
            Ok(l) => l,
            Err(e) => {
                op.fail(e.to_string());
                return Err(e);
            }
        };
        let conn = match self.connection() {
            Ok(c) => c,
            Err(e) => {
                op.fail(e.to_string());
                return Err(e);
            }
        };
        let row = match crate::db::get_instance(&conn, &instance_id) {
            Ok(Some(r)) => r,
            Ok(None) => {
                op.fail(format!("Instance '{instance_id}' not found."));
                return Err(LauncherError::Generic {
                    code: "ERR_INSTANCE_NOT_FOUND".into(),
                    message: format!("Instance '{instance_id}' not found."),
                });
            }
            Err(error) => {
                op.fail(error.to_string());
                return Err(LauncherError::Generic {
                    code: "ERR_LOCAL_STATE_FAILED".into(),
                    message: error.to_string(),
                });
            }
        };
        if row.is_locked {
            op.fail(format!("Instance '{instance_id}' is locked."));
            return Err(LauncherError::Generic {
                code: "ERR_INSTANCE_LOCKED".into(),
                message: format!("Instance '{instance_id}' is locked."),
            });
        }
        let dir = self.ctx.paths.instance_dir(&instance_id)?;
        let result = if std::fs::symlink_metadata(&dir).is_ok() {
            crate::recently_deleted::stash_instance(
                &self.ctx.paths.recently_deleted_root(),
                &conn,
                &row,
                &dir,
                chrono::DateTime::<chrono::Utc>::from(self.ctx.clock.now()),
            )
            .map(|_| ())
        } else {
            // A row whose folder is already gone: nothing to recover, just
            // drop the row (as before).
            crate::db::delete_instance(&conn, &instance_id).map_err(|error| {
                LauncherError::Generic {
                    code: "ERR_LOCAL_STATE_FAILED".into(),
                    message: error.to_string(),
                }
            })
        };
        if let Err(error) = result {
            op.fail(error.to_string());
            return Err(error);
        }
        if let Some(profiles_path) = &self.ctx.launcher_profiles_path {
            let profile_id = crate::launcher_profiles::profile_id_for(&instance_id);
            let _ = crate::launcher_profiles::remove_profile(&profile_id, profiles_path);
            // The launcher restores its installation selection from its own
            // UI state, so removing the profile is not enough: without this it
            // reopens on a pack that no longer exists.
            if let Some(official_root) = profiles_path.parent() {
                if crate::launcher_ui_state::selected_profile_id(official_root).as_deref()
                    == Some(profile_id.as_str())
                {
                    let replacement = crate::launcher_profiles::most_recent_profile_except(
                        profiles_path,
                        &profile_id,
                    );
                    let _ = crate::launcher_ui_state::clear_selection_for(
                        official_root,
                        &profile_id,
                        replacement
                            .as_ref()
                            .map(|(entry, last_used)| (entry, last_used.as_str())),
                    );
                }
            }
            let _ = self.prune_launcher_profiles(&conn, profiles_path);
        }
        op.complete();
        Ok(())
    }

    /// Entries in "Recently deleted", newest first.
    pub fn list_recently_deleted(
        &self,
    ) -> LauncherResult<Vec<crate::recently_deleted::RecentlyDeletedEntry>> {
        Ok(crate::recently_deleted::list_entries(
            &self.ctx.paths.recently_deleted_root(),
        ))
    }

    /// Restore a deleted instance: files and database rows (import provenance,
    /// pack inventory, launch history). If its id has been taken since, it
    /// comes back under a free id and the outcome says so.
    pub fn restore_deleted(
        &self,
        entry_id: &str,
    ) -> LauncherResult<crate::recently_deleted::RestoreOutcome> {
        let root = self.ctx.paths.recently_deleted_root();
        let entry = crate::recently_deleted::list_entries(&root)
            .into_iter()
            .find(|e| e.entry_id == entry_id)
            .ok_or_else(|| LauncherError::Generic {
                code: "ERR_INSTANCE_RESTORE".into(),
                message: "That deleted instance is no longer available.".into(),
            })?;
        let original = self.validate_id(&entry.instance_id)?;
        let op = self
            .ctx
            .operation_manager
            .register_for_instance("Restore instance", &original);
        let _lock = match self.ctx.lock_manager.acquire(
            crate::lock_manager::LockResource::Instance(original.clone()),
            "restore",
        ) {
            Ok(l) => l,
            Err(e) => {
                op.fail(e.to_string());
                return Err(e);
            }
        };
        let (mut conn, probe) = match (self.connection(), self.connection()) {
            (Ok(a), Ok(b)) => (a, b),
            (Err(e), _) | (_, Err(e)) => {
                op.fail(e.to_string());
                return Err(e);
            }
        };
        let paths = &self.ctx.paths;
        let free = |candidate: &str| {
            paths
                .instance_dir(candidate)
                .map(|dir| std::fs::symlink_metadata(dir).is_err())
                .unwrap_or(false)
                && matches!(crate::db::get_instance(&probe, candidate), Ok(None))
        };
        match crate::recently_deleted::restore_entry(
            &root,
            entry_id,
            &mut conn,
            &paths.instances_root(),
            &free,
        ) {
            Ok(outcome) => {
                op.complete();
                Ok(outcome)
            }
            Err(error) => {
                op.fail(error.to_string());
                Err(error)
            }
        }
    }

    /// Send one deleted instance to the OS trash now ("Delete now").
    pub fn purge_deleted(&self, entry_id: &str, trash: &TrashFn) -> LauncherResult<()> {
        crate::recently_deleted::purge_entry(
            &self.ctx.paths.recently_deleted_root(),
            entry_id,
            trash,
        )
    }

    /// Cleanup pass: entries past their window or over the size budget go to
    /// the OS trash; failures leave the entry in place.
    pub fn cleanup_recently_deleted(
        &self,
        trash: &TrashFn,
    ) -> crate::recently_deleted::CleanupSummary {
        // First repair anything a crash left half-done.
        if let Ok(mut conn) = self.connection() {
            crate::recently_deleted::reconcile(
                &self.ctx.paths.recently_deleted_root(),
                &mut conn,
                &self.ctx.paths.instances_root(),
            );
        }
        crate::recently_deleted::cleanup(
            &self.ctx.paths.recently_deleted_root(),
            chrono::DateTime::<chrono::Utc>::from(self.ctx.clock.now()),
            trash,
        )
    }

    /// Clone an existing instance: copy directory contents per prefs, create a
    /// new DB row and launcher profile. Uses safe staging — the target directory
    /// is built in a staging area and atomically renamed on success. The source
    /// is locked for the duration of the operation.
    pub async fn clone(&self, request: CloneRequest) -> LauncherResult<InstanceRow> {
        let source_id = self.validate_id(&request.source_instance_id)?;
        if request.new_name.trim().is_empty() {
            return Err(LauncherError::Generic {
                code: "ERR_INVALID_INSTANCE_NAME".into(),
                message: "Clone name must not be empty.".into(),
            });
        }
        let new_id = crate::paths::sanitize_id(request.new_name.trim());
        crate::app_paths::validate_path_component(&new_id).map_err(|_| LauncherError::Generic {
            code: "ERR_INVALID_INSTANCE_NAME".into(),
            message: format!("'{new_id}' is not a valid instance identifier."),
        })?;
        if source_id == new_id {
            return Err(LauncherError::Generic {
                code: "ERR_CLONE_SAME_ID".into(),
                message: "Source and clone IDs are identical.".into(),
            });
        }

        let op = self
            .ctx
            .operation_manager
            .register_for_instance("Clone instance", &new_id);

        // Validate source
        let conn = match self.connection() {
            Ok(c) => c,
            Err(e) => {
                op.fail(e.to_string());
                return Err(e);
            }
        };
        let source_row = match crate::db::get_instance(&conn, &source_id) {
            Ok(Some(row)) => row,
            Ok(None) => {
                op.fail(format!("Source instance '{source_id}' not found."));
                return Err(LauncherError::Generic {
                    code: "ERR_INSTANCE_NOT_FOUND".into(),
                    message: format!("Source instance '{source_id}' not found."),
                });
            }
            Err(error) => {
                op.fail(error.to_string());
                return Err(LauncherError::Generic {
                    code: "ERR_LOCAL_STATE_FAILED".into(),
                    message: error.to_string(),
                });
            }
        };
        drop(conn);

        // Check destination does not exist before acquiring the ordered locks.
        let dest_dir = self.ctx.paths.instance_dir(&new_id)?;
        if dest_dir.exists() {
            op.fail(format!("An instance named '{new_id}' already exists."));
            return Err(LauncherError::Generic {
                code: "ERR_INSTANCE_EXISTS".into(),
                message: format!("An instance named '{new_id}' already exists."),
            });
        }

        // Acquire source and destination locks in a stable order so reverse
        // clone requests cannot deadlock each other.
        let (first_id, second_id) = if source_id < new_id {
            (source_id.as_str(), new_id.as_str())
        } else {
            (new_id.as_str(), source_id.as_str())
        };
        let _first_lock = match self.ctx.lock_manager.acquire(
            crate::lock_manager::LockResource::Instance(first_id.to_owned()),
            "clone",
        ) {
            Ok(l) => l,
            Err(e) => {
                op.fail(e.to_string());
                return Err(e);
            }
        };
        let _second_lock = match self.ctx.lock_manager.acquire(
            crate::lock_manager::LockResource::Instance(second_id.to_owned()),
            "clone",
        ) {
            Ok(l) => l,
            Err(e) => {
                op.fail(e.to_string());
                return Err(e);
            }
        };

        let conn = match self.connection() {
            Ok(c) => c,
            Err(e) => {
                op.fail(e.to_string());
                return Err(e);
            }
        };
        if crate::db::get_instance(&conn, &new_id)
            .map_err(|error| LauncherError::Generic {
                code: "ERR_LOCAL_STATE_FAILED".into(),
                message: error.to_string(),
            })?
            .is_some()
        {
            op.fail(format!("An instance named '{new_id}' already exists."));
            return Err(LauncherError::Generic {
                code: "ERR_INSTANCE_EXISTS".into(),
                message: format!("An instance named '{new_id}' already exists."),
            });
        }
        drop(conn);

        // Read source manifest via canonical loader — heals pack_managed and
        // synthesizes a name-only PackOrigin for legacy instances.
        let manifest_path = self.ctx.paths.instance_manifest(&source_id)?;
        let source_manifest: InstanceManifest = if manifest_path.exists() {
            crate::helpers::read_manifest(&manifest_path).map_err(|e| LauncherError::Generic {
                code: "ERR_CLONE".into(),
                message: format!("Cannot read source manifest: {e}"),
            })?
        } else {
            // Synthesise a minimal manifest from the DB row.
            InstanceManifest {
                manifest_version: crate::models::CURRENT_MANIFEST_VERSION,
                pack_origin: None,
                instance_id: new_id.clone(),
                name: request.new_name.trim().to_owned(),
                created_from_pack: None,
                minecraft_version: source_row.minecraft_version.clone(),
                loader: source_row.loader.clone(),
                loader_version: source_row.loader_version.clone(),
                is_locked: source_row.is_locked,
                mods: Vec::new(),
                resourcepacks: Vec::new(),
                shaders: Vec::new(),
                datapacks: Vec::new(),
                worlds: Vec::new(),
                user_preferences: serde_json::json!({}),
            }
        };

        // Build new row and manifest
        let now = chrono::Utc::now().to_rfc3339();
        let new_row = InstanceRow {
            instance_id: new_id.clone(),
            name: request.new_name.trim().to_owned(),
            minecraft_version: source_row.minecraft_version.clone(),
            loader: source_row.loader.clone(),
            loader_version: source_row.loader_version.clone(),
            is_modpack: source_row.is_modpack,
            is_locked: false,
            last_launched_at: None,
            jvm_memory_mb: source_row.jvm_memory_mb,
            jvm_memory_mode: source_row.jvm_memory_mode.clone(),
            jvm_gc: source_row.jvm_gc.clone(),
            jvm_custom_args: source_row.jvm_custom_args.clone(),
            jvm_always_pre_touch: source_row.jvm_always_pre_touch,
            created_at: now,
            java_path: source_row.java_path.clone(),
            java_incompatible_override: source_row.java_incompatible_override,
            icon_path: None,
            launch_mode_override: "auto".into(),
            import_source: None,
        };
        let mut new_manifest = InstanceManifest {
            manifest_version: crate::models::CURRENT_MANIFEST_VERSION,
            instance_id: new_id.clone(),
            name: request.new_name.trim().to_owned(),
            // A clone is always a fresh, unlocked working copy — never inherit
            // the source's locked flag (the DB row and manifest must agree).
            is_locked: false,
            ..source_manifest.clone()
        };
        // Clone keeps pack identity so it can still be updated, but records
        // where it came from and refreshes installed_at. pack_content_hash
        // is recomputed after the directory is cloned, so clear it here.
        if let Some(mut origin) = new_manifest.pack_origin.take() {
            origin.cloned_from = Some(source_id.clone());
            origin.installed_at = chrono::Utc::now().to_rfc3339();
            origin.pack_content_hash = None;
            new_manifest.pack_origin = Some(origin);
        }

        let operation_id = op.id().clone();
        self.ctx.progress_sink.report(ProgressEvent::new(
            operation_id.clone(),
            ProgressPhase::Staging,
            "Cloning instance files",
        ));

        // Build in staging then atomically rename
        let staging = self
            .ctx
            .paths
            .staging_dir(&format!("clone-{}", uuid::Uuid::new_v4()))?;
        let src_dir = self.ctx.paths.instance_dir(&source_id)?;

        if let Err(error) = crate::clone::clone_instance(&src_dir, &staging, &request.prefs) {
            let _ = std::fs::remove_dir_all(&staging);
            op.fail(error.clone());
            return Err(LauncherError::Generic {
                code: "ERR_CLONE".into(),
                message: error,
            });
        }

        // Recompute pack content hash from the cloned directory (respecting
        // ClonePrefs; symlinks/hardlinks are hashed via file contents, so
        // recomputing is more accurate than copying DB rows).
        if let Some(origin) = new_manifest.pack_origin.as_mut() {
            let pack_files =
                crate::pack_inventory::collect_pack_inventory(&staging).unwrap_or_default();
            let pack_hash = if pack_files.is_empty() {
                None
            } else {
                Some(crate::pack_inventory::pack_content_hash(&pack_files))
            };
            origin.pack_content_hash = pack_hash;
        }

        // Write updated manifest
        let manifest_bytes = serde_json::to_vec_pretty(&new_manifest)
            .map_err(|_| LauncherError::InstanceCreateFailed)?;
        if let Err(error) = std::fs::write(staging.join("instance_manifest.json"), manifest_bytes) {
            let _ = std::fs::remove_dir_all(&staging);
            op.fail(format!("Cannot write clone manifest: {error}"));
            return Err(LauncherError::Generic {
                code: "ERR_CLONE".into(),
                message: format!("Cannot write clone manifest: {error}"),
            });
        }

        // Mark staging complete
        if let Err(error) = std::fs::write(staging.join("staging-complete"), b"complete") {
            let _ = std::fs::remove_dir_all(&staging);
            op.fail(error.to_string());
            return Err(LauncherError::Generic {
                code: "ERR_CLONE".into(),
                message: error.to_string(),
            });
        }

        // Promote staging to destination
        if let Err(error) = std::fs::remove_file(staging.join("staging-complete")) {
            let _ = std::fs::remove_dir_all(&staging);
            op.fail(format!("Cannot finalize clone staging: {error}"));
            return Err(LauncherError::Generic {
                code: "ERR_CLONE".into(),
                message: format!("Cannot finalize clone staging: {error}"),
            });
        }
        if let Err(error) = std::fs::rename(&staging, &dest_dir) {
            let _ = std::fs::remove_dir_all(&staging);
            op.fail(format!("Cannot promote clone: {error}"));
            return Err(LauncherError::Generic {
                code: "ERR_CLONE".into(),
                message: format!("Cannot promote clone: {error}"),
            });
        }

        // Persist DB row
        let conn = match self.connection() {
            Ok(conn) => conn,
            Err(error) => {
                let _ = std::fs::remove_dir_all(&dest_dir);
                op.fail(error.to_string());
                return Err(error);
            }
        };
        if let Err(error) = crate::db::upsert_instance(&conn, &new_row) {
            let _ = std::fs::remove_dir_all(&dest_dir);
            op.fail(error.to_string());
            return Err(LauncherError::Generic {
                code: "ERR_LOCAL_STATE_FAILED".into(),
                message: error.to_string(),
            });
        }

        // A clone of an import is still "from CurseForge" (or Prism, ...):
        // carry the provenance so the badge survives. The clone gets its own
        // record under a distinct source key, so re-importing the original
        // never matches (and never updates) the clone, and it has no tracked
        // source files of its own.
        if let Ok(Some(source_import)) = crate::db::get_instance_import(&conn, &source_id) {
            let record = crate::db::InstanceImportRecord {
                instance_id: new_id.clone(),
                installation_key: "clone".into(),
                source_instance_key: format!("clone-of:{source_id}"),
                source_path: String::new(),
                source_metadata_json: serde_json::json!({ "cloned_from": source_id }).to_string(),
                imported_settings_json: "{}".into(),
                source_fingerprint: String::new(),
                imported_at: chrono::Utc::now().to_rfc3339(),
                updated_at: chrono::Utc::now().to_rfc3339(),
                last_result: "cloned".into(),
                launcher_kind: source_import.launcher_kind,
            };
            // Fail the clone rather than hand back one that quietly lost its
            // provenance: roll the new instance back completely.
            if let Err(error) = crate::db::replace_instance_import(&conn, &record, &[]) {
                let _ = crate::db::delete_instance(&conn, &new_row.instance_id);
                let _ = std::fs::remove_dir_all(&dest_dir);
                let message = format!("Cannot record where the clone came from: {error}");
                op.fail(message.clone());
                return Err(LauncherError::Generic {
                    code: "ERR_CLONE".into(),
                    message,
                });
            }
        }

        // Persist pack file inventory for the clone — recomputed from the
        // cloned directory so ClonePrefs filtering and symlink/hardlink
        // handling are reflected, not just copied DB rows.
        if new_manifest.pack_origin.is_some() {
            if let Ok(pack_files) = crate::pack_inventory::collect_pack_inventory(&dest_dir) {
                let _ = crate::db::replace_instance_pack_files(&conn, &new_id, &pack_files);
            }
        }

        // Launcher profile
        if let Some(profiles_path) = &self.ctx.launcher_profiles_path {
            let user_override = crate::db::get_setting(&conn, "jvm_always_pre_touch")
                .ok()
                .flatten()
                .and_then(|value| value.as_bool());
            let jvm = crate::models::JvmConfig {
                memory_mb: new_row.jvm_memory_mb,
                gc: new_row.jvm_gc.clone(),
                custom_args: new_row.jvm_custom_args.clone(),
                always_pre_touch: new_row.jvm_always_pre_touch && user_override.unwrap_or(true),
            };
            let profile = crate::launcher_profiles::LauncherProfileEntry {
                profile_id: format!("agora-{}", new_row.instance_id),
                name: format!("{} (Agora)", new_row.name),
                last_version_id: loader_version_id(&new_row),
                game_dir: dest_dir,
                java_args: jvm.to_args_for_java(
                    crate::models::recommended_java_version_for_minecraft(
                        &new_row.minecraft_version,
                    ),
                ),
                select: false,
            };
            if let Err(error) = crate::launcher_profiles::upsert_profile(&profile, profiles_path) {
                let _ = crate::db::delete_instance(&conn, &new_row.instance_id);
                let _ = std::fs::remove_dir_all(&self.ctx.paths.instance_dir(&new_id)?);
                op.fail(error.to_string());
                return Err(error);
            }
        }

        op.complete();
        self.ctx.event_sink.emit(CoreEvent::Instance {
            operation_id,
            instance_id: new_id,
            status: EventStatus::Completed,
            message: "Instance cloned".into(),
        });
        Ok(new_row)
    }

    /// Create the instance directory, bootstrap shared metadata, install its
    /// loader, and persist the row. All partial instance state is removed on
    /// failure; shared loader artifacts are retained.
    /// Resolve which template a create request should use.
    ///
    /// An explicit id must exist — a typo should surface, not silently create a
    /// bare instance. The stored default is best-effort: if the user deleted the
    /// template their setting still points at, creating an instance must still
    /// work.
    fn resolve_template(
        &self,
        requested: Option<&str>,
    ) -> LauncherResult<Option<crate::template_service::InstanceTemplate>> {
        let root = self.ctx.paths.templates_root();
        if let Some(id) = requested.map(str::trim).filter(|id| !id.is_empty()) {
            return crate::template_service::get_template(&root, id)
                .map(Some)
                .map_err(|message| LauncherError::Generic {
                    code: "ERR_TEMPLATE_NOT_FOUND".into(),
                    message,
                });
        }
        let conn = self.connection()?;
        let default_id = crate::db::get_setting(&conn, DEFAULT_TEMPLATE_SETTING_KEY)
            .ok()
            .flatten()
            .and_then(|value| value.as_str().map(str::to_string));
        drop(conn);
        let Some(default_id) = default_id.filter(|id| !id.trim().is_empty()) else {
            return Ok(None);
        };
        Ok(crate::template_service::get_template(&root, &default_id).ok())
    }

    pub async fn create(&self, request: CreateInstanceRequest) -> LauncherResult<InstanceRow> {
        let instance_id = self.validate_id(&request.instance_id)?;
        if request.name.trim().is_empty() || request.minecraft_version.trim().is_empty() {
            return Err(LauncherError::InstanceCreateFailed);
        }
        let op = self
            .ctx
            .operation_manager
            .register_for_instance("Create instance", &instance_id);
        let _lock = self.ctx.lock_manager.acquire(
            crate::lock_manager::LockResource::Instance(instance_id.clone()),
            "create",
        )?;
        let dir = self.ctx.paths.instance_dir(&instance_id)?;
        if dir.exists() {
            op.fail(format!("An instance named '{instance_id}' already exists."));
            return Err(LauncherError::Generic {
                code: "ERR_INSTANCE_EXISTS".into(),
                message: format!("An instance named '{instance_id}' already exists."),
            });
        }
        let staging_dir = self
            .ctx
            .paths
            .staging_dir(&format!("instance-{}", uuid::Uuid::new_v4()))?;
        let operation_id = op.id().clone();
        self.ctx.progress_sink.report(ProgressEvent::new(
            operation_id.clone(),
            ProgressPhase::Staging,
            "Preparing instance directory",
        ));
        // Resolve the template before anything is written: a bad template id
        // should fail the create, not leave a half-configured instance.
        let template = match self.resolve_template(request.template_id.as_deref()) {
            Ok(template) => template,
            Err(error) => {
                let _ = std::fs::remove_dir_all(&staging_dir);
                op.fail(error.to_string());
                return Err(error);
            }
        };
        let mut request = request;
        if let Some(template) = template.as_ref() {
            apply_template_jvm_to_request(&mut request, &template.jvm);
        }
        let request = request;

        let mut row = prepare_row(&instance_id, &request);
        if let Some(template) = template.as_ref() {
            if row.java_path.is_none() {
                row.java_path = template.jvm.java_path.clone();
            }
        }
        let row = row;
        let manifest = manifest_from_request(&instance_id, &request);
        if let Err(error) = self.prepare_files(&staging_dir, &manifest) {
            let _ = std::fs::remove_dir_all(&staging_dir);
            op.fail(error.to_string());
            return Err(error);
        }
        if let Some(template) = template.as_ref() {
            if let Err(error) = crate::template_service::apply_template_files(
                &self.ctx.paths.templates_root(),
                &template.id,
                &staging_dir,
            ) {
                let _ = std::fs::remove_dir_all(&staging_dir);
                op.fail(error.clone());
                return Err(LauncherError::Generic {
                    code: "ERR_TEMPLATE_APPLY".into(),
                    message: error,
                });
            }
        }
        if let Err(error) = std::fs::write(staging_dir.join("staging-complete"), b"complete") {
            let _ = std::fs::remove_dir_all(&staging_dir);
            op.fail(error.to_string());
            return Err(LauncherError::Generic {
                code: "ERR_INSTANCE_STAGING".into(),
                message: error.to_string(),
            });
        }

        let minecraft_root = self.ctx.paths.minecraft_runtime_root();
        if let Err(error) = crate::minecraft_runtime::ensure_runtime_layout(&minecraft_root) {
            let _ = std::fs::remove_dir_all(&staging_dir);
            op.fail(error.to_string());
            return Err(error);
        }
        let conn = match self.connection() {
            Ok(conn) => conn,
            Err(error) => {
                let _ = std::fs::remove_dir_all(&staging_dir);
                op.fail(error.to_string());
                return Err(error);
            }
        };
        let policy = crate::network::NetworkPolicy::from_db(&conn);
        drop(conn);
        self.ctx.progress_sink.report(ProgressEvent::new(
            operation_id.clone(),
            ProgressPhase::Downloading,
            format!(
                "Downloading Minecraft {} metadata",
                request.minecraft_version
            ),
        ));
        if let Err(error) = crate::minecraft_metadata::ensure_base_version_metadata(
            &minecraft_root,
            &request.minecraft_version,
            &policy,
        )
        .await
        .map_err(|e| {
            e.while_attempting(format!(
                "download Minecraft {} metadata",
                request.minecraft_version
            ))
        }) {
            let _ = std::fs::remove_dir_all(&staging_dir);
            op.fail(error.to_string());
            return Err(error);
        }

        if !matches!(request.loader.as_str(), "" | "vanilla") {
            let loader_label = format!(
                "{} {}",
                loader_display_name(&request.loader),
                request.loader_version
            );
            if let Err(error) = LoaderService::new(self.ctx.clone())
                .ensure_installed(
                    &request.loader,
                    &request.minecraft_version,
                    &request.loader_version,
                    false,
                )
                .await
                .map_err(|e| e.while_attempting(format!("download {loader_label}")))
            {
                let _ = std::fs::remove_dir_all(&staging_dir);
                op.fail(error.to_string());
                return Err(error);
            }
        }

        if let Err(error) = std::fs::remove_file(staging_dir.join("staging-complete")) {
            let _ = std::fs::remove_dir_all(&staging_dir);
            op.fail(error.to_string());
            return Err(LauncherError::Generic {
                code: "ERR_INSTANCE_STAGING".into(),
                message: error.to_string(),
            });
        }
        if let Err(error) = std::fs::rename(&staging_dir, &dir) {
            let _ = std::fs::remove_dir_all(&staging_dir);
            op.fail(error.to_string());
            return Err(LauncherError::Generic {
                code: "ERR_INSTANCE_PROMOTE".into(),
                message: error.to_string(),
            });
        }

        let conn = match self.connection() {
            Ok(conn) => conn,
            Err(error) => {
                let _ = std::fs::remove_dir_all(&dir);
                op.fail(error.to_string());
                return Err(error);
            }
        };
        if let Err(error) = crate::db::upsert_instance(&conn, &row) {
            let _ = std::fs::remove_dir_all(&dir);
            op.fail(error.to_string());
            return Err(LauncherError::Generic {
                code: "ERR_LOCAL_STATE_FAILED".into(),
                message: error.to_string(),
            });
        }
        if let Some(profiles_path) = &self.ctx.launcher_profiles_path {
            let user_override = crate::db::get_setting(&conn, "jvm_always_pre_touch")
                .ok()
                .flatten()
                .and_then(|value| value.as_bool());
            let jvm = crate::models::JvmConfig {
                memory_mb: row.jvm_memory_mb,
                gc: row.jvm_gc.clone(),
                custom_args: row.jvm_custom_args.clone(),
                always_pre_touch: row.jvm_always_pre_touch && user_override.unwrap_or(true),
            };
            let profile = crate::launcher_profiles::LauncherProfileEntry {
                profile_id: format!("agora-{}", row.instance_id),
                name: format!("{} (Agora)", row.name),
                last_version_id: loader_version_id(&row),
                game_dir: dir.clone(),
                java_args: jvm.to_args(),
                select: false,
            };
            if let Err(error) = crate::launcher_profiles::upsert_profile(&profile, profiles_path) {
                let _ = crate::db::delete_instance(&conn, &row.instance_id);
                let _ = std::fs::remove_dir_all(&dir);
                op.fail(error.to_string());
                return Err(error);
            }
        }
        op.complete();
        self.ctx
            .event_sink
            .emit(crate::event_sink::CoreEvent::Instance {
                operation_id,
                instance_id,
                status: EventStatus::Completed,
                message: "Instance created".into(),
            });
        Ok(row)
    }

    fn connection(&self) -> LauncherResult<rusqlite::Connection> {
        crate::db::local_state_connection(&self.ctx.paths.local_state_db()).map_err(|error| {
            LauncherError::Generic {
                code: "ERR_LOCAL_STATE_FAILED".into(),
                message: error.to_string(),
            }
        })
    }

    fn validate_id(&self, instance_id: &str) -> LauncherResult<String> {
        let sanitized = crate::paths::sanitize_id(instance_id);
        crate::app_paths::validate_path_component(&sanitized)?;
        Ok(sanitized)
    }

    fn set_locked(&self, instance_id: &str, locked: bool) -> LauncherResult<()> {
        let instance_id = self.validate_id(instance_id)?;
        let conn = self.connection()?;
        let manifest_path = self.ctx.paths.instance_manifest(&instance_id)?;
        let mut manifest = read_manifest(&manifest_path)?;
        if let Some(manifest) = manifest.as_mut() {
            manifest.is_locked = locked;
        }

        crate::db::set_locked(&conn, &instance_id, locked).map_err(|error| {
            LauncherError::Generic {
                code: "ERR_LOCAL_STATE_FAILED".into(),
                message: error.to_string(),
            }
        })?;

        if let Some(manifest) = manifest {
            if let Err(error) = crate::helpers::atomic_write_manifest(&manifest_path, &manifest) {
                // Keep the database and manifest aligned if the filesystem write fails.
                let _ = crate::db::set_locked(&conn, &instance_id, !locked);
                return Err(error);
            }
        }
        self.record_change(&instance_id, if locked { "lock" } else { "unlock" });
        Ok(())
    }

    /// For edits that take no instance lock, which records changes itself.
    fn record_change(&self, instance_id: &str, operation: &str) {
        crate::change_feed::record_instance_change(
            self.ctx.lock_manager.locks_root(),
            instance_id,
            operation,
        );
    }

    fn prepare_files(&self, dir: &Path, manifest: &InstanceManifest) -> LauncherResult<()> {
        for child in [
            "mods",
            "resourcepacks",
            "shaderpacks",
            "datapacks",
            "config",
            "crash-reports",
            "logs",
            "saves",
            "screenshots",
        ] {
            std::fs::create_dir_all(dir.join(child))
                .map_err(|_| LauncherError::InstanceCreateFailed)?;
        }
        let path = dir.join("instance_manifest.json");
        let bytes =
            serde_json::to_vec_pretty(manifest).map_err(|_| LauncherError::InstanceCreateFailed)?;
        std::fs::write(path, bytes).map_err(|_| LauncherError::InstanceCreateFailed)
    }
}

/// Whether the manifest lists a content file with this name. An empty name
/// never matches, so an unnamed pack entry stays a known gap.
fn manifest_has_file(manifest: &InstanceManifest, filename: &str) -> bool {
    !filename.is_empty()
        && manifest
            .mods
            .iter()
            .chain(&manifest.resourcepacks)
            .chain(&manifest.shaders)
            .chain(&manifest.datapacks)
            .chain(&manifest.worlds)
            .any(|entry| entry.filename == filename)
}

/// Fill unset JVM fields on a create request from a template.
///
/// One-directional on purpose: the template is a default, never an override.
fn apply_template_jvm_to_request(
    request: &mut CreateInstanceRequest,
    jvm: &crate::template_service::TemplateJvm,
) {
    if request.jvm_memory_mb.is_none() {
        request.jvm_memory_mb = jvm.jvm_memory_mb;
    }
    if request.jvm_memory_mode.is_none() {
        request.jvm_memory_mode = jvm.jvm_memory_mode.clone();
    }
    if request.jvm_gc.is_none() {
        request.jvm_gc = jvm.jvm_gc.clone();
    }
    if request.jvm_custom_args.is_none() {
        request.jvm_custom_args = jvm.jvm_custom_args.clone();
    }
    if request.jvm_always_pre_touch.is_none() {
        request.jvm_always_pre_touch = jvm.jvm_always_pre_touch;
    }
}

pub(crate) fn prepare_row(instance_id: &str, request: &CreateInstanceRequest) -> InstanceRow {
    InstanceRow {
        instance_id: instance_id.into(),
        name: request.name.clone(),
        minecraft_version: request.minecraft_version.clone(),
        loader: request.loader.clone(),
        loader_version: request.loader_version.clone(),
        is_modpack: request.is_modpack.unwrap_or(false),
        is_locked: false,
        last_launched_at: None,
        jvm_memory_mb: request.jvm_memory_mb.unwrap_or(4096),
        jvm_memory_mode: request
            .jvm_memory_mode
            .as_deref()
            .filter(|mode| *mode == "manual")
            .unwrap_or("auto")
            .into(),
        // Creation takes its GC from a template or a clone source, so the
        // value has not been through `update_jvm`'s validation. Normalize it
        // here rather than persisting a third vocabulary; an unrecognised
        // value decodes to Auto, which is what it would have done anyway.
        jvm_gc: request
            .jvm_gc
            .as_deref()
            .map(|gc| {
                crate::gc::GcSelection::from_persisted(gc)
                    .as_str()
                    .to_string()
            })
            .unwrap_or_else(|| "auto".into()),
        jvm_custom_args: request.jvm_custom_args.clone().unwrap_or_default(),
        jvm_always_pre_touch: request.jvm_always_pre_touch.unwrap_or_else(|| {
            crate::models::recommended_java_version_for_minecraft(&request.minecraft_version) < 21
        }),
        created_at: chrono::Utc::now().to_rfc3339(),
        java_path: None,
        java_incompatible_override: false,
        icon_path: None,
        launch_mode_override: "auto".into(),
        import_source: None,
    }
}

fn manifest_from_request(instance_id: &str, request: &CreateInstanceRequest) -> InstanceManifest {
    let mut user_preferences = serde_json::Map::new();
    if let Some(icon_url) = request
        .pack_icon_url
        .as_deref()
        .filter(|url| url.starts_with("https://"))
    {
        user_preferences.insert(
            "agora_pack_icon_url".into(),
            serde_json::Value::String(icon_url.to_owned()),
        );
    }
    InstanceManifest {
        manifest_version: crate::models::CURRENT_MANIFEST_VERSION,
        pack_origin: None,
        instance_id: instance_id.into(),
        name: request.name.clone(),
        created_from_pack: None,
        minecraft_version: request.minecraft_version.clone(),
        loader: request.loader.clone(),
        loader_version: request.loader_version.clone(),
        is_locked: false,
        mods: Vec::new(),
        resourcepacks: Vec::new(),
        shaders: Vec::new(),
        datapacks: Vec::new(),
        worlds: Vec::new(),
        user_preferences: serde_json::Value::Object(user_preferences),
    }
}

fn loader_version_id(row: &InstanceRow) -> String {
    match row.loader.as_str() {
        "fabric" => format!(
            "fabric-loader-{}-{}",
            row.loader_version, row.minecraft_version
        ),
        "quilt" => format!(
            "quilt-loader-{}-{}",
            row.loader_version, row.minecraft_version
        ),
        "forge" => format!("forge-{}-{}", row.minecraft_version, row.loader_version),
        "neoforge" => format!("neoforge-{}", row.loader_version),
        _ => row.minecraft_version.clone(),
    }
}

fn read_manifest(path: &std::path::Path) -> LauncherResult<Option<InstanceManifest>> {
    if !path.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(path).map_err(|_| LauncherError::InstanceCreateFailed)?;
    serde_json::from_str(&text)
        .map(Some)
        .map_err(|_| LauncherError::InstanceCreateFailed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::operation_manager::OpStatus;
    use std::path::PathBuf;

    fn context() -> (Ctx, PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "agora-instance-service-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let ctx = Ctx::for_testing(root.clone());
        crate::db::init_local_state_db(&ctx.paths.local_state_db()).unwrap();
        (ctx, root)
    }

    #[test]
    fn a_pack_gap_is_over_once_the_file_is_in_the_manifest_by_any_route() {
        let manifest: InstanceManifest = serde_json::from_value(serde_json::json!({
            "instance_id": "inst", "name": "Inst", "minecraft_version": "1.21",
            "loader": "fabric", "loader_version": "0.16.0", "is_locked": false,
            "mods": [{
                "filename": "sodium.jar", "registry_id": null, "modrinth_id": null,
                "source": "manual", "version": null, "sha256": "", "installed_at": "now"
            }],
            "resourcepacks": [], "shaders": [], "datapacks": [], "worlds": [],
            "user_preferences": {}
        }))
        .unwrap();
        assert!(manifest_has_file(&manifest, "sodium.jar"));
        assert!(!manifest_has_file(&manifest, "xaero.jar"));
        // Case and whitespace are not forgiven: a different file is a different file.
        assert!(!manifest_has_file(&manifest, "Sodium.jar"));
        assert!(!manifest_has_file(&manifest, " sodium.jar"));
        // An unnamed gap must never be cleared by an unnamed installed entry.
        assert!(!manifest_has_file(&manifest, ""));
    }

    #[test]
    fn get_reports_only_the_pack_gaps_still_missing() {
        let (ctx, root) = context();
        let request = CreateInstanceRequest {
            name: "Inst".into(),
            instance_id: "inst".into(),
            minecraft_version: "1.21".into(),
            loader: "fabric".into(),
            loader_version: "0.16.0".into(),
            jvm_memory_mb: None,
            jvm_memory_mode: None,
            jvm_gc: None,
            jvm_custom_args: None,
            jvm_always_pre_touch: None,
            is_modpack: None,
            pack_icon_url: None,
            template_id: None,
        };
        let conn = crate::db::local_state_connection(&ctx.paths.local_state_db()).unwrap();
        crate::db::upsert_instance(&conn, &prepare_row("inst", &request)).unwrap();
        let manifest: InstanceManifest = serde_json::from_value(serde_json::json!({
            "instance_id": "inst", "name": "Inst", "minecraft_version": "1.21",
            "loader": "fabric", "loader_version": "0.16.0", "is_locked": false,
            "mods": [{
                "filename": "sodium.jar", "registry_id": null, "modrinth_id": null,
                "source": "manual", "version": null, "sha256": "", "installed_at": "now"
            }],
            "resourcepacks": [], "shaders": [], "datapacks": [], "worlds": [],
            "user_preferences": {}, "manifest_version": crate::models::CURRENT_MANIFEST_VERSION
        }))
        .unwrap();
        std::fs::create_dir_all(ctx.paths.instance_dir("inst").unwrap()).unwrap();
        crate::helpers::atomic_write_manifest(
            &ctx.paths.instance_manifest("inst").unwrap(),
            &manifest,
        )
        .unwrap();
        let gap = |filename: &str| crate::db::InstancePackGap {
            filename: filename.into(),
            display_name: filename.into(),
            status: "failed".into(),
            reason: None,
        };
        crate::db::replace_instance_pack_gaps(
            &conn,
            "inst",
            &[gap("sodium.jar"), gap("xaero.jar")],
        )
        .unwrap();
        drop(conn);

        let detail = InstanceService::new(ctx.clone())
            .get("inst")
            .unwrap()
            .unwrap();
        let missing: Vec<&str> = detail
            .pack_gaps
            .iter()
            .map(|g| g.filename.as_str())
            .collect();
        assert_eq!(
            missing,
            vec!["xaero.jar"],
            "sodium.jar is in the manifest now"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    /// An unlocked instance with one mod on disk and a matching manifest.
    fn instance_with_mod() -> (Ctx, PathBuf) {
        let (ctx, root) = context();
        let request = CreateInstanceRequest {
            name: "Inst".into(),
            instance_id: "inst".into(),
            minecraft_version: "1.21".into(),
            loader: "fabric".into(),
            loader_version: "0.16.0".into(),
            jvm_memory_mb: None,
            jvm_memory_mode: None,
            jvm_gc: None,
            jvm_custom_args: None,
            jvm_always_pre_touch: None,
            is_modpack: None,
            pack_icon_url: None,
            template_id: None,
        };
        let conn = crate::db::local_state_connection(&ctx.paths.local_state_db()).unwrap();
        crate::db::upsert_instance(&conn, &prepare_row("inst", &request)).unwrap();
        drop(conn);
        let dir = ctx.paths.instance_dir("inst").unwrap();
        std::fs::create_dir_all(dir.join("mods")).unwrap();
        std::fs::write(dir.join("mods").join("a.jar"), b"original").unwrap();
        let manifest: InstanceManifest = serde_json::from_value(serde_json::json!({
            "instance_id": "inst", "name": "Inst", "minecraft_version": "1.21",
            "loader": "fabric", "loader_version": "0.16.0", "is_locked": false,
            "mods": [], "resourcepacks": [], "shaders": [], "datapacks": [], "worlds": [],
            "user_preferences": {}, "manifest_version": crate::models::CURRENT_MANIFEST_VERSION
        }))
        .unwrap();
        crate::helpers::atomic_write_manifest(
            &ctx.paths.instance_manifest("inst").unwrap(),
            &manifest,
        )
        .unwrap();
        (ctx, root)
    }

    fn lock_snapshot_ids(dir: &Path) -> Vec<String> {
        crate::snapshot::lock_snapshots(dir)
            .unwrap()
            .into_iter()
            .map(|s| s.id)
            .collect()
    }

    #[test]
    fn lock_then_revert_removes_what_was_added_since_and_the_revert_can_be_undone() {
        let (ctx, root) = instance_with_mod();
        let dir = ctx.paths.instance_dir("inst").unwrap();
        let service = InstanceService::new(ctx.clone());
        service.lock("inst").unwrap();
        assert!(
            crate::snapshot::lock_snapshots(&dir).unwrap()[0].is_preserved(),
            "a lock snapshot is never pushed out by automatic retention"
        );
        service.unlock("inst").unwrap();
        std::fs::write(dir.join("mods").join("b.jar"), b"added").unwrap();

        crate::snapshot_service::SnapshotService::new(ctx.clone())
            .revert_to_lock("inst")
            .unwrap();

        assert!(
            !dir.join("mods").join("b.jar").exists(),
            "the mod added after locking is gone"
        );
        assert_eq!(
            std::fs::read(dir.join("mods").join("a.jar")).unwrap(),
            b"original"
        );
        let conn = crate::db::local_state_connection(&ctx.paths.local_state_db()).unwrap();
        assert!(
            !crate::db::get_instance(&conn, "inst")
                .unwrap()
                .unwrap()
                .is_locked
        );

        // The revert's own undo point brings the removed mod back.
        let undo = crate::snapshot::list_snapshots(&dir)
            .unwrap()
            .into_iter()
            .find(|s| {
                s.label
                    .as_deref()
                    .is_some_and(|l| l.starts_with("pre-restore-"))
            })
            .expect("the revert took no undo snapshot");
        crate::snapshot_service::SnapshotService::new(ctx.clone())
            .restore("inst", &undo.id)
            .unwrap();
        assert_eq!(
            std::fs::read(dir.join("mods").join("b.jar")).unwrap(),
            b"added"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn revert_refuses_an_instance_with_no_lock_snapshot_and_changes_nothing() {
        // Locked before lock snapshots existed: there is no lock snapshot to return to.
        let (ctx, root) = instance_with_mod();
        let dir = ctx.paths.instance_dir("inst").unwrap();
        std::fs::write(dir.join("mods").join("b.jar"), b"added").unwrap();

        match crate::snapshot_service::SnapshotService::new(ctx.clone()).revert_to_lock("inst") {
            Err(LauncherError::Generic { code, .. }) => assert_eq!(code, "ERR_NO_LOCK_SNAPSHOT"),
            Err(other) => panic!("expected a plain refusal, got {other:?}"),
            Ok(_) => panic!("revert with no lock snapshot must not restore anything"),
        }
        assert_eq!(
            std::fs::read(dir.join("mods").join("b.jar")).unwrap(),
            b"added"
        );
        assert!(crate::snapshot::list_snapshots(&dir).unwrap().is_empty());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn relocking_replaces_the_lock_snapshot_and_unlocking_keeps_it() {
        let (ctx, root) = instance_with_mod();
        let dir = ctx.paths.instance_dir("inst").unwrap();
        let service = InstanceService::new(ctx.clone());
        service.lock("inst").unwrap();
        let first = lock_snapshot_ids(&dir);
        assert_eq!(first.len(), 1);
        service.unlock("inst").unwrap();
        assert_eq!(
            lock_snapshot_ids(&dir),
            first,
            "unlocking keeps the lock snapshot for Revert"
        );

        std::fs::write(dir.join("mods").join("b.jar"), b"added").unwrap();
        service.lock("inst").unwrap();
        let second = lock_snapshot_ids(&dir);
        assert_eq!(second.len(), 1, "only the newest lock snapshot is kept");
        assert_ne!(second, first);
        service.unlock("inst").unwrap();

        // Revert returns to the second lock, which already had the added mod.
        crate::snapshot_service::SnapshotService::new(ctx.clone())
            .revert_to_lock("inst")
            .unwrap();
        assert_eq!(
            std::fs::read(dir.join("mods").join("b.jar")).unwrap(),
            b"added"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn revert_refuses_while_the_instance_is_locked() {
        let (ctx, root) = instance_with_mod();
        let dir = ctx.paths.instance_dir("inst").unwrap();
        InstanceService::new(ctx.clone()).lock("inst").unwrap();
        match crate::snapshot_service::SnapshotService::new(ctx.clone()).revert_to_lock("inst") {
            Err(LauncherError::Generic { code, .. }) => assert_eq!(code, "ERR_INSTANCE_LOCKED"),
            Err(other) => panic!("expected a refusal, got {other:?}"),
            Ok(_) => panic!("revert on a locked instance must not restore anything"),
        }
        assert_eq!(
            std::fs::read(dir.join("mods").join("a.jar")).unwrap(),
            b"original"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn crud_uses_core_paths_and_database() {
        let (ctx, root) = context();
        let request = CreateInstanceRequest {
            name: "Test".into(),
            instance_id: "test".into(),
            minecraft_version: "1.21".into(),
            loader: "vanilla".into(),
            loader_version: "".into(),
            jvm_memory_mb: None,
            jvm_memory_mode: None,
            jvm_gc: None,
            jvm_custom_args: None,
            jvm_always_pre_touch: None,
            is_modpack: None,
            pack_icon_url: None,
            template_id: None,
        };
        let row = prepare_row("test", &request);
        assert_eq!(row.jvm_memory_mode, "auto");
        let conn = crate::db::local_state_connection(&ctx.paths.local_state_db()).unwrap();
        crate::db::upsert_instance(&conn, &row).unwrap();
        let manifest = manifest_from_request("test", &request);
        let dir = ctx.paths.instance_dir("test").unwrap();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            ctx.paths.instance_manifest("test").unwrap(),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();

        let service = InstanceService::new(ctx.clone());
        assert_eq!(service.list().unwrap().len(), 1);
        assert!(service.get("test").unwrap().is_some());
        service.lock("test").unwrap();
        let locked_detail = service.get("test").unwrap().unwrap();
        assert!(locked_detail.row.is_locked);
        assert!(locked_detail.manifest.unwrap().is_locked);
        service.rename("test", "Renamed").unwrap();
        assert_eq!(service.list().unwrap()[0].name, "Renamed");
        service.unlock("test").unwrap();
        let unlocked_detail = service.get("test").unwrap().unwrap();
        assert!(!unlocked_detail.row.is_locked);
        assert!(!unlocked_detail.manifest.unwrap().is_locked);
        assert!(crate::install_service::InstallService::new(ctx)
            .check_not_locked("test")
            .is_ok());
        service.delete("test").unwrap();
        assert!(service.list().unwrap().is_empty());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn update_jvm_persists_canonical_gc_and_resolves_launch_profile() {
        let (ctx, root) = context();
        let request = CreateInstanceRequest {
            name: "GC Test".into(),
            instance_id: "gc-test".into(),
            minecraft_version: "1.21".into(),
            loader: "vanilla".into(),
            loader_version: "".into(),
            jvm_memory_mb: None,
            jvm_memory_mode: None,
            jvm_gc: None,
            jvm_custom_args: None,
            jvm_always_pre_touch: None,
            is_modpack: None,
            pack_icon_url: None,
            template_id: None,
        };
        let row = prepare_row("gc-test", &request);
        let conn = crate::db::local_state_connection(&ctx.paths.local_state_db()).unwrap();
        crate::db::upsert_instance(&conn, &row).unwrap();
        drop(conn);

        let service = InstanceService::new(ctx.clone());
        let cases = [
            (
                "low_latency",
                "low_latency",
                Some(crate::gc::GcProfile::LowLatency),
            ),
            (
                "high_efficiency",
                "high_efficiency",
                Some(crate::gc::GcProfile::HighEfficiency),
            ),
            ("manual", "manual", Some(crate::gc::GcProfile::Manual)),
            ("auto", "auto", None),
        ];

        for (input, persisted, expected_profile) in cases {
            service
                .update_jvm("gc-test", 4096, input, true, "", "manual")
                .unwrap();
            let conn = crate::db::local_state_connection(&ctx.paths.local_state_db()).unwrap();
            let row = crate::db::get_instance(&conn, "gc-test")
                .unwrap()
                .expect("fixture row should exist");
            assert_eq!(row.jvm_gc, persisted);
            assert_eq!(
                crate::gc::GcSelection::from_persisted(&row.jvm_gc).resolve(),
                expected_profile
            );
        }

        let auto_java_17 = crate::gc::compute_gc(17, 4096, "", None).jvm_args;
        let low_latency_java_17 =
            crate::gc::compute_gc(17, 4096, "", Some(crate::gc::GcProfile::LowLatency)).jvm_args;
        assert_ne!(low_latency_java_17, auto_java_17);

        let auto_java_21 = crate::gc::compute_gc(21, 4096, "", None).jvm_args;
        let high_efficiency_java_21 =
            crate::gc::compute_gc(21, 4096, "", Some(crate::gc::GcProfile::HighEfficiency))
                .jvm_args;
        assert_ne!(high_efficiency_java_21, auto_java_21);

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn invalid_jvm_gc_is_rejected_without_changing_the_row() {
        let (ctx, root) = context();
        let request = CreateInstanceRequest {
            name: "GC Invalid Test".into(),
            instance_id: "gc-invalid".into(),
            minecraft_version: "1.21".into(),
            loader: "vanilla".into(),
            loader_version: "".into(),
            jvm_memory_mb: None,
            jvm_memory_mode: None,
            jvm_gc: None,
            jvm_custom_args: None,
            jvm_always_pre_touch: None,
            is_modpack: None,
            pack_icon_url: None,
            template_id: None,
        };
        let row = prepare_row("gc-invalid", &request);
        let conn = crate::db::local_state_connection(&ctx.paths.local_state_db()).unwrap();
        crate::db::upsert_instance(&conn, &row).unwrap();
        drop(conn);

        let service = InstanceService::new(ctx.clone());
        service
            .update_jvm("gc-invalid", 4096, "low_latency", true, "", "manual")
            .unwrap();
        let conn = crate::db::local_state_connection(&ctx.paths.local_state_db()).unwrap();
        let before = crate::db::get_instance(&conn, "gc-invalid")
            .unwrap()
            .expect("fixture row should exist");
        drop(conn);

        for invalid in ["shenandoah", "nonsense"] {
            let error = service
                .update_jvm("gc-invalid", 8192, invalid, false, "-Xss1M", "auto")
                .unwrap_err();
            assert_eq!(error.code(), "ERR_INVALID_JVM_GC");
            assert!(error.to_string().contains(invalid));

            let conn = crate::db::local_state_connection(&ctx.paths.local_state_db()).unwrap();
            let after = crate::db::get_instance(&conn, "gc-invalid")
                .unwrap()
                .expect("fixture row should exist");
            assert_eq!(after.jvm_gc, before.jvm_gc);
        }

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn delegated_preparation_materializes_base_and_loader_profiles() {
        let (ctx, root) = context();
        let request = CreateInstanceRequest {
            name: "Delegated".into(),
            instance_id: "delegated".into(),
            minecraft_version: "26.2".into(),
            loader: "fabric".into(),
            loader_version: "0.19.3".into(),
            jvm_memory_mb: None,
            jvm_memory_mode: None,
            jvm_gc: None,
            jvm_custom_args: None,
            jvm_always_pre_touch: None,
            is_modpack: None,
            pack_icon_url: None,
            template_id: None,
        };
        let row = prepare_row("delegated", &request);
        let conn = crate::db::local_state_connection(&ctx.paths.local_state_db()).unwrap();
        crate::db::upsert_instance(&conn, &row).unwrap();
        let instance_dir = ctx.paths.instance_dir("delegated").unwrap();
        std::fs::create_dir_all(&instance_dir).unwrap();
        std::fs::write(
            ctx.paths.instance_manifest("delegated").unwrap(),
            serde_json::to_vec(&manifest_from_request("delegated", &request)).unwrap(),
        )
        .unwrap();

        let runtime_root = ctx.paths.minecraft_runtime_root();
        for (version_id, json) in [
            ("26.2", r#"{"id":"26.2","type":"release"}"#),
            (
                "fabric-loader-0.19.3-26.2",
                r#"{"id":"fabric-loader-0.19.3-26.2","inheritsFrom":"26.2"}"#,
            ),
        ] {
            let path = runtime_root
                .join("versions")
                .join(version_id)
                .join(format!("{version_id}.json"));
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, json).unwrap();
        }

        let official_profiles = ctx.launcher_profiles_path.clone().unwrap();
        let preparation = InstanceService::new(ctx).prepare_delegated_launch("delegated");
        assert_eq!(preparation.unwrap().profile_id, "agora-delegated");
        let official_root = official_profiles.parent().unwrap();
        assert!(official_root.join("versions/26.2/26.2.json").is_file());
        assert!(official_root
            .join("versions/fabric-loader-0.19.3-26.2/fabric-loader-0.19.3-26.2.json")
            .is_file());

        let profiles: serde_json::Value =
            serde_json::from_slice(&std::fs::read(official_profiles).unwrap()).unwrap();
        assert_eq!(
            profiles["profiles"]["agora-delegated"]["lastVersionId"],
            "fabric-loader-0.19.3-26.2"
        );

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn clone_creates_new_instance_with_copied_files() {
        let (ctx, root) = context();
        let request = CreateInstanceRequest {
            name: "Original".into(),
            instance_id: "original".into(),
            minecraft_version: "1.21".into(),
            loader: "vanilla".into(),
            loader_version: "".into(),
            jvm_memory_mb: None,
            jvm_memory_mode: None,
            jvm_gc: None,
            jvm_custom_args: None,
            jvm_always_pre_touch: None,
            is_modpack: None,
            pack_icon_url: None,
            template_id: None,
        };
        let row = prepare_row("original", &request);
        let conn = crate::db::local_state_connection(&ctx.paths.local_state_db()).unwrap();
        crate::db::upsert_instance(&conn, &row).unwrap();
        drop(conn);
        // Create source directory and manifest
        let src_dir = ctx.paths.instance_dir("original").unwrap();
        std::fs::create_dir_all(src_dir.join("mods")).unwrap();
        std::fs::write(src_dir.join("mods").join("test.jar"), b"mod data").unwrap();
        std::fs::create_dir_all(src_dir.join("saves")).unwrap();
        std::fs::write(src_dir.join("saves").join("world.dat"), b"world").unwrap();
        let manifest = manifest_from_request("original", &request);
        std::fs::write(
            ctx.paths.instance_manifest("original").unwrap(),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();

        let service = InstanceService::new(ctx.clone());
        let rt = tokio::runtime::Runtime::new().unwrap();
        let clone_row = rt
            .block_on(service.clone(CloneRequest {
                source_instance_id: "original".into(),
                new_name: "Clone".into(),
                prefs: ClonePrefs::default(),
            }))
            .unwrap();

        assert_eq!(clone_row.name, "Clone");
        assert_eq!(clone_row.minecraft_version, "1.21");
        assert_eq!(clone_row.loader, "vanilla");

        let cloned_dir = ctx.paths.instance_dir("Clone").unwrap();
        assert!(cloned_dir.join("mods").join("test.jar").exists());
        assert!(cloned_dir.join("saves").join("world.dat").exists());

        let cloned_conn = crate::db::local_state_connection(&ctx.paths.local_state_db()).unwrap();
        let cloned_detail = crate::db::get_instance(&cloned_conn, "Clone").unwrap();
        assert!(cloned_detail.is_some());
        assert_eq!(cloned_detail.unwrap().name, "Clone");

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn clone_succeeds_when_source_is_locked_and_clone_is_unlocked() {
        let (ctx, root) = context();
        let request = CreateInstanceRequest {
            name: "Locked".into(),
            instance_id: "locked".into(),
            minecraft_version: "1.21".into(),
            loader: "vanilla".into(),
            loader_version: "".into(),
            jvm_memory_mb: None,
            jvm_memory_mode: None,
            jvm_gc: None,
            jvm_custom_args: None,
            jvm_always_pre_touch: None,
            is_modpack: None,
            pack_icon_url: None,
            template_id: None,
        };
        let row = prepare_row("locked", &request);
        let conn = crate::db::local_state_connection(&ctx.paths.local_state_db()).unwrap();
        crate::db::upsert_instance(&conn, &row).unwrap();
        crate::db::set_locked(&conn, "locked", true).unwrap();
        drop(conn);
        // Fully lock the source: DB row AND manifest.
        let src_dir = ctx.paths.instance_dir("locked").unwrap();
        std::fs::create_dir_all(&src_dir).unwrap();
        let mut manifest = manifest_from_request("locked", &request);
        manifest.is_locked = true;
        std::fs::write(
            ctx.paths.instance_manifest("locked").unwrap(),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();

        let service = InstanceService::new(ctx.clone());
        let rt = tokio::runtime::Runtime::new().unwrap();
        let clone_row = rt
            .block_on(service.clone(CloneRequest {
                source_instance_id: "locked".into(),
                new_name: "Clone".into(),
                prefs: ClonePrefs::default(),
            }))
            .unwrap();

        // Clones are always fresh, unlocked working copies — never inherit the
        // source's locked flag in either the DB row or the manifest.
        assert!(!clone_row.is_locked);
        let cloned_conn = crate::db::local_state_connection(&ctx.paths.local_state_db()).unwrap();
        let cloned_row = crate::db::get_instance(&cloned_conn, "Clone")
            .unwrap()
            .unwrap();
        assert!(!cloned_row.is_locked);
        let cloned_manifest = read_manifest(&ctx.paths.instance_manifest("Clone").unwrap())
            .unwrap()
            .unwrap();
        assert!(!cloned_manifest.is_locked);

        let _ = std::fs::remove_dir_all(root);
    }

    /// The reported symptom: deleting a pack left its entry in the official
    /// launcher's installation list, and the launcher kept it as the selected
    /// installation even though nothing was left to launch.
    #[test]
    fn delete_removes_the_official_launcher_profile_and_selection() {
        let (ctx, root) = context();
        let profiles_path = ctx.launcher_profiles_path.clone().unwrap();
        let official_root = profiles_path.parent().unwrap().to_path_buf();
        let request = CreateInstanceRequest {
            name: "Doomed".into(),
            instance_id: "doomed".into(),
            minecraft_version: "1.21".into(),
            loader: "vanilla".into(),
            loader_version: "".into(),
            jvm_memory_mb: None,
            jvm_memory_mode: None,
            jvm_gc: None,
            jvm_custom_args: None,
            jvm_always_pre_touch: None,
            is_modpack: None,
            pack_icon_url: None,
            template_id: None,
        };
        let row = prepare_row("doomed", &request);
        let conn = crate::db::local_state_connection(&ctx.paths.local_state_db()).unwrap();
        crate::db::upsert_instance(&conn, &row).unwrap();
        // A survivor, so the deleted pack is not the only thing to select.
        let survivor_request = CreateInstanceRequest {
            name: "Keeper".into(),
            instance_id: "keeper".into(),
            ..request.clone()
        };
        crate::db::upsert_instance(&conn, &prepare_row("keeper", &survivor_request)).unwrap();
        drop(conn);
        for id in ["doomed", "keeper"] {
            let dir = ctx.paths.instance_dir(id).unwrap();
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(
                ctx.paths.instance_manifest(id).unwrap(),
                serde_json::to_vec(&manifest_from_request(id, &request)).unwrap(),
            )
            .unwrap();
            crate::launcher_profiles::upsert_profile(
                &crate::launcher_profiles::LauncherProfileEntry {
                    profile_id: crate::launcher_profiles::profile_id_for(id),
                    name: format!("{id} (Agora)"),
                    last_version_id: "1.21".into(),
                    game_dir: dir,
                    java_args: String::new(),
                    select: false,
                },
                &profiles_path,
            )
            .unwrap();
        }
        write_ui_state_selecting(&official_root, "agora-doomed");

        InstanceService::new(ctx).delete("doomed").unwrap();

        let profiles: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&profiles_path).unwrap()).unwrap();
        assert!(
            profiles["profiles"].get("agora-doomed").is_none(),
            "the deleted pack must not stay in the launcher's installation list"
        );
        assert!(profiles["profiles"].get("agora-keeper").is_some());
        assert_eq!(
            crate::launcher_ui_state::selected_profile_id(&official_root).as_deref(),
            Some("agora-keeper"),
            "the launcher must not reopen on the pack that was just deleted"
        );

        let _ = std::fs::remove_dir_all(root);
    }

    /// Minimal stand-in for the launcher's UI-state file: a banner, then a
    /// document whose `UiSettings` is itself a JSON string.
    fn write_ui_state_selecting(official_root: &std::path::Path, profile_id: &str) {
        let settings = serde_json::json!({
            "homePageLastSelected": {
                "productId": "java",
                "javaConfiguration": {
                    "id": profile_id,
                    "name": "doomed (Agora)",
                    "versionId": "1.21",
                    "gameDir": "C:/gone",
                    "javaArgs": "",
                    "type": "custom",
                    "icon": "Furnace"
                }
            }
        });
        let root = serde_json::json!({
            "data": { "UiSettings": serde_json::to_string(&settings).unwrap() }
        });
        std::fs::create_dir_all(official_root).unwrap();
        std::fs::write(
            official_root.join("launcher_ui_state_microsoft_store.json"),
            format!(
                "#$
DO NOT EDIT
$#
{}",
                serde_json::to_string(&root).unwrap()
            ),
        )
        .unwrap();
    }

    #[test]
    fn delete_registers_and_completes_operation() {
        let (ctx, root) = context();
        let op_mgr = ctx.operation_manager.clone();
        let request = CreateInstanceRequest {
            name: "DelOp".into(),
            instance_id: "delop".into(),
            minecraft_version: "1.21".into(),
            loader: "vanilla".into(),
            loader_version: "".into(),
            jvm_memory_mb: None,
            jvm_memory_mode: None,
            jvm_gc: None,
            jvm_custom_args: None,
            jvm_always_pre_touch: None,
            is_modpack: None,
            pack_icon_url: None,
            template_id: None,
        };
        let row = prepare_row("delop", &request);
        let conn = crate::db::local_state_connection(&ctx.paths.local_state_db()).unwrap();
        crate::db::upsert_instance(&conn, &row).unwrap();
        let manifest = manifest_from_request("delop", &request);
        let dir = ctx.paths.instance_dir("delop").unwrap();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            ctx.paths.instance_manifest("delop").unwrap(),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        drop(conn);

        let service = InstanceService::new(ctx);
        assert_eq!(op_mgr.active_count(), 0);

        service.delete("delop").unwrap();

        let all = op_mgr.list_all();
        let del_ops: Vec<_> = all
            .iter()
            .filter(|o| o.label == "Delete instance")
            .collect();
        assert_eq!(del_ops.len(), 1);
        assert_eq!(del_ops[0].status, OpStatus::Completed);

        let _ = std::fs::remove_dir_all(root);
    }

    #[cfg(test)]
    mod clone_tracking {
        use super::*;

        #[test]
        fn clone_registers_and_completes_operation() {
            let (ctx, root) = context();
            let profiles_path = ctx.launcher_profiles_path.clone().unwrap();
            assert!(profiles_path.starts_with(&root));
            let op_mgr = ctx.operation_manager.clone();
            let request = CreateInstanceRequest {
                name: "Src".into(),
                instance_id: "src".into(),
                minecraft_version: "1.21".into(),
                loader: "vanilla".into(),
                loader_version: "".into(),
                jvm_memory_mb: None,
                jvm_memory_mode: None,
                jvm_gc: None,
                jvm_custom_args: None,
                jvm_always_pre_touch: None,
                is_modpack: None,
                pack_icon_url: None,
                template_id: None,
            };
            let row = prepare_row("src", &request);
            let conn = crate::db::local_state_connection(&ctx.paths.local_state_db()).unwrap();
            crate::db::upsert_instance(&conn, &row).unwrap();
            drop(conn);
            let src_dir = ctx.paths.instance_dir("src").unwrap();
            std::fs::create_dir_all(&src_dir).unwrap();
            std::fs::write(
                ctx.paths.instance_manifest("src").unwrap(),
                serde_json::to_vec(&manifest_from_request("src", &request)).unwrap(),
            )
            .unwrap();

            let service = InstanceService::new(ctx);
            let rt = tokio::runtime::Runtime::new().unwrap();
            let _clone_row = rt
                .block_on(service.clone(CloneRequest {
                    source_instance_id: "src".into(),
                    new_name: "CloneOp".into(),
                    prefs: ClonePrefs::default(),
                }))
                .unwrap();

            let all = op_mgr.list_all();
            let clone_ops: Vec<_> = all.iter().filter(|o| o.label == "Clone instance").collect();
            assert_eq!(clone_ops.len(), 1);
            assert_eq!(clone_ops[0].status, OpStatus::Completed);
            let profiles: serde_json::Value = serde_json::from_str(
                &std::fs::read_to_string(&profiles_path).expect("isolated launcher profiles"),
            )
            .unwrap();
            assert!(profiles["profiles"].get("agora-CloneOp").is_some());

            let _ = std::fs::remove_dir_all(root);
        }
    }

    #[test]
    fn clone_failure_registers_failed_operation() {
        let (ctx, root) = context();
        let op_mgr = ctx.operation_manager.clone();
        let service = InstanceService::new(ctx);
        let rt = tokio::runtime::Runtime::new().unwrap();
        // Source does not exist in DB — should fail.
        let err = rt
            .block_on(service.clone(CloneRequest {
                source_instance_id: "nonexistent".into(),
                new_name: "FailClone".into(),
                prefs: ClonePrefs::default(),
            }))
            .unwrap_err();
        assert_eq!(err.code(), "ERR_INSTANCE_NOT_FOUND");

        let all = op_mgr.list_all();
        let clone_ops: Vec<_> = all.iter().filter(|o| o.label == "Clone instance").collect();
        assert_eq!(clone_ops.len(), 1);
        assert!(matches!(clone_ops[0].status, OpStatus::Failed(_)));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn clone_keeps_pack_identity_with_cloned_from_and_fresh_hash() {
        let (ctx, root) = context();
        // Create source instance with a real PackOrigin and a file to hash
        let request = CreateInstanceRequest {
            name: "SourcePack".into(),
            instance_id: "source-pack".into(),
            minecraft_version: "1.20.1".into(),
            loader: "fabric".into(),
            loader_version: "0.15.0".into(),
            jvm_memory_mb: None,
            jvm_memory_mode: None,
            jvm_gc: None,
            jvm_custom_args: None,
            jvm_always_pre_touch: None,
            is_modpack: None,
            pack_icon_url: None,
            template_id: None,
        };
        let row = prepare_row("source-pack", &request);
        let conn = crate::db::local_state_connection(&ctx.paths.local_state_db()).unwrap();
        crate::db::upsert_instance(&conn, &row).unwrap();
        drop(conn);
        let src_dir = ctx.paths.instance_dir("source-pack").unwrap();
        std::fs::create_dir_all(src_dir.join("mods")).unwrap();
        std::fs::write(src_dir.join("mods").join("a.jar"), b"pack mod").unwrap();
        std::fs::create_dir_all(src_dir.join("config")).unwrap();
        std::fs::write(src_dir.join("config").join("pack.toml"), b"cfg").unwrap();

        let mut manifest = manifest_from_request("source-pack", &request);
        let pack_files = crate::pack_inventory::collect_pack_inventory(&src_dir).unwrap();
        let pack_hash = crate::pack_inventory::pack_content_hash(&pack_files);
        manifest.pack_origin = Some(crate::models::PackOrigin {
            platform: crate::models::PackPlatform::Modrinth,
            pack_name: "SourcePack".into(),
            project_id: Some("proj-123".into()),
            version_id: Some("v1".into()),
            version_number: None,
            origin_url: Some("https://cdn.modrinth.com/pack.mrpack".into()),
            pack_content_hash: Some(pack_hash.clone()),
            pack_minecraft_version: Some("1.20.1".into()),
            pack_loader: Some("fabric".into()),
            pack_loader_version: Some("0.15.0".into()),
            launcher_kind: None,
            installation_key: None,
            source_key: None,
            cloned_from: None,
            installed_at: "2024-01-01T00:00:00Z".into(),
        });
        std::fs::write(
            ctx.paths.instance_manifest("source-pack").unwrap(),
            serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .unwrap();
        // Persist pack inventory for source
        let conn = crate::db::local_state_connection(&ctx.paths.local_state_db()).unwrap();
        crate::db::replace_instance_pack_files(&conn, "source-pack", &pack_files).unwrap();
        drop(conn);

        let service = InstanceService::new(ctx.clone());
        let rt = tokio::runtime::Runtime::new().unwrap();
        let clone_row = rt
            .block_on(service.clone(CloneRequest {
                source_instance_id: "source-pack".into(),
                new_name: "ClonedPack".into(),
                prefs: ClonePrefs::default(),
            }))
            .unwrap();
        assert_eq!(clone_row.instance_id, "ClonedPack");

        // Manifest should keep pack identity but set cloned_from and fresh installed_at
        let clone_manifest =
            crate::helpers::read_manifest(&ctx.paths.instance_manifest("ClonedPack").unwrap())
                .unwrap();
        let origin = clone_manifest
            .pack_origin
            .expect("clone must keep PackOrigin");
        assert_eq!(origin.platform, crate::models::PackPlatform::Modrinth);
        assert_eq!(origin.pack_name, "SourcePack");
        assert_eq!(origin.project_id.as_deref(), Some("proj-123"));
        assert_eq!(origin.cloned_from.as_deref(), Some("source-pack"));
        assert_ne!(origin.installed_at, "2024-01-01T00:00:00Z");
        assert!(origin.pack_content_hash.is_some());
        // Hash must match the cloned directory's actual inventory (recomputed, not copied stale)
        let clone_dir = ctx.paths.instance_dir("ClonedPack").unwrap();
        let live_files = crate::pack_inventory::collect_pack_inventory(&clone_dir).unwrap();
        assert_eq!(
            origin.pack_content_hash.as_deref(),
            Some(crate::pack_inventory::pack_content_hash(&live_files).as_str())
        );
        // DB inventory for clone must exist and match live files (recomputed, not empty)
        let conn = crate::db::local_state_connection(&ctx.paths.local_state_db()).unwrap();
        let db_files = crate::db::list_instance_pack_files(&conn, "ClonedPack").unwrap();
        assert!(!db_files.is_empty());
        assert_eq!(db_files, live_files);

        let _ = std::fs::remove_dir_all(root);
    }

    mod recently_deleted_tests {
        use super::*;
        use crate::instance_service::TrashFn;
        use std::sync::Mutex;

        fn request(id: &str) -> CreateInstanceRequest {
            CreateInstanceRequest {
                name: format!("Name {id}"),
                instance_id: id.into(),
                minecraft_version: "1.21".into(),
                loader: "vanilla".into(),
                loader_version: "".into(),
                jvm_memory_mb: None,
                jvm_memory_mode: None,
                jvm_gc: None,
                jvm_custom_args: None,
                jvm_always_pre_touch: None,
                is_modpack: None,
                pack_icon_url: None,
                template_id: None,
            }
        }

        /// An instance with files, pack inventory, import provenance and launch history.
        fn seed(ctx: &Ctx, id: &str) {
            let req = request(id);
            let conn = crate::db::local_state_connection(&ctx.paths.local_state_db()).unwrap();
            crate::db::upsert_instance(&conn, &prepare_row(id, &req)).unwrap();
            let dir = ctx.paths.instance_dir(id).unwrap();
            std::fs::create_dir_all(dir.join("mods")).unwrap();
            std::fs::create_dir_all(dir.join("snapshots")).unwrap();
            std::fs::write(dir.join("mods").join("a.jar"), b"jar-bytes").unwrap();
            std::fs::write(dir.join("snapshots").join("s1"), b"snapshot").unwrap();
            std::fs::write(
                ctx.paths.instance_manifest(id).unwrap(),
                serde_json::to_vec(&manifest_from_request(id, &req)).unwrap(),
            )
            .unwrap();
            crate::db::replace_instance_pack_files(
                &conn,
                id,
                &[crate::db::InstancePackFile {
                    relative_path: "mods/a.jar".into(),
                    sha256: "ab".repeat(32),
                    size: 9,
                }],
            )
            .unwrap();
            let record = crate::db::InstanceImportRecord {
                instance_id: id.into(),
                launcher_kind: "curseforge".into(),
                installation_key: "cf:test".into(),
                source_instance_key: "src-key".into(),
                source_path: "C:/cf/test".into(),
                source_metadata_json: "{}".into(),
                imported_settings_json: "{}".into(),
                source_fingerprint: "fp".into(),
                imported_at: "2026-01-01T00:00:00Z".into(),
                updated_at: "2026-01-01T00:00:00Z".into(),
                last_result: "imported".into(),
            };
            let files = [crate::db::InstanceImportFileRecord {
                relative_path: "mods/a.jar".into(),
                sha256: "cd".repeat(32),
                size: 9,
                source_modified_ns: Some(5),
            }];
            crate::db::replace_instance_import(&conn, &record, &files).unwrap();
            crate::launch_history::begin_launch(
                &conn,
                id,
                "2026-01-02T00:00:00Z",
                3,
                "1.21",
                "vanilla",
            )
            .unwrap();
        }

        fn db_state(ctx: &Ctx, id: &str) -> String {
            let conn = crate::db::local_state_connection(&ctx.paths.local_state_db()).unwrap();
            serde_json::to_string(&crate::recently_deleted::capture_db_state(&conn, id).unwrap())
                .unwrap()
        }

        fn files_of(dir: &std::path::Path) -> Vec<(String, Vec<u8>)> {
            let mut out = Vec::new();
            let mut stack = vec![dir.to_path_buf()];
            while let Some(d) = stack.pop() {
                for e in std::fs::read_dir(&d).unwrap() {
                    let p = e.unwrap().path();
                    let m = std::fs::symlink_metadata(&p).unwrap();
                    if m.file_type().is_symlink() {
                        out.push((
                            format!("LINK:{}", p.strip_prefix(dir).unwrap().display()),
                            Vec::new(),
                        ));
                    } else if m.is_dir() {
                        stack.push(p);
                    } else {
                        out.push((
                            p.strip_prefix(dir).unwrap().display().to_string(),
                            std::fs::read(&p).unwrap(),
                        ));
                    }
                }
            }
            out.sort();
            out
        }

        /// A stand-in for the OS trash: moves the folder aside and remembers it.
        fn fake_trash(bin: std::path::PathBuf) -> (TrashFn, Arc<Mutex<Vec<std::path::PathBuf>>>) {
            let seen = Arc::new(Mutex::new(Vec::new()));
            let seen2 = seen.clone();
            std::fs::create_dir_all(&bin).unwrap();
            let f: TrashFn = Arc::new(move |path: &Path| {
                let dest = bin.join(path.file_name().unwrap());
                std::fs::rename(path, &dest).map_err(|e| LauncherError::Generic {
                    code: "ERR_TEST".into(),
                    message: e.to_string(),
                })?;
                seen2.lock().unwrap().push(dest);
                Ok(())
            });
            (f, seen)
        }

        fn failing_trash() -> TrashFn {
            Arc::new(|_: &Path| {
                Err(LauncherError::Generic {
                    code: "ERR_TEST".into(),
                    message: "trash unavailable".into(),
                })
            })
        }

        #[cfg(windows)]
        fn make_link(target: &Path, link: &Path) {
            junction::create(target, link).unwrap();
        }
        #[cfg(unix)]
        fn make_link(target: &Path, link: &Path) {
            std::os::unix::fs::symlink(target, link).unwrap();
        }

        #[test]
        fn delete_lists_then_restore_is_identical() {
            let (ctx, root) = context();
            seed(&ctx, "pack");
            let dir = ctx.paths.instance_dir("pack").unwrap();
            let files_before = files_of(&dir);
            let db_before = db_state(&ctx, "pack");
            assert!(db_before.contains("curseforge") && db_before.contains("mods/a.jar"));
            let service = InstanceService::new(ctx.clone());

            service.delete("pack").unwrap();
            assert!(service.list().unwrap().is_empty());
            assert!(!dir.exists());
            let entries = service.list_recently_deleted().unwrap();
            assert_eq!(entries.len(), 1);
            assert_eq!(entries[0].instance_id, "pack");
            assert_eq!(entries[0].name, "Name pack");
            assert_eq!(entries[0].import_source.as_deref(), Some("curseforge"));
            assert!(entries[0].size_bytes > 0);
            // The cascade really removed the rows while it was deleted.
            assert!(!db_state(&ctx, "pack").contains("curseforge"));

            let outcome = service.restore_deleted(&entries[0].entry_id).unwrap();
            assert_eq!(outcome.instance_id, "pack");
            assert!(!outcome.renamed);
            assert_eq!(files_of(&dir), files_before);
            assert_eq!(db_state(&ctx, "pack"), db_before);
            assert!(service.list_recently_deleted().unwrap().is_empty());
            assert_eq!(service.list().unwrap()[0].name, "Name pack");
            let _ = std::fs::remove_dir_all(root);
        }

        #[test]
        fn restore_when_id_is_taken_uses_a_free_name_and_says_so() {
            let (ctx, root) = context();
            seed(&ctx, "pack");
            let service = InstanceService::new(ctx.clone());
            service.delete("pack").unwrap();
            seed(&ctx, "pack"); // a new instance took the id
            let entry = service.list_recently_deleted().unwrap().remove(0);

            let outcome = service.restore_deleted(&entry.entry_id).unwrap();
            assert!(outcome.renamed);
            assert_eq!(outcome.original_instance_id, "pack");
            assert_eq!(outcome.instance_id, "pack-restored");
            let rows = service.list().unwrap();
            assert_eq!(rows.len(), 2);
            let restored = rows
                .iter()
                .find(|r| r.instance_id == "pack-restored")
                .unwrap();
            assert_eq!(restored.name, "Name pack (restored)");
            assert_eq!(restored.import_source.as_deref(), Some("curseforge"));
            let manifest = read_manifest(&ctx.paths.instance_manifest("pack-restored").unwrap())
                .unwrap()
                .unwrap();
            assert_eq!(manifest.instance_id, "pack-restored");
            let conn = crate::db::local_state_connection(&ctx.paths.local_state_db()).unwrap();
            assert_eq!(
                crate::db::list_instance_pack_files(&conn, "pack-restored")
                    .unwrap()
                    .len(),
                1
            );
            // The live one is untouched.
            assert_eq!(
                crate::db::list_instance_pack_files(&conn, "pack")
                    .unwrap()
                    .len(),
                1
            );
            let _ = std::fs::remove_dir_all(root);
        }

        #[test]
        fn cleanup_purges_only_after_the_window() {
            let (ctx, root) = context();
            let clock = crate::ctx::TestClock::new(std::time::SystemTime::now());
            let ctx = ctx.with_clock(Arc::new(clock.clone()));
            seed(&ctx, "old");
            seed(&ctx, "fresh");
            let service = InstanceService::new(ctx.clone());
            service.delete("old").unwrap();
            clock.advance(std::time::Duration::from_secs(20 * 24 * 3600));
            service.delete("fresh").unwrap();
            let (trash, seen) = fake_trash(root.join("bin"));

            // Day 20: nothing is old enough.
            let summary = service.cleanup_recently_deleted(&trash);
            assert_eq!((summary.purged, summary.failed, summary.kept), (0, 0, 2));

            // Day 31: "old" is past 30 days, "fresh" (11 days) is not.
            clock.advance(std::time::Duration::from_secs(11 * 24 * 3600));
            let summary = service.cleanup_recently_deleted(&trash);
            assert_eq!((summary.purged, summary.failed, summary.kept), (1, 0, 1));
            assert_eq!(seen.lock().unwrap().len(), 1);
            let left = service.list_recently_deleted().unwrap();
            assert_eq!(left.len(), 1);
            assert_eq!(left[0].instance_id, "fresh");
            let _ = std::fs::remove_dir_all(root);
        }

        #[test]
        fn purge_failure_leaves_the_entry_restorable() {
            let (ctx, root) = context();
            let clock = crate::ctx::TestClock::new(std::time::SystemTime::now());
            let ctx = ctx.with_clock(Arc::new(clock.clone()));
            seed(&ctx, "pack");
            let service = InstanceService::new(ctx.clone());
            service.delete("pack").unwrap();
            let entry = service.list_recently_deleted().unwrap().remove(0);
            clock.advance(std::time::Duration::from_secs(40 * 24 * 3600));

            let summary = service.cleanup_recently_deleted(&failing_trash());
            assert_eq!((summary.purged, summary.failed), (0, 1));
            assert!(service
                .purge_deleted(&entry.entry_id, &failing_trash())
                .is_err());
            assert_eq!(service.list_recently_deleted().unwrap().len(), 1);
            service.restore_deleted(&entry.entry_id).unwrap();
            assert!(ctx
                .paths
                .instance_dir("pack")
                .unwrap()
                .join("mods/a.jar")
                .is_file());
            let _ = std::fs::remove_dir_all(root);
        }

        #[test]
        fn a_link_inside_the_instance_is_never_followed() {
            let (ctx, root) = context();
            seed(&ctx, "linked");
            let outside = root.join("outside");
            std::fs::create_dir_all(&outside).unwrap();
            std::fs::write(outside.join("precious.txt"), b"keep me").unwrap();
            std::fs::write(outside.join("big.bin"), vec![0u8; 2_000_000]).unwrap();
            let dir = ctx.paths.instance_dir("linked").unwrap();
            make_link(&outside, &dir.join("shared"));
            let service = InstanceService::new(ctx.clone());

            service.delete("linked").unwrap();
            let entry = service.list_recently_deleted().unwrap().remove(0);
            // The size counted the instance's own files, not the link target.
            assert!(entry.size_bytes < 1_000_000);
            assert_eq!(
                std::fs::read(outside.join("precious.txt")).unwrap(),
                b"keep me"
            );

            // Restoring brings the link back as a link.
            service.restore_deleted(&entry.entry_id).unwrap();
            assert!(std::fs::symlink_metadata(dir.join("shared"))
                .unwrap()
                .file_type()
                .is_symlink());

            // Purging detaches the link before the (fake) trash sees the tree.
            service.delete("linked").unwrap();
            let entry = service.list_recently_deleted().unwrap().remove(0);
            let (trash, seen) = fake_trash(root.join("bin"));
            service.purge_deleted(&entry.entry_id, &trash).unwrap();
            let moved = seen.lock().unwrap()[0].clone();
            assert!(files_of(&moved)
                .iter()
                .all(|(name, _)| !name.starts_with("LINK:")));
            assert!(std::fs::symlink_metadata(moved.join("instance").join("shared")).is_err());
            assert_eq!(
                std::fs::read(outside.join("precious.txt")).unwrap(),
                b"keep me"
            );
            let _ = std::fs::remove_dir_all(root);
        }

        #[test]
        fn size_budget_purges_oldest_first_and_only_as_much_as_needed() {
            let (ctx, root) = context();
            let clock = crate::ctx::TestClock::new(std::time::SystemTime::now());
            let ctx = ctx.with_clock(Arc::new(clock.clone()));
            for id in ["first", "second", "third"] {
                seed(&ctx, id);
                std::fs::write(
                    ctx.paths.instance_dir(id).unwrap().join("big.bin"),
                    vec![1u8; 1000],
                )
                .unwrap();
            }
            let service = InstanceService::new(ctx.clone());
            for id in ["first", "second", "third"] {
                service.delete(id).unwrap();
                clock.advance(std::time::Duration::from_secs(60));
            }
            let entries = service.list_recently_deleted().unwrap();
            let each = entries[0].size_bytes;
            let (trash, seen) = fake_trash(root.join("bin"));
            // Room for two of the three: only the oldest goes.
            let summary = crate::recently_deleted::cleanup_with_budget(
                &ctx.paths.recently_deleted_root(),
                chrono::DateTime::<chrono::Utc>::from(ctx.clock.now()),
                &trash,
                each * 2 + 10,
            );
            assert_eq!((summary.purged, summary.kept), (1, 2));
            assert_eq!(seen.lock().unwrap().len(), 1);
            let left: Vec<_> = service
                .list_recently_deleted()
                .unwrap()
                .into_iter()
                .map(|e| e.instance_id)
                .collect();
            assert!(left.contains(&"second".to_string()) && left.contains(&"third".to_string()));
            let _ = std::fs::remove_dir_all(root);
        }

        #[test]
        fn locked_instance_is_refused_and_untouched() {
            let (ctx, root) = context();
            seed(&ctx, "pack");
            let service = InstanceService::new(ctx.clone());
            service.lock("pack").unwrap();
            let before = files_of(&ctx.paths.instance_dir("pack").unwrap());
            let error = service.delete("pack").unwrap_err();
            assert!(error.to_string().contains("locked"));
            assert_eq!(files_of(&ctx.paths.instance_dir("pack").unwrap()), before);
            assert!(service.list_recently_deleted().unwrap().is_empty());
            assert_eq!(service.list().unwrap().len(), 1);
            let _ = std::fs::remove_dir_all(root);
        }

        #[test]
        fn instance_held_by_another_operation_is_refused_and_untouched() {
            let (ctx, root) = context();
            seed(&ctx, "pack");
            let _held = ctx
                .lock_manager
                .acquire(
                    crate::lock_manager::LockResource::Instance("pack".into()),
                    "launch",
                )
                .unwrap();
            let service = InstanceService::new(ctx.clone());
            assert!(service.delete("pack").is_err());
            assert!(ctx
                .paths
                .instance_dir("pack")
                .unwrap()
                .join("mods/a.jar")
                .is_file());
            assert_eq!(service.list().unwrap().len(), 1);
            assert!(service.list_recently_deleted().unwrap().is_empty());
            let _ = std::fs::remove_dir_all(root);
        }

        #[test]
        fn delete_fails_closed_when_recently_deleted_cannot_be_used() {
            let (ctx, root) = context();
            seed(&ctx, "pack");
            // A file where the holding area should be makes every move fail.
            std::fs::write(ctx.paths.recently_deleted_root(), b"in the way").unwrap();
            let service = InstanceService::new(ctx.clone());
            let before = db_state(&ctx, "pack");
            assert!(service.delete("pack").is_err());
            assert!(ctx
                .paths
                .instance_dir("pack")
                .unwrap()
                .join("mods/a.jar")
                .is_file());
            assert_eq!(db_state(&ctx, "pack"), before);
            assert_eq!(service.list().unwrap().len(), 1);
            let _ = std::fs::remove_dir_all(root);
        }

        #[test]
        fn restore_rejects_unknown_and_hostile_entry_ids() {
            let (ctx, root) = context();
            seed(&ctx, "pack");
            let service = InstanceService::new(ctx.clone());
            service.delete("pack").unwrap();
            for bad in ["", "..", "../instances", "nope", "a/b", "a\\b"] {
                assert!(service.restore_deleted(bad).is_err(), "{bad:?}");
                assert!(
                    service.purge_deleted(bad, &failing_trash()).is_err(),
                    "{bad:?}"
                );
            }
            assert_eq!(service.list_recently_deleted().unwrap().len(), 1);
            let _ = std::fs::remove_dir_all(root);
        }

        #[test]
        fn corrupt_entry_is_not_listed_and_not_restored() {
            let (ctx, root) = context();
            seed(&ctx, "pack");
            let service = InstanceService::new(ctx.clone());
            service.delete("pack").unwrap();
            let entry = service.list_recently_deleted().unwrap().remove(0);
            let json = ctx
                .paths
                .recently_deleted_root()
                .join(&entry.entry_id)
                .join("entry.json");
            std::fs::write(&json, b"{ not json").unwrap();
            assert!(service.list_recently_deleted().unwrap().is_empty());
            assert!(service.restore_deleted(&entry.entry_id).is_err());
            // Cleanup must not destroy what it cannot read.
            let (trash, seen) = fake_trash(root.join("bin"));
            service.cleanup_recently_deleted(&trash);
            assert!(seen.lock().unwrap().is_empty());
            assert!(ctx
                .paths
                .recently_deleted_root()
                .join(&entry.entry_id)
                .join("instance")
                .is_dir());
            let _ = std::fs::remove_dir_all(root);
        }

        #[test]
        fn clone_of_an_import_keeps_its_source_badge() {
            let (ctx, root) = context();
            seed(&ctx, "pack");
            let service = InstanceService::new(ctx.clone());
            let rt = tokio::runtime::Runtime::new().unwrap();
            let clone = rt
                .block_on(service.clone(CloneRequest {
                    source_instance_id: "pack".into(),
                    new_name: "Copy".into(),
                    prefs: ClonePrefs::default(),
                }))
                .unwrap();
            let row = service
                .list()
                .unwrap()
                .into_iter()
                .find(|r| r.instance_id == clone.instance_id)
                .unwrap();
            assert_eq!(row.import_source.as_deref(), Some("curseforge"));
            // Re-importing the original must never resolve to the clone.
            let conn = crate::db::local_state_connection(&ctx.paths.local_state_db()).unwrap();
            let found = crate::db::find_instance_import_by_source(
                &conn,
                "curseforge",
                "cf:test",
                "src-key",
            )
            .unwrap()
            .unwrap();
            assert_eq!(found.instance_id, "pack");
            let _ = std::fs::remove_dir_all(root);
        }

        /// review: rows keyed by instance_id but not reached by a foreign key
        /// (mcp_approval_grants, crash_events) are neither captured nor deleted,
        /// so a deleted instance's rows stay behind and attach to whatever
        /// instance later takes its id.
        #[test]
        fn review_rows_outside_the_cascade_do_not_follow_a_reused_id() {
            let (ctx, root) = context();
            seed(&ctx, "pack");
            {
                let conn = crate::db::local_state_connection(&ctx.paths.local_state_db()).unwrap();
                conn.execute(
                    "INSERT INTO mcp_approval_grants (tool_name, instance_id, state, granted_at)
                     VALUES ('tool.x', 'pack', 'granted', '2026-01-01T00:00:00Z')",
                    [],
                )
                .unwrap();
                conn.execute(
                    "INSERT INTO crash_events (instance_id, fingerprint, exception_class, top_frames_json, occurred_at)
                     VALUES ('pack', 'fp', 'E', '[]', '2026-01-01T00:00:00Z')",
                    [],
                )
                .unwrap();
            }
            let service = InstanceService::new(ctx.clone());
            service.delete("pack").unwrap();
            // A brand-new instance takes the id.
            seed(&ctx, "pack");
            let conn = crate::db::local_state_connection(&ctx.paths.local_state_db()).unwrap();
            for table in ["mcp_approval_grants", "crash_events"] {
                let leaked: i64 = conn
                    .query_row(
                        &format!("SELECT COUNT(*) FROM {table} WHERE instance_id = 'pack'"),
                        [],
                        |row| row.get(0),
                    )
                    .unwrap();
                assert_eq!(
                    leaked, 0,
                    "{table} rows of the deleted 'pack' now belong to the new 'pack'"
                );
            }
            let _ = std::fs::remove_dir_all(root);
        }

        /// review: a failed "Delete now" must leave the entry exactly as it was.
        /// Today detach_links removes the instance's links before the trash
        /// call, so a trash failure still costs the user their links.
        #[test]
        fn review_failed_delete_now_keeps_the_instances_links() {
            let (ctx, root) = context();
            seed(&ctx, "linked");
            let outside = root.join("outside");
            std::fs::create_dir_all(&outside).unwrap();
            std::fs::write(outside.join("precious.txt"), b"keep me").unwrap();
            make_link(
                &outside,
                &ctx.paths.instance_dir("linked").unwrap().join("shared"),
            );
            let service = InstanceService::new(ctx.clone());
            service.delete("linked").unwrap();
            let entry = service.list_recently_deleted().unwrap().remove(0);
            let link = ctx
                .paths
                .recently_deleted_root()
                .join(&entry.entry_id)
                .join("instance")
                .join("shared");

            assert!(service
                .purge_deleted(&entry.entry_id, &failing_trash())
                .is_err());
            assert!(
                std::fs::symlink_metadata(&link).is_ok(),
                "a failed Delete now removed the instance's link"
            );
            assert_eq!(
                std::fs::read(outside.join("precious.txt")).unwrap(),
                b"keep me"
            );
            let _ = std::fs::remove_dir_all(root);
        }

        /// review: if the clone's provenance write fails, the clone must not
        /// quietly succeed without its badge (either failing or the badge is fine).
        #[test]
        fn review_clone_provenance_write_failure_is_not_silent() {
            let (ctx, root) = context();
            seed(&ctx, "pack");
            {
                let conn = crate::db::local_state_connection(&ctx.paths.local_state_db()).unwrap();
                conn.execute_batch(
                    "CREATE TRIGGER review_block_clone_import BEFORE INSERT ON instance_imports
                     WHEN NEW.installation_key = 'clone'
                     BEGIN SELECT RAISE(ABORT, 'blocked for review'); END;",
                )
                .unwrap();
            }
            let service = InstanceService::new(ctx.clone());
            let rt = tokio::runtime::Runtime::new().unwrap();
            let result = rt.block_on(service.clone(CloneRequest {
                source_instance_id: "pack".into(),
                new_name: "Copy".into(),
                prefs: ClonePrefs::default(),
            }));
            if let Ok(clone) = result {
                let row = service
                    .list()
                    .unwrap()
                    .into_iter()
                    .find(|r| r.instance_id == clone.instance_id)
                    .unwrap();
                assert!(
                    row.import_source.is_some(),
                    "clone succeeded without its import badge and nothing reported it"
                );
            }
            let _ = std::fs::remove_dir_all(root);
        }

        /// Every table carrying an instance_id is captured. When a new one is
        /// added this fails until someone decides how it behaves on delete
        /// (and, if it holds permissions, adds it to PERMISSION_TABLES).
        #[test]
        fn every_instance_id_table_is_covered() {
            let (ctx, root) = context();
            let conn = crate::db::local_state_connection(&ctx.paths.local_state_db()).unwrap();
            let mut covered: Vec<String> = crate::recently_deleted::instance_tables(&conn)
                .unwrap()
                .into_iter()
                .map(|s| s.table)
                .collect();
            covered.sort();
            let mut with_column: Vec<String> = Vec::new();
            let mut stmt = conn
                .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'")
                .unwrap();
            let names: Vec<String> = stmt
                .query_map([], |r| r.get(0))
                .unwrap()
                .map(Result::unwrap)
                .collect();
            for name in names {
                let mut info = conn
                    .prepare(&format!("PRAGMA table_info(\"{name}\")"))
                    .unwrap();
                let has = info
                    .query_map([], |r| r.get::<_, String>(1))
                    .unwrap()
                    .any(|c| c.unwrap() == "instance_id");
                if has {
                    with_column.push(name);
                }
            }
            for t in &with_column {
                assert!(
                    covered.contains(t),
                    "{t} has instance_id but is not captured"
                );
            }
            // Reviewed list: a new table must be added here on purpose.
            let mut reviewed: Vec<&str> = vec![
                "crash_events",
                "crash_survivals",
                "instance_import_files",
                "instance_import_jobs",
                "instance_imports",
                "instance_pack_files",
                "instance_pack_gaps",
                "instance_update_cache",
                "launch_history",
                "mcp_approval_grants",
                "plugin_storage",
                "user_instances",
            ];
            reviewed.sort();
            let mut direct = with_column.clone();
            direct.sort();
            assert_eq!(
                direct, reviewed,
                "a table with an instance_id column was added or removed: review its delete/restore behaviour"
            );
            let _ = std::fs::remove_dir_all(root);
        }

        #[test]
        fn permission_rows_do_not_carry_to_a_renamed_restore_but_do_to_the_same_id() {
            let (ctx, root) = context();
            seed(&ctx, "pack");
            {
                let conn = crate::db::local_state_connection(&ctx.paths.local_state_db()).unwrap();
                conn.execute(
                    "INSERT INTO mcp_approval_grants (tool_name, instance_id, state, granted_at)
                     VALUES ('tool.x', 'pack', 'granted', '2026-01-01T00:00:00Z')",
                    [],
                )
                .unwrap();
                // A child table without instance_id, reached through a foreign key.
                conn.execute(
                    "INSERT INTO crash_survivals (instance_id, occurred_at) VALUES ('pack', 't')",
                    [],
                )
                .unwrap();
                conn.execute(
                    "INSERT INTO crash_survival_mods (survival_id, mod_id) VALUES (last_insert_rowid(), 'm')",
                    [],
                )
                .unwrap();
            }
            let service = InstanceService::new(ctx.clone());
            service.delete("pack").unwrap();
            let conn = crate::db::local_state_connection(&ctx.paths.local_state_db()).unwrap();
            let count = |sql: &str| -> i64 { conn.query_row(sql, [], |r| r.get(0)).unwrap() };
            assert_eq!(count("SELECT COUNT(*) FROM crash_survival_mods"), 0);
            let entry = service.list_recently_deleted().unwrap().remove(0);

            // Same id: everything comes back, grants and linked rows included.
            service.restore_deleted(&entry.entry_id).unwrap();
            assert_eq!(
                count("SELECT COUNT(*) FROM mcp_approval_grants WHERE instance_id='pack'"),
                1
            );
            assert_eq!(count("SELECT COUNT(*) FROM crash_survival_mods"), 1);

            // Renamed restore: no grant for the new id.
            service.delete("pack").unwrap();
            seed(&ctx, "pack");
            let entry = service.list_recently_deleted().unwrap().remove(0);
            let outcome = service.restore_deleted(&entry.entry_id).unwrap();
            assert!(outcome.renamed);
            assert_eq!(count("SELECT COUNT(*) FROM mcp_approval_grants"), 0);
            let _ = std::fs::remove_dir_all(root);
        }

        #[test]
        fn reconcile_finishes_a_delete_that_crashed_before_the_rows_went() {
            let (ctx, root) = context();
            seed(&ctx, "pack");
            let before = {
                let conn = crate::db::local_state_connection(&ctx.paths.local_state_db()).unwrap();
                crate::recently_deleted::capture_db_state(&conn, "pack").unwrap()
            };
            let service = InstanceService::new(ctx.clone());
            service.delete("pack").unwrap();
            // Crash simulation: the folder is in the entry but the rows are back.
            {
                let mut conn =
                    crate::db::local_state_connection(&ctx.paths.local_state_db()).unwrap();
                crate::recently_deleted::replay_db_state(&mut conn, &before, "pack", "pack", None)
                    .unwrap();
            }
            assert_eq!(service.list().unwrap().len(), 1);
            let (trash, _) = fake_trash(root.join("bin"));
            service.cleanup_recently_deleted(&trash);
            assert!(service.list().unwrap().is_empty());
            let entry = service.list_recently_deleted().unwrap().remove(0);
            service.restore_deleted(&entry.entry_id).unwrap();
            assert!(ctx
                .paths
                .instance_dir("pack")
                .unwrap()
                .join("mods/a.jar")
                .is_file());
            let _ = std::fs::remove_dir_all(root);
        }

        #[test]
        fn reconcile_leaves_a_different_instance_with_a_reused_id_alone() {
            let (ctx, root) = context();
            seed(&ctx, "pack");
            let service = InstanceService::new(ctx.clone());
            service.delete("pack").unwrap();
            // A new instance (different created_at) took the id, then lost its folder.
            seed(&ctx, "pack");
            {
                let conn = crate::db::local_state_connection(&ctx.paths.local_state_db()).unwrap();
                conn.execute(
                    "UPDATE user_instances SET created_at = 'other' WHERE instance_id='pack'",
                    [],
                )
                .unwrap();
            }
            std::fs::remove_dir_all(ctx.paths.instance_dir("pack").unwrap()).unwrap();
            let (trash, _) = fake_trash(root.join("bin"));
            service.cleanup_recently_deleted(&trash);
            assert_eq!(service.list().unwrap().len(), 1);
            let _ = std::fs::remove_dir_all(root);
        }

        #[test]
        fn reconcile_finishes_a_restore_that_crashed_before_the_rows_came_back() {
            let (ctx, root) = context();
            seed(&ctx, "pack");
            let service = InstanceService::new(ctx.clone());
            service.delete("pack").unwrap();
            let entry = service.list_recently_deleted().unwrap().remove(0);
            // Crash simulation: folder moved back, rows not restored.
            std::fs::rename(
                ctx.paths
                    .recently_deleted_root()
                    .join(&entry.entry_id)
                    .join("instance"),
                ctx.paths.instance_dir("pack").unwrap(),
            )
            .unwrap();
            assert!(service.list().unwrap().is_empty());
            let (trash, _) = fake_trash(root.join("bin"));
            service.cleanup_recently_deleted(&trash);
            assert_eq!(service.list().unwrap().len(), 1);
            assert!(service.list_recently_deleted().unwrap().is_empty());
            let _ = std::fs::remove_dir_all(root);
        }

        #[test]
        fn hostile_entry_contents_are_refused_on_restore() {
            let (ctx, root) = context();
            seed(&ctx, "pack");
            let service = InstanceService::new(ctx.clone());
            service.delete("pack").unwrap();
            let entry = service.list_recently_deleted().unwrap().remove(0);
            let json_path = ctx
                .paths
                .recently_deleted_root()
                .join(&entry.entry_id)
                .join("entry.json");
            let original = std::fs::read_to_string(&json_path).unwrap();
            let tamper = |f: &dyn Fn(&mut serde_json::Value)| {
                let mut v: serde_json::Value = serde_json::from_str(&original).unwrap();
                f(&mut v);
                std::fs::write(&json_path, serde_json::to_vec(&v).unwrap()).unwrap();
            };
            // Unknown table.
            tamper(&|v| v["db"][0]["table"] = "sqlite_master".into());
            assert!(service.restore_deleted(&entry.entry_id).is_err());
            // Unknown column.
            tamper(&|v| {
                v["db"][0]["columns"][0] = "evil\" TEXT); DROP TABLE user_instances; --".into()
            });
            assert!(service.restore_deleted(&entry.entry_id).is_err());
            // Rows for another instance.
            tamper(&|v| {
                let t = v["db"]
                    .as_array_mut()
                    .unwrap()
                    .iter_mut()
                    .find(|d| d["table"] == "launch_history")
                    .unwrap();
                let col = t["columns"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .position(|c| c == "instance_id")
                    .unwrap();
                t["rows"][0][col] = "victim".into();
            });
            assert!(service.restore_deleted(&entry.entry_id).is_err());
            // Instance id that escapes the instances folder.
            tamper(&|v| v["instance_id"] = "..\\..\\x".into());
            assert!(service.restore_deleted(&entry.entry_id).is_err());
            tamper(&|v| v["instance_id"] = "pack:stream".into());
            assert!(service.restore_deleted(&entry.entry_id).is_err());
            // Nothing was written anywhere.
            assert!(service.list().unwrap().is_empty());
            assert!(!ctx.paths.instance_dir("pack").unwrap().exists());
            // The untampered entry still restores.
            std::fs::write(&json_path, &original).unwrap();
            service.restore_deleted(&entry.entry_id).unwrap();
            let _ = std::fs::remove_dir_all(root);
        }

        #[test]
        fn instance_ids_with_a_colon_are_refused() {
            assert!(crate::app_paths::validate_path_component("pack:stream").is_err());
            assert!(crate::app_paths::validate_path_component("C:pack").is_err());
            assert!(crate::app_paths::validate_path_component("pack").is_ok());
        }
    }
}

/// The name a player sees for a loader id. The id itself is what the manifest
/// and the loader catalog key on, so only the label changes.
fn loader_display_name(loader: &str) -> &str {
    match loader {
        "fabric" => "Fabric",
        "quilt" => "Quilt",
        "forge" => "Forge",
        "neoforge" => "NeoForge",
        other => other,
    }
}

#[cfg(test)]
mod loader_label_tests {
    use super::loader_display_name;

    #[test]
    fn known_loaders_get_player_facing_names() {
        assert_eq!(loader_display_name("fabric"), "Fabric");
        assert_eq!(loader_display_name("neoforge"), "NeoForge");
    }

    #[test]
    fn unknown_or_hostile_ids_pass_through_unchanged() {
        assert_eq!(loader_display_name(""), "");
        assert_eq!(loader_display_name("Fabric"), "Fabric");
        assert_eq!(loader_display_name("fabric "), "fabric ");
    }
}
