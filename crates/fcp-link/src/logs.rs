//! The site's warnings and errors, carried to the cloud (`docs/LOG_VIEWER.md`
//! §5, L5). Every service at the site ships its lines to the site's Redis
//! (`fcp_cluster::logging`), where the site's own Logs page reads them all;
//! this reads each service's stream after where it last was, keeps what is
//! `WARN` or worse, and posts it to the cloud's `/link/v1/logs`. A stream
//! seen at the start is read from its end (a restart does not send the
//! day again); one that appears later, from its start.

use anyhow::{Context, Result};
use fcp_cluster::logging::{self, LogRecord, ShippedLogs};
use fcp_storage_redis::RedisStorage;
use std::collections::HashMap;
use std::time::Duration;
use tracing::{debug, warn};

/// How often the site's streams are read.
const EVERY: Duration = Duration::from_secs(5);
/// Lines read from one stream at a time.
const BATCH: usize = 500;

pub struct LogForwarder {
    redis: RedisStorage,
    client: reqwest::Client,
    link_url: String,
    secret: String,
    /// Each stream's last id read.
    cursors: tokio::sync::Mutex<HashMap<String, String>>,
    started: tokio::sync::Mutex<bool>,
}

impl LogForwarder {
    pub fn new(
        redis: RedisStorage,
        client: reqwest::Client,
        link_url: &str,
        secret: String,
    ) -> Self {
        Self {
            redis,
            client,
            link_url: link_url.trim_end_matches('/').to_string(),
            secret,
            cursors: Default::default(),
            started: tokio::sync::Mutex::new(false),
        }
    }

    pub async fn run(&self) {
        loop {
            if let Err(e) = self.step().await {
                warn!(error = %format!("{e:#}"), "Carrying the site's warnings to the cloud failed; trying again");
            }
            tokio::time::sleep(EVERY).await;
        }
    }

    /// The services with lines at the site.
    async fn streams(&self) -> Result<Vec<String>> {
        let prefix = self.redis.key(&logging::stream_key(""));
        let skip = self.redis.key(&logging::level_key(""));
        let mut conn = self.redis.connection();
        let mut keys = Vec::new();
        let mut cursor: u64 = 0;
        loop {
            let (next, batch): (u64, Vec<String>) = redis::cmd("SCAN")
                .arg(cursor)
                .arg("MATCH")
                .arg(format!("{prefix}*"))
                .arg("COUNT")
                .arg(500)
                .query_async(&mut conn)
                .await?;
            keys.extend(batch);
            cursor = next;
            if cursor == 0 {
                break;
            }
        }
        Ok(keys
            .into_iter()
            .filter(|k| !k.starts_with(&skip))
            .filter_map(|k| k.strip_prefix(&prefix).map(str::to_string))
            .collect())
    }

    async fn step(&self) -> Result<()> {
        let services = self
            .streams()
            .await
            .context("listing the site's log streams")?;
        let mut conn = self.redis.connection();
        let mut started = self.started.lock().await;
        let mut cursors = self.cursors.lock().await;
        let mut batches = Vec::new();
        let mut moved: Vec<(String, String)> = Vec::new();
        for service in services {
            let key = self.redis.key(&logging::stream_key(&service));
            let from = match cursors.get(&service) {
                Some(id) => id.clone(),
                None if !*started => {
                    // Seen at the start: from its end.
                    let last: Vec<(String, Vec<(String, String)>)> = redis::cmd("XREVRANGE")
                        .arg(&key)
                        .arg("+")
                        .arg("-")
                        .arg("COUNT")
                        .arg(1)
                        .query_async(&mut conn)
                        .await?;
                    let id = last
                        .first()
                        .map(|(id, _)| id.clone())
                        .unwrap_or("0-0".into());
                    cursors.insert(service.clone(), id);
                    continue;
                }
                None => "0-0".to_string(),
            };
            let entries: Vec<(String, Vec<(String, String)>)> = redis::cmd("XRANGE")
                .arg(&key)
                .arg(format!("({from}"))
                .arg("+")
                .arg("COUNT")
                .arg(BATCH)
                .query_async(&mut conn)
                .await?;
            let Some(last) = entries.last().map(|(id, _)| id.clone()) else {
                continue;
            };
            let lines: Vec<LogRecord> = entries
                .iter()
                .filter_map(|(_, fields)| fields.iter().find(|(k, _)| k == "r"))
                .filter_map(|(_, v)| serde_json::from_str::<LogRecord>(v).ok())
                .filter(|r| matches!(r.level.as_str(), "WARN" | "ERROR"))
                .collect();
            moved.push((service.clone(), last));
            if !lines.is_empty() {
                batches.push(ShippedLogs { service, lines });
            }
        }
        *started = true;
        if !batches.is_empty() {
            let count: usize = batches.iter().map(|b| b.lines.len()).sum();
            let response = self
                .client
                .post(format!("{}/link/v1/logs", self.link_url))
                .bearer_auth(&self.secret)
                .json(&batches)
                .send()
                .await
                .context("reaching the cloud with the site's warnings")?;
            let status = response.status();
            if !status.is_success() {
                let body = response.text().await.unwrap_or_default();
                anyhow::bail!("the cloud refused the site's warnings: {status} {body}");
            }
            debug!(lines = count, "The site's warnings carried to the cloud");
        }
        // Only once the cloud has them: a failure reads them again.
        for (service, last) in moved {
            cursors.insert(service, last);
        }
        Ok(())
    }
}
