//! Progress tracking for the document processing pipeline.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, RwLock};
use tokio::time::MissedTickBehavior;

use super::types::Stage;
use super::AppEvent;

/// How often buffered progress snapshots are forwarded to the application.
const PROGRESS_FLUSH_INTERVAL: Duration = Duration::from_millis(50);

/// Progress for a single processing stage.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StageProgress {
    pub pending: usize,
    pub active: usize,
    pub completed: usize,
    pub failed: usize,
}

impl StageProgress {
    /// Total documents that entered this stage.
    pub fn total(&self) -> usize {
        self.pending + self.active + self.completed + self.failed
    }

    /// Check if stage has any active work.
    pub fn is_active(&self) -> bool {
        self.pending > 0 || self.active > 0
    }
}

/// Progress for a collection across all pipeline stages.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PipelineProgress {
    pub collection_id: String,
    pub store: StageProgress,
    pub extract: StageProgress,
    pub index: StageProgress,
}

impl PipelineProgress {
    pub fn new(collection_id: String) -> Self {
        Self {
            collection_id,
            ..Default::default()
        }
    }

    /// Get mutable reference to stage progress.
    fn stage_mut(&mut self, stage: Stage) -> &mut StageProgress {
        match stage {
            Stage::Store => &mut self.store,
            Stage::Extract => &mut self.extract,
            Stage::Index => &mut self.index,
        }
    }

    /// Check if any stage has active work.
    pub fn is_active(&self) -> bool {
        self.store.is_active() || self.extract.is_active() || self.index.is_active()
    }
}

/// Tracks progress for all collections.
///
/// Every stage transition is `enqueue -> start -> finish_ok|finish_err`, in
/// that order, exactly once per job. `pending` counts enqueued jobs not yet
/// started, `active` counts started jobs without an outcome, and
/// `pending + active + completed + failed` is the number of enqueues.
#[derive(Clone)]
pub struct ProgressTracker {
    collections: Arc<RwLock<HashMap<String, PipelineProgress>>>,
    /// Channel to the task that coalesces progress before forwarding it.
    notify_tx: mpsc::UnboundedSender<PipelineProgress>,
}

impl ProgressTracker {
    pub fn new(events: mpsc::UnboundedSender<AppEvent>) -> Self {
        let (notify_tx, notify_rx) = mpsc::unbounded_channel();
        tokio::spawn(forward_progress(notify_rx, events));
        Self {
            collections: Arc::new(RwLock::new(HashMap::new())),
            notify_tx,
        }
    }

    /// Record that a job was queued for processing.
    pub async fn enqueue(&self, collection_id: &str, stage: Stage) {
        let mut collections = self.collections.write().await;
        let progress = collections
            .entry(collection_id.to_string())
            .or_insert_with(|| PipelineProgress::new(collection_id.to_string()));
        progress.stage_mut(stage).pending += 1;
        self.notify(progress);
    }

    /// Record that a queued job was received and started.
    pub async fn start(&self, collection_id: &str, stage: Stage) {
        let mut collections = self.collections.write().await;
        let progress = collections.get_mut(collection_id).unwrap_or_else(|| {
            panic!(
                "pipeline invariant violated: {stage} job started for untracked collection \
                 {collection_id}"
            )
        });
        let stage_progress = progress.stage_mut(stage);
        assert!(
            stage_progress.pending > 0,
            "pipeline invariant violated: {stage} job started with no pending job for \
             collection {collection_id}"
        );
        stage_progress.pending -= 1;
        stage_progress.active += 1;
        self.notify(progress);
    }

    /// Record that a started job completed successfully.
    pub async fn finish_ok(&self, collection_id: &str, stage: Stage) {
        self.finish(collection_id, stage, false).await;
    }

    /// Record that a started job failed.
    pub async fn finish_err(&self, collection_id: &str, stage: Stage) {
        self.finish(collection_id, stage, true).await;
    }

    async fn finish(&self, collection_id: &str, stage: Stage, failed: bool) {
        let mut collections = self.collections.write().await;
        let progress = collections.get_mut(collection_id).unwrap_or_else(|| {
            panic!(
                "pipeline invariant violated: {stage} job finished for untracked collection \
                 {collection_id}"
            )
        });
        let stage_progress = progress.stage_mut(stage);
        assert!(
            stage_progress.active > 0,
            "pipeline invariant violated: {stage} job finished with no active job for \
             collection {collection_id}"
        );
        stage_progress.active -= 1;
        if failed {
            stage_progress.failed += 1;
        } else {
            stage_progress.completed += 1;
        }
        self.notify(progress);
    }

    fn notify(&self, progress: &PipelineProgress) {
        let _ = self.notify_tx.send(progress.clone());
    }

