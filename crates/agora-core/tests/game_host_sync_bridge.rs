//! Proves that an async GameHost can be driven from synchronous code inside
//! `tokio::task::spawn_blocking` without deadlocking a current-thread runtime,
//! and that dropping or timing out a call cancels it.

use agora_game_api::{
    ArchiveRequest, ArtifactReadRequest, DownloadRequest, DownloadedArtifact, ExtractedArchive,
    FomodRequest, FomodResult, GameError, GameEvent, GameFuture, GameHost, GameResult,
    InstallReadRequest, LoadOrder, SortLoadOrderRequest, ToolRunRequest, ToolRunResult,
};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

struct CancelGuard {
    completed: bool,
    cancelled: Arc<AtomicBool>,
}

impl Drop for CancelGuard {
    fn drop(&mut self) {
        if !self.completed {
            self.cancelled.store(true, Ordering::SeqCst);
        }
    }
}

struct FakeGameHost {
    download_cancelled: Arc<AtomicBool>,
    calls: Arc<AtomicUsize>,
}

impl FakeGameHost {
    fn new(download_cancelled: Arc<AtomicBool>, calls: Arc<AtomicUsize>) -> Self {
        Self {
            download_cancelled,
            calls,
        }
    }
}

impl GameHost for FakeGameHost {
    fn download(&self, _request: DownloadRequest) -> GameFuture<'_, DownloadedArtifact> {
        let cancelled = self.download_cancelled.clone();
        let calls = self.calls.clone();
        Box::pin(async move {
            calls.fetch_add(1, Ordering::SeqCst);
            let mut guard = CancelGuard {
                completed: false,
                cancelled,
            };

            // Simulate slow download operation that yields to the runtime
            tokio::time::sleep(Duration::from_millis(500)).await;

            guard.completed = true;
            Ok(DownloadedArtifact {
                id: "downloaded-1".into(),
                sha256: "abc".into(),
                source_hash_verified: true,
            })
        })
    }

    fn read_install_file(&self, _request: InstallReadRequest) -> GameFuture<'_, Vec<u8>> {
        let calls = self.calls.clone();
        Box::pin(async move {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok(b"install-content".to_vec())
        })
    }

    fn read_artifact(&self, _request: ArtifactReadRequest) -> GameFuture<'_, Vec<u8>> {
        let calls = self.calls.clone();
        Box::pin(async move {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok(b"artifact-content".to_vec())
        })
    }

    fn extract_archive(&self, _request: ArchiveRequest) -> GameFuture<'_, ExtractedArchive> {
        Box::pin(async {
            Ok(ExtractedArchive {
                content: "extracted".into(),
                files: vec!["file1".into()],
            })
        })
    }

    fn run_fomod(&self, _request: FomodRequest) -> GameFuture<'_, FomodResult> {
        Box::pin(async {
            Ok(FomodResult {
                content: "fomod-result".into(),
                choices: BTreeMap::new(),
            })
        })
    }

    fn sort_load_order(&self, request: SortLoadOrderRequest) -> GameFuture<'_, LoadOrder> {
        Box::pin(async move { Ok(request.order) })
    }

    fn run_tool(&self, _request: ToolRunRequest) -> GameFuture<'_, ToolRunResult> {
        Box::pin(async {
            Err(GameError {
                code: "not_implemented".into(),
                message: "stub".into(),
            })
        })
    }

    fn emit_event(&self, _event: GameEvent) -> GameResult<()> {
        Ok(())
    }
}

#[test]
fn test_sync_caller_can_drive_game_host_on_current_thread_runtime_without_deadlock() {
    // Explicitly use a single-threaded current_thread runtime to prove no deadlock occurs.
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build current-thread runtime");

    runtime.block_on(async {
        let handle = tokio::runtime::Handle::current();
        let cancelled = Arc::new(AtomicBool::new(false));
        let calls = Arc::new(AtomicUsize::new(0));
        let host = Arc::new(FakeGameHost::new(cancelled.clone(), calls.clone()));

        // Spawn a concurrent background task on the current-thread runtime.
        // If the blocking call deadlocked the current-thread runtime, this counter would freeze.
        let background_ticks = Arc::new(AtomicUsize::new(0));
        let bg_ticks_clone = background_ticks.clone();
        let bg_task = tokio::spawn(async move {
            for _ in 0..20 {
                tokio::time::sleep(Duration::from_millis(5)).await;
                bg_ticks_clone.fetch_add(1, Ordering::SeqCst);
            }
        });

        // 1. Successful synchronous call from spawn_blocking thread
        let host_clone = host.clone();
        let handle_clone = handle.clone();
        let sync_result = tokio::task::spawn_blocking(move || {
            // This is synchronous code (like QuickJS HostBridge dispatch)
            handle_clone.block_on(async {
                host_clone
                    .read_artifact(ArtifactReadRequest {
                        artifact: "art-1".into(),
                        max_bytes: 1024,
                    })
                    .await
            })
        })
        .await
        .expect("spawn_blocking join");

        assert_eq!(sync_result.unwrap(), b"artifact-content");
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        // 2. Timeout and cancellation test: dropping the future cancels the host operation
        let host_clone = host.clone();
        let handle_clone = handle.clone();
        let timed_out = tokio::task::spawn_blocking(move || {
            handle_clone.block_on(async {
                // Timeout after 30ms while download takes 500ms
                tokio::time::timeout(
                    Duration::from_millis(30),
                    host_clone.download(DownloadRequest {
                        url: "https://example.com/mod.zip".into(),
                        sha256: None,
                        purpose: "test".into(),
                    }),
                )
                .await
            })
        })
        .await
        .expect("spawn_blocking join");

        assert!(timed_out.is_err(), "host call must time out");
        assert!(
            cancelled.load(Ordering::SeqCst),
            "dropping the timed-out future must trigger cancellation in the host"
        );

        // Verify background task progressed on the current-thread runtime
        bg_task.await.expect("bg task join");
        assert!(
            background_ticks.load(Ordering::SeqCst) > 0,
            "current_thread runtime must remain active and not deadlock"
        );
    });
}
