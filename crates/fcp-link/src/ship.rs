//! Records up (`docs/HYBRID_CONTROL_PLANE.md` §3.3): the site's call
//! records, its recordings' and voicemail's rows, and the deletions its
//! log recorded, read after a cursor per table in shipping order and sent
//! to the cloud in batches. The cursors are kept beside the certificate;
//! a batch the cloud did not take is sent again, and the cloud takes each
//! record once.

use crate::sync::SiteStores;
use anyhow::{Context, Result};
use fcp_core::compliance::DeletionRecord;
use fcp_core::storage::{CdrStore, VoicemailStore};
use fcp_core::{ShippedAnswer, ShippedDeletion, ShippedRecord, ShippedRecords, TenantId};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};
use tracing::{debug, info, warn};

/// Where each table's shipping stands: the last sequence the cloud took.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShipCursors {
    #[serde(default)]
    pub cdrs: i64,
    #[serde(default)]
    pub call_recordings: i64,
    #[serde(default)]
    pub conference_recordings: i64,
    #[serde(default)]
    pub voicemail_mailboxes: i64,
    #[serde(default)]
    pub voicemail_messages: i64,
    #[serde(default)]
    pub deletions: i64,
    #[serde(default)]
    pub personal_contacts: i64,
    #[serde(default)]
    pub presence_statuses: i64,
    #[serde(default)]
    pub user_presence: i64,
    /// The site's speech usage (docs/TENANT_SPEECH.md S5).
    #[serde(default)]
    pub speech_usage: i64,
}

/// What the status page shows of the shipping.
#[derive(Debug, Clone, Serialize)]
pub struct ShipStatus {
    pub cursors: ShipCursors,
    /// Records the cloud took since start.
    pub shipped: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_shipped_at: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// The deletion classes the cloud mirrors.
const SHIPPED_DELETION_CLASSES: [&str; 3] = ["call_recording", "conference_recording", "voicemail"];

pub struct Shipper {
    stores: SiteStores,
    client: reqwest::Client,
    link_url: String,
    secret: String,
    tenant: TenantId,
    state_path: PathBuf,
    batch: usize,
    interval: Duration,
    cursors: Mutex<ShipCursors>,
    shipped: AtomicU64,
    last_shipped_at: Mutex<Option<SystemTime>>,
    error: Mutex<Option<String>>,
}

impl Shipper {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        stores: SiteStores,
        client: reqwest::Client,
        link_url: &str,
        secret: String,
        tenant: TenantId,
        state_dir: &Path,
        batch: usize,
        interval: Duration,
    ) -> Result<Self> {
        let state_path = state_dir.join("ship.json");
        let cursors: ShipCursors = match std::fs::read_to_string(&state_path) {
            Ok(text) => serde_json::from_str(&text)
                .with_context(|| format!("reading {}", state_path.display()))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => ShipCursors::default(),
            Err(e) => return Err(e).with_context(|| format!("reading {}", state_path.display())),
        };
        if cursors != ShipCursors::default() {
            info!(
                ?cursors,
                "Records shipped through the kept cursors; continuing from there"
            );
        }
        Ok(Self {
            stores,
            client,
            link_url: link_url.trim_end_matches('/').to_string(),
            secret,
            tenant,
            state_path,
            batch: batch.clamp(1, 5000),
            interval,
            cursors: Mutex::new(cursors),
            shipped: AtomicU64::new(0),
            last_shipped_at: Mutex::new(None),
            error: Mutex::new(None),
        })
    }

    pub fn status(&self) -> ShipStatus {
        ShipStatus {
            cursors: self
                .cursors
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone(),
            shipped: self.shipped.load(Ordering::Relaxed),
            last_shipped_at: self
                .last_shipped_at
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .map(|t| {
                    t.duration_since(SystemTime::UNIX_EPOCH)
                        .map(|d| d.as_secs())
                        .unwrap_or_default()
                }),
            error: self.error.lock().unwrap_or_else(|e| e.into_inner()).clone(),
        }
    }

