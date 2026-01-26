//! Worker implementation for processing jobs.

use crate::config::{RetryConfig, WorkerConfig};
use crate::job::{Job, JobResult, JobType};
use crate::queue::JobQueue;
use anyhow::Result;
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
        }
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

            let handle = tokio::spawn(async move {
                let worker = Worker {
                    id: worker_id,
                    queue,
                    config,
                    retry_config,
                    active_workers,
                    shutdown,
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
            * self.retry_config.backoff_multiplier.powi(retry_count as i32);

        Duration::from_secs_f64(delay.min(self.retry_config.max_delay.as_secs_f64()))
    }

    /// Process CDR job
    async fn process_cdr(&self, job: &Job) -> Result<JobResult> {
        info!(job_id = %job.id, "Processing CDR");

        // TODO: Implement CDR processing logic
        // - Extract CDR data from payload
        // - Store in database
        // - Generate billing records
        // - Send to analytics

        tokio::time::sleep(Duration::from_millis(100)).await;

        Ok(JobResult::success_with_data(serde_json::json!({
            "cdr_processed": true,
            "call_id": job.payload.get("call_id")
        })))
    }

    /// Process recording job
    async fn process_recording(&self, job: &Job) -> Result<JobResult> {
        info!(job_id = %job.id, "Processing recording");

        // TODO: Implement recording processing logic
        // - Transcode audio/video
        // - Upload to storage (S3, etc.)
        // - Generate thumbnails
        // - Update database

        tokio::time::sleep(Duration::from_millis(500)).await;

        Ok(JobResult::success_with_data(serde_json::json!({
            "recording_processed": true,
            "recording_id": job.payload.get("recording_id")
        })))
    }

    /// Process webhook job
    async fn process_webhook(&self, job: &Job) -> Result<JobResult> {
        info!(job_id = %job.id, "Delivering webhook");

        // TODO: Implement webhook delivery logic
        // - Extract webhook URL and payload
        // - Make HTTP POST request
        // - Handle response
        // - Retry on failure

        let url = job.payload.get("url").and_then(|v| v.as_str());
        let payload = job.payload.get("payload");

        if url.is_none() || payload.is_none() {
            return Ok(JobResult::failure("Missing URL or payload"));
        }

        tokio::time::sleep(Duration::from_millis(200)).await;

        // Simulate occasional failures for retry testing
        if job.retry_count < 2 && rand::random::<f32>() < 0.3 {
            return Ok(JobResult::retry("Webhook delivery failed, will retry"));
        }

        Ok(JobResult::success_with_data(serde_json::json!({
            "webhook_delivered": true,
            "url": url
        })))
    }

    /// Process cleanup job
    async fn process_cleanup(&self, job: &Job) -> Result<JobResult> {
        info!(job_id = %job.id, "Running cleanup task");

        // TODO: Implement cleanup logic
        // - Delete old CDRs
        // - Delete old recordings
        // - Clean up temporary files
        // - Vacuum database

        tokio::time::sleep(Duration::from_millis(100)).await;

        Ok(JobResult::success_with_data(serde_json::json!({
            "cleanup_completed": true,
            "items_removed": 42
        })))
    }

    /// Process custom job
    async fn process_custom(&self, job: &Job) -> Result<JobResult> {
        info!(job_id = %job.id, "Processing custom job");

        // TODO: Implement custom job processing
        // - Route to custom handlers based on payload

        tokio::time::sleep(Duration::from_millis(50)).await;

        Ok(JobResult::success())
    }
}
