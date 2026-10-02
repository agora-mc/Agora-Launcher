use crate::{GamePath, InstallId, LayerId, LoadOrder, ToolId};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, future::Future, pin::Pin};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("{code}: {message}")]
pub struct GameError {
    pub code: String,
    pub message: String,
}

pub type GameResult<T> = Result<T, GameError>;
/// Object-safe async operations without tying packages to Tokio or async-trait.
pub type GameFuture<'a, T> = Pin<Box<dyn Future<Output = GameResult<T>> + Send + 'a>>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DownloadRequest {
    pub url: String,
    pub sha256: Option<String>,
    /// Host interprets the purpose against policy, never as a grant itself.
    pub purpose: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DownloadedArtifact {
    pub id: String,
    pub sha256: String,
    pub source_hash_verified: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstallReadRequest {
    pub install: InstallId,
    pub path: crate::RelPath,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactReadRequest {
    pub artifact: String,
    pub max_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArchiveRequest {
    pub artifact: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtractedArchive {
    pub content: String,
    pub files: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FomodRequest {
    pub content: String,
    /// Saved choices for deterministic replay. None asks the host's own UI.
    pub choices: Option<BTreeMap<String, Vec<String>>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FomodResult {
    pub content: String,
    pub choices: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SortLoadOrderRequest {
    pub engine: String,
    pub order: LoadOrder,
    pub roots: Vec<GamePath>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolRunRequest {
    pub instance: String,
    /// Host resolves a registered definition; no arbitrary executable here.
    pub tool: ToolId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolRunResult {
    pub run: String,
    pub generated_layer: LayerId,
    pub input_fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameEvent {
    pub name: String,
    pub fields: BTreeMap<String, String>,
}

/// Core supplies a host scoped to the calling package/instance and its grants.
/// Every operation checks that scope (including relative paths), cancellation
/// and policy. Tool execution owns snapshots, staging, promotion and process
/// tracking; extraction and load-order algorithms stay in core.
/// These method names are also the future script HostBridge method table.
pub trait GameHost: Send + Sync {
    /// Verify the source hash when present; surface missing source hashes to the
    /// user through the host's download/provenance flow when absent.
    fn download(&self, request: DownloadRequest) -> GameFuture<'_, DownloadedArtifact>;
    fn read_install_file(&self, request: InstallReadRequest) -> GameFuture<'_, Vec<u8>>;
    /// Metadata downloads must be readable by a package without giving it a
    /// cache path or unchecked network access (e.g. Minecraft version JSON).
    fn read_artifact(&self, request: ArtifactReadRequest) -> GameFuture<'_, Vec<u8>>;
    fn extract_archive(&self, request: ArchiveRequest) -> GameFuture<'_, ExtractedArchive>;
    fn run_fomod(&self, request: FomodRequest) -> GameFuture<'_, FomodResult>;
    fn sort_load_order(&self, request: SortLoadOrderRequest) -> GameFuture<'_, LoadOrder>;
    fn run_tool(&self, request: ToolRunRequest) -> GameFuture<'_, ToolRunResult>;
    fn emit_event(&self, event: GameEvent) -> GameResult<()>;
}
