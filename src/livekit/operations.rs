//! Queue-based operations for non-blocking LiveKit client
//!
//! This module provides an operation queue system that eliminates lock contention
//! by processing all LiveKit operations sequentially through a dedicated worker task.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::Value;
use tokio::sync::{mpsc, oneshot};
use tracing::{debug, error};

use crate::AppError;

/// Priority levels for operations
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum OperationPriority {
    /// Highest priority - used for audio operations
    High = 0,
    /// Medium priority - used for control operations
    Medium = 1,
    /// Low priority - used for data messages
    Low = 2,
}

/// LiveKit operation variants
#[derive(Debug)]
pub enum LiveKitOperation {
    /// Send TTS audio data
    SendAudio {
        audio_data: Vec<u8>,
        response_tx: oneshot::Sender<Result<(), AppError>>,
        /// Generation at queue time - operations with older generation are skipped
        generation: u64,
    },
    /// Send a data message
    SendMessage {
        message: String,
        role: String,
        topic: Option<String>,
        debug: Option<Value>,
        response_tx: oneshot::Sender<Result<(), AppError>>,
        retry_count: u32,
    },
    /// Send raw data message
    SendDataMessage {
        topic: String,
        data: Value,
        response_tx: oneshot::Sender<Result<(), AppError>>,
        retry_count: u32,
    },
    /// Clear audio buffer
    ClearAudio {
        response_tx: oneshot::Sender<Result<(), AppError>>,
    },
    /// Check connection status
    IsConnected { response_tx: oneshot::Sender<bool> },
    /// Check if audio source is available
    HasAudioSource { response_tx: oneshot::Sender<bool> },
    /// Reconnect to LiveKit room
    Reconnect {
        response_tx: oneshot::Sender<Result<(), AppError>>,
    },
    /// Shutdown the worker
    Shutdown {
        /// Optional acknowledgement channel to signal completion
        ack_tx: Option<oneshot::Sender<()>>,
    },
}

impl LiveKitOperation {
    /// Get the priority of this operation
    pub fn priority(&self) -> OperationPriority {
        match self {
            LiveKitOperation::SendAudio { .. } | LiveKitOperation::ClearAudio { .. } => {
                OperationPriority::High
            }
            LiveKitOperation::IsConnected { .. } | LiveKitOperation::HasAudioSource { .. } => {
                OperationPriority::Medium
            }
            LiveKitOperation::SendMessage { .. } | LiveKitOperation::SendDataMessage { .. } => {
                OperationPriority::Low
            }
            LiveKitOperation::Reconnect { .. } | LiveKitOperation::Shutdown { .. } => {
                OperationPriority::High
            }
        }
    }
}

/// Queued operation with priority
#[derive(Debug)]
pub struct QueuedOperation {
    pub operation: LiveKitOperation,
    pub priority: OperationPriority,
    pub queued_at: std::time::Instant,
}

impl QueuedOperation {
    pub fn new(operation: LiveKitOperation) -> Self {
        let priority = operation.priority();
        Self {
            operation,
            priority,
            queued_at: std::time::Instant::now(),
        }
    }
}

/// Operation queue manager
pub struct OperationQueue {
    sender: mpsc::Sender<QueuedOperation>,
    /// Generation counter - audio operations with lower generation are skipped
    audio_generation: Arc<AtomicU64>,
}

impl OperationQueue {
    /// Create a new operation queue with the specified buffer size.
    ///
    /// Returns the queue, receiver, and a shared generation counter that can be
    /// passed to the operation worker for checking during audio operations.
    pub fn new(buffer_size: usize) -> (Self, mpsc::Receiver<QueuedOperation>, Arc<AtomicU64>) {
        let (sender, receiver) = mpsc::channel(buffer_size);
        let audio_generation = Arc::new(AtomicU64::new(0));
        (
            Self {
                sender,
                audio_generation: audio_generation.clone(),
            },
            receiver,
            audio_generation,
        )
    }

