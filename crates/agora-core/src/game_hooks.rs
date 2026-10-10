//! Where compiled game packages plug into core's lifecycle (MASTER_SPEC §26.12).
//!
//! Core knows no game. A compiled package that still works through core's own
//! types (Minecraft's catalogs, providers and instances) attaches
//! [`CompiledServices`] to its entry in the context's
//! [`GameRegistry`](crate::game_registry::GameRegistry). Nothing here is
//! process-global: each context sees exactly the packages it was built with.
//! Package-owned state lives in [`Extensions`] on the context rather than in
//! fields core would have to name.

use crate::app_paths::AppPaths;
use crate::ctx::Ctx;
use crate::error::LauncherResult;
use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

/// Why a package's catalogs are being loaded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CatalogEvent {
    /// The context is being built.
    Startup,
    /// A fresh registry was installed; the package should replace its catalogs
    /// only if every one of them parses, and otherwise keep the active ones.
    Reload,
}

/// What a compiled package provides through core's own types. Each part moves
/// onto a `GameHost` service when another game needs the generic version of what it
/// uses (§26.12). Plugin packages have no equivalent because these are seams
/// for code that still depends on `agora-core`, not privileges.
pub trait CompiledServices: Send + Sync {
    /// Load the package's catalogs from the signed registry. `registry` is
    /// `None` when there is no usable cached registry, so the package can fall
    /// back to embedded data. Returns human-readable warnings, or an error when
    /// a catalog that parsed could not be activated: a reload reports that to
    /// its caller (so `registry sync` fails), while startup keeps going.
    fn load_catalogs(
        &self,
        _ctx: &Ctx,
        _registry: Option<&rusqlite::Connection>,
        _event: CatalogEvent,
    ) -> LauncherResult<Vec<String>> {
        Ok(Vec::new())
    }

    /// Runs once while the context is built, before any command, to recover
    /// the package's interrupted work. Returns warnings.
    fn recover_at_startup(&self, _paths: &AppPaths) -> Vec<String> {
        Vec::new()
    }

    /// The package's compiled-in content providers for a context. Plugin
    /// providers are discovered separately, by the plugin service.
    fn providers(&self, _ctx: &Ctx) -> Vec<Arc<dyn crate::providers::ContentProvider>> {
        Vec::new()
    }

    /// The package's instances, for the plugin host and repair actions.
    fn instances(&self) -> Option<Arc<dyn InstanceBackend>> {
        None
    }
}

/// The instance operations the plugin host and repair actions perform.
pub trait InstanceBackend: Send + Sync {
    fn list(&self, ctx: &Ctx) -> LauncherResult<Vec<crate::models::InstanceRow>>;
    fn get(
        &self,
        ctx: &Ctx,
        instance_id: &str,
    ) -> LauncherResult<
        Option<(
            crate::models::InstanceRow,
            Option<crate::models::InstanceManifest>,
        )>,
    >;
    fn rename(&self, ctx: &Ctx, instance_id: &str, name: &str) -> LauncherResult<()>;
    #[allow(clippy::too_many_arguments)]
    fn update_jvm(
        &self,
        ctx: &Ctx,
        instance_id: &str,
        memory_mb: i64,
        gc: &str,
        always_pre_touch: bool,
        custom_args: &str,
        memory_mode: &str,
    ) -> LauncherResult<()>;
    /// The instance's installed content, or `None` when there is no such
    /// instance. An instance without a manifest has no content.
    fn content(
        &self,
        ctx: &Ctx,
        instance_id: &str,
        content_type: Option<&str>,
    ) -> LauncherResult<Option<Vec<agora_plugin_api::dto::ContentEntry>>>;
    fn set_update_pinned(
        &self,
        ctx: &Ctx,
        instance_id: &str,
        filename: &str,
        pinned: bool,
    ) -> LauncherResult<bool>;
}

