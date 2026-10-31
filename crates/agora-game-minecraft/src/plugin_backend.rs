//! Minecraft's implementation of the instance operations core's plugin host
//! and repair actions perform.

use agora_core::ctx::Ctx;
use agora_core::error::LauncherResult;
use agora_core::game_hooks::InstanceBackend;
use agora_core::models::{InstanceManifest, InstanceRow};
use agora_plugin_api::dto;

pub struct MinecraftInstances;

impl InstanceBackend for MinecraftInstances {
    fn list(&self, ctx: &Ctx) -> LauncherResult<Vec<InstanceRow>> {
        crate::instance_service::InstanceService::new(ctx.clone()).list()
    }

    fn get(
        &self,
        ctx: &Ctx,
        instance_id: &str,
    ) -> LauncherResult<Option<(InstanceRow, Option<InstanceManifest>)>> {
        Ok(crate::instance_service::InstanceService::new(ctx.clone())
            .get(instance_id)?
            .map(|detail| (detail.row, detail.manifest)))
    }

    fn rename(&self, ctx: &Ctx, instance_id: &str, name: &str) -> LauncherResult<()> {
        crate::instance_service::InstanceService::new(ctx.clone()).rename(instance_id, name)
    }

    fn update_jvm(
        &self,
        ctx: &Ctx,
        instance_id: &str,
        memory_mb: i64,
        gc: &str,
        always_pre_touch: bool,
        custom_args: &str,
        memory_mode: &str,
    ) -> LauncherResult<()> {
        crate::instance_service::InstanceService::new(ctx.clone()).update_jvm(
            instance_id,
            memory_mb,
            gc,
            always_pre_touch,
            custom_args,
            memory_mode,
        )
    }

    fn content(
        &self,
        ctx: &Ctx,
        instance_id: &str,
        content_type: Option<&str>,
    ) -> LauncherResult<Option<Vec<dto::ContentEntry>>> {
        let service = crate::instance_service::InstanceService::new(ctx.clone());
        let Some(detail) = service.get(instance_id)? else {
            return Ok(None);
        };
        let Some(manifest) = detail.manifest else {
            return Ok(Some(Vec::new()));
        };
        let instance_dir = ctx.paths.instance_dir(instance_id)?;
        let rows = crate::installed_content::list_installed_content(
            &instance_dir,
            &manifest,
            content_type,
            None,
        );
        Ok(Some(
            rows.into_iter()
                .map(|row| dto::ContentEntry {
                    key: row.key,
                    filename: row.filename,
                    display_name: row.display_name,
                    version: row.version,
                    content_type: row.content_type,
                    enabled: row.enabled,
                    installed_at: row.installed_at,
                    source_label: row.source_label,
                    pack_managed: row.pack_managed,
                    installed_as_dependency: row.installed_as_dependency,
                    update_pinned: row.update_pinned,
                    file_present: row.file_present,
                    size_bytes: row.size_bytes,
                    author: row.author,
                    categories: row.categories,
                    source_url: row.source_url,
                    registry_id: row.registry_id,
                    modrinth_id: row.modrinth_id,
                    // `resolved_path` is deliberately dropped here.
                })
                .collect(),
        ))
    }

    fn set_update_pinned(
        &self,
        ctx: &Ctx,
        instance_id: &str,
        filename: &str,
        pinned: bool,
    ) -> LauncherResult<bool> {
        crate::install_service::InstallService::new(ctx.clone()).set_update_pinned(
            instance_id,
            filename,
            pinned,
        )
    }
}
