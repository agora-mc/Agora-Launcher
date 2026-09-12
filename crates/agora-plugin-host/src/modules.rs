//! Module resolution, confined to one plugin's package directory.
//!
//! A plugin may `import "./util.js"` and `import "agora"`. It may not import
//! anything else — not a path outside its package, not a URL, not a bare
//! specifier hoping to hit something on disk. The resolver is the only place
//! that decides, and it decides by canonicalising and comparing against the
//! package root rather than by inspecting the string, so a symlink pointing
//! out of the package is caught along with `../../`.

use rquickjs::loader::{ImportAttributes, Loader, Resolver};
use rquickjs::{Ctx, Error, Module, Result};
use std::path::{Component, Path, PathBuf};

use crate::sdk::AGORA_MODULE_NAME;

/// Resolves relative specifiers against the plugin's package root.
pub struct PackageResolver {
    root: PathBuf,
}

impl PackageResolver {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }
}

impl Resolver for PackageResolver {
    fn resolve<'js>(
        &mut self,
        _ctx: &Ctx<'js>,
        base: &str,
        name: &str,
        _attributes: Option<ImportAttributes<'js>>,
    ) -> Result<String> {
        if name == AGORA_MODULE_NAME {
            return Ok(name.to_string());
        }
        if !(name.starts_with("./") || name.starts_with("../")) {
            // Bare specifiers are how Node finds `node_modules`. There is no
            // node_modules here, and pretending otherwise would give authors a
            // confusing "module not found" instead of the real answer.
            return Err(Error::new_resolving_message(
                base.to_string(),
                name.to_string(),
                format!(
                    "`{name}` is not resolvable: a plugin may import \"{AGORA_MODULE_NAME}\" or a \
                     relative path inside its own package. Bundle your dependencies."
                ),
            ));
        }

        // `base` is the resolved name of the importing module, which for
        // everything but the entrypoint is a package-relative path.
        let base_dir = Path::new(base).parent().unwrap_or(Path::new(""));
        let joined = normalize(&base_dir.join(name));

        // Climbing out is rejected on the logical path first, so the message
        // names the specifier the author wrote.
        if joined
            .components()
            .any(|c| matches!(c, Component::ParentDir))
        {
            return Err(Error::new_resolving_message(
                base.to_string(),
                name.to_string(),
                format!("`{name}` resolves outside the plugin package"),
            ));
        }

        let candidate = self.root.join(&joined);
        let Ok(real) = candidate.canonicalize() else {
            return Err(Error::new_resolving_message(
                base.to_string(),
                name.to_string(),
                format!("`{name}` does not exist in the plugin package"),
            ));
        };
        // Canonicalising both sides is what makes a symlink out of the package
        // fail here rather than at read time.
        let Ok(real_root) = self.root.canonicalize() else {
            return Err(Error::new_resolving_message(
                base.to_string(),
                name.to_string(),
                "the plugin package directory is unreadable".to_string(),
            ));
        };
        if !real.starts_with(&real_root) {
            return Err(Error::new_resolving_message(
                base.to_string(),
                name.to_string(),
                format!("`{name}` resolves outside the plugin package"),
            ));
        }

        Ok(joined.to_string_lossy().replace('\\', "/"))
    }
}

/// Reads modules the [`PackageResolver`] approved, plus the builtin SDK.
pub struct PackageLoader {
    root: PathBuf,
    sdk_source: &'static str,
}

impl PackageLoader {
    pub fn new(root: PathBuf, sdk_source: &'static str) -> Self {
        Self { root, sdk_source }
    }
}

impl Loader for PackageLoader {
    fn load<'js>(
        &mut self,
        ctx: &Ctx<'js>,
        name: &str,
        _attributes: Option<ImportAttributes<'js>>,
    ) -> Result<Module<'js, rquickjs::module::Declared>> {
        if name == AGORA_MODULE_NAME {
            return Module::declare(ctx.clone(), name, self.sdk_source);
        }
        let path = self.root.join(name);
        let source = std::fs::read_to_string(&path).map_err(|e| {
            Error::new_loading_message(name.to_string(), format!("could not read module: {e}"))
        })?;
        Module::declare(ctx.clone(), name, source)
    }
}

/// Collapse `.` segments without touching the filesystem.
///
/// `..` is deliberately *kept* rather than collapsed: the caller rejects any
/// path that still contains one, and collapsing first would let
/// `a/../../b` look like `b`.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_drops_current_dir_segments() {
        assert_eq!(normalize(Path::new("./a/./b.js")), PathBuf::from("a/b.js"));
    }

    #[test]
    fn normalize_keeps_a_parent_ref_that_escapes_the_root() {
        // "a/../../b" must not quietly become "b".
        assert_eq!(normalize(Path::new("a/../../b")), PathBuf::from("../b"));
    }

    #[test]
    fn normalize_collapses_a_parent_ref_that_stays_inside() {
        assert_eq!(normalize(Path::new("a/b/../c.js")), PathBuf::from("a/c.js"));
    }
}