    /// Increment the audio generation counter to invalidate pending audio operations.
    ///
    /// This increments the generation counter which causes the operation worker
    /// to skip any pending `SendAudio` operations with older generations.
    /// Returns the new generation value.
    pub fn cancel_audio(&self) -> u64 {
        self.audio_generation.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// Get the current audio generation value.
    ///
    /// Used when queueing audio operations to stamp them with the current generation.
    pub fn current_audio_generation(&self) -> u64 {
        self.audio_generation.load(Ordering::Acquire)
    }

    /// Queue an operation for processing.
    ///
    /// For `SendAudio` operations, use `queue_audio` instead to properly stamp
    /// the operation with the current generation.
    pub async fn queue(&self, operation: LiveKitOperation) -> Result<(), AppError> {
        let queued_op = QueuedOperation::new(operation);
        self.sender.send(queued_op).await.map_err(|e| {
            error!("Failed to queue operation: {}", e);
            AppError::InternalServerError("Operation queue is full or closed".to_string())
        })
    }

    /// Queue a SendAudio operation stamped with the current generation.
    ///
    /// This ensures the operation is properly tagged for staleness detection.
    pub async fn queue_audio(
        &self,
        audio_data: Vec<u8>,
        response_tx: oneshot::Sender<Result<(), AppError>>,
    ) -> Result<(), AppError> {
        let generation = self.current_audio_generation();
        let operation = LiveKitOperation::SendAudio {
            audio_data,
            response_tx,
            generation,
        };
        self.queue(operation).await
    }

    /// Get the number of pending operations (approximate)
    pub fn pending_count(&self) -> usize {
        self.sender.capacity() - self.sender.max_capacity()
    }
}

impl Clone for OperationQueue {
    fn clone(&self) -> Self {
        Self {
            sender: self.sender.clone(),
            audio_generation: self.audio_generation.clone(),
        }
    }
}

/// Stats for monitoring queue performance
#[derive(Debug, Default)]
pub struct QueueStats {
    pub total_operations: u64,
    pub successful_operations: u64,
    pub failed_operations: u64,
    pub audio_operations: u64,
    pub message_operations: u64,
    pub average_latency_ms: u64,
    pub max_latency_ms: u64,
}

impl QueueStats {
    pub fn record_operation(
        &mut self,
        operation: &LiveKitOperation,
        success: bool,
        latency: std::time::Duration,
    ) {
        self.total_operations += 1;

        if success {
            self.successful_operations += 1;
        } else {
            self.failed_operations += 1;
        }

        match operation {
            LiveKitOperation::SendAudio { .. } | LiveKitOperation::ClearAudio { .. } => {
                self.audio_operations += 1;
            }
            LiveKitOperation::SendMessage { .. } | LiveKitOperation::SendDataMessage { .. } => {
                self.message_operations += 1;
            }
            _ => {}
        }

        let latency_ms = latency.as_millis() as u64;
        self.max_latency_ms = self.max_latency_ms.max(latency_ms);

        // Simple moving average
        if self.total_operations == 1 {
            self.average_latency_ms = latency_ms;
        } else {
            self.average_latency_ms = (self.average_latency_ms * (self.total_operations - 1)
                + latency_ms)
                / self.total_operations;
        }
    }