    /// Get progress for a collection.
    pub async fn get(&self, collection_id: &str) -> Option<PipelineProgress> {
        self.collections.read().await.get(collection_id).cloned()
    }

    /// Get progress for all active collections.
    pub async fn get_all_active(&self) -> Vec<PipelineProgress> {
        self.collections
            .read()
            .await
            .values()
            .filter(|p| p.is_active())
            .cloned()
            .collect()
    }

    /// Remove a collection from tracking.
    pub async fn remove(&self, collection_id: &str) {
        self.collections.write().await.remove(collection_id);
    }
}

/// Coalesce active snapshots by collection until the next flush, but forward
/// an idle snapshot immediately and discard any buffered active state for it.
async fn forward_progress(
    mut updates: mpsc::UnboundedReceiver<PipelineProgress>,
    events: mpsc::UnboundedSender<AppEvent>,
) {
    let mut latest = HashMap::new();
    let mut interval = tokio::time::interval_at(
        tokio::time::Instant::now() + PROGRESS_FLUSH_INTERVAL,
        PROGRESS_FLUSH_INTERVAL,
    );
    interval.set_missed_tick_behavior(MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            update = updates.recv() => {
                let Some(progress) = update else {
                    flush(&mut latest, &events);
                    break;
                };
                observe(&mut latest, &events, progress);
            }
            _ = interval.tick() => {
                while let Ok(progress) = updates.try_recv() {
                    observe(&mut latest, &events, progress);
                }
                flush(&mut latest, &events);
            }
        }
    }
}

fn observe(
    latest: &mut HashMap<String, PipelineProgress>,
    events: &mpsc::UnboundedSender<AppEvent>,
    progress: PipelineProgress,
) {
    if progress.is_active() {
        latest.insert(progress.collection_id.clone(), progress);
    } else {
        latest.remove(&progress.collection_id);
        let _ = events.send(AppEvent::Progress(progress));
    }
}

