//! Minecraft's state on core's context, and the hooks that register Minecraft
//! with core.
//!
//! Core carries no Minecraft fields. The Java runtime catalog and the path of
//! the official launcher's profile file live in [`MinecraftExtension`] on the
//! context's extension store; the signed-registry catalogs are loaded by a
//! catalog hook; providers, host classification and the plugin instance
//! backend are registered by [`register`].

use crate::loader_manifests::LoaderCatalog;
use crate::runtime_catalog::{RuntimeCatalog, RuntimeCatalogHandle};
use agora_core::ctx::Ctx;
use agora_core::game_hooks::{self, CatalogEvent};
use agora_core::network::NetworkCategory;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

/// Minecraft-owned state carried on a [`Ctx`].
pub struct MinecraftExtension {
    /// Validated Java runtime catalog. Shared, and replaceable at runtime when
    /// a fresh registry arrives.
    pub runtime_catalog: RuntimeCatalogHandle,
    /// The official Mojang launcher's profile file. Test contexts point this
    /// at an isolated fixture instead of the user's real Minecraft data.
    launcher_profiles_path: RwLock<Option<PathBuf>>,
}

fn extension(ctx: &Ctx) -> Arc<MinecraftExtension> {
    ctx.extensions.get_or_insert_with(|| MinecraftExtension {
        runtime_catalog: RuntimeCatalogHandle::new(RuntimeCatalog::embedded()),
        launcher_profiles_path: RwLock::new(match &ctx.external_data_root {
            Some(root) => Some(
                root.join("official-minecraft")
                    .join("launcher_profiles.json"),
            ),
            None => agora_core::paths::launcher_profiles_path(),
        }),
    })
}

/// The Java runtime catalog active for this context.
pub fn runtime_catalog(ctx: &Ctx) -> RuntimeCatalogHandle {
    extension(ctx).runtime_catalog.clone()
}

/// The official launcher's profile file for this context, if there is one.
pub fn launcher_profiles_path(ctx: &Ctx) -> Option<PathBuf> {
    extension(ctx)
        .launcher_profiles_path
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
}

/// Point this context at a different launcher profile file (tests, and
/// adapters that let the user choose).
pub fn set_launcher_profiles_path(ctx: &Ctx, path: Option<PathBuf>) {
    *extension(ctx)
        .launcher_profiles_path
        .write()
        .unwrap_or_else(|e| e.into_inner()) = path;
}

/// Register Minecraft with core. Idempotent; adapters call it before building
/// their context, and tests may call it freely.
pub fn register() {
    // Guarded rather than relying on the hooks' own duplicate check, which
    // compares function addresses: closures are not guaranteed one address.
    static REGISTERED: std::sync::Once = std::sync::Once::new();
    REGISTERED.call_once(register_once);
}

fn register_once() {
    game_hooks::register_catalog_hook(load_catalogs);
    game_hooks::register_startup_hook(crate::launcher_import_service::recover_interrupted_jobs);
    game_hooks::register_provider(|ctx| {
        Arc::new(crate::providers::modrinth::ModrinthProvider::new(
            ctx.clone(),
        ))
    });
    game_hooks::register_provider(|ctx| {
        Arc::new(crate::providers::technic::TechnicProvider::new(ctx.clone()))
    });
    game_hooks::set_instance_backend(Arc::new(crate::plugin_backend::MinecraftInstances));
}

/// The network category of one of Minecraft's hosts (Mojang's, or a pinned
/// loader's), or `None` for any other host. The launch planner trusts only
/// these categories, so this stays the package's own decision rather than a
/// hook any registered package could answer.
pub fn classify_host(host: &str) -> Option<NetworkCategory> {
    if matches!(host, "piston-meta.mojang.com" | "launcher.mojang.com") {
        return Some(NetworkCategory::MojangMetadata);
    }
    if matches!(
        host,
        "piston-data.mojang.com" | "libraries.minecraft.net" | "resources.download.minecraft.net"
    ) {
        return Some(NetworkCategory::MojangContent);
    }
    if crate::loader_manifests::is_allowed_host(host) {
        return Some(NetworkCategory::LoaderMetadataAndContent);
    }
    None
}

/// [`classify_host`] for a full URL; `None` when it does not parse.
pub fn classify_url(raw: &str) -> Option<NetworkCategory> {
    let url = reqwest::Url::parse(raw).ok()?;
    classify_host(url.host_str()?)
}

