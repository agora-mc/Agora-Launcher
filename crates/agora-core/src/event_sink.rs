//! Canonical core-owned primitives for progress events, core events,
//! cancellation, and event sinks.
//!
//! No Tauri/CLI/MCP formatting happens here. Adapters implement the sink
//! traits to forward events to their respective transport.
//!
//! # Authority
//!
//! This module is the **single authoritative home** for:
//! - [`CancellationToken`] (formerly in `install_pipeline`)
//! - [`ProgressEvent`] and [`ProgressPhase`] (merged from both `event_sink`
//!   and `install_pipeline`)
//! - [`ProgressSink`] (the canonical progress sink trait)
//! - [`EventSink`] for core events
//!
//! `install_pipeline` now re-exports `CancellationToken` from here.
//! `ProgressReporter` is a **compatibility adapter** wrapping `ProgressSink`;
//! new code should use `ProgressSink` directly.

use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

// ---------------------------------------------------------------------------
// CancellationToken
// ---------------------------------------------------------------------------

/// A simple cancellation flag, shareable across threads.
///
/// Cloning shares the same underlying flag — cancelling one cancels all clones.
#[derive(Clone, Debug)]
pub struct CancellationToken(Arc<AtomicBool>);

impl CancellationToken {
    pub fn new() -> Self {
        Self(Arc::new(AtomicBool::new(false)))
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

impl Default for CancellationToken {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Operation identity
// ---------------------------------------------------------------------------

/// An opaque, transport-safe operation identifier.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct OperationId(pub String);

impl OperationId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }
}

// ---------------------------------------------------------------------------
// Progress events (merged model)
// ---------------------------------------------------------------------------

/// Phase of a multi-step operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProgressPhase {
    Resolving,
    Staging,
    Verifying,
    Snapshotting,
    Applying,
    Cleaning,
    HealthScan,
    Downloading,
    Extracting,
    Installing,
    Done,
    Failed,
    Cancelled,
    Idle,
}

/// Serializable progress event emitted during long-running operations.
///
/// Carries fields from both the original event_sink model (operation_id,
/// sub_label, progress fraction) and the install_pipeline model (plan_id,
/// step/total steps, bytes). Optional fields are `None` when not relevant
/// for the current phase.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgressEvent {
    /// The operation this event belongs to (event_sink model).
    pub operation_id: OperationId,
    /// Current phase of the operation.
    pub phase: ProgressPhase,
    /// Human-readable label (e.g. "Downloading Sodium 0.5.8").
    pub message: String,
    /// Optional progress fraction (0.0 — 1.0, event_sink model).
    pub progress: Option<f64>,
    /// Optional sub-operation label (event_sink model).
    pub sub_label: Option<String>,
    /// Install-plan ID (install_pipeline model).
    pub plan_id: Option<String>,
    /// Current step number in multi-step plan (install_pipeline model).
    pub step: Option<u32>,
    /// Total steps in multi-step plan (install_pipeline model).
    pub total_steps: Option<u32>,
    /// Bytes downloaded so far (install_pipeline model).
    pub bytes_downloaded: Option<u64>,
    /// Total bytes to download (install_pipeline model).
    pub bytes_total: Option<u64>,
}

impl ProgressEvent {
    /// Create a minimal progress event with only operation_id, phase, and message.
    pub fn new(
        operation_id: OperationId,
        phase: ProgressPhase,
        message: impl Into<String>,
    ) -> Self {
        Self {
            operation_id,
            phase,
            message: message.into(),
            progress: None,
            sub_label: None,
            plan_id: None,
            step: None,
            total_steps: None,
            bytes_downloaded: None,
            bytes_total: None,
        }
    }

    pub fn with_progress(mut self, progress: f64) -> Self {
        self.progress = Some(progress.clamp(0.0, 1.0));
        self
    }

    pub fn with_sub_label(mut self, label: impl Into<String>) -> Self {
        self.sub_label = Some(label.into());
        self
    }

    pub fn with_plan(mut self, plan_id: impl Into<String>, step: u32, total_steps: u32) -> Self {
        self.plan_id = Some(plan_id.into());
        self.step = Some(step);
        self.total_steps = Some(total_steps);
        self
    }

    pub fn with_bytes(mut self, downloaded: u64, total: u64) -> Self {
        self.bytes_downloaded = Some(downloaded);
        self.bytes_total = Some(total);
        self
    }
}

