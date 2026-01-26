//! Job types and processing logic.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;
use uuid::Uuid;

/// Job identifier
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct JobId(Uuid);

impl JobId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl fmt::Display for JobId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "job-{}", self.0)
    }
}

/// Job status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum JobStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Retrying,
}

/// Job priority
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum JobPriority {
    Low = 1,
    Normal = 2,
    High = 3,
    Critical = 4,
}

/// Job definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Job {
    /// Unique job identifier
    pub id: JobId,

    /// Job type
    pub job_type: JobType,

    /// Job priority
    #[serde(default = "default_priority")]
    pub priority: JobPriority,

    /// Job status
    pub status: JobStatus,

    /// Number of retry attempts
    #[serde(default)]
    pub retry_count: u32,

    /// Maximum retry attempts
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,

    /// Job payload
    pub payload: serde_json::Value,

    /// Creation timestamp
    pub created_at: DateTime<Utc>,

    /// Last updated timestamp
    pub updated_at: DateTime<Utc>,

    /// Next retry time (if retrying)
    pub retry_at: Option<DateTime<Utc>>,

    /// Error message (if failed)
    pub error: Option<String>,

    /// Result (if completed)
    pub result: Option<serde_json::Value>,
}

fn default_priority() -> JobPriority {
    JobPriority::Normal
}

fn default_max_retries() -> u32 {
    5
}

impl Job {
    /// Create a new job
    pub fn new(job_type: JobType, payload: serde_json::Value) -> Self {
        let now = Utc::now();
        Self {
            id: JobId::new(),
            job_type,
            priority: JobPriority::Normal,
            status: JobStatus::Pending,
            retry_count: 0,
            max_retries: 5,
            payload,
            created_at: now,
            updated_at: now,
            retry_at: None,
            error: None,
            result: None,
        }
    }

    /// Create a new job with priority
    pub fn with_priority(mut self, priority: JobPriority) -> Self {
        self.priority = priority;
        self
    }

    /// Create a new job with max retries
    pub fn with_max_retries(mut self, max_retries: u32) -> Self {
        self.max_retries = max_retries;
        self
    }

    /// Mark job as running
    pub fn mark_running(&mut self) {
        self.status = JobStatus::Running;
        self.updated_at = Utc::now();
    }

    /// Mark job as completed
    pub fn mark_completed(&mut self, result: serde_json::Value) {
        self.status = JobStatus::Completed;
        self.result = Some(result);
        self.updated_at = Utc::now();
    }

    /// Mark job as failed
    pub fn mark_failed(&mut self, error: String) {
        self.status = JobStatus::Failed;
        self.error = Some(error);
        self.updated_at = Utc::now();
    }

    /// Mark job for retry
    pub fn mark_retrying(&mut self, retry_at: DateTime<Utc>) {
        self.status = JobStatus::Retrying;
        self.retry_count += 1;
        self.retry_at = Some(retry_at);
        self.updated_at = Utc::now();
    }

    /// Check if job can be retried
    pub fn can_retry(&self) -> bool {
        self.retry_count < self.max_retries
    }

    /// Check if job is ready for retry
    pub fn is_ready_for_retry(&self) -> bool {
        if let Some(retry_at) = self.retry_at {
            Utc::now() >= retry_at
        } else {
            false
        }
    }
}

/// Job type enumeration
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobType {
    /// Process CDR (Call Detail Record)
    CdrProcessing,

    /// Process recording file
    RecordingProcessing,

    /// Deliver webhook
    WebhookDelivery,

    /// Cleanup old data
    CleanupTask,

    /// Custom job type
    Custom,
}

impl fmt::Display for JobType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            JobType::CdrProcessing => write!(f, "cdr_processing"),
            JobType::RecordingProcessing => write!(f, "recording_processing"),
            JobType::WebhookDelivery => write!(f, "webhook_delivery"),
            JobType::CleanupTask => write!(f, "cleanup_task"),
            JobType::Custom => write!(f, "custom"),
        }
    }
}

/// Job processing result
#[derive(Debug)]
pub enum JobResult {
    /// Job completed successfully
    Success(serde_json::Value),

    /// Job failed with error
    Failure(String),

    /// Job should be retried later
    Retry(String),
}

impl JobResult {
    pub fn success() -> Self {
        Self::Success(serde_json::json!({"status": "success"}))
    }

    pub fn success_with_data(data: serde_json::Value) -> Self {
        Self::Success(data)
    }

    pub fn failure(error: impl Into<String>) -> Self {
        Self::Failure(error.into())
    }

    pub fn retry(reason: impl Into<String>) -> Self {
        Self::Retry(reason.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_job_creation() {
        let payload = serde_json::json!({"call_id": "call-123"});
        let job = Job::new(JobType::CdrProcessing, payload.clone());

        assert_eq!(job.status, JobStatus::Pending);
        assert_eq!(job.job_type, JobType::CdrProcessing);
        assert_eq!(job.priority, JobPriority::Normal);
        assert_eq!(job.retry_count, 0);
        assert_eq!(job.payload, payload);
    }

    #[test]
    fn test_job_with_priority() {
        let job = Job::new(JobType::WebhookDelivery, serde_json::json!({}))
            .with_priority(JobPriority::High);

        assert_eq!(job.priority, JobPriority::High);
    }

    #[test]
    fn test_job_lifecycle() {
        let mut job = Job::new(JobType::CdrProcessing, serde_json::json!({}));

        // Start job
        job.mark_running();
        assert_eq!(job.status, JobStatus::Running);

        // Complete job
        job.mark_completed(serde_json::json!({"processed": true}));
        assert_eq!(job.status, JobStatus::Completed);
        assert!(job.result.is_some());
    }

    #[test]
    fn test_job_retry() {
        let mut job = Job::new(JobType::WebhookDelivery, serde_json::json!({}))
            .with_max_retries(3);

        assert!(job.can_retry());
        assert_eq!(job.retry_count, 0);

        // Mark for retry
        let retry_time = Utc::now() + chrono::Duration::seconds(10);
        job.mark_retrying(retry_time);
        assert_eq!(job.status, JobStatus::Retrying);
        assert_eq!(job.retry_count, 1);
        assert!(job.can_retry());

        // Max retries reached
        job.retry_count = 3;
        assert!(!job.can_retry());
    }
}