    pub async fn run(&self) {
        loop {
            match self.tick().await {
                Ok(0) => {
                    *self.error.lock().unwrap_or_else(|e| e.into_inner()) = None;
                    tokio::time::sleep(self.interval).await;
                }
                Ok(n) => {
                    *self.error.lock().unwrap_or_else(|e| e.into_inner()) = None;
                    self.shipped.fetch_add(n as u64, Ordering::Relaxed);
                    *self
                        .last_shipped_at
                        .lock()
                        .unwrap_or_else(|e| e.into_inner()) = Some(SystemTime::now());
                    // More may wait: straight on.
                }
                Err(e) => {
                    let text = format!("{e:#}");
                    warn!(error = %text, "Shipping records failed; trying again");
                    *self.error.lock().unwrap_or_else(|e| e.into_inner()) = Some(text);
                    tokio::time::sleep(Duration::from_secs(5)).await;
                }
            }
        }
    }

    /// One pass over every table; how many records the cloud took.
    async fn tick(&self) -> Result<usize> {
        let mut shipped = self.ship_cdrs().await?;
        shipped += self.ship_records().await?;
        Ok(shipped)
    }

    fn cursors(&self) -> ShipCursors {
        self.cursors
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    fn keep(&self, cursors: ShipCursors) -> Result<()> {
        let text = serde_json::to_string_pretty(&cursors)?;
        std::fs::write(&self.state_path, text)
            .with_context(|| format!("writing {}", self.state_path.display()))?;
        *self.cursors.lock().unwrap_or_else(|e| e.into_inner()) = cursors;
        Ok(())
    }

    async fn post<T: Serialize + ?Sized>(
        &self,
        path: &str,
        body: &T,
        what: &str,
    ) -> Result<ShippedAnswer> {
        let response = self
            .client
            .post(format!("{}{}", self.link_url, path))
            .bearer_auth(&self.secret)
            .json(body)
            .send()
            .await
            .with_context(|| format!("reaching the cloud with {what}"))?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("the cloud refused {what}: {status} {body}");
        }
        response
            .json()
            .await
            .with_context(|| format!("reading the cloud's answer to {what}"))
    }

    async fn ship_cdrs(&self) -> Result<usize> {
        let cursors = self.cursors();
        let rows = self
            .stores
            .storage
            .cdrs_after(cursors.cdrs, self.batch)
            .await
            .context("reading the site's records")?;
        if rows.is_empty() {
            return Ok(0);
        }
        let last = rows.last().map(|(seq, _)| *seq).unwrap_or(cursors.cdrs);
        let records: Vec<_> = rows
            .into_iter()
            .map(|(_, r)| r)
            .filter(|r| r.tenant_id == self.tenant)
            .collect();
        let count = records.len();
        let answer = self
            .post("/link/v1/cdrs", &records, "the site's records")
            .await?;
        for (index, reason) in &answer.rejected {
            warn!(index, reason, "The cloud refused one of the site's records");
        }
        debug!(
            sent = count,
            stored = answer.applied,
            skipped = answer.skipped,
            through = last,
            "Records shipped"
        );
        let mut cursors = cursors;
        cursors.cdrs = last;
        self.keep(cursors)?;
        Ok(answer.applied)
    }

