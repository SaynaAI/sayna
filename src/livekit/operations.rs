//! Queue-based operations for non-blocking LiveKit client
//!
//! This module provides an operation queue system that eliminates lock contention
//! by processing all LiveKit operations sequentially through a dedicated worker task.

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
    Shutdown,
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
            LiveKitOperation::Reconnect { .. } | LiveKitOperation::Shutdown => {
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
}

impl OperationQueue {
    /// Create a new operation queue with the specified buffer size
    pub fn new(buffer_size: usize) -> (Self, mpsc::Receiver<QueuedOperation>) {
        let (sender, receiver) = mpsc::channel(buffer_size);
        (Self { sender }, receiver)
    }

    /// Queue an operation for processing
    pub async fn queue(&self, operation: LiveKitOperation) -> Result<(), AppError> {
        let queued_op = QueuedOperation::new(operation);
        self.sender.send(queued_op).await.map_err(|e| {
            error!("Failed to queue operation: {}", e);
            AppError::InternalServerError("Operation queue is full or closed".to_string())
        })
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