/// Load the loader and Java runtime catalogs from the signed registry.
///
/// At startup each catalog falls back to embedded data on its own. On reload,
/// both are parsed before either is replaced, so a corrupt catalog keeps the
/// active pair consistent rather than replacing only one of them.
fn load_catalogs(
    ctx: &Ctx,
    registry: Option<&rusqlite::Connection>,
    event: CatalogEvent,
) -> agora_core::error::LauncherResult<Vec<String>> {
    let mut warnings = Vec::new();
    // Core passes no registry only at startup, when there is none to read.
    let Some(conn) = registry else {
        warnings.push("Using embedded loader and Java runtime catalogs".into());
        return Ok(warnings);
    };
    match event {
        CatalogEvent::Startup => {
            match LoaderCatalog::init_from_registry(conn) {
                Ok(true) => {
                    warnings.push("Using merged signed and embedded loader catalogs".into())
                }
                Ok(false) => warnings.push("Using embedded loader catalog".into()),
                Err(error) => warnings.push(format!(
                    "Cannot load signed loader catalog; using embedded fallback: {error}"
                )),
            }
            match RuntimeCatalog::from_registry_db(conn) {
                Ok(Some(catalog)) => {
                    runtime_catalog(ctx).replace(catalog);
                    warnings.push("Using signed registry Java runtime catalog".into());
                }
                Ok(None) => warnings.push("Using embedded Java runtime catalog".into()),
                Err(errors) => warnings.push(format!(
                    "Cannot load signed Java runtime catalog; using embedded fallback: {errors:?}"
                )),
            }
        }
        CatalogEvent::Reload => {
            let registry_loader_catalog = match LoaderCatalog::from_registry(conn) {
                Ok(catalog) => catalog,
                Err(error) => {
                    warnings.push(format!(
                        "Cannot load signed loader catalog; preserving existing active catalogs: {error}"
                    ));
                    return Ok(warnings);
                }
            };
            let has_registry_loader_catalog = registry_loader_catalog.is_some();
            let loader_catalog = match LoaderCatalog::merge_with_embedded(registry_loader_catalog) {
                Ok(catalog) => catalog,
                Err(error) => {
                    warnings.push(format!(
                        "Cannot merge signed loader catalog; preserving existing active catalogs: {error}"
                    ));
                    return Ok(warnings);
                }
            };
            let catalog = match RuntimeCatalog::from_registry_db(conn) {
                Ok(Some(catalog)) => {
                    warnings.push("Using signed registry Java runtime catalog".into());
                    catalog
                }
                Ok(None) => {
                    warnings.push("No runtime catalog in registry; using embedded".into());
                    RuntimeCatalog::embedded()
                }
                Err(errors) => {
                    warnings.push(format!(
                        "Cannot reload Java runtime catalog; preserving existing active catalogs: {errors:?}"
                    ));
                    return Ok(warnings);
                }
            };
            LoaderCatalog::replace_active(Some(loader_catalog))?;
            runtime_catalog(ctx).replace(catalog);
            warnings.push(if has_registry_loader_catalog {
                "Using merged signed and embedded loader catalogs".into()
            } else {
                "Using embedded loader catalog".into()
            });
        }
    }
    Ok(warnings)
}

#[cfg(test)]
mod tests {

    #[test]
    fn classify_mojang_metadata_hosts() {
        assert_eq!(
            super::classify_host("piston-meta.mojang.com"),
            Some(agora_core::network::NetworkCategory::MojangMetadata)
        );
        assert_eq!(
            super::classify_host("launcher.mojang.com"),
            Some(agora_core::network::NetworkCategory::MojangMetadata)
        );
    }

    #[test]
    fn classify_mojang_content_hosts() {
        assert_eq!(
            super::classify_host("piston-data.mojang.com"),
            Some(agora_core::network::NetworkCategory::MojangContent)
        );
        assert_eq!(
            super::classify_host("libraries.minecraft.net"),
            Some(agora_core::network::NetworkCategory::MojangContent)
        );
        assert_eq!(
            super::classify_host("resources.download.minecraft.net"),
            Some(agora_core::network::NetworkCategory::MojangContent)
        );
    }

    #[test]
    fn classify_url_parses_host() {
        assert_eq!(
            super::classify_url("https://piston-meta.mojang.com/mc/game/version_manifest_v2.json"),
            Some(agora_core::network::NetworkCategory::MojangMetadata)
        );
        assert_eq!(
            super::classify_url("https://piston-data.mojang.com/v2/1.21/client.jar"),
            Some(agora_core::network::NetworkCategory::MojangContent)
        );
        assert_eq!(
            super::classify_url("https://maven.fabricmc.net/v2/0.19.0/profile.json"),
            Some(agora_core::network::NetworkCategory::LoaderMetadataAndContent)
        );
    }

    #[test]
    fn classify_loader_hosts() {
        // Fabric Maven — should NOT classify as Mojang content
        if let Some(cat) = super::classify_host("maven.fabricmc.net") {
            assert_eq!(
                cat,
                agora_core::network::NetworkCategory::LoaderMetadataAndContent
            );
        }
        if let Some(cat) = super::classify_host("maven.quiltmc.org") {
            assert_eq!(
                cat,
                agora_core::network::NetworkCategory::LoaderMetadataAndContent
            );
        }
    }

