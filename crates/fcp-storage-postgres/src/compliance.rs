//! PostgreSQL implementation of [`ComplianceStore`]: retention/consent
//! policies, the admin audit log, and retention run reports (migration 051).

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use fcp_core::compliance::{
    AuditEvent, AuditQuery, CompliancePolicy, ComplianceStore, ConsentFailureAction, DeletionQuery,
    DeletionRecord, PendingFileDeletion, RecordingConsentMode, RetentionReport,
};
use fcp_core::TenantId;
use sqlx::{Pool, Postgres, Row};
use std::str::FromStr;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

fn to_datetime(st: SystemTime) -> DateTime<Utc> {
    let d = st.duration_since(UNIX_EPOCH).unwrap_or_default();
    DateTime::from_timestamp(d.as_secs() as i64, d.subsec_nanos()).unwrap_or_default()
}

fn to_system_time(dt: DateTime<Utc>) -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(dt.timestamp().max(0) as u64)
}

/// Whether a live row names the file at `path` (an SQL expression): a call
/// or conference recording, a voicemail message or greeting, a prompt, a
/// profile picture.
fn referenced_sql(path: &str) -> String {
    format!(
        "EXISTS (SELECT 1 FROM call_recordings WHERE path = {path})
         OR EXISTS (SELECT 1 FROM conference_recordings WHERE path = {path})
         OR EXISTS (SELECT 1 FROM voicemail_messages WHERE recording_path = {path})
         OR EXISTS (SELECT 1 FROM voicemail_greetings WHERE audio_path = {path})
         OR EXISTS (SELECT 1 FROM prompts WHERE audio_path = {path})
         OR EXISTS (SELECT 1 FROM users WHERE profile_picture IN ({path}, regexp_replace({path}, '^.*/', '')))"
    )
}

#[derive(Clone)]
pub struct PostgresComplianceStore {
    pool: Pool<Postgres>,
}

impl PostgresComplianceStore {
    pub fn new(pool: Pool<Postgres>) -> Self {
        Self { pool }
    }

    fn row_to_audit(row: sqlx::postgres::PgRow) -> anyhow::Result<AuditEvent> {
        let tenant: Option<String> = row.try_get("tenant_id")?;
        let status: i32 = row.try_get("status")?;
        let occurred_at: DateTime<Utc> = row.try_get("occurred_at")?;
        Ok(AuditEvent {
            id: row.try_get("id")?,
            tenant_id: tenant.map(|t| TenantId::from_str(&t)).transpose()?,
            actor: row.try_get("actor")?,
            actor_kind: row.try_get("actor_kind")?,
            method: row.try_get("method")?,
            path: row.try_get("path")?,
            status: status as u16,
            action: row.try_get("action")?,
            detail: row.try_get("detail")?,
            source_ip: row.try_get("source_ip")?,
            occurred_at: to_system_time(occurred_at),
        })
    }
}

#[async_trait]
impl ComplianceStore for PostgresComplianceStore {
    async fn get_compliance_policy(
        &self,
        tenant_id: &TenantId,
    ) -> anyhow::Result<Option<CompliancePolicy>> {
        let row = sqlx::query(
            "SELECT cdr_retention_days, sip_trace_retention_days, flow_trace_retention_days,
                    recording_retention_days, audit_retention_days, voicemail_retention_days,
                    cdr_archive_required,
                    recording_consent, recording_consent_prompt_id, updated_at,
                    recording_consent_on_failure, recording_notice_interval_secs
             FROM compliance_policies WHERE tenant_id = $1",
        )
        .bind(tenant_id.to_string())
        .fetch_optional(&self.pool)
        .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        let days = |col: &str| -> anyhow::Result<Option<u32>> {
            let v: Option<i32> = row.try_get(col)?;
            Ok(v.and_then(|d| u32::try_from(d).ok()))
        };
        let consent: String = row.try_get("recording_consent")?;
        let updated_at: DateTime<Utc> = row.try_get("updated_at")?;
        Ok(Some(CompliancePolicy {
            tenant_id: *tenant_id,
            cdr_retention_days: days("cdr_retention_days")?,
            sip_trace_retention_days: days("sip_trace_retention_days")?,
            flow_trace_retention_days: days("flow_trace_retention_days")?,
            recording_retention_days: days("recording_retention_days")?,
            audit_retention_days: days("audit_retention_days")?,
            voicemail_retention_days: days("voicemail_retention_days")?,
            cdr_archive_required: row.try_get("cdr_archive_required")?,
            recording_consent: if consent == "announce" {
                RecordingConsentMode::Announce
            } else {
                RecordingConsentMode::None
            },
            recording_consent_prompt_id: row.try_get("recording_consent_prompt_id")?,
            recording_consent_on_failure: ConsentFailureAction::parse(
                &row.try_get::<String, _>("recording_consent_on_failure")?,
            )
            .unwrap_or_default(),
            recording_notice_interval_secs: days("recording_notice_interval_secs")?,
            updated_at: to_system_time(updated_at),
        }))
    }

