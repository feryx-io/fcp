//! Subject requests in Postgres (`docs/COMPLIANCE_FOLLOWUPS.md` §6.2; the
//! model is `fcp_core::subject_request`, migration 065).

use anyhow::Result;
use chrono::{DateTime, Utc};
use fcp_core::subject_request::{SubjectRequest, SubjectRequestKind, SubjectRequestStatus};
use fcp_core::{SubjectRequestId, TenantId};
use sqlx::{Pool, Postgres, Row};
use std::str::FromStr;
use std::time::SystemTime;

const COLUMNS: &str = "id, tenant_id, kind, subjects, expansion, status, received_at, due_at, \
                       requested_by, reference, started_at, completed_at, report, bundle, error, \
                       confirmed_at, confirmed_by, preview, erasure";

fn at(t: SystemTime) -> DateTime<Utc> {
    DateTime::<Utc>::from(t)
}

fn optional<T: serde::de::DeserializeOwned>(v: Option<serde_json::Value>) -> Result<Option<T>> {
    Ok(match v {
        Some(serde_json::Value::Null) | None => None,
        Some(v) => Some(serde_json::from_value(v)?),
    })
}

fn row_to_request(row: sqlx::postgres::PgRow) -> Result<SubjectRequest> {
    let kind: String = row.try_get("kind")?;
    let status: String = row.try_get("status")?;
    let when = |col: &str| -> Result<Option<SystemTime>> {
        Ok(row
            .try_get::<Option<DateTime<Utc>>, _>(col)?
            .map(SystemTime::from))
    };
    Ok(SubjectRequest {
        id: SubjectRequestId::from_str(&row.try_get::<String, _>("id")?)?,
        tenant_id: TenantId::from_str(&row.try_get::<String, _>("tenant_id")?)?,
        kind: SubjectRequestKind::parse(&kind)
            .ok_or_else(|| anyhow::anyhow!("unknown request kind {kind}"))?,
        subjects: serde_json::from_value(row.try_get("subjects")?)?,
        expansion: optional(row.try_get("expansion")?)?,
        status: SubjectRequestStatus::parse(&status)
            .ok_or_else(|| anyhow::anyhow!("unknown request status {status}"))?,
        received_at: SystemTime::from(row.try_get::<DateTime<Utc>, _>("received_at")?),
        due_at: SystemTime::from(row.try_get::<DateTime<Utc>, _>("due_at")?),
        requested_by: row.try_get("requested_by")?,
        reference: row.try_get("reference")?,
        started_at: when("started_at")?,
        completed_at: when("completed_at")?,
        report: optional(row.try_get("report")?)?,
        bundle: optional(row.try_get("bundle")?)?,
        error: row.try_get("error")?,
        confirmed_at: when("confirmed_at")?,
        confirmed_by: row.try_get("confirmed_by")?,
        preview: optional(row.try_get("preview")?)?,
        erasure: optional(row.try_get("erasure")?)?,
    })
}

pub(crate) async fn create(pool: &Pool<Postgres>, r: &SubjectRequest) -> Result<()> {
    sqlx::query(&format!(
        "INSERT INTO subject_requests ({COLUMNS})
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15,
                 $16, $17, $18, $19)"
    ))
    .bind(r.id.to_string())
    .bind(r.tenant_id.to_string())
    .bind(r.kind.as_str())
    .bind(serde_json::to_value(&r.subjects)?)
    .bind(serde_json::to_value(&r.expansion)?)
    .bind(r.status.as_str())
    .bind(at(r.received_at))
    .bind(at(r.due_at))
    .bind(&r.requested_by)
    .bind(&r.reference)
    .bind(r.started_at.map(at))
    .bind(r.completed_at.map(at))
    .bind(serde_json::to_value(&r.report)?)
    .bind(serde_json::to_value(&r.bundle)?)
    .bind(&r.error)
    .bind(r.confirmed_at.map(at))
    .bind(&r.confirmed_by)
    .bind(serde_json::to_value(&r.preview)?)
    .bind(serde_json::to_value(&r.erasure)?)
    .execute(pool)
    .await?;
    Ok(())
}

pub(crate) async fn get(
    pool: &Pool<Postgres>,
    tenant: &TenantId,
    id: &SubjectRequestId,
) -> Result<Option<SubjectRequest>> {
    sqlx::query(&format!(
        "SELECT {COLUMNS} FROM subject_requests WHERE tenant_id = $1 AND id = $2"
    ))
    .bind(tenant.to_string())
    .bind(id.to_string())
    .fetch_optional(pool)
    .await?
    .map(row_to_request)
    .transpose()
}

