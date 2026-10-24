use crate::ctx::Ctx;
use crate::error::{LauncherError, LauncherResult};
use crate::event_sink::{
    CancellationToken, CoreEvent, OperationId, ProgressEvent, ProgressPhase, ProgressSink,
};
use crate::health::{self, HealthScore};
use crate::import::{DetectedLauncher, ImportResult};
use crate::loader_service::LoaderService;
use crate::models::InstanceRow;
use crate::network::NetworkPolicy;
use crate::operation_manager::OpHandle;
use crate::pack_install::PackInstallResult;
use std::path::PathBuf;
use std::sync::Arc;

/// Typed import source.
#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant)]
pub enum ImportSource {
    /// A `.mrpack` (Modrinth modpack) file.
    ///
    /// `origin_url` is the presigned-aware URL the file was downloaded from
    /// when the import came from `run_mrpack_url`. Query and fragment are
    /// stripped before storage so manifests never carry tokens.
    Mrpack {
        path: PathBuf,
        origin_url: Option<String>,
        /// Leave out files that cannot be fetched instead of stopping (the
        /// user chose "Import anyway" in the review).
        skip_unfetchable: bool,
    },
    /// A PrismLauncher / MultiMC instance `.zip`.
    PrismZip(PathBuf),
    /// An existing instance directory to copy.
    Directory(PathBuf),
    /// A pack manifest (Tier 1 or Tier 2 JSON) installed into an existing instance.
    PackManifest {
        /// Raw JSON string of the pack manifest.
        manifest_json: String,
        /// Target instance ID to install into.
        target_instance_id: String,
    },
    /// A resolved Technic Solder pack (Tier S). Downloads each mod with MD5
    /// verification (when reported) through the consented-content policy.
    TechnicSolder(crate::import::TechnicSolderPack),
    /// A consented Technic zip archive (Tier Z, or Tier C when SHA-256-pinned).
    TechnicZip(crate::import::TechnicZipPack),
    /// A content provider's pack plan, already validated and authorized by
    /// `crate::providers::install`. Each file is downloaded under the
    /// provider's declared scope and checked against its published digests.
    ProviderPack(crate::import::ProviderPackImport),
}

impl ImportSource {
    /// Convenience for a local `.mrpack` file with no remote origin.
    pub fn mrpack(path: PathBuf) -> Self {
        Self::Mrpack {
            path,
            origin_url: None,
            skip_unfetchable: false,
        }
    }
}

/// Strip query and fragment from a URL so a manifest never carries a
/// presigned token. Falls back to truncating at `?`/`#` if the URL does
/// not parse.
pub(crate) fn strip_url_query(url: &str) -> String {
    if let Ok(mut parsed) = reqwest::Url::parse(url) {
        parsed.set_query(None);
        parsed.set_fragment(None);
        parsed.to_string()
    } else {
        url.split('?')
            .next()
            .unwrap_or(url)
            .split('#')
            .next()
            .unwrap_or(url)
            .to_string()
    }
}

/// SHA-256 (lowercase hex) and size of a file, read in chunks.
fn file_sha256_and_size(path: &std::path::Path) -> LauncherResult<(String, u64)> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let unreadable = |e: std::io::Error| LauncherError::Generic {
        code: "ERR_PACK_READ".into(),
        message: format!("Cannot read pack file {}: {e}", path.display()),
    };
    let mut file = std::fs::File::open(path).map_err(unreadable)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 256 * 1024];
    let mut size = 0u64;
    loop {
        let n = file.read(&mut buf).map_err(unreadable)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        size += n as u64;
    }
    Ok((hex::encode(hasher.finalize()), size))
}

/// The `launcher_kind` recorded for an instance imported from a local pack
/// file. The file name rides along after the colon so the instance list can
/// say "Imported from <file>"; it never matches a launcher re-import.
pub fn mrpack_origin_kind(file_name: &str) -> String {
    format!("mrpack:{file_name}")
}

/// Import configuration passed to [`ImportService::run_import`].
#[derive(Debug, Clone)]
pub struct ImportRequest {
    pub source: ImportSource,
    pub symlink_saves: bool,
}

/// What [`ImportService::preview_import_name`] reports for an import source.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ImportNamePreview {
    pub default_name: String,
    pub name_taken: bool,
    pub suggested_name: String,
}

/// Core-owned import service.
///
/// Wraps the free functions in [`crate::import`] behind a typed API that
/// uses [`Ctx`] / [`crate::app_paths::AppPaths`] for path resolution and
/// preserves all existing path/hash/collision protections.
#[derive(Clone)]
pub struct ImportService {
    ctx: Ctx,
}

impl ImportService {
    pub fn new(ctx: Ctx) -> Self {
        Self { ctx }
    }

    /// Download and import a Modrinth pack URL using the same transactional
    /// import path as a local `.mrpack` file.
    pub async fn run_mrpack_url(&self, download_url: &str) -> LauncherResult<ImportResult> {
        self.run_mrpack_url_with_sink(
            download_url,
            self.ctx.progress_sink.clone(),
            CancellationToken::new(),
        )
        .await
    }

    /// Download and import a Modrinth pack URL while forwarding progress to a
    /// host-provided sink. The URL download and the archive import use distinct
    /// operation IDs because the archive import registers its own cancellable
    /// operation after the outer archive has been written.
    pub async fn run_mrpack_url_with_sink(
        &self,
        download_url: &str,
        sink: Arc<dyn ProgressSink>,
        cancel: CancellationToken,
    ) -> LauncherResult<ImportResult> {
        let download_operation =
            OperationId::new(format!("pack-download-{}", uuid::Uuid::new_v4()));
        sink.report(ProgressEvent::new(
            download_operation.clone(),
            ProgressPhase::Downloading,
            "Downloading modpack archive…",
        ));
        let progress_sink = sink.clone();
        let progress_operation = download_operation.clone();
        let bytes = crate::download::download_modpack_bytes_with_progress(
            &self.ctx.http_clients,
            download_url,
            move |downloaded, total| {
                let mut event = ProgressEvent::new(
                    progress_operation.clone(),
                    ProgressPhase::Downloading,
                    "Downloading modpack archive…",
                )
                .with_bytes(downloaded, total.unwrap_or(0));
                if let Some(total) = total.filter(|total| *total > 0) {
                    event = event.with_progress((downloaded as f64 / total as f64).min(1.0));
                }
                progress_sink.report(event);
            },
        )
        .await?;
        let temp_path = self
            .ctx
            .paths
            .staging_root()
            .join(format!("pack-download-{}.mrpack", uuid::Uuid::new_v4()));
        tokio::fs::write(&temp_path, bytes)
            .await
            .map_err(|e| LauncherError::Generic {
                code: "ERR_FILE_WRITE".into(),
                message: format!("Failed to write temporary pack archive: {e}"),
            })?;
        let stripped = strip_url_query(download_url);
        let result = self
            .run_import_with_sink(
                ImportRequest {
                    source: ImportSource::Mrpack {
                        path: temp_path.clone(),
                        origin_url: Some(stripped),
                        skip_unfetchable: false,
                    },
                    symlink_saves: false,
                },
                sink,
                cancel,
            )
            .await;
        let _ = tokio::fs::remove_file(temp_path).await;
        result
    }

    /// Run an import with progress reporting and cancellation support.
    ///
    /// The operation emits progress events tagged with the operation ID for
    /// phases: validation, extraction, metadata, loader, health, registration,
    /// snapshot, and completion.  Cancellation is checked before and after
    /// each phase and cooperatively during archive extraction (token is
    /// cloned into the blocking task).
    ///
    /// The external `cancel` token and the [`OperationManager`] token are
    /// kept coherent: cancelling either one causes the next checkpoint to
    /// return `ERR_OPERATION_CANCELLED` and marks the operation as cancelled.
    ///
    /// Atomic operations (filesystem rename / DB commit) are NOT interrupted,
    /// ensuring no partially-promoted import can occur.
    pub async fn run_import_with_sink(
        &self,
        request: ImportRequest,
        sink: Arc<dyn crate::event_sink::ProgressSink>,
        cancel: CancellationToken,
    ) -> LauncherResult<ImportResult> {
        self.run_import_named(request, None, sink, cancel).await
    }

    /// Inspect a `.mrpack`, Prism zip or instance folder before importing: the
    /// name it would get, whether that name is already taken, and a free
    /// "<name> (2)" style suggestion.  Never touches existing instances.
    pub fn preview_import_name(
        &self,
        source: &std::path::Path,
    ) -> LauncherResult<ImportNamePreview> {
        let default_name = crate::import::default_import_name(source)?;
        self.preview_name(&default_name)
    }

    /// Whether an instance already has `name`: its folder exists, or an
    /// instance the user can see carries the same name (case ignored). The
    /// folder alone misses an instance whose id differs from its name, such
    /// as one that was renamed.
    fn name_in_use(&self, name: &str, existing_names: &[String]) -> bool {
        let wanted = name.trim().to_lowercase();
        existing_names
            .iter()
            .any(|existing| existing.trim().to_lowercase() == wanted)
            || crate::import::import_name_taken(&self.ctx.paths.instances_root(), name)
                .unwrap_or(false)
    }