/// Every registered package's instances behind one [`InstanceBackend`].
/// Listing concatenates them; an operation on one instance goes to the backend
/// that owns it, so a second game's backend can never displace the first.
/// An instance no backend owns goes to the first one, whose own not-found
/// handling applies, exactly as with a single backend.
pub(crate) struct InstanceBackends(pub(crate) Vec<Arc<dyn InstanceBackend>>);

impl InstanceBackends {
    fn owner(&self, ctx: &Ctx, instance_id: &str) -> LauncherResult<&Arc<dyn InstanceBackend>> {
        for backend in &self.0 {
            if backend.get(ctx, instance_id)?.is_some() {
                return Ok(backend);
            }
        }
        Ok(&self.0[0])
    }
}

impl InstanceBackend for InstanceBackends {
    fn list(&self, ctx: &Ctx) -> LauncherResult<Vec<crate::models::InstanceRow>> {
        let mut all = Vec::new();
        for backend in &self.0 {
            all.extend(backend.list(ctx)?);
        }
        Ok(all)
    }

    fn get(
        &self,
        ctx: &Ctx,
        instance_id: &str,
    ) -> LauncherResult<
        Option<(
            crate::models::InstanceRow,
            Option<crate::models::InstanceManifest>,
        )>,
    > {
        for backend in &self.0 {
            if let Some(found) = backend.get(ctx, instance_id)? {
                return Ok(Some(found));
            }
        }
        Ok(None)
    }

    fn rename(&self, ctx: &Ctx, instance_id: &str, name: &str) -> LauncherResult<()> {
        self.owner(ctx, instance_id)?.rename(ctx, instance_id, name)
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
        self.owner(ctx, instance_id)?.update_jvm(
            ctx,
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
    ) -> LauncherResult<Option<Vec<agora_plugin_api::dto::ContentEntry>>> {
        self.owner(ctx, instance_id)?
            .content(ctx, instance_id, content_type)
    }

    fn set_update_pinned(
        &self,
        ctx: &Ctx,
        instance_id: &str,
        filename: &str,
        pinned: bool,
    ) -> LauncherResult<bool> {
        self.owner(ctx, instance_id)?
            .set_update_pinned(ctx, instance_id, filename, pinned)
    }
}

/// Package-owned state carried on the context, keyed by type. Clones share the
/// same store, as the rest of the context does.
#[derive(Clone, Default)]
pub struct Extensions {
    map: Arc<RwLock<HashMap<TypeId, Arc<dyn Any + Send + Sync>>>>,
}

impl Extensions {
    /// The stored value of type `T`, if any.
    pub fn get<T: Any + Send + Sync>(&self) -> Option<Arc<T>> {
        let map = self.map.read().unwrap_or_else(|e| e.into_inner());
        map.get(&TypeId::of::<T>())
            .cloned()
            .and_then(|value| value.downcast::<T>().ok())
    }

    /// Store `value`, replacing any earlier value of the same type.
    pub fn insert<T: Any + Send + Sync>(&self, value: T) {
        let mut map = self.map.write().unwrap_or_else(|e| e.into_inner());
        map.insert(TypeId::of::<T>(), Arc::new(value));
    }

    /// The stored value of type `T`, inserting `init()` first if there is none.
    pub fn get_or_insert_with<T: Any + Send + Sync>(&self, init: impl FnOnce() -> T) -> Arc<T> {
        if let Some(existing) = self.get::<T>() {
            return existing;
        }
        let mut map = self.map.write().unwrap_or_else(|e| e.into_inner());
        let value = map
            .entry(TypeId::of::<T>())
            .or_insert_with(|| Arc::new(init()))
            .clone();
        value
            .downcast::<T>()
            .unwrap_or_else(|_| unreachable!("an extension is stored under its own TypeId"))
    }
}

impl std::fmt::Debug for Extensions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let count = self.map.read().map(|m| m.len()).unwrap_or(0);
        f.debug_struct("Extensions").field("count", &count).finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extensions_store_by_type_and_share_across_clones() {
        let ext = Extensions::default();
        let clone = ext.clone();
        ext.insert(7u32);
        assert_eq!(clone.get::<u32>().as_deref(), Some(&7));
        assert!(clone.get::<String>().is_none());
        let first = clone.get_or_insert_with(|| String::from("a"));
        let second = ext.get_or_insert_with(|| String::from("b"));
        assert_eq!(*first, "a");
        assert_eq!(*second, "a", "the first insert wins");
    }

