//! PostgreSQL implementation of [`ComplianceStore`]: retention/consent
//! policies, the admin audit log, and retention run reports (migration 051).

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use fcp_core::compliance::{
    AuditEvent, AuditQuery, CompliancePolicy, ComplianceStore, RecordingConsentMode,
    RetentionReport,
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
                    recording_retention_days, audit_retention_days, recording_consent,
                    recording_consent_prompt_id, updated_at
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
            recording_consent: if consent == "announce" {
                RecordingConsentMode::Announce
            } else {
                RecordingConsentMode::None
            },
            recording_consent_prompt_id: row.try_get("recording_consent_prompt_id")?,
            updated_at: to_system_time(updated_at),
        }))
    }

    async fn put_compliance_policy(&self, policy: &CompliancePolicy) -> anyhow::Result<()> {
        let days = |d: Option<u32>| d.map(|v| v as i32);
        sqlx::query(
            "INSERT INTO compliance_policies
                (tenant_id, cdr_retention_days, sip_trace_retention_days,
                 flow_trace_retention_days, recording_retention_days, audit_retention_days,
                 recording_consent, recording_consent_prompt_id, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
             ON CONFLICT (tenant_id) DO UPDATE SET
                cdr_retention_days = EXCLUDED.cdr_retention_days,
                sip_trace_retention_days = EXCLUDED.sip_trace_retention_days,
                flow_trace_retention_days = EXCLUDED.flow_trace_retention_days,
                recording_retention_days = EXCLUDED.recording_retention_days,
                audit_retention_days = EXCLUDED.audit_retention_days,
                recording_consent = EXCLUDED.recording_consent,
                recording_consent_prompt_id = EXCLUDED.recording_consent_prompt_id,
                updated_at = EXCLUDED.updated_at",
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
        let result =
            sqlx::query("DELETE FROM admin_audit_log WHERE tenant_id = $1 AND occurred_at < $2")
                .bind(tenant_id.to_string())
                .bind(to_datetime(before))
                .execute(&self.pool)
                .await?;
        Ok(result.rows_affected())
    }

    async fn record_retention_run(&self, report: &RetentionReport) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO retention_runs
                (tenant_id, ran_at, cdrs_deleted, sip_traces_deleted, flow_traces_deleted,
                 recordings_deleted, conference_recordings_deleted, audit_events_deleted,
                 errors)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
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
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn list_retention_runs(
        &self,
        tenant_id: &TenantId,
        limit: usize,
    ) -> anyhow::Result<Vec<RetentionReport>> {
        let rows = sqlx::query(
            "SELECT ran_at, cdrs_deleted, sip_traces_deleted, flow_traces_deleted,
                    recordings_deleted, conference_recordings_deleted, audit_events_deleted,
                    errors
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
                    audit_events_deleted: n("audit_events_deleted")?,
                    errors: row.try_get("errors")?,
                })
            })
            .collect()
    }
}