fn flush(latest: &mut HashMap<String, PipelineProgress>, events: &mpsc::UnboundedSender<AppEvent>) {
    for (_, progress) in latest.drain() {
        let _ = events.send(AppEvent::Progress(progress));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tracker() -> (ProgressTracker, mpsc::UnboundedReceiver<AppEvent>) {
        let (events_tx, events_rx) = mpsc::unbounded_channel();
        (ProgressTracker::new(events_tx), events_rx)
    }

    /// A successful job moves from pending to active to completed and leaves
    /// the stage idle with every job accounted for.
    #[tokio::test]
    async fn successful_job_moves_through_each_counter_once() {
        let (progress, _events) = tracker();
        let collection_id = "collection";

        progress.enqueue(collection_id, Stage::Extract).await;
        let current = progress
            .get(collection_id)
            .await
            .expect("collection progress");
        assert_eq!(current.extract.pending, 1);
        assert_eq!(current.extract.active, 0);
        assert_eq!(current.extract.total(), 1);

        progress.start(collection_id, Stage::Extract).await;
        let current = progress
            .get(collection_id)
            .await
            .expect("collection progress");
        assert_eq!(current.extract.pending, 0);
        assert_eq!(current.extract.active, 1);
        assert!(current.extract.is_active());

        progress.finish_ok(collection_id, Stage::Extract).await;
        let current = progress
            .get(collection_id)
            .await
            .expect("collection progress");
        assert_eq!(current.extract.pending, 0);
        assert_eq!(current.extract.active, 0);
        assert_eq!(current.extract.completed, 1);
        assert_eq!(current.extract.failed, 0);
        assert_eq!(current.extract.total(), 1);
        assert!(!current.extract.is_active());
        assert!(!current.is_active());
        assert!(progress.get_all_active().await.is_empty());
    }

    /// A failed job also has exactly one terminal outcome and leaves the
    /// stage idle.
    #[tokio::test]
    async fn failed_job_has_one_terminal_outcome_and_settles_idle() {
        let (progress, _events) = tracker();
        let collection_id = "collection";

        progress.enqueue(collection_id, Stage::Index).await;
        progress.start(collection_id, Stage::Index).await;
        progress.finish_err(collection_id, Stage::Index).await;

        let current = progress
            .get(collection_id)
            .await
            .expect("collection progress");
        assert_eq!(current.index.pending, 0);
        assert_eq!(current.index.active, 0);
        assert_eq!(current.index.completed, 0);
        assert_eq!(current.index.failed, 1);
        assert_eq!(current.index.total(), 1);
        assert!(!current.is_active());
        assert!(progress.get_all_active().await.is_empty());
    }

    /// Starting a job that was never enqueued is an invariant violation, not
    /// an anonymous decrement.
    #[tokio::test]
    #[should_panic(expected = "started with no pending job")]
    async fn start_without_pending_job_panics() {
        let (progress, _events) = tracker();
        let collection_id = "collection";

        progress.enqueue(collection_id, Stage::Extract).await;
        progress.start(collection_id, Stage::Extract).await;
        progress.start(collection_id, Stage::Extract).await;
    }

    /// Finishing a job that never started is an invariant violation.
    #[tokio::test]
    #[should_panic(expected = "finished with no active job")]
    async fn finish_without_active_job_panics() {
        let (progress, _events) = tracker();
        let collection_id = "collection";

        progress.enqueue(collection_id, Stage::Index).await;
        progress.finish_ok(collection_id, Stage::Index).await;
    }

    async fn next_progress(events: &mut mpsc::UnboundedReceiver<AppEvent>) -> PipelineProgress {
        match events.recv().await.expect("progress event channel closed") {
            AppEvent::Progress(progress) => progress,
            other => panic!("expected progress event, got {other:?}"),
        }
    }

    /// Several active snapshots within one flush window produce a single
    /// event carrying the latest state.
    #[tokio::test(start_paused = true)]
    async fn active_updates_coalesce_to_the_latest_snapshot() {
        let (progress, mut events) = tracker();
        let collection_id = "collection";

        for _ in 0..3 {
            progress.enqueue(collection_id, Stage::Extract).await;
        }
        progress.start(collection_id, Stage::Extract).await;
        progress.start(collection_id, Stage::Extract).await;

        assert!(
            events.try_recv().is_err(),
            "an active snapshot was forwarded before the flush interval"
        );

        tokio::time::sleep(PROGRESS_FLUSH_INTERVAL).await;
        let progress = next_progress(&mut events).await;
        assert_eq!(progress.collection_id, collection_id);
        assert_eq!(progress.extract.pending, 1);
        assert_eq!(progress.extract.active, 2);
        assert_eq!(progress.extract.total(), 3);
        assert!(
            tokio::time::timeout(PROGRESS_FLUSH_INTERVAL, events.recv())
                .await
                .is_err(),
            "more than one active snapshot was forwarded"
        );
    }

    /// An idle snapshot is forwarded immediately and drops the buffered active
    /// snapshot, so no stale active event follows it.
    #[tokio::test(start_paused = true)]
    async fn idle_snapshot_is_forwarded_immediately_and_discards_stale_active() {
        let (progress, mut events) = tracker();
        let collection_id = "collection";

        progress.enqueue(collection_id, Stage::Index).await;
        progress.start(collection_id, Stage::Index).await;
        assert!(
            events.try_recv().is_err(),
            "an active snapshot was forwarded before the flush interval"
        );

        progress.finish_ok(collection_id, Stage::Index).await;
        let progress = next_progress(&mut events).await;
        assert_eq!(progress.collection_id, collection_id);
        assert_eq!(progress.index.pending, 0);
        assert_eq!(progress.index.active, 0);
        assert_eq!(progress.index.completed, 1);
        assert_eq!(progress.index.failed, 0);

        assert!(
            tokio::time::timeout(PROGRESS_FLUSH_INTERVAL * 2, events.recv())
                .await
                .is_err(),
            "a stale active snapshot was forwarded after the idle snapshot"
        );
    }

    /// Coalescing changes only the notifications; reads and the forwarded
    /// snapshot carry the exact counters.
    #[tokio::test(start_paused = true)]
    async fn counters_remain_exact_while_notifications_coalesce() {
        let (progress, mut events) = tracker();
        let collection_id = "collection";

        for _ in 0..3 {
            progress.enqueue(collection_id, Stage::Index).await;
        }
        progress.start(collection_id, Stage::Index).await;
        progress.start(collection_id, Stage::Index).await;
        progress.finish_err(collection_id, Stage::Index).await;

        let current = progress
            .get(collection_id)
            .await
            .expect("collection progress");
        assert_eq!(current.index.pending, 1);
        assert_eq!(current.index.active, 1);
        assert_eq!(current.index.completed, 0);
        assert_eq!(current.index.failed, 1);
        assert_eq!(current.index.total(), 3);

        tokio::time::sleep(PROGRESS_FLUSH_INTERVAL).await;
        let notified = next_progress(&mut events).await;
        assert_eq!(notified.index.pending, current.index.pending);
        assert_eq!(notified.index.active, current.index.active);
        assert_eq!(notified.index.completed, current.index.completed);
        assert_eq!(notified.index.failed, current.index.failed);
        assert_eq!(notified.index.total(), current.index.total());
    }
}
