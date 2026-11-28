//! Backup destinations and runs (`docs/SCHEDULED_BACKUPS.md` B1).

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use fcp_core::backup::{
    BackupDestination, BackupRetention, BackupRun, BackupRunKind, BackupRunState, BackupSchedule,
    BackupStore, BackupTarget,
};
use sqlx::Row;

use crate::{datetime_to_system_time, system_time_to_datetime, PostgresStorage};

const DESTINATION_COLUMNS: &str = "id, name, target, password, private_key, secret_access_key, \
     schedule, keep, include_key, enabled, created_at, updated_at";
const RUN_COLUMNS: &str = "id, destination_id, kind, state, trigger, requested_at, started_at, \
     finished_at, snapshots, bytes_added, message, node";

impl PostgresStorage {
    async fn destination_from(
        &self,
        row: &sqlx::postgres::PgRow,
    ) -> anyhow::Result<BackupDestination> {
        let mut target: BackupTarget = serde_json::from_value(row.try_get("target")?)?;
        match &mut target {
            BackupTarget::Sftp { private_key, .. } => {
                *private_key = self.open_opt(row.try_get("private_key")?).await?;
            }
            BackupTarget::S3 {
                secret_access_key, ..
            } => {
                *secret_access_key = self.open_opt(row.try_get("secret_access_key")?).await?;
            }
        }
        Ok(BackupDestination {
            id: row.try_get::<String, _>("id")?.parse()?,
            name: row.try_get("name")?,
            target,
            password: self.open_opt(row.try_get("password")?).await?,
            schedule: serde_json::from_value::<BackupSchedule>(row.try_get("schedule")?).unwrap_or(
                BackupSchedule {
                    daily_at: None,
                    cron: None,
                },
            ),
            keep: serde_json::from_value::<BackupRetention>(row.try_get("keep")?)
                .unwrap_or_default(),
            include_key: row.try_get("include_key")?,
            enabled: row.try_get("enabled")?,
            created_at: datetime_to_system_time(row.try_get::<DateTime<Utc>, _>("created_at")?),
            updated_at: datetime_to_system_time(row.try_get::<DateTime<Utc>, _>("updated_at")?),
        })
    }
}

fn run_from(row: &sqlx::postgres::PgRow) -> anyhow::Result<BackupRun> {
    let kind: String = row.try_get("kind")?;
    let state: String = row.try_get("state")?;
    let at = |c: &str| -> anyhow::Result<Option<std::time::SystemTime>> {
        Ok(row
            .try_get::<Option<DateTime<Utc>>, _>(c)?
            .map(datetime_to_system_time))
    };
    Ok(BackupRun {
        id: row.try_get::<String, _>("id")?.parse()?,
        destination_id: row.try_get::<String, _>("destination_id")?.parse()?,
        kind: BackupRunKind::parse(&kind).ok_or_else(|| anyhow::anyhow!("run kind {kind:?}"))?,
        state: BackupRunState::parse(&state)
            .ok_or_else(|| anyhow::anyhow!("run state {state:?}"))?,
        trigger: row.try_get("trigger")?,
        requested_at: datetime_to_system_time(row.try_get::<DateTime<Utc>, _>("requested_at")?),
        started_at: at("started_at")?,
        finished_at: at("finished_at")?,
        snapshots: row.try_get("snapshots")?,
        bytes_added: row.try_get::<i64, _>("bytes_added")?.max(0) as u64,
        message: row.try_get("message")?,
        node: row.try_get("node")?,
    })
}

#[async_trait]
impl BackupStore for PostgresStorage {
    async fn list_backup_destinations(&self) -> anyhow::Result<Vec<BackupDestination>> {
        let rows = sqlx::query(&format!(
            "SELECT {DESTINATION_COLUMNS} FROM backup_destinations ORDER BY name"
        ))
        .fetch_all(self.reads())
        .await?;
        let mut out = Vec::with_capacity(rows.len());
        for r in &rows {
            out.push(self.destination_from(r).await?);
        }
        Ok(out)
    }

    async fn get_backup_destination(
        &self,
        id: &uuid::Uuid,
    ) -> anyhow::Result<Option<BackupDestination>> {
        let row = sqlx::query(&format!(
            "SELECT {DESTINATION_COLUMNS} FROM backup_destinations WHERE id = $1"
        ))
        .bind(id.to_string())
        .fetch_optional(self.reads())
        .await?;
        match row {
            Some(r) => Ok(Some(self.destination_from(&r).await?)),
            None => Ok(None),
        }
    }