    pub fn log_stats(&self) {
        debug!(
            "Queue stats - Total: {}, Success: {}, Failed: {}, Audio: {}, Messages: {}, Avg latency: {}ms, Max latency: {}ms",
            self.total_operations,
            self.successful_operations,
            self.failed_operations,
            self.audio_operations,
            self.message_operations,
            self.average_latency_ms,
            self.max_latency_ms
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn test_send_audio_priority() {
        let (tx, _rx) = oneshot::channel();
        let op = LiveKitOperation::SendAudio {
            audio_data: vec![1, 2, 3],
            response_tx: tx,
            generation: 0,
        };

        assert_eq!(op.priority(), OperationPriority::High);
    }

    #[test]
    fn test_clear_audio_priority() {
        let (tx, _rx) = oneshot::channel();
        let op = LiveKitOperation::ClearAudio { response_tx: tx };

        assert_eq!(op.priority(), OperationPriority::High);
    }

    #[test]
    fn test_send_message_priority() {
        let (tx, _rx) = oneshot::channel();
        let op = LiveKitOperation::SendMessage {
            message: "test".to_string(),
            role: "user".to_string(),
            topic: None,
            debug: None,
            response_tx: tx,
            retry_count: 0,
        };

        assert_eq!(op.priority(), OperationPriority::Low);
    }

    #[test]
    fn test_is_connected_priority() {
        let (tx, _rx) = oneshot::channel();
        let op = LiveKitOperation::IsConnected { response_tx: tx };

        assert_eq!(op.priority(), OperationPriority::Medium);
    }

    #[test]
    fn test_has_audio_source_priority() {
        let (tx, _rx) = oneshot::channel();
        let op = LiveKitOperation::HasAudioSource { response_tx: tx };

        assert_eq!(op.priority(), OperationPriority::Medium);
    }

    #[test]
    fn test_reconnect_priority() {
        let (tx, _rx) = oneshot::channel();
        let op = LiveKitOperation::Reconnect { response_tx: tx };

        assert_eq!(op.priority(), OperationPriority::High);
    }

    #[test]
    fn test_shutdown_priority() {
        let op = LiveKitOperation::Shutdown { ack_tx: None };

        assert_eq!(op.priority(), OperationPriority::High);
    }

    #[test]
    fn test_priority_ordering() {
        // High should be less than Medium (higher priority = lower number)
        assert!(OperationPriority::High < OperationPriority::Medium);
        assert!(OperationPriority::Medium < OperationPriority::Low);
        assert!(OperationPriority::High < OperationPriority::Low);
    }

    #[test]
    fn test_queue_stats_default() {
        let stats = QueueStats::default();
        assert_eq!(stats.total_operations, 0);
        assert_eq!(stats.successful_operations, 0);
        assert_eq!(stats.failed_operations, 0);
        assert_eq!(stats.audio_operations, 0);
        assert_eq!(stats.message_operations, 0);
        assert_eq!(stats.average_latency_ms, 0);
        assert_eq!(stats.max_latency_ms, 0);
    }

    #[test]
    fn test_queue_stats_record_successful_audio_operation() {
        let mut stats = QueueStats::default();
        let (tx, _rx) = oneshot::channel();
        let op = LiveKitOperation::SendAudio {
            audio_data: vec![],
            response_tx: tx,
            generation: 0,
        };

        stats.record_operation(&op, true, Duration::from_millis(10));

        assert_eq!(stats.total_operations, 1);
        assert_eq!(stats.successful_operations, 1);
        assert_eq!(stats.failed_operations, 0);
        assert_eq!(stats.audio_operations, 1);
        assert_eq!(stats.message_operations, 0);
        assert_eq!(stats.average_latency_ms, 10);
        assert_eq!(stats.max_latency_ms, 10);
    }

    #[test]
    fn test_queue_stats_record_failed_message_operation() {
        let mut stats = QueueStats::default();
        let (tx, _rx) = oneshot::channel();
        let op = LiveKitOperation::SendMessage {
            message: "test".to_string(),
            role: "user".to_string(),
            topic: None,
            debug: None,
            response_tx: tx,
            retry_count: 0,
        };

        stats.record_operation(&op, false, Duration::from_millis(50));

        assert_eq!(stats.total_operations, 1);
        assert_eq!(stats.successful_operations, 0);
        assert_eq!(stats.failed_operations, 1);
        assert_eq!(stats.audio_operations, 0);
        assert_eq!(stats.message_operations, 1);
        assert_eq!(stats.average_latency_ms, 50);
        assert_eq!(stats.max_latency_ms, 50);
    }

    #[test]
    fn test_queue_stats_max_latency_updates() {
        let mut stats = QueueStats::default();
        let (tx1, _rx1) = oneshot::channel();
        let op1 = LiveKitOperation::ClearAudio { response_tx: tx1 };
        let (tx2, _rx2) = oneshot::channel();
        let op2 = LiveKitOperation::ClearAudio { response_tx: tx2 };

        stats.record_operation(&op1, true, Duration::from_millis(10));
        stats.record_operation(&op2, true, Duration::from_millis(100));

        assert_eq!(stats.max_latency_ms, 100);
    }

    #[test]
    fn test_queue_stats_average_latency() {
        let mut stats = QueueStats::default();

        // Record 3 operations with different latencies
        let (tx1, _rx1) = oneshot::channel();
        let op1 = LiveKitOperation::ClearAudio { response_tx: tx1 };
        let (tx2, _rx2) = oneshot::channel();
        let op2 = LiveKitOperation::ClearAudio { response_tx: tx2 };
        let (tx3, _rx3) = oneshot::channel();
        let op3 = LiveKitOperation::ClearAudio { response_tx: tx3 };

        stats.record_operation(&op1, true, Duration::from_millis(10));
        stats.record_operation(&op2, true, Duration::from_millis(20));
        stats.record_operation(&op3, true, Duration::from_millis(30));

        // Average of 10, 20, 30 = 20
        assert_eq!(stats.average_latency_ms, 20);
    }

    #[tokio::test]
    async fn test_operation_queue_creation() {
        let (queue, _receiver, _audio_generation) = OperationQueue::new(100);

        // Queue should start with capacity
        // pending_count starts at 0 when no operations are queued
        assert_eq!(queue.pending_count(), 0);
    }

    #[tokio::test]
    async fn test_operation_queue_clone() {
        let (queue, _receiver, _audio_generation) = OperationQueue::new(100);
        let _cloned_queue = queue.clone();

        // Both queues should be usable
        assert_eq!(queue.pending_count(), 0);
    }

    #[test]
    fn test_audio_generation_initial_state() {
        let (queue, _receiver, audio_generation) = OperationQueue::new(100);

        // Generation should start at 0
        assert_eq!(queue.current_audio_generation(), 0);
        assert_eq!(audio_generation.load(Ordering::Acquire), 0);
    }

    #[test]
    fn test_audio_generation_increments_on_cancel() {
        let (queue, _receiver, audio_generation) = OperationQueue::new(100);

        // First cancel increments to 1
        let new_gen = queue.cancel_audio();
        assert_eq!(new_gen, 1);
        assert_eq!(queue.current_audio_generation(), 1);
        assert_eq!(audio_generation.load(Ordering::Acquire), 1);

        // Second cancel increments to 2
        let new_gen = queue.cancel_audio();
        assert_eq!(new_gen, 2);
        assert_eq!(queue.current_audio_generation(), 2);
        assert_eq!(audio_generation.load(Ordering::Acquire), 2);
    }

    #[test]
    fn test_audio_generation_shared_between_queue_and_arc() {
        let (queue, _receiver, audio_generation) = OperationQueue::new(100);

        // Increment via queue, check via arc
        queue.cancel_audio();
        assert_eq!(audio_generation.load(Ordering::Acquire), 1);

        // Increment via arc (simulating worker), check via queue
        audio_generation.fetch_add(1, Ordering::SeqCst);
        assert_eq!(queue.current_audio_generation(), 2);
    }

    #[test]
    fn test_audio_generation_cloned_queue() {
        let (queue, _receiver, audio_generation) = OperationQueue::new(100);
        let cloned_queue = queue.clone();

        // Increment via original queue
        queue.cancel_audio();

        // Both queues and the arc should see the new generation
        assert_eq!(queue.current_audio_generation(), 1);
        assert_eq!(cloned_queue.current_audio_generation(), 1);
        assert_eq!(audio_generation.load(Ordering::Acquire), 1);

        // Increment via cloned queue
        cloned_queue.cancel_audio();

        // All should see the new generation
        assert_eq!(queue.current_audio_generation(), 2);
        assert_eq!(cloned_queue.current_audio_generation(), 2);
        assert_eq!(audio_generation.load(Ordering::Acquire), 2);
    }

    #[test]
    fn test_stale_generation_detection() {
        let (queue, _receiver, audio_generation) = OperationQueue::new(100);

        // Capture initial generation for an operation
        let op_generation = queue.current_audio_generation();
        assert_eq!(op_generation, 0);

        // Simulate clear being called (increments generation)
        queue.cancel_audio();
        let current_gen = queue.current_audio_generation();
        assert_eq!(current_gen, 1);

        // The operation's generation is now stale
        assert!(op_generation < current_gen);

        // New operations after clear have current generation
        let new_op_generation = queue.current_audio_generation();
        assert_eq!(new_op_generation, current_gen);
        assert!(new_op_generation >= current_gen);

        // Verify via arc as well
        assert!(op_generation < audio_generation.load(Ordering::Acquire));
    }

    #[test]
    fn test_concurrent_cancel_calls() {
        let (queue, _receiver, _audio_generation) = OperationQueue::new(100);

        // Multiple cancel calls should each increment
        let gen1 = queue.cancel_audio();
        let gen2 = queue.cancel_audio();
        let gen3 = queue.cancel_audio();

        assert_eq!(gen1, 1);
        assert_eq!(gen2, 2);
        assert_eq!(gen3, 3);
        assert_eq!(queue.current_audio_generation(), 3);
    }

    #[tokio::test]
    async fn test_queue_audio_stamps_generation() {
        let (queue, mut receiver, _audio_generation) = OperationQueue::new(100);

        // Queue audio at generation 0
        let (tx1, _rx1) = oneshot::channel();
        queue.queue_audio(vec![1, 2, 3], tx1).await.unwrap();

        // Cancel to increment to generation 1
        queue.cancel_audio();

        // Queue audio at generation 1
        let (tx2, _rx2) = oneshot::channel();
        queue.queue_audio(vec![4, 5, 6], tx2).await.unwrap();

        // Receive and verify generations
        let queued_op1 = receiver.recv().await.unwrap();
        if let LiveKitOperation::SendAudio { generation, .. } = queued_op1.operation {
            assert_eq!(generation, 0);
        } else {
            panic!("Expected SendAudio operation");
        }

        let queued_op2 = receiver.recv().await.unwrap();
        if let LiveKitOperation::SendAudio { generation, .. } = queued_op2.operation {
            assert_eq!(generation, 1);
        } else {
            panic!("Expected SendAudio operation");
        }
    }
}
