//! Bounded admission control for blocking work.
//!
//! Tokio's blocking pool is the right place for filesystem scans, archive
//! parsing, and hashing, but an unbounded stream of `spawn_blocking` calls can
//! still saturate a user's disk and make an interactive launch wait behind
//! background maintenance.  This scheduler keeps those jobs on worker
//! threads while reserving capacity for interactive work.

use std::sync::Arc;

use tokio::sync::{OwnedSemaphorePermit, Semaphore};

/// Work priority used by the scheduler's admission lanes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockingPriority {
    /// Launch preparation and safety checks.  This lane may use all capacity.
    Launch,
    /// Work explicitly requested by the user.  This lane may use all capacity.
    UserInitiated,
    /// Maintenance and precomputation.  This lane is restricted to its
    /// reserved permits so it cannot consume all worker capacity.
    Background,
}

/// A small, cloneable scheduler for filesystem and CPU-heavy tasks.
///
/// The scheduler uses Tokio's blocking worker pool, but applies an explicit
/// semaphore before submitting work.  Interactive work can use every permit;
/// background work must acquire both a background permit and a total permit.
/// That leaves the interactive capacity available even when maintenance is
/// already running.
#[derive(Clone)]
pub struct TaskScheduler {
    total: Arc<Semaphore>,
    background: Arc<Semaphore>,
    total_slots: usize,
    background_slots: usize,
}

impl TaskScheduler {
    /// Create a scheduler with explicit interactive and background capacity.
    /// Zero values are normalized to one so a caller cannot accidentally make
    /// a lane permanently unable to run.
    pub fn with_limits(interactive_slots: usize, background_slots: usize) -> Self {
        let interactive_slots = interactive_slots.max(1);
        let background_slots = background_slots.max(1);
        Self {
            total: Arc::new(Semaphore::new(interactive_slots + background_slots)),
            background: Arc::new(Semaphore::new(background_slots)),
            total_slots: interactive_slots + background_slots,
            background_slots,
        }
    }

    /// Total number of blocking jobs admitted at once.
    pub fn total_slots(&self) -> usize {
        self.total_slots
    }

    /// Number of background jobs admitted at once.
    pub fn background_slots(&self) -> usize {
        self.background_slots
    }

    /// Submit a blocking task after acquiring the appropriate lane permits.
    ///
    /// The closure runs on Tokio's blocking worker pool and must therefore own
    /// all of its inputs.  A panic or runtime cancellation is returned as the
    /// normal Tokio [`JoinError`].  Callers that need cooperative cancellation
    /// should pass their existing core `CancellationToken` into the closure.
    pub async fn run_blocking<T, F>(
        &self,
        priority: BlockingPriority,
        task: F,
    ) -> Result<T, tokio::task::JoinError>
    where
        T: Send + 'static,
        F: FnOnce() -> T + Send + 'static,
    {
        let permits: Vec<OwnedSemaphorePermit> = match priority {
            BlockingPriority::Launch | BlockingPriority::UserInitiated => {
                vec![self.total.clone().acquire_owned().await.expect(
                    "TaskScheduler total semaphore must remain open for the scheduler lifetime",
                )]
            }
            BlockingPriority::Background => {
                // Acquire the background permit first.  Background tasks do
                // not queue on the total semaphore: a queued background
                // waiter could otherwise sit ahead of a newly submitted
                // launch in Tokio's fair semaphore queue.  Polling here keeps
                // interactive work ahead of maintenance whenever capacity is
                // released.
                let mut background = self.background.clone().acquire_owned().await.expect(
                    "TaskScheduler background semaphore must remain open for the scheduler lifetime",
                );
                let total = loop {
                    match self.total.clone().try_acquire_owned() {
                        Ok(permit) => break permit,
                        Err(_) => {
                            drop(background);
                            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                            background = self.background.clone().acquire_owned().await.expect(
                                "TaskScheduler background semaphore must remain open for the scheduler lifetime",
                            );
                        }
                    }
                };
                vec![background, total]
            }
        };

        tokio::task::spawn_blocking(move || {
            let _permits = permits;
            task()
        })
        .await
    }
}