    use std::sync::Mutex;

    /// One game's instances, recording the renames it is asked to do.
    struct Backend {
        ids: Vec<&'static str>,
        renamed: Mutex<Vec<String>>,
    }

    fn row(id: &str) -> crate::models::InstanceRow {
        serde_json::from_value(serde_json::json!({
            "instance_id": id, "name": id, "minecraft_version": "", "loader": "",
            "loader_version": "", "is_modpack": false, "is_locked": false,
            "last_launched_at": null, "jvm_memory_mb": 0, "jvm_memory_mode": "",
            "jvm_gc": "", "jvm_custom_args": "", "jvm_always_pre_touch": false,
            "created_at": ""
        }))
        .unwrap()
    }

    impl InstanceBackend for Backend {
        fn list(&self, _: &Ctx) -> LauncherResult<Vec<crate::models::InstanceRow>> {
            Ok(self.ids.iter().map(|id| row(id)).collect())
        }
        fn get(
            &self,
            _: &Ctx,
            id: &str,
        ) -> LauncherResult<
            Option<(
                crate::models::InstanceRow,
                Option<crate::models::InstanceManifest>,
            )>,
        > {
            Ok(self.ids.contains(&id).then(|| (row(id), None)))
        }
        fn rename(&self, _: &Ctx, id: &str, _: &str) -> LauncherResult<()> {
            self.renamed.lock().unwrap().push(id.to_string());
            Ok(())
        }
        fn update_jvm(
            &self,
            _: &Ctx,
            _: &str,
            _: i64,
            _: &str,
            _: bool,
            _: &str,
            _: &str,
        ) -> LauncherResult<()> {
            Ok(())
        }
        fn content(
            &self,
            _: &Ctx,
            _: &str,
            _: Option<&str>,
        ) -> LauncherResult<Option<Vec<agora_plugin_api::dto::ContentEntry>>> {
            Ok(None)
        }
        fn set_update_pinned(&self, _: &Ctx, _: &str, _: &str, _: bool) -> LauncherResult<bool> {
            Ok(false)
        }
    }

    #[test]
    fn a_second_game_s_instances_never_displace_the_first() {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = crate::ctx::CoreContext::for_testing(tmp.path().to_path_buf());
        let first = Arc::new(Backend {
            ids: vec!["mc-1", "mc-2"],
            renamed: Mutex::default(),
        });
        let second = Arc::new(Backend {
            ids: vec!["sky-1"],
            renamed: Mutex::default(),
        });
        let all = InstanceBackends(vec![first.clone(), second.clone()]);

        let listed: Vec<_> = all
            .list(&ctx)
            .unwrap()
            .into_iter()
            .map(|r| r.instance_id)
            .collect();
        assert_eq!(listed, ["mc-1", "mc-2", "sky-1"]);
        assert!(all.get(&ctx, "sky-1").unwrap().is_some());
        assert!(all.get(&ctx, "nobody").unwrap().is_none());

        all.rename(&ctx, "sky-1", "x").unwrap();
        all.rename(&ctx, "mc-2", "x").unwrap();
        // An instance nobody owns goes to the first backend, whose own
        // not-found handling applies, as it did with a single backend.
        all.rename(&ctx, "nobody", "x").unwrap();
        assert_eq!(*first.renamed.lock().unwrap(), ["mc-2", "nobody"]);
        assert_eq!(*second.renamed.lock().unwrap(), ["sky-1"]);
    }
}