    fn existing_instance_names(&self) -> Vec<String> {
        crate::db::local_state_connection(&self.ctx.paths.local_state_db())
            .ok()
            .and_then(|conn| crate::db::list_instances(&conn).ok())
            .map(|rows| rows.into_iter().map(|row| row.name).collect())
            .unwrap_or_default()
    }

    /// The same name check for a pack that has no file to inspect (a Technic
    /// or provider pack): its display name, whether that is taken, and a free
    /// "<name> (2)" style suggestion.
    pub fn preview_name(&self, default_name: &str) -> LauncherResult<ImportNamePreview> {
        let names = self.existing_instance_names();
        // A name that cannot become a folder is an error here, as it would be
        // at import time.
        crate::import::import_name_taken(&self.ctx.paths.instances_root(), default_name)?;
        let name_taken = self.name_in_use(default_name, &names);
        let suggested_name = crate::import::next_free_name(default_name, |candidate| {
            self.name_in_use(candidate, &names)
        });
        Ok(ImportNamePreview {
            default_name: default_name.to_string(),
            name_taken,
            suggested_name,
        })
    }

    /// What importing a `.mrpack` would do, read from the file alone: the
    /// mods and where each comes from, what is left out, and anything that
    /// would stop the import. Writes nothing and downloads nothing. The plan
    /// carries the file's fingerprint, which [`Self::take_reviewed_copy`]
    /// holds the import to.
    pub fn plan_mrpack_import(
        &self,
        source: &std::path::Path,
    ) -> LauncherResult<crate::agora_pack_import::PackImportPlan> {
        let policy = crate::override_sanitizer::OverridePolicy::from_settings(&self.ctx);
        // One copy is hashed, planned and (later) imported, so what the user
        // reviewed is exactly what is imported even if the original changes.
        let (staged, sha256, size) = self.stage_for_review(source)?;
        let mut plan = crate::import::plan_mrpack(&staged, policy)?;
        plan.file_sha256 = sha256;
        plan.file_size = size;
        let check = self.preview_name(&plan.pack_name)?;
        plan.name_taken = check.name_taken;
        plan.suggested_name = check.suggested_name;
        Ok(plan)
    }

    fn review_root(&self) -> PathBuf {
        self.ctx.paths.staging_root().join("pack-review")
    }

