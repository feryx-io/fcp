//! Worker implementation for processing jobs.

use crate::config::{RetryConfig, WorkerConfig};
use crate::job::{Job, JobResult, JobType};
use crate::queue::JobQueue;
use anyhow::Result;
use fcp_core::storage::{CallDetailRecord, CdrStore};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::time::timeout;
use tracing::{debug, error, info, warn};

/// Worker pool for processing jobs
pub struct WorkerPool {
    queue: Arc<JobQueue>,
    config: WorkerConfig,
    retry_config: RetryConfig,
    active_workers: Arc<AtomicUsize>,
    shutdown: Arc<AtomicBool>,
    storage: Option<Arc<fcp_core::DynStorage>>,
}

impl WorkerPool {
    /// Create a new worker pool
    pub fn new(queue: JobQueue, config: WorkerConfig, retry_config: RetryConfig) -> Self {
        Self {
            queue: Arc::new(queue),
            config,
            retry_config,
            active_workers: Arc::new(AtomicUsize::new(0)),
            shutdown: Arc::new(AtomicBool::new(false)),
            storage: None,
        }
    }

    /// Set storage backend for CDR processing
    #[allow(dead_code)]
    pub fn with_storage(mut self, storage: Arc<fcp_core::DynStorage>) -> Self {
        self.storage = Some(storage);
        self
    }

    /// Start the worker pool
    pub async fn start(&self) -> Vec<tokio::task::JoinHandle<()>> {
        info!(worker_count = self.config.count, "Starting worker pool");

        let mut handles = Vec::new();

        for worker_id in 0..self.config.count {
            let queue = self.queue.clone();
            let config = self.config.clone();
            let retry_config = self.retry_config.clone();
            let active_workers = self.active_workers.clone();
            let shutdown = self.shutdown.clone();
            let storage = self.storage.clone();

            let handle = tokio::spawn(async move {
                let worker = Worker {
                    id: worker_id,
                    queue,
                    config,
                    retry_config,
                    active_workers,
                    shutdown,
                    storage,
                };

                worker.run().await;
            });

            handles.push(handle);
        }

        info!("Worker pool started");
        handles
    }

