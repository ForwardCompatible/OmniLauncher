//! In-flight download tracking + cancellation.
//!
//! Mirrors [`crate::sidecar::SidecarController`]'s shape: a managed-state
//! struct holding a map of active downloads keyed by id, each with a cancel
//! signal so the UI can cancel without killing the OS task.
//!
//! Uses `tokio::sync::watch` (shipped with `tokio` "full") rather than pulling
//! `tokio-util` for `CancellationToken`. Each download gets a `watch::Sender`;
//! sending `true` cancels it.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use tokio::sync::{watch, Mutex};

/// A unique download id (monotonic counter, process-scoped).
pub type DownloadId = u64;

/// Cancel handle for a single download.
pub type CancelSender = watch::Sender<bool>;
pub type CancelReceiver = watch::Receiver<bool>;

/// Tracks active downloads and their cancel signals. One instance lives in
/// Tauri managed state for the app's lifetime.
pub struct DownloadManager {
    active: Mutex<HashMap<DownloadId, CancelSender>>,
    next_id: AtomicU64,
}

impl DownloadManager {
    pub fn new() -> Self {
        Self {
            active: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
        }
    }

    /// Register a new download; return `(id, receiver)`. The download task
    /// polls the receiver inside `HfClient::download`'s `should_cancel` closure.
    pub async fn register(&self) -> (DownloadId, CancelReceiver) {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = watch::channel(false);
        self.active.lock().await.insert(id, tx);
        (id, rx)
    }

    /// Request cancellation. Returns `false` if the id isn't active.
    pub async fn cancel(&self, id: DownloadId) -> bool {
        match self.active.lock().await.remove(&id) {
            Some(tx) => {
                let _ = tx.send(true);
                true
            }
            None => false,
        }
    }

    /// Remove a finished download from the active map.
    pub async fn finish(&self, id: DownloadId) {
        self.active.lock().await.remove(&id);
    }
}

impl Default for DownloadManager {
    fn default() -> Self {
        Self::new()
    }
}

pub type SharedDownloadManager = Arc<DownloadManager>;