pub(crate) async fn list(
    pool: &Pool<Postgres>,
    tenant: &TenantId,
    status: Option<SubjectRequestStatus>,
    limit: usize,
) -> Result<Vec<SubjectRequest>> {
    sqlx::query(&format!(
        "SELECT {COLUMNS} FROM subject_requests
         WHERE tenant_id = $1 AND ($2::text IS NULL OR status = $2)
         ORDER BY received_at DESC, id LIMIT $3"
    ))
    .bind(tenant.to_string())
    .bind(status.map(|s| s.as_str()))
    .bind(limit.min(10_000) as i64)
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(row_to_request)
    .collect()
}

pub(crate) async fn claim(pool: &Pool<Postgres>) -> Result<Option<SubjectRequest>> {
    sqlx::query(&format!(
        "UPDATE subject_requests SET status = 'running', started_at = now()
         WHERE id = (SELECT id FROM subject_requests WHERE status IN ('received', 'confirmed')
                     ORDER BY received_at, id LIMIT 1 FOR UPDATE SKIP LOCKED)
         RETURNING {COLUMNS}"
    ))
    .fetch_optional(pool)
    .await?
    .map(row_to_request)
    .transpose()
}

pub(crate) async fn update(pool: &Pool<Postgres>, r: &SubjectRequest) -> Result<()> {
    sqlx::query(
        "UPDATE subject_requests
         SET expansion = $3, status = $4, started_at = $5, completed_at = $6,
             report = $7, bundle = $8, error = $9, confirmed_at = $10, confirmed_by = $11,
             preview = $12, erasure = $13
         WHERE tenant_id = $1 AND id = $2",
    )
    .bind(r.tenant_id.to_string())
    .bind(r.id.to_string())
    .bind(serde_json::to_value(&r.expansion)?)
    .bind(r.status.as_str())
    .bind(r.started_at.map(at))
    .bind(r.completed_at.map(at))
    .bind(serde_json::to_value(&r.report)?)
    .bind(serde_json::to_value(&r.bundle)?)
    .bind(&r.error)
    .bind(r.confirmed_at.map(at))
    .bind(&r.confirmed_by)
    .bind(serde_json::to_value(&r.preview)?)
    .bind(serde_json::to_value(&r.erasure)?)
    .execute(pool)
    .await?;
    Ok(())
}

pub(crate) async fn requeue_running(pool: &Pool<Postgres>) -> Result<u64> {
    Ok(sqlx::query(
        "UPDATE subject_requests
         SET status = CASE WHEN confirmed_at IS NULL THEN 'received' ELSE 'confirmed' END,
             started_at = NULL
         WHERE status = 'running'",
    )
    .execute(pool)
    .await?
    .rows_affected())
}

pub(crate) async fn expired_bundles(
    pool: &Pool<Postgres>,
    now: SystemTime,
) -> Result<Vec<SubjectRequest>> {
    sqlx::query(&format!(
        "SELECT {COLUMNS} FROM subject_requests
         WHERE bundle IS NOT NULL AND bundle <> 'null'::jsonb
           AND NOT COALESCE((bundle->>'removed')::boolean, false)
           AND (bundle->'expires_at'->>'secs_since_epoch')::bigint < $1
         ORDER BY completed_at LIMIT 500"
    ))
    .bind(
        now.duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0),
    )
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(row_to_request)
    .collect()
}

pub(crate) async fn counts(
    pool: &Pool<Postgres>,
    now: SystemTime,
) -> Result<Vec<(String, u64, u64)>> {
    let rows = sqlx::query(
        "SELECT kind, count(*) AS open, count(*) FILTER (WHERE due_at < $1) AS overdue
         FROM subject_requests WHERE status IN ('received', 'running', 'preview', 'confirmed')
         GROUP BY kind ORDER BY kind",
    )
    .bind(at(now))
    .fetch_all(pool)
    .await?;
    rows.into_iter()
        .map(|r| {
            Ok((
                r.try_get::<String, _>("kind")?,
                r.try_get::<i64, _>("open")?.max(0) as u64,
                r.try_get::<i64, _>("overdue")?.max(0) as u64,
            ))
        })
        .collect()
}