// ---------------------------------------------------------------------------
// Core events (notifications, not progress)
// ---------------------------------------------------------------------------

/// Significant events emitted by core operations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CoreEvent {
    RegistrySync {
        status: EventStatus,
        message: String,
        new_tag: Option<String>,
    },
    Launch {
        operation_id: OperationId,
        instance_id: String,
        status: EventStatus,
        pid: Option<u32>,
    },
    Instance {
        operation_id: OperationId,
        instance_id: String,
        status: EventStatus,
        message: String,
    },
    ModOperation {
        operation_id: OperationId,
        instance_id: String,
        action: ModAction,
        status: EventStatus,
        message: String,
    },
    JavaRuntime {
        major: u32,
        status: EventStatus,
        message: String,
    },
    Warning {
        message: String,
        details: Option<String>,
    },
    Error {
        code: String,
        message: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EventStatus {
    Started,
    Progress,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ModAction {
    Install,
    Update,
    Remove,
    Disable,
    Enable,
}

// ---------------------------------------------------------------------------
// Sink traits
// ---------------------------------------------------------------------------

/// Trait for accepting progress events (canonical progress sink).
///
/// Implementations must be thread-safe (`Send + Sync`) since core operations
/// may emit events from background threads.
pub trait ProgressSink: Send + Sync {
    fn report(&self, event: ProgressEvent);
}

/// Trait for accepting core events.
pub trait EventSink: Send + Sync {
    fn emit(&self, event: CoreEvent);
}

// ---------------------------------------------------------------------------
// ProgressReporter compatibility adapter
// ---------------------------------------------------------------------------

/// Compatibility adapter that wraps a [`ProgressSink`] as a [`ProgressReporter`].
///
/// `ProgressReporter` was the original trait in `install_pipeline`. This adapter
/// lets existing `&dyn ProgressReporter` callers work with the canonical
/// `ProgressSink` without changing every call site. New code should use
/// `ProgressSink` directly.
pub struct ProgressReporterAdapter {
    pub inner: Arc<dyn ProgressSink>,
}

impl ProgressReporterAdapter {
    pub fn new(inner: Arc<dyn ProgressSink>) -> Self {
        Self { inner }
    }
}

// ---------------------------------------------------------------------------
// Noop implementations (safe defaults)
// ---------------------------------------------------------------------------

/// A progress sink that discards all events.
pub struct NoopProgressSink;

impl ProgressSink for NoopProgressSink {
    fn report(&self, _event: ProgressEvent) {}
}

/// An event sink that discards all events.
pub struct NoopEventSink;

impl EventSink for NoopEventSink {
    fn emit(&self, _event: CoreEvent) {}
}

// ---------------------------------------------------------------------------
// Collecting implementations (for tests)
// ---------------------------------------------------------------------------

/// A progress sink that collects all events into a `Vec`.
pub struct CollectingProgressSink {
    events: Arc<std::sync::Mutex<Vec<ProgressEvent>>>,
}

impl Default for CollectingProgressSink {
    fn default() -> Self {
        Self::new()
    }
}

impl CollectingProgressSink {
    pub fn new() -> Self {
        Self {
            events: Arc::new(std::sync::Mutex::new(Vec::new())),
        }
    }

    pub fn events(&self) -> Vec<ProgressEvent> {
        self.events.lock().unwrap().clone()
    }

    pub fn clear(&self) {
        self.events.lock().unwrap().clear();
    }
}

impl ProgressSink for CollectingProgressSink {
    fn report(&self, event: ProgressEvent) {
        self.events.lock().unwrap().push(event);
    }
}

/// An event sink that collects all events into a `Vec`.
pub struct CollectingEventSink {
    events: Arc<std::sync::Mutex<Vec<CoreEvent>>>,
}

impl Default for CollectingEventSink {
    fn default() -> Self {
        Self::new()
    }
}

impl CollectingEventSink {
    pub fn new() -> Self {
        Self {
            events: Arc::new(std::sync::Mutex::new(Vec::new())),
        }
    }

    pub fn events(&self) -> Vec<CoreEvent> {
        self.events.lock().unwrap().clone()
    }
}

impl EventSink for CollectingEventSink {
    fn emit(&self, event: CoreEvent) {
        self.events.lock().unwrap().push(event);
    }
}

// ---------------------------------------------------------------------------
// Tee implementation (multicast to multiple sinks)
// ---------------------------------------------------------------------------

/// A progress sink that fans out to multiple inner sinks.
pub struct TeeProgressSink {
    sinks: Vec<Box<dyn ProgressSink>>,
}

impl TeeProgressSink {
    pub fn new(sinks: Vec<Box<dyn ProgressSink>>) -> Self {
        Self { sinks }
    }
}

impl ProgressSink for TeeProgressSink {
    fn report(&self, event: ProgressEvent) {
        for sink in &self.sinks {
            sink.report(event.clone());
        }
    }
}

/// An event sink that fans out to multiple inner sinks.
pub struct TeeEventSink {
    sinks: Vec<Box<dyn EventSink>>,
}

impl TeeEventSink {
    pub fn new(sinks: Vec<Box<dyn EventSink>>) -> Self {
        Self { sinks }
    }
}

impl EventSink for TeeEventSink {
    fn emit(&self, event: CoreEvent) {
        for sink in &self.sinks {
            sink.emit(event.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cancellation_token_roundtrip() {
        let t = CancellationToken::new();
        assert!(!t.is_cancelled());
        t.cancel();
        assert!(t.is_cancelled());
    }

    #[test]
    fn test_cancellation_token_clone_shares_state() {
        let t1 = CancellationToken::new();
        let t2 = t1.clone();
        t1.cancel();
        assert!(t2.is_cancelled());
    }

    #[test]
    fn test_noop_sink_does_not_panic() {
        let sink = NoopProgressSink;
        sink.report(ProgressEvent::new(
            OperationId::new("test"),
            ProgressPhase::Idle,
            "noop",
        ));
    }

    #[test]
    fn test_collecting_sink_records_events() {
        let sink = CollectingProgressSink::new();
        assert!(sink.events().is_empty());

        let ev = ProgressEvent::new(OperationId::new("op1"), ProgressPhase::Downloading, "dl");
        sink.report(ev);

        let events = sink.events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].operation_id, OperationId::new("op1"));
        assert_eq!(events[0].phase, ProgressPhase::Downloading);
    }

    #[test]
    fn test_progress_event_with_progress() {
        let ev = ProgressEvent::new(OperationId::new("op"), ProgressPhase::Staging, "stage")
            .with_progress(0.5);
        assert!((ev.progress.unwrap() - 0.5).abs() < 1e-6);
    }

    #[test]
    fn test_progress_event_with_plan() {
        let ev = ProgressEvent::new(OperationId::new("op"), ProgressPhase::Staging, "stage")
            .with_plan("plan-1", 2, 5);
        assert_eq!(ev.plan_id, Some("plan-1".into()));
        assert_eq!(ev.step, Some(2));
        assert_eq!(ev.total_steps, Some(5));
    }

    #[test]
    fn test_progress_event_with_bytes() {
        let ev = ProgressEvent::new(OperationId::new("op"), ProgressPhase::Downloading, "dl")
            .with_bytes(500, 1000);
        assert_eq!(ev.bytes_downloaded, Some(500));
        assert_eq!(ev.bytes_total, Some(1000));
    }

    #[test]
    fn test_progress_event_clamps_progress() {
        let ev = ProgressEvent::new(OperationId::new("op"), ProgressPhase::Staging, "test")
            .with_progress(1.5);
        assert!((ev.progress.unwrap() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_collecting_event_sink() {
        let sink = CollectingEventSink::new();
        sink.emit(CoreEvent::Warning {
            message: "test warning".into(),
            details: None,
        });
        assert_eq!(sink.events().len(), 1);
    }

    #[test]
    fn test_progress_phase_serialize_roundtrip() {
        for phase in &[
            ProgressPhase::Resolving,
            ProgressPhase::Staging,
            ProgressPhase::Applying,
            ProgressPhase::HealthScan,
            ProgressPhase::Done,
            ProgressPhase::Failed,
            ProgressPhase::Cancelled,
            ProgressPhase::Idle,
        ] {
            let json = serde_json::to_string(phase).unwrap();
            let back: ProgressPhase = serde_json::from_str(&json).unwrap();
            assert_eq!(*phase, back);
        }
    }

    #[test]
    fn test_event_status_serialize() {
        for status in &[
            EventStatus::Started,
            EventStatus::Completed,
            EventStatus::Failed,
            EventStatus::Cancelled,
        ] {
            let json = serde_json::to_string(status).unwrap();
            let back: EventStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(*status, back);
        }
    }
}