    /// Shutdown the worker pool
    pub async fn shutdown(&self, timeout_duration: Duration) {
        info!("Shutting down worker pool");
        self.shutdown.store(true, Ordering::SeqCst);

        // Wait for workers to finish with timeout
        let start = std::time::Instant::now();
        while self.active_workers.load(Ordering::SeqCst) > 0 {
            if start.elapsed() > timeout_duration {
                warn!("Worker shutdown timeout reached, forcing shutdown");
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        info!("Worker pool shutdown complete");
    }

    /// Get number of active workers
    pub fn active_workers(&self) -> usize {
        self.active_workers.load(Ordering::SeqCst)
    }
}

/// Individual worker
struct Worker {
    id: usize,
    queue: Arc<JobQueue>,
    config: WorkerConfig,
    retry_config: RetryConfig,
    active_workers: Arc<AtomicUsize>,
    shutdown: Arc<AtomicBool>,
    storage: Option<Arc<fcp_core::DynStorage>>,
}

impl Worker {
    /// Run the worker loop
    async fn run(&self) {
        info!(worker_id = self.id, "Worker started");

        loop {
            // Check for shutdown signal
            if self.shutdown.load(Ordering::SeqCst) {
                info!(worker_id = self.id, "Worker received shutdown signal");
                break;
            }

            // Try to dequeue a job
            match self.queue.dequeue().await {
                Ok(Some(mut job)) => {
                    self.active_workers.fetch_add(1, Ordering::SeqCst);

                    debug!(
                        worker_id = self.id,
                        job_id = %job.id,
                        job_type = %job.job_type,
                        "Processing job"
                    );

                    // Process job with timeout
                    let result = timeout(self.config.job_timeout, self.process_job(&job)).await;

                    match result {
                        Ok(Ok(job_result)) => {
                            self.handle_job_result(&mut job, job_result).await;
                        }
                        Ok(Err(e)) => {
                            error!(
                                worker_id = self.id,
                                job_id = %job.id,
                                error = %e,
                                "Job processing error"
                            );
                            job.mark_failed(format!("Processing error: {}", e));
                            let _ = self.queue.fail_job(&job).await;
                        }
                        Err(_) => {
                            error!(
                                worker_id = self.id,
                                job_id = %job.id,
                                timeout_secs = self.config.job_timeout.as_secs(),
                                "Job processing timeout"
                            );
                            job.mark_failed(format!(
                                "Timeout after {} seconds",
                                self.config.job_timeout.as_secs()
                            ));
                            let _ = self.queue.fail_job(&job).await;
                        }
                    }

                    self.active_workers.fetch_sub(1, Ordering::SeqCst);
                }
                Ok(None) => {
                    // No jobs available, sleep before polling again
                    debug!(worker_id = self.id, "No jobs available, sleeping");
                    tokio::time::sleep(self.config.poll_interval).await;
                }
                Err(e) => {
                    error!(
                        worker_id = self.id,
                        error = %e,
                        "Failed to dequeue job"
                    );
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
            }
        }

        info!(worker_id = self.id, "Worker stopped");
    }

    /// Process a job
    async fn process_job(&self, job: &Job) -> Result<JobResult> {
        match job.job_type {
            JobType::CdrProcessing => self.process_cdr(job).await,
            JobType::RecordingProcessing => self.process_recording(job).await,
            JobType::WebhookDelivery => self.process_webhook(job).await,
            JobType::CleanupTask => self.process_cleanup(job).await,
            JobType::Custom => self.process_custom(job).await,
        }
    }

    /// Handle job result
    async fn handle_job_result(&self, job: &mut Job, result: JobResult) {
        match result {
            JobResult::Success(data) => {
                job.mark_completed(data);
                if let Err(e) = self.queue.complete_job(job).await {
                    error!(job_id = %job.id, error = %e, "Failed to mark job as completed");
                }
            }
            JobResult::Failure(error) => {
                job.mark_failed(error);
                if let Err(e) = self.queue.fail_job(job).await {
                    error!(job_id = %job.id, error = %e, "Failed to mark job as failed");
                }
            }
            JobResult::Retry(reason) => {
                if job.can_retry() {
                    // Calculate retry delay with exponential backoff
                    let delay = self.calculate_retry_delay(job.retry_count);
                    let retry_at = chrono::Utc::now() + chrono::Duration::from_std(delay).unwrap();

                    job.mark_retrying(retry_at);

                    if let Err(e) = self.queue.retry_job(job).await {
                        error!(job_id = %job.id, error = %e, "Failed to schedule job retry");
                    }
                } else {
                    job.mark_failed(format!("Max retries exceeded: {}", reason));
                    if let Err(e) = self.queue.fail_job(job).await {
                        error!(job_id = %job.id, error = %e, "Failed to mark job as failed");
                    }
                }
            }
        }
    }

    /// Calculate retry delay with exponential backoff
    fn calculate_retry_delay(&self, retry_count: u32) -> Duration {
        let delay = self.retry_config.initial_delay.as_secs_f64()
            * self
                .retry_config
                .backoff_multiplier
                .powi(retry_count as i32);

        Duration::from_secs_f64(delay.min(self.retry_config.max_delay.as_secs_f64()))
    }

    /// Process CDR job
    ///
    /// Deserializes a `CallDetailRecord` from the job payload and stores it
    /// in the configured storage backend.
    async fn process_cdr(&self, job: &Job) -> Result<JobResult> {
        info!(job_id = %job.id, "Processing CDR");

        let storage = match &self.storage {
            Some(s) => s,
            None => {
                warn!(job_id = %job.id, "No storage backend configured for CDR processing");
                return Ok(JobResult::retry("Storage backend not available"));
            }
        };

        // Deserialize the CDR from the job payload
        let cdr: CallDetailRecord = match serde_json::from_value(job.payload.clone()) {
            Ok(cdr) => cdr,
            Err(e) => {
                error!(
                    job_id = %job.id,
                    error = %e,
                    "Failed to deserialize CDR from job payload"
                );
                return Ok(JobResult::failure(format!("Invalid CDR payload: {}", e)));
            }
        };

        // Validate required fields
        if cdr.from_uri.is_empty() {
            return Ok(JobResult::failure("CDR missing from_uri"));
        }
        if cdr.to_uri.is_empty() {
            return Ok(JobResult::failure("CDR missing to_uri"));
        }

        let call_id_str = cdr.call_id.to_string();

        // Store the CDR
        match storage.insert_cdr(cdr).await {
            Ok(()) => {
                info!(
                    job_id = %job.id,
                    call_id = %call_id_str,
                    "CDR stored successfully"
                );
                Ok(JobResult::success_with_data(serde_json::json!({
                    "cdr_processed": true,
                    "call_id": call_id_str
                })))
            }
            Err(e) => {
                error!(
                    job_id = %job.id,
                    call_id = %call_id_str,
                    error = %e,
                    "Failed to store CDR"
                );
                // Retry on storage errors (transient)
                Ok(JobResult::retry(format!("Storage error: {}", e)))
            }
        }
    }

    /// Process recording job by validating the recording file and returning metadata.
    async fn process_recording(&self, job: &Job) -> Result<JobResult> {
        let path = match job.payload.get("path").and_then(|v| v.as_str()) {
            Some(path) => path,
            None => return Ok(JobResult::failure("Missing recording path in payload")),
        };

        let metadata = match tokio::fs::metadata(path).await {
            Ok(metadata) => metadata,
            Err(e) => {
                warn!(job_id = %job.id, path = %path, error = %e, "Recording file unavailable");
                return Ok(JobResult::retry(format!(
                    "Recording file unavailable: {}",
                    e
                )));
            }
        };

        if !metadata.is_file() {
            return Ok(JobResult::failure("Recording path is not a file"));
        }

        let delete_after_processing = job
            .payload
            .get("delete_after_processing")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        if delete_after_processing {
            if let Err(e) = tokio::fs::remove_file(path).await {
                warn!(job_id = %job.id, path = %path, error = %e, "Failed to remove processed recording");
                return Ok(JobResult::retry(format!(
                    "Recording processed but cleanup failed: {}",
                    e
                )));
            }
        }

        Ok(JobResult::success_with_data(serde_json::json!({
            "path": path,
            "size_bytes": metadata.len(),
            "readonly": metadata.permissions().readonly(),
            "deleted_after_processing": delete_after_processing,
        })))
    }

    /// Process webhook job — delivers HTTP POST to the configured URL.
    async fn process_webhook(&self, job: &Job) -> Result<JobResult> {
        let url = match job.payload.get("url").and_then(|v| v.as_str()) {
            Some(u) => u,
            None => return Ok(JobResult::failure("Missing webhook URL in payload")),
        };
        let payload = match job.payload.get("payload") {
            Some(p) => p.to_string(),
            None => return Ok(JobResult::failure("Missing webhook payload")),
        };

        info!(job_id = %job.id, url = %url, "Delivering webhook");

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .build()
            .map_err(|e| anyhow::anyhow!("Failed to create HTTP client: {}", e))?;

        match client
            .post(url)
            .header("Content-Type", "application/json")
            .header("User-Agent", "FCP-Webhook/1.0")
            .body(payload)
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() || resp.status().is_redirection() => {
                info!(job_id = %job.id, status = %resp.status(), "Webhook delivered");
                Ok(JobResult::success_with_data(serde_json::json!({
                    "webhook_delivered": true,
                    "url": url,
                    "status": resp.status().as_u16(),
                })))
            }
            Ok(resp) => {
                let status = resp.status().as_u16();
                warn!(job_id = %job.id, status, "Webhook delivery failed");
                Ok(JobResult::retry(format!(
                    "Webhook returned HTTP {}",
                    status
                )))
            }
            Err(e) => {
                warn!(job_id = %job.id, error = %e, "Webhook delivery error");
                Ok(JobResult::retry(format!("Webhook error: {}", e)))
            }
        }
    }

    /// Process cleanup job for old completed/failed queue entries.
    async fn process_cleanup(&self, job: &Job) -> Result<JobResult> {
        let retention_days = job
            .payload
            .get("retention_days")
            .and_then(|v| v.as_i64())
            .unwrap_or(30);

        if retention_days < 1 {
            return Ok(JobResult::failure("retention_days must be >= 1"));
        }

        let removed = self.queue.cleanup_old_jobs(retention_days).await?;
        info!(
            job_id = %job.id,
            retention_days,
            removed,
            "Cleanup job completed"
        );

        Ok(JobResult::success_with_data(serde_json::json!({
            "retention_days": retention_days,
            "removed_jobs": removed,
        })))
    }

    /// Process custom job as a generic passthrough task with audit-friendly output.
    async fn process_custom(&self, job: &Job) -> Result<JobResult> {
        let operation = job
            .payload
            .get("operation")
            .and_then(|v| v.as_str())
            .unwrap_or("custom");

        info!(job_id = %job.id, operation = %operation, "Custom job completed");
        Ok(JobResult::success_with_data(serde_json::json!({
            "operation": operation,
            "accepted": true,
            "payload": job.payload.clone(),
        })))
    }
}