    async fn put_backup_destination(&self, d: &BackupDestination) -> anyhow::Result<()> {
        // The secrets come out of the JSON and are sealed in their columns.
        let mut target = d.target.clone();
        let (private_key, secret_access_key) = match &mut target {
            BackupTarget::Sftp { private_key, .. } => (private_key.take(), None),
            BackupTarget::S3 {
                secret_access_key, ..
            } => (None, secret_access_key.take()),
        };
        sqlx::query(
            "INSERT INTO backup_destinations
                 (id, name, target, password, private_key, secret_access_key, schedule, keep,
                  include_key, enabled, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
             ON CONFLICT (id) DO UPDATE SET
                 name = EXCLUDED.name, target = EXCLUDED.target, password = EXCLUDED.password,
                 private_key = EXCLUDED.private_key,
                 secret_access_key = EXCLUDED.secret_access_key,
                 schedule = EXCLUDED.schedule, keep = EXCLUDED.keep,
                 include_key = EXCLUDED.include_key, enabled = EXCLUDED.enabled,
                 updated_at = EXCLUDED.updated_at",
        )
        .bind(d.id.to_string())
        .bind(&d.name)
        .bind(serde_json::to_value(&target)?)
        .bind(self.seal_opt(d.password.as_deref())?)
        .bind(self.seal_opt(private_key.as_deref())?)
        .bind(self.seal_opt(secret_access_key.as_deref())?)
        .bind(serde_json::to_value(&d.schedule)?)
        .bind(serde_json::to_value(d.keep)?)
        .bind(d.include_key)
        .bind(d.enabled)
        .bind(system_time_to_datetime(d.created_at))
        .bind(system_time_to_datetime(d.updated_at))
        .execute(self.writes())
        .await?;
        Ok(())
    }

    async fn delete_backup_destination(&self, id: &uuid::Uuid) -> anyhow::Result<bool> {
        let done = sqlx::query("DELETE FROM backup_destinations WHERE id = $1")
            .bind(id.to_string())
            .execute(self.writes())
            .await?;
        Ok(done.rows_affected() > 0)
    }

    async fn request_backup_run(&self, run: &BackupRun) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO backup_runs (id, destination_id, kind, state, trigger, requested_at)
             VALUES ($1, $2, $3, 'requested', $4, $5)",
        )
        .bind(run.id.to_string())
        .bind(run.destination_id.to_string())
        .bind(run.kind.as_str())
        .bind(&run.trigger)
        .bind(system_time_to_datetime(run.requested_at))
        .execute(self.writes())
        .await?;
        Ok(())
    }

    async fn claim_backup_run(&self, node: &str) -> anyhow::Result<Option<BackupRun>> {
        let row = sqlx::query(&format!(
            "UPDATE backup_runs SET state = 'running', started_at = NOW(), node = $1
              WHERE id = (SELECT id FROM backup_runs WHERE state = 'requested'
                           ORDER BY requested_at LIMIT 1 FOR UPDATE SKIP LOCKED)
              RETURNING {RUN_COLUMNS}"
        ))
        .bind(node)
        .fetch_optional(self.writes())
        .await?;
        row.as_ref().map(run_from).transpose()
    }

    async fn finish_backup_run(&self, run: &BackupRun) -> anyhow::Result<()> {
        sqlx::query(
            "UPDATE backup_runs SET state = $2, finished_at = $3, snapshots = $4,
                    bytes_added = $5, message = $6
              WHERE id = $1",
        )
        .bind(run.id.to_string())
        .bind(run.state.as_str())
        .bind(run.finished_at.map(system_time_to_datetime))
        .bind(&run.snapshots)
        .bind(run.bytes_added.min(i64::MAX as u64) as i64)
        .bind(&run.message)
        .execute(self.writes())
        .await?;
        Ok(())
    }

    async fn get_backup_run(&self, id: &uuid::Uuid) -> anyhow::Result<Option<BackupRun>> {
        let row = sqlx::query(&format!(
            "SELECT {RUN_COLUMNS} FROM backup_runs WHERE id = $1"
        ))
        .bind(id.to_string())
        .fetch_optional(self.reads())
        .await?;
        row.as_ref().map(run_from).transpose()
    }

    async fn list_backup_runs(
        &self,
        destination: Option<&uuid::Uuid>,
        limit: usize,
    ) -> anyhow::Result<Vec<BackupRun>> {
        let rows = sqlx::query(&format!(
            "SELECT {RUN_COLUMNS} FROM backup_runs
              WHERE $1::text IS NULL OR destination_id = $1
              ORDER BY requested_at DESC LIMIT $2"
        ))
        .bind(destination.map(|d| d.to_string()))
        .bind(limit.min(1000) as i64)
        .fetch_all(self.reads())
        .await?;
        rows.iter().map(run_from).collect()
    }

    async fn tenant_files(
        &self,
        tenant: &str,
        limit: usize,
    ) -> anyhow::Result<Vec<fcp_core::backup::TenantFile>> {
        // Each row naming a file the tenant owns, held here rather than at a
        // site; a webinar's stream is a directory and is left out.
        let rows: Vec<(
            String,
            String,
            String,
            Option<chrono::DateTime<chrono::Utc>>,
        )> = sqlx::query_as(
            "SELECT * FROM (
                   SELECT 'call_recording' AS class, id::text AS item, path, started_at AS at
                     FROM call_recordings WHERE tenant_id = $1 AND site_id IS NULL AND path <> ''
                   UNION ALL
                   SELECT 'conference_recording', id, path, started_at
                     FROM conference_recordings
                    WHERE tenant_id = $1 AND site_id IS NULL AND kind <> 'stream' AND path <> ''
                   UNION ALL
                   SELECT 'voicemail', id, recording_path, created_at
                     FROM voicemail_messages
                    WHERE tenant_id = $1 AND site_id IS NULL AND recording_path <> ''
                   UNION ALL
                   SELECT 'voicemail_greeting', g.id::text, g.audio_path, g.created_at
                     FROM voicemail_greetings g JOIN voicemail_mailboxes mb ON mb.id = g.mailbox_id
                    WHERE mb.tenant_id = $1 AND g.audio_path <> ''
                   UNION ALL
                   SELECT 'prompt', id, audio_path, created_at
                     FROM prompts
                    WHERE tenant_id = $1 AND source_type = 'file' AND COALESCE(audio_path, '') <> ''
                 ) f ORDER BY at DESC NULLS LAST LIMIT $2",
        )
        .bind(tenant)
        .bind(limit as i64)
        .fetch_all(self.reads())
        .await?;
        Ok(rows
            .into_iter()
            .map(|(class, item, path, at)| fcp_core::backup::TenantFile {
                class,
                item,
                path,
                created_at: at.map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)),
            })
            .collect())
    }

    async fn active_kek_id(&self) -> anyhow::Result<Option<String>> {
        Ok(sqlx::query_scalar::<_, String>(
            "SELECT kek_id FROM data_encryption_keys WHERE active ORDER BY created_at DESC LIMIT 1",
        )
        .fetch_optional(self.reads())
        .await?)
    }

    async fn record_key_export(&self, kek_id: &str, actor: &str) -> anyhow::Result<()> {
        sqlx::query("INSERT INTO backup_key_exports (kek_id, actor) VALUES ($1, $2)")
            .bind(kek_id)
            .bind(actor)
            .execute(self.writes())
            .await?;
        Ok(())
    }

    async fn last_key_export(
        &self,
        kek_id: &str,
    ) -> anyhow::Result<Option<(std::time::SystemTime, String)>> {
        let row = sqlx::query(
            "SELECT exported_at, actor FROM backup_key_exports WHERE kek_id = $1
              ORDER BY exported_at DESC LIMIT 1",
        )
        .bind(kek_id)
        .fetch_optional(self.reads())
        .await?;
        row.map(|r| {
            Ok((
                datetime_to_system_time(r.try_get::<DateTime<Utc>, _>("exported_at")?),
                r.try_get("actor")?,
            ))
        })
        .transpose()
    }

    async fn last_scheduled_run(
        &self,
        destination: &uuid::Uuid,
    ) -> anyhow::Result<Option<BackupRun>> {
        let row = sqlx::query(&format!(
            "SELECT {RUN_COLUMNS} FROM backup_runs
              WHERE destination_id = $1 AND trigger = 'schedule'
              ORDER BY requested_at DESC LIMIT 1"
        ))
        .bind(destination.to_string())
        .fetch_optional(self.reads())
        .await?;
        row.as_ref().map(run_from).transpose()
    }
}
