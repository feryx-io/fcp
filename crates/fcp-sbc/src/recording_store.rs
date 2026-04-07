//! Recording metadata store for local call capture.
//!
//! Tracks in-memory metadata for recordings written to disk by the
//! `LocalCaptureManager`. Provides list/get/status-update operations
//! used by the admin API.

use dashmap::DashMap;
use serde::Serialize;
use std::time::{SystemTime, UNIX_EPOCH};

/// Status of a local recording.
#[derive(Debug, Clone, Serialize)]
pub enum LocalRecordingStatus {
    InProgress,
    Completed,
    Failed(String),
}

/// Metadata for a single recording.
#[derive(Debug, Clone, Serialize)]
pub struct RecordingMetadata {
    pub recording_id: String,
    pub call_id: String,
    pub caller: String,
    pub callee: String,
    pub file_path: String,
    pub format: String,
    pub started_at: f64,
    pub completed_at: Option<f64>,
    pub status: LocalRecordingStatus,
}

/// Thread-safe in-memory store for recording metadata, keyed by call_id.
pub struct RecordingStore {
    recordings: DashMap<String, RecordingMetadata>,
}

impl RecordingStore {
    pub fn new() -> Self {
        Self {
            recordings: DashMap::new(),
        }
    }

    /// Insert a new recording metadata entry.
    pub fn insert(&self, metadata: RecordingMetadata) {
        self.recordings.insert(metadata.call_id.clone(), metadata);
    }

    /// Get a clone of the metadata for a given call_id.
    pub fn get(&self, call_id: &str) -> Option<RecordingMetadata> {
        self.recordings.get(call_id).map(|r| r.clone())
    }

    /// Mark a recording as completed.
    pub fn mark_completed(&self, call_id: &str) {
        if let Some(mut entry) = self.recordings.get_mut(call_id) {
            entry.status = LocalRecordingStatus::Completed;
            entry.completed_at = Some(epoch_secs_now());
        }
    }

    /// Mark a recording as failed with a reason.
    pub fn mark_failed(&self, call_id: &str, reason: String) {
        if let Some(mut entry) = self.recordings.get_mut(call_id) {
            entry.status = LocalRecordingStatus::Failed(reason);
            entry.completed_at = Some(epoch_secs_now());
        }
    }

    /// List all recordings, most-recent first (by started_at descending).
    pub fn list(&self) -> Vec<RecordingMetadata> {
        let mut entries: Vec<RecordingMetadata> =
            self.recordings.iter().map(|r| r.value().clone()).collect();
        entries.sort_by(|a, b| {
            b.started_at
                .partial_cmp(&a.started_at)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        entries
    }
}

fn epoch_secs_now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_metadata(call_id: &str) -> RecordingMetadata {
        RecordingMetadata {
            recording_id: format!("rec-{call_id}"),
            call_id: call_id.to_string(),
            caller: "sip:alice@example.com".to_string(),
            callee: "sip:bob@example.com".to_string(),
            file_path: format!("./recordings/{call_id}.wav"),
            format: "wav".to_string(),
            started_at: 1700000000.0,
            completed_at: None,
            status: LocalRecordingStatus::InProgress,
        }
    }

    #[test]
    fn test_recording_store_lifecycle() {
        let store = RecordingStore::new();
        let meta = sample_metadata("call-1");
        store.insert(meta);

        // Get
        let got = store.get("call-1").unwrap();
        assert_eq!(got.recording_id, "rec-call-1");
        assert!(matches!(got.status, LocalRecordingStatus::InProgress));
        assert!(got.completed_at.is_none());

        // Mark completed
        store.mark_completed("call-1");
        let got = store.get("call-1").unwrap();
        assert!(matches!(got.status, LocalRecordingStatus::Completed));
        assert!(got.completed_at.is_some());

        // List
        let list = store.list();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].call_id, "call-1");
    }

    #[test]
    fn test_recording_store_mark_failed() {
        let store = RecordingStore::new();
        store.insert(sample_metadata("call-2"));

        store.mark_failed("call-2", "disk full".to_string());
        let got = store.get("call-2").unwrap();
        assert!(matches!(got.status, LocalRecordingStatus::Failed(ref r) if r == "disk full"));
        assert!(got.completed_at.is_some());
    }

    #[test]
    fn test_recording_store_get_missing() {
        let store = RecordingStore::new();
        assert!(store.get("nonexistent").is_none());
    }

    #[test]
    fn test_recording_store_list_ordering() {
        let store = RecordingStore::new();

        let mut m1 = sample_metadata("call-old");
        m1.started_at = 1000.0;
        let mut m2 = sample_metadata("call-new");
        m2.started_at = 2000.0;

        store.insert(m1);
        store.insert(m2);

        let list = store.list();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].call_id, "call-new");
        assert_eq!(list[1].call_id, "call-old");
    }
}
