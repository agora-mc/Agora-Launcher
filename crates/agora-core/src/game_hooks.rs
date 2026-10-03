//! Where game packages plug into core's lifecycle (MASTER_SPEC §26.12).
//!
//! Core knows no game. A package that needs to load data from the signed
//! registry (Minecraft's loader and Java runtime catalogs, for example) or to
//! recover its own interrupted work at startup registers a hook here before the
//! adapter builds its [`Ctx`](crate::ctx::Ctx). Package-owned state lives in
//! [`Extensions`] on the context rather than in fields core would have to name.

use crate::app_paths::AppPaths;
use crate::ctx::Ctx;
use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock, RwLock};

/// Why a catalog hook is being run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CatalogEvent {
    /// The context is being built.
    Startup,
    /// A fresh registry was installed; the hook should replace its catalogs
    /// only if every one of them parses, and otherwise keep the active ones.
    Reload,
}

/// Loads a package's catalogs from the signed registry. `registry` is `None`
/// when there is no usable cached registry, so the package can fall back to
/// embedded data. Returns human-readable warnings, or an error when a catalog
/// that parsed could not be activated: a reload reports that to its caller
/// (so `registry sync` fails), while startup keeps going on embedded data.
pub type CatalogHook = fn(
    ctx: &Ctx,
    registry: Option<&rusqlite::Connection>,
    event: CatalogEvent,
) -> crate::error::LauncherResult<Vec<String>>;

/// Runs once while the context is built, before any command. Returns warnings.
pub type StartupHook = fn(paths: &AppPaths) -> Vec<String>;

/// Builds one of a package's compiled-in content providers for a context.
/// Plugin providers are discovered separately, by the plugin service.
pub type ProviderFactory = fn(ctx: &Ctx) -> Arc<dyn crate::providers::ContentProvider>;

/// The instance operations the plugin host and repair actions perform. One
/// game package supplies them today; per-game routing arrives with the second
/// game (Phase 2).
pub trait InstanceBackend: Send + Sync {
    fn list(&self, ctx: &Ctx) -> crate::error::LauncherResult<Vec<crate::models::InstanceRow>>;
    fn get(
        &self,
        ctx: &Ctx,
        instance_id: &str,
    ) -> crate::error::LauncherResult<
        Option<(
            crate::models::InstanceRow,
            Option<crate::models::InstanceManifest>,
        )>,
    >;
    fn rename(&self, ctx: &Ctx, instance_id: &str, name: &str) -> crate::error::LauncherResult<()>;
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
    ) -> crate::error::LauncherResult<()>;
    /// The instance's installed content, or `None` when there is no such
    /// instance. An instance without a manifest has no content.
    fn content(
        &self,
        ctx: &Ctx,
        instance_id: &str,
        content_type: Option<&str>,
    ) -> crate::error::LauncherResult<Option<Vec<agora_plugin_api::dto::ContentEntry>>>;
    fn set_update_pinned(
        &self,
        ctx: &Ctx,
        instance_id: &str,
        filename: &str,
        pinned: bool,
    ) -> crate::error::LauncherResult<bool>;
}

static INSTANCE_BACKEND: RwLock<Option<Arc<dyn InstanceBackend>>> = RwLock::new(None);

/// Install the instance backend. Replaces any earlier one.
pub fn set_instance_backend(backend: Arc<dyn InstanceBackend>) {
    *INSTANCE_BACKEND.write().unwrap_or_else(|e| e.into_inner()) = Some(backend);
}

/// The installed instance backend, or an error naming the missing setup.
pub fn instance_backend() -> crate::error::LauncherResult<Arc<dyn InstanceBackend>> {
    INSTANCE_BACKEND
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
        .ok_or_else(|| crate::error::LauncherError::Generic {
            code: "ERR_NO_GAME_PACKAGE".into(),
            message: "No game package is registered to handle instances.".into(),
        })
}

#[derive(Default)]
struct Hooks {
    catalog: Vec<CatalogHook>,
    startup: Vec<StartupHook>,
    providers: Vec<ProviderFactory>,
}

fn hooks() -> &'static Mutex<Hooks> {
    static HOOKS: OnceLock<Mutex<Hooks>> = OnceLock::new();
    HOOKS.get_or_init(|| Mutex::new(Hooks::default()))
}

/// Register a catalog hook. Registering the same function twice is a no-op, so
/// adapters and tests can call a package's `register` freely.
pub fn register_catalog_hook(hook: CatalogHook) {
    let mut hooks = hooks().lock().unwrap_or_else(|e| e.into_inner());
    if !hooks.catalog.iter().any(|h| *h as usize == hook as usize) {
        hooks.catalog.push(hook);
    }
}

/// Register a startup hook. Idempotent, like [`register_catalog_hook`].
pub fn register_startup_hook(hook: StartupHook) {
    let mut hooks = hooks().lock().unwrap_or_else(|e| e.into_inner());
    if !hooks.startup.iter().any(|h| *h as usize == hook as usize) {
        hooks.startup.push(hook);
    }
}

/// Register a compiled-in content provider. Idempotent; providers keep the
/// order they were first registered in.
pub fn register_provider(factory: ProviderFactory) {
    let mut hooks = hooks().lock().unwrap_or_else(|e| e.into_inner());
    if !hooks
        .providers
        .iter()
        .any(|h| *h as usize == factory as usize)
    {
        hooks.providers.push(factory);
    }
}

pub(crate) fn builtin_providers(ctx: &Ctx) -> Vec<Arc<dyn crate::providers::ContentProvider>> {
    let providers = hooks()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .providers
        .clone();
    providers.iter().map(|factory| factory(ctx)).collect()
}

/// Run every catalog hook. All hooks run even if one fails; the first error
/// is returned after the others have had their turn.
pub(crate) fn run_catalog_hooks(
    ctx: &Ctx,
    registry: Option<&rusqlite::Connection>,
    event: CatalogEvent,
) -> crate::error::LauncherResult<Vec<String>> {
    let catalog = hooks()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .catalog
        .clone();
    let mut warnings = Vec::new();
    let mut first_error = None;
    for hook in &catalog {
        match hook(ctx, registry, event) {
            Ok(mut more) => warnings.append(&mut more),
            Err(error) => {
                first_error.get_or_insert(error);
            }
        }
    }
    match first_error {
        Some(error) => Err(error),
        None => Ok(warnings),
    }
}

/// Whether any game package has registered with core. Core runs without one,
/// but an adapter that forgot to register would lose every game's catalogs
/// and providers without an error, so startup says so.
pub fn any_package_registered() -> bool {
    let hooks = hooks().lock().unwrap_or_else(|e| e.into_inner());
    !hooks.catalog.is_empty() || !hooks.providers.is_empty() || !hooks.startup.is_empty()
}

pub(crate) fn run_startup_hooks(paths: &AppPaths) -> Vec<String> {
    let startup = hooks()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .startup
        .clone();
    startup.iter().flat_map(|hook| hook(paths)).collect()
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
}