    /// A fingerprint is 64 hex digits; anything else never names a folder.
    fn review_dir(&self, sha256: &str) -> LauncherResult<PathBuf> {
        let sha = sha256.trim().to_ascii_lowercase();
        if sha.len() != 64 || !sha.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(LauncherError::Generic {
                code: "ERR_PACK_NOT_REVIEWED".into(),
                message: "Review the pack before importing it.".into(),
            });
        }
        Ok(self.review_root().join(sha))
    }

    /// Copies `source` once into a staging folder named for its SHA-256,
    /// hashing as it copies. Review and import both use this copy, so a file
    /// swapped after review is never imported. Returns (staged path, sha256, size).
    fn stage_for_review(&self, source: &std::path::Path) -> LauncherResult<(PathBuf, String, u64)> {
        use sha2::{Digest, Sha256};
        use std::io::{Read, Write};
        let io_err = |e: std::io::Error| LauncherError::Generic {
            code: "ERR_PACK_READ".into(),
            message: format!("Cannot read pack file {}: {e}", source.display()),
        };
        let root = self.review_root();
        std::fs::create_dir_all(&root).map_err(io_err)?;
        // Copies nobody imported or cancelled (a crash, say) do not pile up.
        if let Ok(read) = std::fs::read_dir(&root) {
            for entry in read.flatten() {
                let old = entry
                    .metadata()
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|t| t.elapsed().ok())
                    .is_some_and(|age| age > std::time::Duration::from_secs(24 * 3600));
                if old {
                    let _ = std::fs::remove_dir_all(entry.path());
                }
            }
        }
        let file_name = source.file_name().ok_or_else(|| LauncherError::Generic {
            code: "ERR_PACK_READ".into(),
            message: "The pack path has no file name.".into(),
        })?;
        let tmp_dir = root.join(format!(".tmp-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&tmp_dir).map_err(io_err)?;
        let copy = (|| -> std::io::Result<(String, u64)> {
            let mut input = std::fs::File::open(source)?;
            let mut output = std::fs::File::create(tmp_dir.join(file_name))?;
            let mut hasher = Sha256::new();
            let mut buf = vec![0u8; 256 * 1024];
            let mut size = 0u64;
            loop {
                let n = input.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                hasher.update(&buf[..n]);
                output.write_all(&buf[..n])?;
                size += n as u64;
            }
            output.flush()?;
            Ok((hex::encode(hasher.finalize()), size))
        })();
        let (sha, size) = match copy {
            Ok(done) => done,
            Err(e) => {
                let _ = std::fs::remove_dir_all(&tmp_dir);
                return Err(io_err(e));
            }
        };
        let final_dir = root.join(&sha);
        if final_dir.exists() {
            // Reviewed before: keep that copy (it hashes to the same value).
            let _ = std::fs::remove_dir_all(&tmp_dir);
        } else if let Err(e) = std::fs::rename(&tmp_dir, &final_dir) {
            let _ = std::fs::remove_dir_all(&tmp_dir);
            return Err(io_err(e));
        }
        Ok((final_dir.join(file_name), sha, size))
    }

    /// The staged copy the review recorded, re-checked against the reviewed
    /// fingerprint. Refuses when there was no review, or the copy is gone or
    /// no longer matches.
    pub fn take_reviewed_copy(&self, reviewed_sha256: Option<&str>) -> LauncherResult<PathBuf> {
        let Some(reviewed) = reviewed_sha256 else {
            return Err(LauncherError::Generic {
                code: "ERR_PACK_NOT_REVIEWED".into(),
                message: "Review the pack before importing it.".into(),
            });
        };
        let dir = self.review_dir(reviewed)?;
        let gone = || LauncherError::Generic {
            code: "ERR_PACK_NOT_REVIEWED".into(),
            message: "The reviewed copy of the pack is gone. Review it again.".into(),
        };
        let file = std::fs::read_dir(&dir)
            .map_err(|_| gone())?
            .flatten()
            .find(|entry| entry.path().is_file())
            .map(|entry| entry.path())
            .ok_or_else(gone)?;
        let (actual, _) = file_sha256_and_size(&file)?;
        if !actual.eq_ignore_ascii_case(reviewed.trim()) {
            return Err(LauncherError::Generic {
                code: "ERR_PACK_CHANGED".into(),
                message: "The reviewed copy of the pack changed. Review it again.".into(),
            });
        }
        Ok(file)
    }

    /// Removes the staged copy of a reviewed pack (after the import, or when
    /// the review is cancelled). Nothing to remove is fine.
    pub fn discard_reviewed_copy(&self, sha256: &str) {
        if let Ok(dir) = self.review_dir(sha256) {
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    /// [`ImportService::run_import_with_sink`] with an optional user-chosen
    /// instance name, honoured for `.mrpack`, Prism zip, directory, Technic
    /// and provider pack imports.
    /// A name that collides with an existing instance still fails; the
    /// existing instance is never overwritten.
    pub async fn run_import_named(
        &self,
        request: ImportRequest,
        name_override: Option<String>,
        sink: Arc<dyn crate::event_sink::ProgressSink>,
        cancel: CancellationToken,
    ) -> LauncherResult<ImportResult> {
        let op = self.ctx.operation_manager.register("Import instance");
        let op_id = op.id().clone();
        let is_modpack = matches!(
            request.source,
            ImportSource::Mrpack { .. }
                | ImportSource::TechnicSolder(_)
                | ImportSource::TechnicZip(_)
                | ImportSource::ProviderPack(_)
        );

        // Check cancellation, keeping external and operation-manager tokens
        // coherent.  If the external token has been fired we also cancel the
        // op handle so the manager reflects the cancelled state.
        let check = |op: &OpHandle, cancel: &CancellationToken| -> LauncherResult<()> {
            if cancel.is_cancelled() {
                op.cancel();
            }
            if op.token().is_cancelled() {
                return Err(LauncherError::Generic {
                    code: "ERR_OPERATION_CANCELLED".into(),
                    message: "Import was cancelled.".into(),
                });
            }
            Ok(())
        };

        // ── Phase 1: Validation / network policy ────────────────────────
        sink.report(ProgressEvent::new(
            op_id.clone(),
            ProgressPhase::Resolving,
            "Resolving network policy…",
        ));
        check(&op, &cancel)?;
        let policy = NetworkPolicy::from_ctx(&self.ctx)?;
        check(&op, &cancel)?;

        // ── Phase 2: Archive / filesystem extraction ─────────────────────
        let extracting_msg = match &request.source {
            ImportSource::Mrpack { path, .. } => format!("Extracting mrpack: {}", path.display()),
            ImportSource::PrismZip(p) => format!("Extracting Prism zip: {}", p.display()),
            ImportSource::Directory(d) => format!("Copying directory: {}", d.display()),
            ImportSource::PackManifest { .. } => "Preparing pack manifest…".into(),
            ImportSource::TechnicSolder(pack) => {
                format!(
                    "Installing Technic pack '{}', downloading {} mods…",
                    pack.display_name,
                    pack.mods.len()
                )
            }
            ImportSource::TechnicZip(pack) => {
                format!("Installing Technic pack '{}'…", pack.display_name)
            }
            ImportSource::ProviderPack(pack) => format!(
                "Installing '{}' from {}, downloading {} files…",
                pack.plan.name,
                pack.provider_title,
                pack.plan.files.len()
            ),
        };
        sink.report(ProgressEvent::new(
            op_id.clone(),
            ProgressPhase::Extracting,
            &extracting_msg,
        ));
        check(&op, &cancel)?;

        let override_policy = crate::override_sanitizer::OverridePolicy::from_settings(&self.ctx);
        let (blocking_instances_root, blocking_source, blocking_symlink) = (
            self.ctx.paths.instances_root(),
            request.source.clone(),
            request.symlink_saves,
        );
        let blocking_sink = sink.clone();
        let blocking_operation_id = op_id.clone();

        let result = match tokio::task::spawn_blocking(move || match blocking_source {
            ImportSource::Mrpack {
                path,
                origin_url,
                skip_unfetchable,
            } => crate::import::import_mrpack_inner(
                &path,
                &blocking_instances_root,
                blocking_symlink,
                Some(blocking_sink),
                Some(blocking_operation_id),
                origin_url,
                override_policy,
                name_override.as_deref(),
                skip_unfetchable,
            ),
            ImportSource::PrismZip(path) => crate::import::import_prism_zip_named(
                &path,
                &blocking_instances_root,
                blocking_symlink,
                name_override.as_deref(),
            ),
            ImportSource::Directory(path) => crate::import::import_directory_named(
                &path,
                &blocking_instances_root,
                blocking_symlink,
                name_override.as_deref(),
            ),
            ImportSource::PackManifest { .. } => Err(LauncherError::Generic {
                code: "ERR_IMPORT_SOURCE".into(),
                message: "PackManifest imports must use ImportService::install_pack (async)."
                    .into(),
            }),
            ImportSource::TechnicSolder(pack) => crate::import::import_technic_solder_pack(
                &pack,
                &blocking_instances_root,
                name_override.as_deref(),
            ),
            ImportSource::TechnicZip(pack) => crate::import::import_technic_zip_pack(
                &pack,
                &blocking_instances_root,
                override_policy,
                name_override.as_deref(),
            ),
            ImportSource::ProviderPack(pack) => crate::import::import_provider_pack(
                &pack,
                &blocking_instances_root,
                name_override.as_deref(),
            ),
        })
        .await
        {
            Ok(Ok(r)) => r,
            Ok(Err(e)) => {
                op.fail(e.to_string());
                return Err(e);
            }
            Err(join_err) => {
                let msg = format!("The import task panicked or was cancelled: {join_err}");
                op.fail(msg.clone());
                return Err(LauncherError::Generic {
                    code: "ERR_IMPORT_TASK".into(),
                    message: msg,
                });
            }
        };

        check(&op, &cancel)?;

        // Rollback helper — removes the *promoted* instance directory.
        let rollback =
            || -> LauncherResult<()> { remove_promoted(&self.ctx.paths, &result.instance_id) };

        // ── Phase 3: Bootstrap Mojang version metadata ──────────────────
        if !result.minecraft_version.is_empty() {
            sink.report(ProgressEvent::new(
                op_id.clone(),
                ProgressPhase::Verifying,
                "Verifying Minecraft base version metadata…",
            ));
            check(&op, &cancel)?;

            let minecraft_root = self.ctx.paths.minecraft_runtime_root();
            if let Err(e) = crate::minecraft_metadata::ensure_base_version_metadata(
                &minecraft_root,
                &result.minecraft_version,
                &policy,
            )
            .await
            {
                let _ = rollback();
                op.fail(e.to_string());
                return Err(e);
            }
            check(&op, &cancel)?;
        }

        // ── Phase 4: Install loader (if applicable) ─────────────────────
        if !result.loader.is_empty() && result.loader != "vanilla" {
            sink.report(ProgressEvent::new(
                op_id.clone(),
                ProgressPhase::Installing,
                format!("Installing {} {}…", result.loader, result.loader_version),
            ));
            check(&op, &cancel)?;

            if let Err(e) = LoaderService::new(self.ctx.clone())
                .ensure_installed(
                    &result.loader,
                    &result.minecraft_version,
                    &result.loader_version,
                    false,
                )
                .await
            {
                let _ = rollback();
                op.fail(e.to_string());
                return Err(e);
            }
            check(&op, &cancel)?;
        }

        // ── Phase 5: DB registration (atomic — no cancellation check) ──
        let _lock = match self.ctx.lock_manager.acquire(
            crate::lock_manager::LockResource::Instance(result.instance_id.clone()),
            "import-register",
        ) {
            Ok(l) => l,
            Err(e) => {
                let _ = rollback();
                op.fail(e.to_string());
                return Err(e);
            }
        };

        let row = InstanceRow {
            instance_id: result.instance_id.clone(),
            name: result.name.clone(),
            minecraft_version: result.minecraft_version.clone(),
            loader: result.loader.clone(),
            loader_version: result.loader_version.clone(),
            is_modpack,
            is_locked: false,
            last_launched_at: None,
            jvm_memory_mb: 4096,
            jvm_memory_mode: "auto".into(),
            jvm_gc: "auto".into(),
            jvm_custom_args: String::new(),
            jvm_always_pre_touch: crate::models::recommended_java_version_for_minecraft(
                &result.minecraft_version,
            ) < 21,
            created_at: chrono::Utc::now().to_rfc3339(),
            java_path: None,
            java_incompatible_override: false,
            icon_path: None,
            launch_mode_override: "auto".into(),
            import_source: None,
        };

        let conn = match crate::db::local_state_connection(&self.ctx.paths.local_state_db()) {
            Ok(c) => c,
            Err(error) => {
                let _ = rollback();
                op.fail(error.to_string());
                return Err(LauncherError::Generic {
                    code: "ERR_LOCAL_STATE_FAILED".into(),
                    message: format!("Failed to open local state DB: {error}"),
                });
            }
        };

        if let Err(error) = crate::db::upsert_instance(&conn, &row) {
            let _ = rollback();
            op.fail(error.to_string());
            return Err(LauncherError::Generic {
                code: "ERR_LOCAL_STATE_FAILED".into(),
                message: format!("Failed to register imported instance: {error}"),
            });
        }

        if let Err(error) = check(&op, &cancel) {
            let _ = rollback();
            let _ = crate::db::delete_instance(&conn, &result.instance_id);
            return Err(error);
        }

        // A pack imported from a file on disk remembers which file, so the
        // instance list can say where it came from. Cosmetic: a failure to
        // write it must not undo an import that otherwise worked.
        if let ImportSource::Mrpack {
            path,
            origin_url: None,
            ..
        } = &request.source
        {
            if let Some(file_name) = path.file_name().and_then(|n| n.to_str()) {
                let now = chrono::Utc::now().to_rfc3339();
                let record = crate::db::InstanceImportRecord {
                    instance_id: result.instance_id.clone(),
                    launcher_kind: mrpack_origin_kind(file_name),
                    installation_key: "file".into(),
                    source_instance_key: result.instance_id.clone(),
                    source_path: file_name.to_string(),
                    source_metadata_json: "{}".into(),
                    imported_settings_json: "{}".into(),
                    source_fingerprint: String::new(),
                    imported_at: now.clone(),
                    updated_at: now,
                    last_result: "imported".into(),
                };
                let _ = crate::db::replace_instance_import(&conn, &record, &[]);
            }
        }

        // ── Pack file inventory for any pack-like import (M1c).
        // Full per-file list lives in `instance_pack_files`; only the hash
        // lives in the manifest. `import.rs` already stamped the hash for
        // direct `import_mrpack` calls, but the service is the durable DB
        // writer — collect from the promoted dir and persist.
        if matches!(
            request.source,
            ImportSource::Mrpack { .. }
                | ImportSource::PrismZip(_)
                | ImportSource::Directory(_)
                | ImportSource::TechnicSolder(_)
                | ImportSource::TechnicZip(_)
                | ImportSource::ProviderPack(_)
        ) {
            if let Ok(instance_dir) = self.ctx.paths.instance_dir(&result.instance_id) {
                if instance_dir.exists() {
                    if let Ok(files) = crate::pack_inventory::collect_pack_inventory(&instance_dir)
                    {
                        let computed_hash = if files.is_empty() {
                            None
                        } else {
                            Some(crate::pack_inventory::pack_content_hash(&files))
                        };
                        // Best-effort DB write; do not fail import on DB error.
                        let _ = crate::db::replace_instance_pack_files(
                            &conn,
                            &result.instance_id,
                            &files,
                        );
                        // Ensure manifest hash matches computed hash (defense in depth
                        // for older import paths or empty inventory).
                        let manifest_path = instance_dir.join("instance_manifest.json");
                        if let Ok(mut manifest) = crate::helpers::read_manifest(&manifest_path) {
                            let needs_patch = match &manifest.pack_origin {
                                Some(o) => o.pack_content_hash != computed_hash,
                                None => computed_hash.is_some(),
                            };
                            if needs_patch {
                                if let Some(origin) = manifest.pack_origin.as_mut() {
                                    origin.pack_content_hash = computed_hash;
                                    if let Ok(json) = serde_json::to_string_pretty(&manifest) {
                                        let _ = std::fs::write(&manifest_path, json);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // ── Phase 6: Health validation ──────────────────────────────────
        if let Ok(instance_dir) = self.ctx.paths.instance_dir(&result.instance_id) {
            if instance_dir.exists() {
                sink.report(ProgressEvent::new(
                    op_id.clone(),
                    ProgressPhase::HealthScan,
                    "Running health scan…",
                ));
                if let Err(error) = check(&op, &cancel) {
                    let _ = rollback();
                    let _ = crate::db::delete_instance(&conn, &result.instance_id);
                    return Err(error);
                }

                let manifest_path = instance_dir.join("instance_manifest.json");
                {
                    if let Ok(manifest) = crate::helpers::read_manifest(&manifest_path) {
                        // Use the cache-aware scan so the durable report is
                        // published here and the first launch of the imported
                        // instance does not repeat the full JAR parse (11s+
                        // for a large pack). The report is identical to the
                        // uncached scan; publishing only happens when the
                        // scanned state is stable.
                        let report = health::cached_health(
                            &instance_dir,
                            &manifest,
                            Some(&self.ctx.paths.registry_db()),
                            None,
                        );
                        if report.score == HealthScore::Red {
                            let details = report
                                .blockers
                                .iter()
                                .map(|blocker| blocker.message.as_str())
                                .collect::<Vec<_>>()
                                .join("; ");
                            self.ctx.event_sink.emit(CoreEvent::Warning {
                                message: format!(
                                    "Imported '{}' with health blockers; review before launch.",
                                    result.instance_id
                                ),
                                details: Some(details.clone()),
                            });
                            sink.report(ProgressEvent::new(
                                op_id.clone(),
                                ProgressPhase::HealthScan,
                                format!("Import completed with health blockers: {details}"),
                            ));
                        }
                    }
                }
                if let Err(error) = check(&op, &cancel) {
                    let _ = rollback();
                    let _ = crate::db::delete_instance(&conn, &result.instance_id);
                    return Err(error);
                }
            }
        }

        // ── Phase 7: Initial snapshot ───────────────────────────────────
        // The instance is already fully extracted, registered, and health
        // checked at this point. Keep it visible, but mark it read-only until
        // its first recovery point is durable. Snapshotting is isolated on a
        // blocking worker so browsing can begin immediately.
        if let Ok(instance_dir) = self.ctx.paths.instance_dir(&result.instance_id) {
            if instance_dir.exists() {
                // The extraction rewrote tracked content; the pre-launch
                // snapshot reuse must not outlive the import.
                let _ = crate::snapshot::mark_instance_mutated(&instance_dir);
                crate::snapshot::mark_snapshot_pending(&instance_dir).map_err(|error| {
                    let _ = rollback();
                    let _ = crate::db::delete_instance(&conn, &result.instance_id);
                    op.clone().fail(error.clone());
                    LauncherError::Generic {
                        code: "ERR_SNAPSHOT_FAILED".into(),
                        message: error,
                    }
                })?;
                sink.report(ProgressEvent::new(
                    op_id.clone(),
                    ProgressPhase::Snapshotting,
                    "Finalizing the initial recovery snapshot…",
                ));
                if let Err(error) = check(&op, &cancel) {
                    let _ = rollback();
                    let _ = crate::db::delete_instance(&conn, &result.instance_id);
                    return Err(error);
                }
                let snapshot_sink = sink.clone();
                let snapshot_operation_id = op_id.clone();
                tokio::task::spawn_blocking(move || {
                    match crate::snapshot::create_snapshot(
                        &instance_dir,
                        Some("Initial import state"),
                    ) {
                        Ok(_) => {
                            let _ = crate::snapshot::mark_snapshot_ready(&instance_dir);
                            snapshot_sink.report(ProgressEvent::new(
                                snapshot_operation_id,
                                ProgressPhase::Done,
                                "Initial recovery snapshot ready",
                            ));
                        }
                        Err(error) => {
                            let _ = crate::snapshot::mark_snapshot_failed(&instance_dir, &error);
                            snapshot_sink.report(ProgressEvent::new(
                                snapshot_operation_id,
                                ProgressPhase::Failed,
                                format!("Initial recovery snapshot failed: {error}"),
                            ));
                        }
                    }
                });
            }
        }

        // ── Done ────────────────────────────────────────────────────────
        sink.report(ProgressEvent::new(
            op_id,
            ProgressPhase::Done,
            "Import complete",
        ));
        op.complete();
        Ok(result)
    }

    /// Run an import asynchronously.
    ///
    /// This convenience wrapper uses the context's configured progress sink and
    /// a default (never-cancelled) [`CancellationToken`]. Callers that need
    /// an operation-specific sink or cancellation should use
    /// [`ImportService::run_import_with_sink`] instead.
    ///
    /// Synchronous archive/filesystem extraction is offloaded via
    /// `spawn_blocking`. Metadata bootstrap, loader installation, health
    /// validation, and DB registration are genuinely async and respect
    /// the configured [`NetworkPolicy`]. On any failure after promotion,
    /// the instance directory and DB row are rolled back so no orphan
    /// instance remains.
    pub async fn run_import(&self, request: ImportRequest) -> LauncherResult<ImportResult> {
        self.run_import_with_sink(
            request,
            self.ctx.progress_sink.clone(),
            CancellationToken::new(),
        )
        .await
    }

    /// Detect installed launchers and their instance directories.
    pub fn auto_detect_launchers(&self) -> Vec<DetectedLauncher> {
        crate::import::auto_detect_launchers()
    }

    /// Install a pack manifest (Tier 1 simple or Tier 2 complex) into an existing
    /// instance.
    ///
    /// The manifest is validated, hashes are verified, and override-bundle SHA-256
    /// pins are enforced — the same protections that `pack_install` always applied.
    pub async fn install_pack(&self, request: ImportRequest) -> LauncherResult<PackInstallResult> {
        let (manifest_json, target_instance_id) = match request.source {
            ImportSource::PackManifest {
                manifest_json,
                target_instance_id,
            } => (manifest_json, target_instance_id),
            _ => {
                return Err(LauncherError::Generic {
                    code: "ERR_IMPORT_SOURCE".into(),
                    message: "Expected PackManifest import source.".into(),
                })
            }
        };

        let op = self
            .ctx
            .operation_manager
            .register_for_instance("Install pack", &target_instance_id);

        let manifest = match crate::pack_install::parse_pack_manifest(&manifest_json) {
            Ok(m) => m,
            Err(e) => {
                op.fail(e.clone());
                return Err(LauncherError::Generic {
                    code: "ERR_PACK_PARSE".into(),
                    message: e,
                });
            }
        };

        let sanitized = crate::paths::sanitize_id(&target_instance_id);
        let instance_dir = match self.ctx.paths.instance_dir(&sanitized) {
            Ok(d) => d,
            Err(e) => {
                op.fail(e.to_string());
                return Err(e);
            }
        };
        if !instance_dir.exists() {
            op.fail(format!("Instance '{target_instance_id}' not found."));
            return Err(LauncherError::Generic {
                code: "ERR_INSTANCE_NOT_FOUND".into(),
                message: format!("Instance '{target_instance_id}' not found."),
            });
        }

        let client = reqwest::Client::new();
        let result = if manifest.override_source.is_some() {
            crate::pack_install::install_complex_pack(&client, &manifest, &instance_dir).await
        } else {
            crate::pack_install::install_simple_pack(&client, &manifest, &instance_dir).await
        };
        match result {
            Ok(r) => {
                op.complete();
                Ok(r)
            }
            Err(e) => {
                op.fail(e.clone());
                Err(LauncherError::Generic {
                    code: "ERR_PACK".into(),
                    message: e,
                })
            }
        }
    }
}

/// Remove a promoted instance directory so no filesystem-only unregistered
/// instance remains after a registration failure.
fn remove_promoted(paths: &crate::app_paths::AppPaths, instance_id: &str) -> LauncherResult<()> {
    let dir = paths.instance_dir(instance_id)?;
    if dir.exists() {
        std::fs::remove_dir_all(&dir).map_err(|e| LauncherError::Generic {
            code: "ERR_IMPORT_CLEANUP".into(),
            message: format!("Failed to remove unregistered instance directory: {e}"),
        })
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ctx::CoreContext;
    use crate::event_sink::CollectingProgressSink;
    use crate::operation_manager::OpStatus;
    use std::io::Write;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn test_tmp(label: &str) -> std::path::PathBuf {
        static NEXT_ID: AtomicU64 = AtomicU64::new(0);
        std::env::temp_dir().join(format!(
            "agora-import-svc-{label}-{}-{}",
            std::process::id(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        ))
    }

    fn test_ctx() -> Ctx {
        let tmp = test_tmp("ctx");
        let _ = std::fs::create_dir_all(&tmp);
        let ctx = CoreContext::for_testing(tmp);
        let _ = crate::db::init_local_state_db(&ctx.paths.local_state_db());
        ctx
    }

    fn listing_contains(
        service: &crate::instance_service::InstanceService,
        instance_id: &str,
    ) -> bool {
        service
            .list()
            .ok()
            .map(|rows| rows.iter().any(|r| r.instance_id == instance_id))
            .unwrap_or(false)
    }

    #[tokio::test]
    async fn test_import_directory_through_service() {
        let ctx = test_ctx();
        let svc = ImportService::new(ctx);

        let tmp = test_tmp("dir");
        let _ = std::fs::create_dir_all(&tmp);
        let src = tmp.join("my-instance");
        std::fs::create_dir_all(src.join("mods")).unwrap();
        std::fs::write(src.join("mods").join("test-mod.jar"), b"fake jar").unwrap();

        let request = ImportRequest {
            source: ImportSource::Directory(src),
            symlink_saves: false,
        };
        let result = svc.run_import(request).await.unwrap();
        assert_eq!(result.name, "my-instance");
        assert_eq!(result.instance_id, "my-instance");
        assert_eq!(result.imported_mods, 1);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn test_import_mrpack_through_service() {
        let ctx = test_ctx();
        let svc = ImportService::new(ctx);

        let tmp = test_tmp("mrp");
        let _ = std::fs::create_dir_all(&tmp);
        let mrpack_path = tmp.join("a-pack.mrpack");
        let file = std::fs::File::create(&mrpack_path).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        writer
            .start_file(
                "modrinth.index.json",
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
        writer
            .write_all(br#"{"name":"a-pack","dependencies":{},"files":[]}"#)
            .unwrap();
        writer.finish().unwrap();

        let request = ImportRequest {
            source: ImportSource::mrpack(mrpack_path),
            symlink_saves: false,
        };
        let result = svc.run_import(request).await.unwrap();
        assert_eq!(result.instance_id, "a-pack");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_auto_detect_launchers_does_not_panic() {
        let ctx = test_ctx();
        let svc = ImportService::new(ctx);
        let launchers = svc.auto_detect_launchers();
        for l in &launchers {
            assert!(!l.launcher_type.is_empty());
        }
    }

    #[tokio::test]
    async fn test_import_registers_and_completes_operation() {
        let ctx = test_ctx();
        let op_mgr = ctx.operation_manager.clone();
        let svc = ImportService::new(ctx);

        let tmp = test_tmp("op");
        let _ = std::fs::create_dir_all(&tmp);
        let src = tmp.join("op-import");
        std::fs::create_dir_all(src.join("mods")).unwrap();
        std::fs::write(src.join("mods").join("a.jar"), b"mod").unwrap();

        assert_eq!(op_mgr.active_count(), 0);

        let request = ImportRequest {
            source: ImportSource::Directory(src),
            symlink_saves: false,
        };
        let _result = svc.run_import(request).await.unwrap();

        let all = op_mgr.list_all();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].status, OpStatus::Completed);
        assert_eq!(all[0].label, "Import instance");
        assert_eq!(op_mgr.active_count(), 0);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn test_import_fails_registers_failed_operation() {
        let ctx = test_ctx();
        let op_mgr = ctx.operation_manager.clone();
        let svc = ImportService::new(ctx);

        let request = ImportRequest {
            source: ImportSource::Directory(std::path::PathBuf::from(
                "/nonexistent/path/import-test",
            )),
            symlink_saves: false,
        };
        assert!(svc.run_import(request).await.is_err());

        let all = op_mgr.list_all();
        assert_eq!(all.len(), 1);
        assert!(matches!(all[0].status, OpStatus::Failed(_)));
    }

    #[tokio::test]
    async fn test_import_is_listed_by_instance_service() {
        let ctx = test_ctx();
        let svc = ImportService::new(ctx.clone());
        let instance_svc = crate::instance_service::InstanceService::new(ctx);

        let tmp = test_tmp("list");
        let _ = std::fs::create_dir_all(&tmp);
        let src = tmp.join("listed-instance");
        std::fs::create_dir_all(src.join("mods")).unwrap();
        std::fs::write(src.join("mods").join("mod.jar"), b"mod").unwrap();

        let request = ImportRequest {
            source: ImportSource::Directory(src),
            symlink_saves: false,
        };
        let result = svc.run_import(request).await.unwrap();
        assert_eq!(result.instance_id, "listed-instance");

        assert!(listing_contains(&instance_svc, "listed-instance"));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn test_import_mrpack_is_listed_by_instance_service() {
        let ctx = test_ctx();
        let svc = ImportService::new(ctx.clone());
        let instance_svc = crate::instance_service::InstanceService::new(ctx);

        let tmp = test_tmp("mrp-list");
        let _ = std::fs::create_dir_all(&tmp);
        let mrpack_path = tmp.join("pack.mrpack");
        let file = std::fs::File::create(&mrpack_path).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        writer
            .start_file(
                "modrinth.index.json",
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
        writer
            .write_all(br#"{"name":"mrp-list-pack","dependencies":{},"files":[]}"#)
            .unwrap();
        writer.finish().unwrap();

        let request = ImportRequest {
            source: ImportSource::mrpack(mrpack_path),
            symlink_saves: false,
        };
        let result = svc.run_import(request).await.unwrap();
        assert_eq!(result.instance_id, "mrp-list-pack");

        assert!(listing_contains(&instance_svc, "mrp-list-pack"));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn test_import_fast_fail_broken_db_cleans_nothing() {
        let tmp = test_tmp("brokendb");
        let _ = std::fs::create_dir_all(&tmp);
        std::fs::write(tmp.join("local_state.db"), b"not a database").unwrap();
        let ctx = CoreContext::for_testing(tmp.clone());
        let svc = ImportService::new(ctx);

        let src = tmp.join("to-import");
        std::fs::create_dir_all(src.join("mods")).unwrap();
        std::fs::write(src.join("mods").join("a.jar"), b"mod").unwrap();

        let request = ImportRequest {
            source: ImportSource::Directory(src),
            symlink_saves: false,
        };

        let err = svc.run_import(request).await.unwrap_err();
        assert_eq!(err.code(), "ERR_LOCAL_STATE_FAILED");

        let instances = tmp.join("instances");
        assert!(!instances.exists(), "no instance dir should exist");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn test_import_policy_disabled_bootstrap_failure_cleans_instance() {
        let tmp = test_tmp("polfail");
        let _ = std::fs::create_dir_all(&tmp);

        let conn = init_ctx_db(&tmp);
        crate::db::set_setting(
            &conn,
            "network_mojang_metadata_enabled",
            &serde_json::Value::Bool(false),
        )
        .unwrap();
        drop(conn);

        let ctx = CoreContext::for_testing(tmp.clone());
        let svc = ImportService::new(ctx);

        let mrpack_path = tmp.join("polfail-pack.mrpack");
        let file = std::fs::File::create(&mrpack_path).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        writer
            .start_file(
                "modrinth.index.json",
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
        writer
            .write_all(br#"{"name":"polfail","dependencies":{"minecraft":"1.21"},"files":[]}"#)
            .unwrap();
        writer.finish().unwrap();

        let request = ImportRequest {
            source: ImportSource::mrpack(mrpack_path),
            symlink_saves: false,
        };

        let err = svc.run_import(request).await.unwrap_err();
        assert_eq!(err.code(), "ERR_NETWORK_MOJANG_METADATA_DISABLED");

        let instance_dir = tmp.join("instances").join("polfail");
        assert!(
            !instance_dir.exists(),
            "orphaned instance dir must be removed after bootstrap failure"
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn test_import_produces_launchable_instance_through_service() {
        let ctx = test_ctx();
        let svc = ImportService::new(ctx);

        let tmp = test_tmp("svc-launchable");
        let _ = std::fs::create_dir_all(&tmp);
        let src = tmp.join("test-launchable");
        std::fs::create_dir_all(src.join("mods")).unwrap();
        std::fs::write(src.join("mods").join("mod.jar"), b"fake mod").unwrap();

        let request = ImportRequest {
            source: ImportSource::Directory(src),
            symlink_saves: false,
        };
        let result = svc.run_import(request).await.unwrap();
        assert_eq!(result.instance_id, "test-launchable");

        let manifest_path = svc
            .ctx
            .paths
            .instance_dir("test-launchable")
            .unwrap()
            .join("instance_manifest.json");
        assert!(manifest_path.exists());
        let manifest = crate::helpers::read_manifest(&manifest_path).unwrap();
        assert_eq!(manifest.instance_id, "test-launchable");
        assert!(manifest.mods.is_empty());
        assert!(manifest.resourcepacks.is_empty());

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn test_import_mrpack_manifest_roundtrip_through_service() {
        let ctx = test_ctx();
        let svc = ImportService::new(ctx);

        let tmp = test_tmp("svc-mrp-rt");
        let _ = std::fs::create_dir_all(&tmp);
        let mrpack_path = tmp.join("rt-pack.mrpack");
        let file = std::fs::File::create(&mrpack_path).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        writer
            .start_file(
                "modrinth.index.json",
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
        writer
            .write_all(br#"{"name":"rt-pack","dependencies":{"minecraft":"1.21"},"files":[]}"#)
            .unwrap();
        writer.finish().unwrap();

        let request = ImportRequest {
            source: ImportSource::mrpack(mrpack_path),
            symlink_saves: false,
        };
        let result = svc.run_import(request).await.unwrap();
        assert_eq!(result.instance_id, "rt-pack");
        assert_eq!(result.minecraft_version, "1.21");

        let manifest_path = svc
            .ctx
            .paths
            .instance_dir("rt-pack")
            .unwrap()
            .join("instance_manifest.json");
        // Asserts on the bytes actually written, so it must not heal.
        // allow-raw-instance-manifest
        let manifest: crate::models::InstanceManifest =
            serde_json::from_str(&std::fs::read_to_string(&manifest_path).unwrap()).unwrap();
        let serialized = serde_json::to_string_pretty(&manifest).unwrap();
        // Asserts on the bytes actually written, so it must not heal.
        // allow-raw-instance-manifest
        let deserialized: crate::models::InstanceManifest =
            serde_json::from_str(&serialized).unwrap();
        assert_eq!(deserialized.instance_id, manifest.instance_id);
        assert_eq!(deserialized.name, manifest.name);
        assert_eq!(deserialized.minecraft_version, manifest.minecraft_version);
        assert_eq!(deserialized.loader, manifest.loader);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    // ── Cancellation tests ──────────────────────────────────────────────

    #[tokio::test]
    async fn test_import_cancelled_before_extraction() {
        let ctx = test_ctx();
        let op_mgr = ctx.operation_manager.clone();
        let sink = Arc::new(CollectingProgressSink::new());
        let cancel = CancellationToken::new();
        let svc = ImportService::new(ctx);

        let tmp = test_tmp("cancel-before-extract");
        let _ = std::fs::create_dir_all(&tmp);
        let src = tmp.join("some-instance");
        std::fs::create_dir_all(src.join("mods")).unwrap();

        let request = ImportRequest {
            source: ImportSource::Directory(src),
            symlink_saves: false,
        };

        // Cancel before calling the sink API.
        cancel.cancel();

        let err = svc
            .run_import_with_sink(request, sink.clone(), cancel)
            .await
            .unwrap_err();
        assert_eq!(err.code(), "ERR_OPERATION_CANCELLED");

        // No instance directory was created.
        let instances = tmp.join("instances");
        assert!(!instances.exists(), "no instance dir should exist");

        // The operation manager entry was cleaned up on handle drop.
        assert_eq!(op_mgr.active_count(), 0);

        // Should have emitted at least the Resolving phase.
        let events = sink.events();
        let phases: Vec<ProgressPhase> = events.iter().map(|e| e.phase).collect();
        assert!(
            phases.contains(&ProgressPhase::Resolving),
            "should have emitted Resolving phase"
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn test_import_cancelled_after_extraction_before_metadata() {
        let ctx = test_ctx();
        let op_mgr = ctx.operation_manager.clone();
        let cancel = CancellationToken::new();
        let svc = ImportService::new(ctx);

        let tmp = test_tmp("cancel-after-extract");
        let _ = std::fs::create_dir_all(&tmp);
        let mrpack_path = tmp.join("cancel-me.mrpack");
        {
            let file = std::fs::File::create(&mrpack_path).unwrap();
            let mut writer = zip::ZipWriter::new(file);
            writer
                .start_file(
                    "modrinth.index.json",
                    zip::write::SimpleFileOptions::default(),
                )
                .unwrap();
            writer
                .write_all(
                    br#"{"name":"cancel-me","dependencies":{"minecraft":"1.21"},"files":[]}"#,
                )
                .unwrap();
            writer.finish().unwrap();
        }

        // Use a sink that fires the cancel token on the Verifying phase
        // (which runs after extraction completes).
        let cancel_on_verify = Arc::new(CancelOnPhaseSink {
            cancel: cancel.clone(),
            phase: ProgressPhase::Verifying,
        });

        let request = ImportRequest {
            source: ImportSource::mrpack(mrpack_path),
            symlink_saves: false,
        };

        let err = svc
            .run_import_with_sink(request, cancel_on_verify, cancel.clone())
            .await
            .unwrap_err();
        assert_eq!(err.code(), "ERR_OPERATION_CANCELLED");

        // The promoted instance directory must be cleaned up.
        let promoted_dir = tmp.join("instances").join("cancel-me");
        assert!(
            !promoted_dir.exists(),
            "orphaned instance dir must be removed after cancellation"
        );

        // No DB row exists.
        assert_eq!(op_mgr.active_count(), 0);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn test_import_cancelled_via_operation_manager() {
        let ctx = test_ctx();
        let op_mgr = ctx.operation_manager.clone();
        let cancel = CancellationToken::new();
        let svc = ImportService::new(ctx);

        let tmp = test_tmp("cancel-via-opmgr");
        let _ = std::fs::create_dir_all(&tmp);
        let mrpack_path = tmp.join("cancel-opmgr.mrpack");
        {
            let file = std::fs::File::create(&mrpack_path).unwrap();
            let mut writer = zip::ZipWriter::new(file);
            writer
                .start_file(
                    "modrinth.index.json",
                    zip::write::SimpleFileOptions::default(),
                )
                .unwrap();
            writer
                .write_all(
                    br#"{"name":"cancel-opmgr","dependencies":{"minecraft":"1.21"},"files":[]}"#,
                )
                .unwrap();
            writer.finish().unwrap();
        }

        let cancel_on_verify = Arc::new(CancelOnPhaseSink {
            cancel: cancel.clone(),
            phase: ProgressPhase::Verifying,
        });

        let request = ImportRequest {
            source: ImportSource::mrpack(mrpack_path),
            symlink_saves: false,
        };

        let err = svc
            .run_import_with_sink(request, cancel_on_verify, cancel)
            .await
            .unwrap_err();
        assert_eq!(err.code(), "ERR_OPERATION_CANCELLED");

        // Cleanup was performed.
        let promoted_dir = tmp.join("instances").join("cancel-opmgr");
        assert!(!promoted_dir.exists());

        assert_eq!(op_mgr.active_count(), 0);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn test_import_sink_emits_expected_phases() {
        let ctx = test_ctx();
        let sink = Arc::new(CollectingProgressSink::new());
        let svc = ImportService::new(ctx);

        let tmp = test_tmp("sink-phases");
        let _ = std::fs::create_dir_all(&tmp);
        let src = tmp.join("phase-instance");
        std::fs::create_dir_all(src.join("mods")).unwrap();
        std::fs::write(src.join("mods").join("m.jar"), b"x").unwrap();

        let request = ImportRequest {
            source: ImportSource::Directory(src),
            symlink_saves: false,
        };
        svc.run_import_with_sink(request, sink.clone(), CancellationToken::new())
            .await
            .unwrap();

        let events = sink.events();
        let phases: Vec<ProgressPhase> = events.iter().map(|e| e.phase).collect();

        // Must include at least these phases in order.
        assert!(
            phases.contains(&ProgressPhase::Resolving),
            "missing Resolving"
        );
        assert!(
            phases.contains(&ProgressPhase::Extracting),
            "missing Extracting"
        );
        assert!(phases.contains(&ProgressPhase::Done), "missing Done");

        // Every event should carry a non-empty operation_id.
        for ev in &events {
            assert!(!ev.operation_id.0.is_empty(), "empty operation_id");
        }

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn test_mrpack_import_stamps_pack_origin_and_inventory_via_service() {
        let ctx = test_ctx();
        let svc = ImportService::new(ctx.clone());

        let tmp = test_tmp("mrpack-service-pack");
        let _ = std::fs::create_dir_all(&tmp);
        let mrpack_path = tmp.join("pack.mrpack");
        {
            let file = std::fs::File::create(&mrpack_path).unwrap();
            let mut writer = zip::ZipWriter::new(file);
            let opts = zip::write::SimpleFileOptions::default();
            writer.start_file("modrinth.index.json", opts).unwrap();
            // versionId present; dependencies for pack loader/mc
            writer
                .write_all(
                    br#"{"name":"Service Pack","versionId":"2.0.1","dependencies":{"minecraft":"1.20.1","fabric-loader":"0.15.0"},"files":[]}"#,
                )
                .unwrap();
            writer
                .start_file("overrides/config/service.toml", opts)
                .unwrap();
            writer.write_all(b"service cfg").unwrap();
            writer.finish().unwrap();
        }

        let presigned = "https://cdn.modrinth.com/data/abc/versions/xyz/pack.mrpack?token=secret&expires=123#frag";
        let request = ImportRequest {
            source: ImportSource::Mrpack {
                path: mrpack_path,
                origin_url: Some(presigned.to_string()),
                skip_unfetchable: false,
            },
            symlink_saves: false,
        };
        let result = svc.run_import(request).await.unwrap();
        assert_eq!(result.instance_id, "Service-Pack");

        // Check manifest PackOrigin
        let manifest_path = svc
            .ctx
            .paths
            .instance_dir("Service-Pack")
            .unwrap()
            .join("instance_manifest.json");
        // Asserts on the bytes actually written, so it must not heal.
        // allow-raw-instance-manifest
        let manifest: crate::models::InstanceManifest =
            serde_json::from_str(&std::fs::read_to_string(&manifest_path).unwrap()).unwrap();
        let origin = manifest.pack_origin.expect("service must stamp PackOrigin");
        assert_eq!(origin.platform, crate::models::PackPlatform::Modrinth);
        assert_eq!(origin.pack_name, "Service Pack");
        assert_eq!(origin.version_id.as_deref(), Some("2.0.1"));
        assert!(origin.version_number.is_none());
        assert!(origin.project_id.is_none(), "mrpack has no project_id");
        assert_eq!(
            origin.origin_url.as_deref(),
            Some("https://cdn.modrinth.com/data/abc/versions/xyz/pack.mrpack")
        );
        assert!(!origin.origin_url.as_deref().unwrap().contains("token"));
        assert!(origin.pack_content_hash.is_some());

        // Check DB inventory is non-empty and hash matches manifest
        let conn = crate::db::local_state_connection(&svc.ctx.paths.local_state_db()).unwrap();
        let files = crate::db::list_instance_pack_files(&conn, "Service-Pack").unwrap();
        assert!(!files.is_empty(), "stored inventory must be non-empty");
        let computed = crate::pack_inventory::pack_content_hash(&files);
        assert_eq!(origin.pack_content_hash.as_deref(), Some(computed.as_str()));

        // Also verify that the inventory collected from the instance dir matches DB
        let instance_dir = svc.ctx.paths.instance_dir("Service-Pack").unwrap();
        let live = crate::pack_inventory::collect_pack_inventory(&instance_dir).unwrap();
        assert_eq!(live, files);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn test_technic_solder_import_via_service_stamps_origin_and_inventory() {
        let ctx = test_ctx();
        let svc = ImportService::new(ctx.clone());

        let tmp = test_tmp("technic-solder-service");
        let _ = std::fs::create_dir_all(&tmp);
        // Use empty mods to avoid network; still should stamp origin and produce
        // an inventory that is empty (hash None) — honest for empty pack.
        // For non-empty inventory, we rely on the file-system pack inventory
        // after the import: the Solder import writes no files, so we manually
        // seed a config file to simulate a pack that contributed a file,
        // then verify the service's DB store would have captured it if it had
        // been there at import time. Simpler: verify origin fields are correct
        // and that an empty inventory still results in a valid PackOrigin.
        let pack = crate::import::TechnicSolderPack {
            display_name: "Solder Service Pack".into(),
            minecraft_version: "1.20.1".into(),
            loader: "".into(),
            loader_version: "".into(),
            mods: vec![],
            slug: "solder-pack".into(),
            solder_url: "https://solder.example.com/api?token=secret#frag".into(),
            build: "1.5.2".into(),
        };
        let request = ImportRequest {
            source: ImportSource::TechnicSolder(pack),
            symlink_saves: false,
        };
        let result = svc.run_import(request).await.unwrap();
        assert_eq!(result.instance_id, "Solder-Service-Pack");

        let manifest_path = svc
            .ctx
            .paths
            .instance_dir("Solder-Service-Pack")
            .unwrap()
            .join("instance_manifest.json");
        // Asserts on the bytes actually written, so it must not heal.
        // allow-raw-instance-manifest
        let manifest: crate::models::InstanceManifest =
            serde_json::from_str(&std::fs::read_to_string(&manifest_path).unwrap()).unwrap();
        let origin = manifest.pack_origin.expect("solder must stamp PackOrigin");
        assert_eq!(origin.platform, crate::models::PackPlatform::TechnicSolder);
        assert_eq!(origin.pack_name, "Solder Service Pack");
        assert_eq!(origin.project_id.as_deref(), Some("solder-pack"));
        assert_eq!(origin.version_id.as_deref(), Some("1.5.2"));
        assert!(origin.version_number.is_none());
        assert_eq!(
            origin.origin_url.as_deref(),
            Some("https://solder.example.com/api")
        );
        assert!(!origin.origin_url.as_deref().unwrap().contains("token"));
        // No files, so hash None is honest; DB inventory should be empty.
        assert!(origin.pack_content_hash.is_none());
        let conn = crate::db::local_state_connection(&svc.ctx.paths.local_state_db()).unwrap();
        let files = crate::db::list_instance_pack_files(&conn, "Solder-Service-Pack").unwrap();
        assert!(files.is_empty());

        // Now test non-empty inventory via manual file + service DB write path:
        // Simulate a pack that had written a config file by creating one
        // and re-collecting.
        let instance_dir = svc.ctx.paths.instance_dir("Solder-Service-Pack").unwrap();
        std::fs::create_dir_all(instance_dir.join("config")).unwrap();
        std::fs::write(
            instance_dir.join("config").join("solder.toml"),
            b"solder cfg",
        )
        .unwrap();
        let files2 = crate::pack_inventory::collect_pack_inventory(&instance_dir).unwrap();
        assert!(!files2.is_empty());
        let hash2 = crate::pack_inventory::pack_content_hash(&files2);
        // Store and verify round-trip
        crate::db::replace_instance_pack_files(&conn, "Solder-Service-Pack", &files2).unwrap();
        let listed = crate::db::list_instance_pack_files(&conn, "Solder-Service-Pack").unwrap();
        assert_eq!(listed, files2);
        assert_eq!(crate::pack_inventory::pack_content_hash(&listed), hash2);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_strip_url_query_removes_query_and_fragment() {
        assert_eq!(
            strip_url_query("https://cdn.modrinth.com/pack.mrpack?token=abc&x=1#frag"),
            "https://cdn.modrinth.com/pack.mrpack"
        );
        assert_eq!(
            strip_url_query("https://example.com/a?b=1"),
            "https://example.com/a"
        );
        assert_eq!(
            strip_url_query("https://example.com/a#frag"),
            "https://example.com/a"
        );
        assert_eq!(
            strip_url_query("https://example.com/a"),
            "https://example.com/a"
        );
    }

    /// Progress sink that cancels the given token when a specific phase is
    /// observed.  Used to test mid-import cancellation.
    struct CancelOnPhaseSink {
        cancel: CancellationToken,
        phase: ProgressPhase,
    }

    impl crate::event_sink::ProgressSink for CancelOnPhaseSink {
        fn report(&self, event: ProgressEvent) {
            if event.phase == self.phase {
                self.cancel.cancel();
            }
        }
    }

    fn init_ctx_db(tmp: &std::path::Path) -> rusqlite::Connection {
        let db_path = tmp.join("local_state.db");
        let _ = crate::db::init_local_state_db(&db_path);
        rusqlite::Connection::open(&db_path).expect("test db")
    }
}

#[cfg(test)]
mod review_tests {
    use super::*;
    use crate::agora_pack_import::PlannedSource;
    use crate::ctx::CoreContext;
    use std::io::Write;

    fn new_ctx() -> (tempfile::TempDir, Ctx) {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = CoreContext::for_testing(tmp.path().to_path_buf());
        let _ = crate::db::init_local_state_db(&ctx.paths.local_state_db());
        (tmp, ctx)
    }

    /// An `.mrpack` with `index` as its index and `files` as extra entries.
    fn pack(dir: &std::path::Path, file: &str, index: &str, files: &[(&str, &[u8])]) -> PathBuf {
        let path = dir.join(file);
        let mut writer = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        let opts = zip::write::SimpleFileOptions::default();
        writer.start_file("modrinth.index.json", opts).unwrap();
        writer.write_all(index.as_bytes()).unwrap();
        for (name, bytes) in files {
            writer.start_file(*name, opts).unwrap();
            writer.write_all(bytes).unwrap();
        }
        writer.finish().unwrap();
        path
    }

    const SHA1: &str = "da39a3ee5e6b4b0d3255bfef95601890afd80709";

    fn index_with_files(files: serde_json::Value) -> String {
        serde_json::json!({
            "name": "Spiff", "versionId": "1",
            "dependencies": { "minecraft": "1.20.4", "fabric-loader": "0.15.0" },
            "files": files,
        })
        .to_string()
    }

    #[test]
    fn the_review_says_where_each_file_comes_from_what_is_left_out_and_what_would_fail() {
        let (tmp, ctx) = new_ctx();
        let index = index_with_files(serde_json::json!([
            { "path": "mods/sodium.jar", "hashes": { "sha1": SHA1 },
              "downloads": ["https://cdn.modrinth.com/data/AANobbMI/versions/x/sodium.jar"] },
            { "path": "mods/homebrew.jar", "hashes": { "sha1": SHA1 }, "downloads": [] },
            { "path": "mods/nohash.jar", "hashes": {},
              "downloads": ["https://cdn.modrinth.com/data/a/versions/x/nohash.jar"] },
            { "path": "mods/ghost.jar", "hashes": { "sha1": SHA1 },
              "downloads": ["https://example.com/ghost.jar"] },
        ]));
        let path = pack(
            tmp.path(),
            "spiff.mrpack",
            &index,
            &[
                ("mods/homebrew.jar", b""),
                ("overrides/config/sodium.json", b"{}"),
                ("overrides/shaderpacks/BSL.zip", b"zip"),
                ("overrides/saves/world/level.dat", b"x"),
                ("server-overrides/server.properties", b"x"),
            ],
        );

        let plan = ImportService::new(ctx).plan_mrpack_import(&path).unwrap();

        let source_of = |name: &str| {
            plan.entries
                .iter()
                .find(|e| e.name == name)
                .unwrap_or_else(|| panic!("{name} missing from {:?}", plan.entries))
        };
        assert_eq!(source_of("mods/sodium.jar").source, PlannedSource::Modrinth);
        assert_eq!(
            source_of("mods/homebrew.jar").source,
            PlannedSource::Bundled
        );
        assert_eq!(
            source_of("shaderpacks/BSL.zip").source,
            PlannedSource::Bundled
        );
        assert!(source_of("mods/nohash.jar")
            .reason
            .as_deref()
            .unwrap()
            .contains("SHA-1"));
        assert_eq!(source_of("mods/ghost.jar").source, PlannedSource::CannotGet);
        assert_eq!(plan.problems.len(), 1, "{:?}", plan.problems);
        assert_eq!(plan.soft_problems.len(), 1, "{:?}", plan.soft_problems);
        assert!(plan
            .left_out
            .iter()
            .any(|l| l.starts_with("saves/world/level.dat")));
        assert!(plan
            .left_out
            .iter()
            .any(|l| l.starts_with("server-overrides/")));
        assert!(plan
            .notes
            .iter()
            .any(|n| n.contains("1 settings and data file")));
        assert_eq!(plan.instance.loader, "fabric");
        assert_eq!(plan.file_size, std::fs::metadata(&path).unwrap().len());
        assert_eq!(plan.file_sha256.len(), 64);
    }

    #[test]
    fn a_pack_the_importer_would_refuse_is_reported_as_a_problem_not_discovered_midway() {
        let (tmp, ctx) = new_ctx();
        let index = serde_json::json!({
            "name": "NoLoader", "dependencies": { "minecraft": "1.20.4" }, "files": [],
        })
        .to_string();
        let path = pack(
            tmp.path(),
            "nl.mrpack",
            &index,
            &[("overrides/mods/a.jar", b"jar")],
        );
        let plan = ImportService::new(ctx).plan_mrpack_import(&path).unwrap();
        // Standard policy declines a loose jar, so it is left out, not imported.
        assert!(
            plan.left_out.iter().any(|l| l.starts_with("mods/a.jar")),
            "{:?}",
            plan.left_out
        );

        let index = index_with_files(serde_json::json!([
            { "path": "mods/a.jar", "hashes": { "sha1": SHA1 }, "downloads": [] },
        ]))
        .replace("\"fabric-loader\":\"0.15.0\"", "\"x\":\"1\"");
        let (tmp2, ctx2) = new_ctx();
        let path = pack(tmp2.path(), "nl2.mrpack", &index, &[("mods/a.jar", b"jar")]);
        let plan = ImportService::new(ctx2).plan_mrpack_import(&path).unwrap();
        assert!(
            plan.problems.iter().any(|p| p.contains("no mod loader")),
            "{:?}",
            plan.problems
        );
    }

    #[test]
    fn a_taken_name_gets_a_free_numbered_suggestion_even_when_the_folder_differs() {
        let (tmp, ctx) = new_ctx();
        // An instance the user renamed: its folder is "old-id", its name "Spiff".
        let row = InstanceRow {
            instance_id: "old-id".into(),
            name: "Spiff".into(),
            minecraft_version: "1.20.4".into(),
            loader: "fabric".into(),
            loader_version: "0.15.0".into(),
            is_modpack: false,
            is_locked: false,
            last_launched_at: None,
            jvm_memory_mb: 4096,
            jvm_memory_mode: "auto".into(),
            jvm_gc: "auto".into(),
            jvm_custom_args: String::new(),
            jvm_always_pre_touch: false,
            created_at: chrono::Utc::now().to_rfc3339(),
            java_path: None,
            java_incompatible_override: false,
            icon_path: None,
            launch_mode_override: "auto".into(),
            import_source: None,
        };
        let conn = crate::db::local_state_connection(&ctx.paths.local_state_db()).unwrap();
        crate::db::upsert_instance(&conn, &row).unwrap();
        let path = pack(
            tmp.path(),
            "spiff.mrpack",
            &index_with_files(serde_json::json!([])),
            &[],
        );

        let plan = ImportService::new(ctx).plan_mrpack_import(&path).unwrap();

        assert!(plan.name_taken);
        assert_eq!(plan.pack_name, "Spiff");
        assert_eq!(plan.suggested_name, "Spiff (2)");
    }

    #[test]
    fn an_import_uses_the_copy_that_was_reviewed_even_if_the_original_is_swapped() {
        let (tmp, ctx) = new_ctx();
        let path = pack(
            tmp.path(),
            "p.mrpack",
            &index_with_files(serde_json::json!([])),
            &[],
        );
        let svc = ImportService::new(ctx);
        let plan = svc.plan_mrpack_import(&path).unwrap();
        let reviewed_bytes = std::fs::read(&path).unwrap();

        // The original is replaced after review.
        pack(
            tmp.path(),
            "p.mrpack",
            &index_with_files(serde_json::json!([])),
            &[("overrides/config/x.txt", b"swapped")],
        );

        let staged = svc.take_reviewed_copy(Some(&plan.file_sha256)).unwrap();
        assert_eq!(std::fs::read(&staged).unwrap(), reviewed_bytes);
        assert_eq!(staged.file_name().unwrap(), "p.mrpack");

        let unreviewed = svc.take_reviewed_copy(None).unwrap_err();
        assert!(
            unreviewed.to_string().contains("Review the pack"),
            "{unreviewed}"
        );
        let bogus = svc.take_reviewed_copy(Some("../../etc")).unwrap_err();
        assert!(bogus.to_string().contains("Review the pack"), "{bogus}");

        // A tampered staged copy is refused.
        std::fs::write(&staged, b"tampered").unwrap();
        let changed = svc.take_reviewed_copy(Some(&plan.file_sha256)).unwrap_err();
        assert!(changed.to_string().contains("changed"), "{changed}");

        svc.discard_reviewed_copy(&plan.file_sha256);
        assert!(!staged.exists());
        let gone = svc.take_reviewed_copy(Some(&plan.file_sha256)).unwrap_err();
        assert!(gone.to_string().contains("Review it again"), "{gone}");
    }

    #[test]
    fn unfetchable_files_are_soft_and_unsafe_or_unverifiable_ones_are_hard() {
        let (tmp, ctx) = new_ctx();
        let index = index_with_files(serde_json::json!([
            { "path": "mods/ghost.jar", "hashes": { "sha1": SHA1 }, "downloads": [] },
            { "path": "mods/tampered.jar", "hashes": { "sha1": SHA1 }, "downloads": [] },
            { "path": "mods/nohash.jar", "hashes": {},
              "downloads": ["https://cdn.modrinth.com/data/a/versions/x/nohash.jar"] },
            { "path": "../escape.jar", "hashes": { "sha1": SHA1 }, "downloads": [] },
        ]));
        let path = pack(
            tmp.path(),
            "p.mrpack",
            &index,
            &[("mods/tampered.jar", b"not empty")],
        );
        let plan = ImportService::new(ctx).plan_mrpack_import(&path).unwrap();

        assert_eq!(plan.soft_problems.len(), 1, "{:?}", plan.soft_problems);
        assert!(plan.soft_problems[0].contains("mods/ghost.jar"));
        assert_eq!(plan.problems.len(), 3, "{:?}", plan.problems);
        let joined = plan.problems.join("\n");
        assert!(
            joined.contains("mods/tampered.jar"),
            "hash mismatch is hard"
        );
        assert!(joined.contains("mods/nohash.jar"), "no hash is hard");
        assert!(joined.contains("escape.jar"), "path traversal is hard");
    }

    #[tokio::test]
    async fn importing_anyway_leaves_out_exactly_the_unfetchable_files_and_strict_import_still_refuses(
    ) {
        let (tmp, ctx) = new_ctx();
        let index = r#"{"name":"Gappy","dependencies":{},"files":[
            {"path":"mods/ghost.jar","hashes":{"sha1":"da39a3ee5e6b4b0d3255bfef95601890afd80709"},"downloads":[]}
        ]}"#;
        let path = pack(
            tmp.path(),
            "gappy.mrpack",
            index,
            &[("overrides/config/a.toml", b"x")],
        );
        let svc = ImportService::new(ctx.clone());
        let request = |skip: bool| ImportRequest {
            source: ImportSource::Mrpack {
                path: path.clone(),
                origin_url: None,
                skip_unfetchable: skip,
            },
            symlink_saves: false,
        };

        let strict = svc.run_import(request(false)).await;
        assert!(
            strict.is_err(),
            "a file with no source stops a strict import"
        );

        let result = svc.run_import(request(true)).await.unwrap();
        let root = ctx.paths.instance_dir(&result.instance_id).unwrap();
        assert!(!root.join("mods").join("ghost.jar").exists());
        assert!(root.join("config").join("a.toml").is_file());
    }

    #[tokio::test]
    async fn an_imported_pack_keeps_the_name_chosen_and_remembers_its_file() {
        let (tmp, ctx) = new_ctx();
        let path = pack(
            tmp.path(),
            "spiff-fabric-1.20.4.mrpack",
            r#"{"name":"Spiff","dependencies":{},"files":[]}"#,
            &[],
        );
        let svc = ImportService::new(ctx.clone());
        let result = svc
            .run_import_named(
                ImportRequest {
                    source: ImportSource::mrpack(path),
                    symlink_saves: false,
                },
                Some("Spiff (2)".into()),
                ctx.progress_sink.clone(),
                CancellationToken::new(),
            )
            .await
            .unwrap();

        assert_eq!(result.name, "Spiff (2)");
        let conn = crate::db::local_state_connection(&ctx.paths.local_state_db()).unwrap();
        let row = crate::db::get_instance(&conn, &result.instance_id)
            .unwrap()
            .unwrap();
        assert_eq!(row.name, "Spiff (2)");
        assert_eq!(
            row.import_source.as_deref(),
            Some("mrpack:spiff-fabric-1.20.4.mrpack")
        );
    }
}
