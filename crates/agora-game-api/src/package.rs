use crate::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageDefinition {
    pub id: String,
    pub version: semver::Version,
    pub api_range: semver::VersionReq,
    /// Optional toolchain parents; games can stand alone.
    pub parents: Vec<String>,
    pub games: Vec<GameDefinition>,
    pub frameworks: Vec<FrameworkDefinition>,
    pub tools: Vec<ToolDefinition>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameContext {
    pub instance: String,
    pub runtime: RuntimeIdentity,
    pub base: Option<BaseReference>,
    pub frameworks: Vec<InstalledFramework>,
    pub layers: LayerStack,
    pub load_order: LoadOrder,
    pub settings: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContentClassification {
    pub file: RelPath,
    pub destination: RelPath,
    pub content_kind: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagnostic {
    pub code: String,
    pub message: String,
    pub severity: DiagnosticSeverity,
    pub repairs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticSeverity {
    Info,
    Warning,
    Error,
}

/// One registry entry point for compiled and script-backed packages. Returning
/// None asks core to use declarative data; optional behaviour needs no private
/// access to services. Core, not the package, spawns a prepared launch recipe.
pub trait GamePackage: Send + Sync {
    fn definition(&self) -> &PackageDefinition;

    fn detect_runtime<'a>(
        &'a self,
        _host: &'a dyn GameHost,
        _install: GameInstall,
    ) -> GameFuture<'a, Option<RuntimeIdentity>> {
        Box::pin(async { Ok(None) })
    }

    fn classify<'a>(
        &'a self,
        _host: &'a dyn GameHost,
        _context: GameContext,
        _archive: ExtractedArchive,
    ) -> GameFuture<'a, Option<Vec<ContentClassification>>> {
        Box::pin(async { Ok(None) })
    }

    fn load_order_rules<'a>(
        &'a self,
        _host: &'a dyn GameHost,
        _context: GameContext,
    ) -> GameFuture<'a, Option<Vec<LoadOrderRule>>> {
        Box::pin(async { Ok(None) })
    }

    fn diagnostics<'a>(
        &'a self,
        _host: &'a dyn GameHost,
        _context: GameContext,
    ) -> GameFuture<'a, Vec<Diagnostic>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    fn prepare_launch<'a>(
        &'a self,
        _host: &'a dyn GameHost,
        _context: GameContext,
    ) -> GameFuture<'a, Option<LaunchRecipe>> {
        Box::pin(async { Ok(None) })
    }
}

// These signatures fail to compile if either interface stops being object-safe.
const _: Option<&dyn GameHost> = None;
const _: Option<&dyn GamePackage> = None;
