//! Job queue implementation using Redis.

use crate::job::{Job, JobId, JobPriority, JobStatus};
use anyhow::{Context, Result};
use redis::aio::ConnectionManager;
use redis::{AsyncCommands, Client};
use tracing::{debug, error, info, warn};

/// Job queue backed by Redis
#[derive(Clone)]
pub struct JobQueue {
    _client: Client,
    connection_manager: ConnectionManager,
    queue_prefix: String,
}

impl JobQueue {
    /// Create a new job queue
    pub async fn new(redis_url: &str, queue_prefix: String) -> Result<Self> {
        let client = Client::open(redis_url).context("Failed to create Redis client")?;

        let connection_manager = ConnectionManager::new(client.clone())
            .await
            .context("Failed to create Redis connection manager")?;

        info!(redis_url = %redis_url, "Connected to Redis");

        Ok(Self {
            _client: client,
            connection_manager,
            queue_prefix,
        })
    }

    /// Enqueue a job
    #[allow(dead_code)]
    pub async fn enqueue(&self, job: Job) -> Result<()> {
        let job_id = job.id;
        let priority = job.priority;
        let job_json = serde_json::to_string(&job).context("Failed to serialize job")?;

        let queue_key = self.queue_key_for_priority(priority);
        let job_key = self.job_key(job_id);

        let mut conn = self.connection_manager.clone();

        // Store job data
        conn.set::<_, _, ()>(&job_key, &job_json)
            .await
            .context("Failed to store job data")?;

        // Add job ID to priority queue (sorted set with timestamp as score)
        let score = chrono::Utc::now().timestamp();
        conn.zadd::<_, _, _, ()>(&queue_key, job_id.to_string(), score)
            .await
            .context("Failed to add job to queue")?;

        debug!(
            job_id = %job_id,
            job_type = %job.job_type,
            priority = ?priority,
            "Job enqueued"
        );

        Ok(())
    }

    /// Dequeue the highest priority job that is ready to run.
    ///
    /// Jobs scheduled for future retry (score > now) are left in the queue.
    pub async fn dequeue(&self) -> Result<Option<Job>> {
        let mut conn = self.connection_manager.clone();
        let now_score = chrono::Utc::now().timestamp() as f64;

        // Try each priority level from highest to lowest
        for priority in [
            JobPriority::Critical,
            JobPriority::High,
            JobPriority::Normal,
            JobPriority::Low,
        ] {
            let queue_key = self.queue_key_for_priority(priority);

            // Peek at the lowest-score job without removing it
            let peek: Vec<(String, f64)> = redis::cmd("ZRANGEBYSCORE")
                .arg(&queue_key)
                .arg("-inf")
                .arg(now_score)
                .arg("WITHSCORES")
                .arg("LIMIT")
                .arg(0i64)
                .arg(1i64)
                .query_async(&mut conn)
                .await
                .context("Failed to peek queue")?;

            if peek.is_empty() {
                continue;
            }

            let (job_id_str, _score) = &peek[0];

            // Remove it atomically
            let removed: i64 = redis::cmd("ZREM")
                .arg(&queue_key)
                .arg(job_id_str.as_str())
                .query_async(&mut conn)
                .await
                .context("Failed to remove job from queue")?;

            if removed == 0 {
                // Another worker grabbed it
                continue;
            }

            let job_key = format!("{}:job:{}", self.queue_prefix, job_id_str);

            // Get job data
            let job_json: Option<String> =
                conn.get(&job_key).await.context("Failed to get job data")?;

            if let Some(json) = job_json {
                let mut job: Job =
                    serde_json::from_str(&json).context("Failed to deserialize job")?;

                debug!(
                    job_id = %job.id,
                    job_type = %job.job_type,
                    priority = ?priority,
                    "Job dequeued"
                );

                // Mark as running
                job.mark_running();

                // Update job status in Redis
                self.update_job(&job).await?;

                return Ok(Some(job));
            }
        }

        Ok(None)
    }

    /// Update job status
    pub async fn update_job(&self, job: &Job) -> Result<()> {
        let job_json = serde_json::to_string(job).context("Failed to serialize job")?;

        let job_key = self.job_key(job.id);

        let mut conn = self.connection_manager.clone();
        conn.set::<_, _, ()>(&job_key, &job_json)
            .await
            .context("Failed to update job")?;

        debug!(job_id = %job.id, status = ?job.status, "Job updated");

        Ok(())
    }

    /// Complete a job
    pub async fn complete_job(&self, job: &Job) -> Result<()> {
        self.update_job(job).await?;

        // Move to completed set
        let completed_key = format!("{}:completed", self.queue_prefix);
        let mut conn = self.connection_manager.clone();
        conn.sadd::<_, _, ()>(&completed_key, job.id.to_string())
            .await
            .context("Failed to add to completed set")?;

        info!(
            job_id = %job.id,
            job_type = %job.job_type,
            duration_ms = (job.updated_at - job.created_at).num_milliseconds(),
            "Job completed"
        );

        Ok(())
    }