    async fn ship_records(&self) -> Result<usize> {
        let cursors = self.cursors();
        let mut next = cursors.clone();
        let mut batch = ShippedRecords::default();
        let storage = &self.stores.storage;
        if let Some(store) = storage.call_recording_store() {
            for (seq, row) in store
                .call_recordings_after(cursors.call_recordings, self.batch)
                .await
                .context("reading the site's call recordings")?
            {
                next.call_recordings = seq;
                if row.tenant_id == self.tenant {
                    batch.puts.push(record("call_recordings", &row.id, &row)?);
                }
            }
        }
        if let Some(store) = storage.recording_store() {
            for (seq, row) in store
                .recordings_after(cursors.conference_recordings, self.batch)
                .await
                .context("reading the site's conference recordings")?
            {
                next.conference_recordings = seq;
                if row.tenant_id == self.tenant {
                    batch
                        .puts
                        .push(record("conference_recordings", &row.id, &row)?);
                }
            }
        }
        for (seq, row) in storage
            .mailboxes_after(cursors.voicemail_mailboxes, self.batch)
            .await
            .context("reading the site's mailboxes")?
        {
            next.voicemail_mailboxes = seq;
            if row.tenant_id == self.tenant {
                batch
                    .puts
                    .push(record("voicemail_mailboxes", &row.id, &row)?);
            }
        }
        for (seq, row) in storage
            .messages_after(cursors.voicemail_messages, self.batch)
            .await
            .context("reading the site's voicemail")?
        {
            next.voicemail_messages = seq;
            batch
                .puts
                .push(record("voicemail_messages", &row.id, &row)?);
        }
        // Users' contacts, written here or taken from the cloud: the
        // cloud keeps what is newer than its copy, so the echo of its own
        // change is let go there (docs/RECORD_AND_CONTACTS.md §3).
        if let Some(store) = storage.contact_store() {
            for (seq, row) in store
                .contacts_after(cursors.personal_contacts, self.batch)
                .await
                .context("reading the site's contacts")?
            {
                next.personal_contacts = seq;
                if row.tenant_id == self.tenant {
                    batch.puts.push(record("personal_contacts", &row.id, &row)?);
                }
            }
        }
        // Presence a person sets, the same way (docs/PRESENCE.md §3): a
        // deleted status is a tombstone and a cleared one a plain
        // `available` row, so both cross as rows.
        if let Some(store) = storage.presence_status_store() {
            for (seq, row) in store
                .own_statuses_after(cursors.presence_statuses, self.batch)
                .await
                .context("reading the site's presence statuses")?
            {
                next.presence_statuses = seq;
                if row.tenant_id == self.tenant {
                    batch.puts.push(record("presence_statuses", &row.id, &row)?);
                }
            }
            for (seq, row) in store
                .chosen_statuses_after(cursors.user_presence, self.batch)
                .await
                .context("reading the site's chosen statuses")?
            {
                next.user_presence = seq;
                if row.tenant_id == self.tenant {
                    batch
                        .puts
                        .push(record("user_presence", &row.user_id, &row)?);
                }
            }
        }
        // The site's speech by hour (docs/TENANT_SPEECH.md S5): a row is
        // shipped again as its hour fills, the cloud keeping the last.
        if let Some(store) = storage.speech_usage_store() {
            for (seq, row) in store
                .speech_usage_after(cursors.speech_usage, self.batch)
                .await
                .context("reading the site's speech usage")?
            {
                next.speech_usage = seq;
                if row.tenant_id == self.tenant {
                    let id = format!(
                        "{}:{}",
                        row.provider,
                        row.hour
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_secs()
                    );
                    batch.puts.push(record("speech_usage", &id, &row)?);
                }
            }
        }
        if let Some(store) = storage.compliance_store() {
            let deletions: Vec<DeletionRecord> = store
                .deletions_after(cursors.deletions, self.batch)
                .await
                .context("reading the site's deletion log")?;
            for d in deletions {
                next.deletions = d.id;
                if SHIPPED_DELETION_CLASSES.contains(&d.class.as_str())
                    && d.tenant_id.is_none_or(|t| t == self.tenant)
                {
                    batch.deletions.push(ShippedDeletion {
                        class: d.class,
                        item: d.item,
                    });
                }
            }
        }
        if next == cursors {
            return Ok(0);
        }
        let count = batch.puts.len() + batch.deletions.len();
        let answer = if count == 0 {
            ShippedAnswer::default()
        } else {
            self.post(
                "/link/v1/records",
                &batch,
                "the site's recordings, voicemail, contacts and presence",
            )
            .await?
        };
        debug!(
            sent = count,
            applied = answer.applied,
            skipped = answer.skipped,
            "Recording and voicemail rows shipped"
        );
        self.keep(next)?;
        Ok(answer.applied)
    }
}

fn record<T: Serialize>(table: &str, id: &dyn std::fmt::Display, row: &T) -> Result<ShippedRecord> {
    Ok(ShippedRecord {
        table: table.to_string(),
        id: id.to_string(),
        object: serde_json::to_value(row)?,
    })
}