    #[test]
    fn test_runtime_catalog_handle_snapshot_via_ctx() {
        let tmp = std::env::temp_dir().join(format!("agora-ctx-snap-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp);
        let ctx = agora_core::ctx::CoreContext::for_testing(tmp.clone());
        let catalog = super::runtime_catalog(&ctx).snapshot();
        assert!(
            !catalog.entries.is_empty(),
            "snapshot should have embedded entries"
        );
        // A second snapshot is independent
        let catalog2 = super::runtime_catalog(&ctx).snapshot();
        assert_eq!(catalog, catalog2);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_reload_no_registry_db_returns_warning() {
        let tmp =
            std::env::temp_dir().join(format!("agora-ctx-reload-nodb-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp);
        let ctx = agora_core::ctx::CoreContext::for_testing(tmp.clone());
        let warnings = ctx.reload_game_catalogs().unwrap();
        assert!(
            warnings
                .iter()
                .any(|w| w.contains("No registry database found")),
            "should warn when registry.db is absent: {:?}",
            warnings
        );
        // Snapshot should still return embedded catalog (unchanged).
        let catalog = super::runtime_catalog(&ctx).snapshot();
        assert!(!catalog.entries.is_empty());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_reload_runtime_catalog_with_valid_registry() {
        let tmp = std::env::temp_dir().join(format!("agora-ctx-reload-ok-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        // Create a minimal registry.db with runtime_catalog and loader_catalog tables.
        let reg_path = tmp.join("registry.db");
        let conn = rusqlite::Connection::open(&reg_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE runtime_catalog (singleton_id INTEGER PRIMARY KEY, catalog_json TEXT NOT NULL);
             CREATE TABLE loader_catalog (singleton_id INTEGER PRIMARY KEY, catalog_json TEXT NOT NULL);
             CREATE TABLE schema_version (version INTEGER PRIMARY KEY);"
        ).unwrap();

        let embedded_json = include_str!("../../../runtime-catalog/runtime_catalog.json");
        conn.execute(
            "INSERT INTO runtime_catalog (singleton_id, catalog_json) VALUES (1, ?1)",
            [embedded_json],
        )
        .unwrap();
        let manifests = include_str!("../../../loader-manifests/loader_manifests.json");
        conn.execute(
            "INSERT INTO loader_catalog (singleton_id, catalog_json) VALUES (1, ?1)",
            [manifests],
        )
        .unwrap();
        drop(conn);

        let ctx = agora_core::ctx::CoreContext::for_testing(tmp.clone());

        // Confirm we start with embedded.
        let before = super::runtime_catalog(&ctx).snapshot();
        assert!(!before.entries.is_empty());

        // Reload from the registry.db we just placed.
        let warnings = ctx.reload_game_catalogs().unwrap();
        assert!(
            warnings.iter().any(|w| w.contains("signed registry")),
            "should report signed registry load: {:?}",
            warnings
        );

        // After reload, the catalog should still be valid (same data).
        let after = super::runtime_catalog(&ctx).snapshot();
        assert!(!after.entries.is_empty());
        assert_eq!(after.schema_version, before.schema_version);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_reload_runtime_catalog_old_preserved_on_failure() {
        let tmp =
            std::env::temp_dir().join(format!("agora-ctx-reload-fail-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        // Create a registry.db with invalid runtime catalog JSON.
        let reg_path = tmp.join("registry.db");
        let conn = rusqlite::Connection::open(&reg_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE runtime_catalog (singleton_id INTEGER PRIMARY KEY, catalog_json TEXT NOT NULL);
             CREATE TABLE schema_version (version INTEGER PRIMARY KEY);"
        ).unwrap();
        conn.execute(
            "INSERT INTO runtime_catalog (singleton_id, catalog_json) VALUES (1, ?1)",
            [r#"{"invalid": "no schema version"}"#],
        )
        .unwrap();
        drop(conn);

        let ctx = agora_core::ctx::CoreContext::for_testing(tmp.clone());

        // Snapshot before reload is the embedded catalog.
        let before = super::runtime_catalog(&ctx).snapshot();
        assert!(!before.entries.is_empty());

        // Reload should fail validation but NOT replace the catalog.
        let warnings = ctx.reload_game_catalogs().unwrap();
        assert!(
            warnings.iter().any(|w| w.contains("preserving existing")),
            "should warn about preserving existing catalog: {:?}",
            warnings
        );

        // The catalog must still be the embedded one.
        let after = super::runtime_catalog(&ctx).snapshot();
        assert!(!after.entries.is_empty());
        assert_eq!(
            after, before,
            "catalog should be unchanged after failed reload"
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn classify_unknown_host_returns_none() {
        assert_eq!(super::classify_host("example.com"), None);
        assert_eq!(super::classify_host("127.0.0.1"), None);
    }

    #[test]
    fn classify_url_rejects_invalid_urls() {
        assert_eq!(super::classify_url("not-a-url"), None);
    }
}