    /// Fail a job
    pub async fn fail_job(&self, job: &Job) -> Result<()> {
        self.update_job(job).await?;

        // Move to failed set
        let failed_key = format!("{}:failed", self.queue_prefix);
        let mut conn = self.connection_manager.clone();
        conn.sadd::<_, _, ()>(&failed_key, job.id.to_string())
            .await
            .context("Failed to add to failed set")?;

        error!(
            job_id = %job.id,
            job_type = %job.job_type,
            error = ?job.error,
            "Job failed"
        );

        Ok(())
    }

    /// Retry a job
    pub async fn retry_job(&self, job: &Job) -> Result<()> {
        self.update_job(job).await?;

        // Re-enqueue the job
        if let Some(retry_at) = job.retry_at {
            let queue_key = self.queue_key_for_priority(job.priority);
            let score = retry_at.timestamp();

            let mut conn = self.connection_manager.clone();
            conn.zadd::<_, _, _, ()>(&queue_key, job.id.to_string(), score)
                .await
                .context("Failed to re-enqueue job for retry")?;

            warn!(
                job_id = %job.id,
                job_type = %job.job_type,
                retry_count = job.retry_count,
                retry_at = %retry_at,
                "Job scheduled for retry"
            );
        }

        Ok(())
    }

    /// Get queue statistics
    pub async fn get_stats(&self) -> Result<QueueStats> {
        let mut conn = self.connection_manager.clone();

        let mut stats = QueueStats::default();

        // Count pending jobs by priority
        for priority in [
            JobPriority::Critical,
            JobPriority::High,
            JobPriority::Normal,
            JobPriority::Low,
        ] {
            let queue_key = self.queue_key_for_priority(priority);
            let count: usize = conn.zcard(&queue_key).await.unwrap_or(0);

            match priority {
                JobPriority::Critical => stats.critical_pending = count,
                JobPriority::High => stats.high_pending = count,
                JobPriority::Normal => stats.normal_pending = count,
                JobPriority::Low => stats.low_pending = count,
            }
        }

        // Count completed and failed
        let completed_key = format!("{}:completed", self.queue_prefix);
        let failed_key = format!("{}:failed", self.queue_prefix);

        stats.completed = conn.scard(&completed_key).await.unwrap_or(0);
        stats.failed = conn.scard(&failed_key).await.unwrap_or(0);

        Ok(stats)
    }

    /// Cleanup old completed and failed jobs
    #[allow(dead_code)]
    pub async fn cleanup_old_jobs(&self, retention_days: i64) -> Result<usize> {
        let mut conn = self.connection_manager.clone();
        let cutoff = chrono::Utc::now() - chrono::Duration::days(retention_days);

        let completed_key = format!("{}:completed", self.queue_prefix);
        let failed_key = format!("{}:failed", self.queue_prefix);

        let mut removed = 0;

        // Get all completed and failed job IDs
        let completed: Vec<String> = conn.smembers(&completed_key).await.unwrap_or_default();
        let failed: Vec<String> = conn.smembers(&failed_key).await.unwrap_or_default();

        for job_id_str in completed.iter().chain(failed.iter()) {
            let job_key = format!("{}:job:{}", self.queue_prefix, job_id_str);

            // Get job data
            let job_json: Option<String> = conn.get(&job_key).await.ok().flatten();

            if let Some(json) = job_json {
                if let Ok(job) = serde_json::from_str::<Job>(&json) {
                    if job.updated_at < cutoff {
                        // Delete job data
                        let _: () = conn.del(&job_key).await.unwrap_or(());

                        // Remove from completed/failed set
                        if job.status == JobStatus::Completed {
                            let _: () = conn.srem(&completed_key, job_id_str).await.unwrap_or(());
                        } else if job.status == JobStatus::Failed {
                            let _: () = conn.srem(&failed_key, job_id_str).await.unwrap_or(());
                        }

                        removed += 1;
                    }
                }
            }
        }

        if removed > 0 {
            info!(removed, retention_days, "Cleaned up old jobs");
        }

        Ok(removed)
    }

    /// Get queue key for priority level
    fn queue_key_for_priority(&self, priority: JobPriority) -> String {
        format!("{}:queue:{:?}", self.queue_prefix, priority).to_lowercase()
    }

    /// Get job key
    fn job_key(&self, job_id: JobId) -> String {
        format!("{}:job:{}", self.queue_prefix, job_id)
    }
}

/// Queue statistics
#[derive(Debug, Default)]
pub struct QueueStats {
    pub critical_pending: usize,
    pub high_pending: usize,
    pub normal_pending: usize,
    pub low_pending: usize,
    pub completed: usize,
    pub failed: usize,
}

impl QueueStats {
    pub fn total_pending(&self) -> usize {
        self.critical_pending + self.high_pending + self.normal_pending + self.low_pending
    }
}
