//! Worker abstractions for batching and processing jobs.

use std::time::{Duration, Instant};

use tokio::sync::mpsc;

/// Configuration for batching workers
#[derive(Debug, Clone)]
pub struct BatchConfig {
    /// Maximum items per batch
    pub max_size: usize,
    /// Maximum time to wait before flushing a partial batch
    pub max_wait: Duration,
}

impl Default for BatchConfig {
    fn default() -> Self {
        Self {
            max_size: 50,
            max_wait: Duration::from_millis(100),
        }
    }
}

/// Collects items into batches by size or timeout.
///
/// Use this to accumulate work items and process them in efficient batches
/// rather than one at a time.
pub struct Batcher<T> {
    rx: mpsc::Receiver<T>,
    config: BatchConfig,
    buffer: Vec<T>,
    deadline: Option<Instant>,
}

impl<T> Batcher<T> {
    /// Create a new batcher wrapping the given receiver.
    pub fn new(rx: mpsc::Receiver<T>, config: BatchConfig) -> Self {
        let capacity = config.max_size;
        Self {
            rx,
            config,
            buffer: Vec::with_capacity(capacity),
            deadline: None,
        }
    }

    /// Returns the next batch of items.
    ///
    /// A batch is returned when:
    /// - The buffer reaches `max_size` items
    /// - `max_wait` time has passed since the first item was buffered
    /// - The channel is closed (returns remaining items)
    ///
    /// Returns `None` when the channel is closed and no items remain.
    pub async fn next_batch(&mut self) -> Option<Vec<T>> {
        loop {
            let timeout = self
                .deadline
                .map(|d| d.saturating_duration_since(Instant::now()))
                .unwrap_or(Duration::MAX);

            tokio::select! {
                biased;

                // Check for timeout first if we have items buffered
                _ = tokio::time::sleep(timeout), if self.deadline.is_some() => {
                    self.deadline = None;
                    if !self.buffer.is_empty() {
                        return Some(std::mem::take(&mut self.buffer));
                    }
                }

                // Receive new items
                item = self.rx.recv() => {
                    match item {
                        Some(item) => {
                            // Start the deadline timer on first item
                            if self.buffer.is_empty() {
                                self.deadline = Some(Instant::now() + self.config.max_wait);
                            }
                            self.buffer.push(item);

                            // Flush if we've reached max size
                            if self.buffer.len() >= self.config.max_size {
                                self.deadline = None;
                                return Some(std::mem::take(&mut self.buffer));
                            }
                        }
                        None => {
                            // Channel closed, flush remaining items
                            self.deadline = None;
                            return if self.buffer.is_empty() {
                                None
                            } else {
                                Some(std::mem::take(&mut self.buffer))
                            };
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_batcher_by_size() {
        let (tx, rx) = mpsc::channel(10);
        let config = BatchConfig {
            max_size: 3,
            max_wait: Duration::from_secs(10), // Long timeout, shouldn't trigger
        };
        let mut batcher = Batcher::new(rx, config);

        // Send 5 items
        for i in 0..5 {
            tx.send(i).await.unwrap();
        }
        drop(tx);

        // First batch should be 3 items (max_size)
        let batch1 = batcher.next_batch().await.unwrap();
        assert_eq!(batch1, vec![0, 1, 2]);

        // Second batch should be remaining 2 items (channel closed)
        let batch2 = batcher.next_batch().await.unwrap();
        assert_eq!(batch2, vec![3, 4]);

        // No more batches
        assert!(batcher.next_batch().await.is_none());
    }

    #[tokio::test]
    async fn test_batcher_by_timeout() {
        let (tx, rx) = mpsc::channel(10);
        let config = BatchConfig {
            max_size: 100,                       // Large, shouldn't trigger
            max_wait: Duration::from_millis(50), // Short timeout
        };
        let mut batcher = Batcher::new(rx, config);

        // Send 2 items
        tx.send(1).await.unwrap();
        tx.send(2).await.unwrap();

        // Should get batch after timeout even though we haven't hit max_size
        let batch = batcher.next_batch().await.unwrap();
        assert_eq!(batch, vec![1, 2]);
    }

    #[tokio::test]
    async fn test_batcher_empty_channel() {
        let (_tx, rx) = mpsc::channel::<i32>(10);
        let config = BatchConfig::default();
        let mut batcher = Batcher::new(rx, config);

        // Drop sender immediately
        drop(_tx);

        // Should return None immediately
        assert!(batcher.next_batch().await.is_none());
    }
}