    async fn put_compliance_policy(&self, policy: &CompliancePolicy) -> anyhow::Result<()> {
        let days = |d: Option<u32>| d.map(|v| v as i32);
        sqlx::query(
            "INSERT INTO compliance_policies
                (tenant_id, cdr_retention_days, sip_trace_retention_days,
                 flow_trace_retention_days, recording_retention_days, audit_retention_days,
                 recording_consent, recording_consent_prompt_id, updated_at, cdr_archive_required,
                 voicemail_retention_days, recording_consent_on_failure,
                 recording_notice_interval_secs)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
             ON CONFLICT (tenant_id) DO UPDATE SET
                cdr_retention_days = EXCLUDED.cdr_retention_days,
                sip_trace_retention_days = EXCLUDED.sip_trace_retention_days,
                flow_trace_retention_days = EXCLUDED.flow_trace_retention_days,
                recording_retention_days = EXCLUDED.recording_retention_days,
                audit_retention_days = EXCLUDED.audit_retention_days,
                recording_consent = EXCLUDED.recording_consent,
                recording_consent_prompt_id = EXCLUDED.recording_consent_prompt_id,
                updated_at = EXCLUDED.updated_at,
                cdr_archive_required = EXCLUDED.cdr_archive_required,
                voicemail_retention_days = EXCLUDED.voicemail_retention_days,
                recording_consent_on_failure = EXCLUDED.recording_consent_on_failure,
                recording_notice_interval_secs = EXCLUDED.recording_notice_interval_secs",
        )
        .bind(policy.tenant_id.to_string())
        .bind(days(policy.cdr_retention_days))
        .bind(days(policy.sip_trace_retention_days))
        .bind(days(policy.flow_trace_retention_days))
        .bind(days(policy.recording_retention_days))
        .bind(days(policy.audit_retention_days))
        .bind(policy.recording_consent.to_string())
        .bind(&policy.recording_consent_prompt_id)
        .bind(to_datetime(policy.updated_at))
        .bind(policy.cdr_archive_required)
        .bind(days(policy.voicemail_retention_days))
        .bind(policy.recording_consent_on_failure.as_str())
        .bind(days(policy.recording_notice_interval_secs))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn record_audit_event(&self, event: &AuditEvent) -> anyhow::Result<i64> {
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO admin_audit_log
                (tenant_id, actor, actor_kind, method, path, status, action, detail,
                 source_ip, occurred_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
             RETURNING id",
        )
        .bind(event.tenant_id.map(|t| t.to_string()))
        .bind(&event.actor)
        .bind(&event.actor_kind)
        .bind(&event.method)
        .bind(&event.path)
        .bind(i32::from(event.status))
        .bind(&event.action)
        .bind(&event.detail)
        .bind(&event.source_ip)
        .bind(to_datetime(event.occurred_at))
        .fetch_one(&self.pool)
        .await?;
        Ok(id)
    }

    async fn list_audit_events(&self, query: &AuditQuery) -> anyhow::Result<Vec<AuditEvent>> {
        let mut sql = String::from(
            "SELECT id, tenant_id, actor, actor_kind, method, path, status, action, detail,
                    source_ip, occurred_at
             FROM admin_audit_log WHERE 1 = 1",
        );
        let mut binds: Vec<String> = Vec::new();
        if let Some(t) = &query.tenant_id {
            binds.push(t.to_string());
            sql.push_str(&format!(" AND tenant_id = ${}", binds.len()));
        }
        if let Some(a) = &query.actor {
            binds.push(a.clone());
            sql.push_str(&format!(" AND actor = ${}", binds.len()));
        }
        if let Some(p) = &query.action_prefix {
            binds.push(format!("{p}%"));
            sql.push_str(&format!(" AND action LIKE ${}", binds.len()));
        }
        let since = query.since.map(to_datetime);
        if since.is_some() {
            binds.push(String::new()); // placeholder index; bound as timestamp below
            sql.push_str(&format!(" AND occurred_at >= ${}", binds.len()));
        }
        sql.push_str(" ORDER BY occurred_at DESC, id DESC LIMIT ");
        sql.push_str(&query.limit.clamp(1, 1000).to_string());

        let mut q = sqlx::query(&sql);
        let last = binds.len().saturating_sub(1);
        for (i, b) in binds.iter().enumerate() {
            match since {
                Some(ts) if i == last => q = q.bind(ts),
                _ => q = q.bind(b.clone()),
            }
        }
        let rows = q.fetch_all(&self.pool).await?;
        rows.into_iter().map(Self::row_to_audit).collect()
    }

    async fn delete_audit_events_before(
        &self,
        tenant_id: &TenantId,
        before: SystemTime,
    ) -> anyhow::Result<u64> {
        Ok(crate::deletion::execute(
            &self.pool,
            sqlx::query(
                "DELETE FROM admin_audit_log
                 WHERE tenant_id = $1 AND occurred_at < $2
                   AND fcp_audit_event_hold(tenant_id, occurred_at, actor) IS NULL",
            )
            .bind(tenant_id.to_string())
            .bind(to_datetime(before)),
        )
        .await?)
    }

    async fn record_retention_run(&self, report: &RetentionReport) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO retention_runs
                (tenant_id, ran_at, cdrs_deleted, sip_traces_deleted, flow_traces_deleted,
                 recordings_deleted, conference_recordings_deleted, audit_events_deleted,
                 errors, voicemail_messages_deleted, held)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
        )
        .bind(report.tenant_id.to_string())
        .bind(to_datetime(report.ran_at))
        .bind(report.cdrs_deleted as i64)
        .bind(report.sip_traces_deleted as i64)
        .bind(report.flow_traces_deleted as i64)
        .bind(report.recordings_deleted as i64)
        .bind(report.conference_recordings_deleted as i64)
        .bind(report.audit_events_deleted as i64)
        .bind(&report.errors)
        .bind(report.voicemail_messages_deleted as i64)
        .bind(serde_json::to_value(&report.held)?)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn list_deletions(&self, query: &DeletionQuery) -> anyhow::Result<Vec<DeletionRecord>> {
        let mut sql = String::from(
            "SELECT id, tenant_id, class, item, detail, cause, actor, deleted_at
             FROM deletion_log WHERE 1=1",
        );
        let mut n = 0;
        let mut next = || {
            n += 1;
            n
        };
        if query.tenant_id.is_some() {
            sql.push_str(&format!(" AND tenant_id = ${}", next()));
        }
        if query.class.is_some() {
            sql.push_str(&format!(" AND class = ${}", next()));
        }
        if query.since.is_some() {
            sql.push_str(&format!(" AND deleted_at >= ${}", next()));
        }
        sql.push_str(&format!(
            " ORDER BY deleted_at DESC, id DESC LIMIT ${}",
            next()
        ));
        let mut q = sqlx::query(&sql);
        if let Some(t) = &query.tenant_id {
            q = q.bind(t.to_string());
        }
        if let Some(c) = &query.class {
            q = q.bind(c);
        }
        if let Some(since) = query.since {
            q = q.bind(to_datetime(since));
        }
        q = q.bind(query.limit.clamp(1, 1000) as i64);
        q.fetch_all(&self.pool)
            .await?
            .into_iter()
            .map(|row| {
                let tenant: Option<String> = row.try_get("tenant_id")?;
                let at: DateTime<Utc> = row.try_get("deleted_at")?;
                Ok(DeletionRecord {
                    id: row.try_get("id")?,
                    tenant_id: tenant.and_then(|t| TenantId::from_str(&t).ok()),
                    class: row.try_get("class")?,
                    item: row.try_get("item")?,
                    detail: row.try_get("detail")?,
                    cause: row.try_get("cause")?,
                    actor: row.try_get("actor")?,
                    deleted_at: to_system_time(at),
                })
            })
            .collect()
    }