impl Default for TaskScheduler {
    fn default() -> Self {
        let available = std::thread::available_parallelism()
            .map(|parallelism| parallelism.get())
            .unwrap_or(2);
        // Keep the default conservative for laptops and leave one permit for
        // maintenance without allowing maintenance to consume launch slots.
        let total_slots = available.clamp(2, 4);
        Self::with_limits(total_slots - 1, 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn background_lane_is_bounded() {
        let scheduler = TaskScheduler::with_limits(1, 1);
        let active = Arc::new(AtomicUsize::new(0));
        let maximum = Arc::new(AtomicUsize::new(0));
        let mut tasks = Vec::new();

        for _ in 0..4 {
            let scheduler = scheduler.clone();
            let active = active.clone();
            let maximum = maximum.clone();
            tasks.push(tokio::spawn(async move {
                scheduler
                    .run_blocking(BlockingPriority::Background, move || {
                        let now = active.fetch_add(1, Ordering::SeqCst) + 1;
                        maximum.fetch_max(now, Ordering::SeqCst);
                        std::thread::sleep(Duration::from_millis(15));
                        active.fetch_sub(1, Ordering::SeqCst);
                    })
                    .await
                    .unwrap();
            }));
        }

        for task in tasks {
            task.await.unwrap();
        }
        assert_eq!(maximum.load(Ordering::SeqCst), 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn interactive_work_runs_while_background_work_holds_its_slot() {
        let scheduler = TaskScheduler::with_limits(1, 1);
        let (background_started_tx, background_started_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();

        let background = tokio::spawn({
            let scheduler = scheduler.clone();
            async move {
                scheduler
                    .run_blocking(BlockingPriority::Background, move || {
                        let _ = background_started_tx.send(());
                        release_rx.recv().unwrap();
                    })
                    .await
                    .unwrap();
            }
        });
        background_started_rx.await.unwrap();

        let (interactive_started_tx, interactive_started_rx) = tokio::sync::oneshot::channel();
        let interactive = tokio::spawn({
            let scheduler = scheduler.clone();
            async move {
                scheduler
                    .run_blocking(BlockingPriority::Launch, move || {
                        let _ = interactive_started_tx.send(());
                    })
                    .await
                    .unwrap();
            }
        });

        tokio::time::timeout(Duration::from_secs(1), interactive_started_rx)
            .await
            .expect("interactive work should have a reserved slot")
            .unwrap();
        release_tx.send(()).unwrap();
        interactive.await.unwrap();
        background.await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn queued_background_work_does_not_delay_a_new_launch() {
        let scheduler = TaskScheduler::with_limits(1, 1);
        let (first_started_tx, first_started_rx) = tokio::sync::oneshot::channel();
        let (first_release_tx, first_release_rx) = std::sync::mpsc::channel();
        let first = tokio::spawn({
            let scheduler = scheduler.clone();
            async move {
                scheduler
                    .run_blocking(BlockingPriority::Launch, move || {
                        let _ = first_started_tx.send(());
                        first_release_rx.recv().unwrap();
                    })
                    .await
                    .unwrap();
            }
        });
        first_started_rx.await.unwrap();

        let (background_started_tx, background_started_rx) = tokio::sync::oneshot::channel();
        let (background_release_tx, background_release_rx) = std::sync::mpsc::channel();
        let background = tokio::spawn({
            let scheduler = scheduler.clone();
            async move {
                scheduler
                    .run_blocking(BlockingPriority::Background, move || {
                        let _ = background_started_tx.send(());
                        background_release_rx.recv().unwrap();
                    })
                    .await
                    .unwrap();
            }
        });
        tokio::time::sleep(Duration::from_millis(20)).await;

        let (second_started_tx, second_started_rx) = tokio::sync::oneshot::channel();
        let second = tokio::spawn({
            let scheduler = scheduler.clone();
            async move {
                scheduler
                    .run_blocking(BlockingPriority::Launch, move || {
                        let _ = second_started_tx.send(());
                    })
                    .await
                    .unwrap();
            }
        });
        tokio::time::sleep(Duration::from_millis(20)).await;
        first_release_tx.send(()).unwrap();

        let second_result = tokio::time::timeout(Duration::from_secs(1), second_started_rx).await;
        background_release_tx.send(()).unwrap();
        second_result
            .expect("new launch should not wait behind queued maintenance")
            .unwrap();
        tokio::time::timeout(Duration::from_secs(1), background_started_rx)
            .await
            .expect("background work should eventually acquire its reserved slot")
            .unwrap();
        first.await.unwrap();
        second.await.unwrap();
        background.await.unwrap();
    }
}