    async fn record_deletion(&self, record: &DeletionRecord) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO deletion_log (tenant_id, class, item, detail, cause, actor, deleted_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(record.tenant_id.map(|t| t.to_string()))
        .bind(&record.class)
        .bind(&record.item)
        .bind(&record.detail)
        .bind(&record.cause)
        .bind(&record.actor)
        .bind(to_datetime(record.deleted_at))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn due_file_deletions(&self, limit: usize) -> anyhow::Result<Vec<PendingFileDeletion>> {
        let rows = sqlx::query(&format!(
            "SELECT p.id, p.tenant_id, p.class, p.path, p.attempts,
                    ({REFERENCED}) AS still_referenced
             FROM pending_file_deletions p
             WHERE p.next_attempt_at <= now()
             ORDER BY p.next_attempt_at, p.id
             LIMIT $1",
            REFERENCED = referenced_sql("p.path")
        ))
        .bind(limit.clamp(1, 10_000) as i64)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                let tenant: Option<String> = row.try_get("tenant_id")?;
                let attempts: i32 = row.try_get("attempts")?;
                Ok(PendingFileDeletion {
                    id: row.try_get("id")?,
                    tenant_id: tenant.and_then(|t| TenantId::from_str(&t).ok()),
                    class: row.try_get("class")?,
                    path: row.try_get("path")?,
                    attempts: attempts.max(0) as u32,
                    still_referenced: row.try_get("still_referenced")?,
                })
            })
            .collect()
    }

    async fn file_deletion_done(&self, id: i64) -> anyhow::Result<()> {
        sqlx::query("DELETE FROM pending_file_deletions WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn file_deletion_failed(
        &self,
        id: i64,
        error: &str,
        retry_in: Duration,
    ) -> anyhow::Result<()> {
        sqlx::query(
            "UPDATE pending_file_deletions
             SET attempts = attempts + 1, last_error = $2,
                 next_attempt_at = now() + make_interval(secs => $3)
             WHERE id = $1",
        )
        .bind(id)
        .bind(error)
        .bind(retry_in.as_secs_f64())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn referenced_paths(&self, paths: &[String]) -> anyhow::Result<Vec<String>> {
        if paths.is_empty() {
            return Ok(Vec::new());
        }
        Ok(sqlx::query_scalar(&format!(
            "SELECT p FROM unnest($1::text[]) AS p WHERE {}",
            referenced_sql("p")
        ))
        .bind(paths)
        .fetch_all(&self.pool)
        .await?)
    }

    async fn place_hold(&self, hold: &fcp_core::legal_hold::LegalHold) -> anyhow::Result<()> {
        crate::holds::place(&self.pool, hold).await
    }

    async fn get_hold(
        &self,
        tenant_id: &TenantId,
        id: &fcp_core::LegalHoldId,
    ) -> anyhow::Result<Option<fcp_core::legal_hold::LegalHold>> {
        crate::holds::get(&self.pool, tenant_id, id).await
    }

    async fn list_holds(
        &self,
        tenant_id: &TenantId,
        include_released: bool,
    ) -> anyhow::Result<Vec<fcp_core::legal_hold::LegalHold>> {
        crate::holds::list(&self.pool, tenant_id, include_released).await
    }

    async fn release_hold(
        &self,
        tenant_id: &TenantId,
        id: &fcp_core::LegalHoldId,
        release: &fcp_core::legal_hold::HoldRelease,
    ) -> anyhow::Result<bool> {
        crate::holds::release(&self.pool, tenant_id, id, release).await
    }

    async fn held_counts(
        &self,
        tenant_id: &TenantId,
        id: &fcp_core::LegalHoldId,
    ) -> anyhow::Result<Vec<(fcp_core::legal_hold::DataClass, Option<u64>)>> {
        crate::holds::counts(&self.pool, tenant_id, id).await
    }

    async fn hold_on(
        &self,
        tenant_id: &TenantId,
        class: fcp_core::legal_hold::DataClass,
        item: &str,
    ) -> anyhow::Result<Option<fcp_core::LegalHoldId>> {
        crate::holds::hold_on(&self.pool, tenant_id, class, item).await
    }

    async fn held_before(
        &self,
        tenant_id: &TenantId,
        class: fcp_core::legal_hold::DataClass,
        before: SystemTime,
    ) -> anyhow::Result<u64> {
        crate::holds::held_before(&self.pool, tenant_id, class, before).await
    }

    async fn list_retention_runs(
        &self,
        tenant_id: &TenantId,
        limit: usize,
    ) -> anyhow::Result<Vec<RetentionReport>> {
        let rows = sqlx::query(
            "SELECT ran_at, cdrs_deleted, sip_traces_deleted, flow_traces_deleted,
                    recordings_deleted, conference_recordings_deleted, audit_events_deleted,
                    errors, voicemail_messages_deleted, held
             FROM retention_runs WHERE tenant_id = $1
             ORDER BY ran_at DESC, id DESC LIMIT $2",
        )
        .bind(tenant_id.to_string())
        .bind(limit.clamp(1, 1000) as i64)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                let ran_at: DateTime<Utc> = row.try_get("ran_at")?;
                let n = |col: &str| -> anyhow::Result<u64> {
                    let v: i64 = row.try_get(col)?;
                    Ok(v.max(0) as u64)
                };
                Ok(RetentionReport {
                    tenant_id: *tenant_id,
                    ran_at: to_system_time(ran_at),
                    cdrs_deleted: n("cdrs_deleted")?,
                    sip_traces_deleted: n("sip_traces_deleted")?,
                    flow_traces_deleted: n("flow_traces_deleted")?,
                    recordings_deleted: n("recordings_deleted")?,
                    conference_recordings_deleted: n("conference_recordings_deleted")?,
                    voicemail_messages_deleted: n("voicemail_messages_deleted")?,
                    held: serde_json::from_value(row.try_get("held")?).unwrap_or_default(),
                    audit_events_deleted: n("audit_events_deleted")?,
                    errors: row.try_get("errors")?,
                })
            })
            .collect()
    }
}
