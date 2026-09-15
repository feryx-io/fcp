//! The CDR archive's bookkeeping in Postgres (`docs/CDR_PIPELINE.md` §13,
//! phase 5; the model is `fcp_core::cdr_archive`). Migration 061 holds the
//! tables and the trigger that notes late records.

use anyhow::{bail, Result};
use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use fcp_core::cdr_archive::{CdrArchiveEntry, CdrArchiveStatus, CdrArchiveWrite};
use fcp_core::storage::{CallDetailRecord, CdrCursor};
use fcp_core::TenantId;
use sqlx::{Pool, Postgres, Row};
use std::str::FromStr;
use std::time::SystemTime;
use tracing::warn;

fn midnight(day: NaiveDate) -> DateTime<Utc> {
    Utc.from_utc_datetime(&day.and_hms_opt(0, 0, 0).expect("midnight"))
}

fn next(day: NaiveDate) -> Result<NaiveDate> {
    day.succ_opt()
        .ok_or_else(|| anyhow::anyhow!("no day after {day}"))
}

/// Tenant ids as the record table holds them; one that does not parse is
/// left out, with a warning, rather than failing the walk.
fn tenants(ids: Vec<String>) -> Vec<TenantId> {
    ids.into_iter()
        .filter_map(|id| match TenantId::from_str(&id) {
            Ok(t) => Some(t),
            Err(_) => {
                warn!(tenant_id = %id, "CDRs under a tenant id that does not parse are not archived");
                None
            }
        })
        .collect()
}

/// Every tenant id in the record table, one index probe per tenant per
/// partition rather than a scan of the rows.
const TENANT_WALK: &str = "WITH RECURSIVE t(id) AS (
        SELECT min(tenant_id) FROM cdrs
      UNION ALL
        SELECT (SELECT min(tenant_id) FROM cdrs WHERE tenant_id > t.id)
        FROM t WHERE t.id IS NOT NULL
    )";

pub(crate) async fn next_day(pool: &Pool<Postgres>) -> Result<Option<NaiveDate>> {
    let walked: Option<NaiveDate> = sqlx::query_scalar("SELECT next_day FROM cdr_archive_state")
        .fetch_optional(pool)
        .await?
        .flatten();
    if walked.is_some() {
        return Ok(walked);
    }
    // The first run: start at the oldest record's day. The state row is held
    // while the oldest day is found, so a record committed meanwhile is
    // either seen by the search or noted as late against the day it sets.
    let mut tx = pool.begin().await?;
    let walked: Option<NaiveDate> =
        sqlx::query_scalar("SELECT next_day FROM cdr_archive_state FOR UPDATE")
            .fetch_one(&mut *tx)
            .await?;
    if walked.is_some() {
        tx.commit().await?;
        return Ok(walked);
    }
    let oldest: Option<NaiveDate> = sqlx::query_scalar(&format!(
        "{TENANT_WALK}
         SELECT (min((SELECT min(start_time) FROM cdrs c WHERE c.tenant_id = t.id))
                 AT TIME ZONE 'UTC')::date
         FROM t WHERE t.id IS NOT NULL"
    ))
    .fetch_one(&mut *tx)
    .await?;
    if let Some(day) = oldest {
        sqlx::query("UPDATE cdr_archive_state SET next_day = $1, updated_at = now()")
            .bind(day)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(oldest)
}

pub(crate) async fn advance(pool: &Pool<Postgres>, day: NaiveDate) -> Result<()> {
    // FOR UPDATE waits out every insert holding the row in KEY SHARE: a
    // record for `day` being committed now is either in the walk that just
    // finished or noted late, never neither.
    let mut tx = pool.begin().await?;
    let walked: Option<NaiveDate> =
        sqlx::query_scalar("SELECT next_day FROM cdr_archive_state FOR UPDATE")
            .fetch_one(&mut *tx)
            .await?;
    if walked != Some(day) {
        bail!("the archive walk is at {walked:?}, not {day}");
    }
    sqlx::query("UPDATE cdr_archive_state SET next_day = $1, updated_at = now()")
        .bind(next(day)?)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

pub(crate) async fn tenants_on(pool: &Pool<Postgres>, day: NaiveDate) -> Result<Vec<TenantId>> {
    let ids: Vec<String> = sqlx::query_scalar(&format!(
        "{TENANT_WALK}
         SELECT id FROM t
         WHERE id IS NOT NULL
           AND EXISTS (SELECT 1 FROM cdrs c
                       WHERE c.tenant_id = t.id AND c.start_time >= $1 AND c.start_time < $2)
         ORDER BY id"
    ))
    .bind(midnight(day))
    .bind(midnight(next(day)?))
    .fetch_all(pool)
    .await?;
    Ok(tenants(ids))
}

pub(crate) async fn pending(
    pool: &Pool<Postgres>,
    before: NaiveDate,
    limit: usize,
) -> Result<Vec<(TenantId, NaiveDate)>> {
    let rows = sqlx::query(
        "SELECT tenant_id, day FROM (
             SELECT tenant_id, day FROM cdr_archives WHERE archived_at IS NULL AND day < $1
             UNION
             SELECT tenant_id, day FROM cdr_archive_late WHERE day < $1
         ) p
         ORDER BY day, tenant_id
         LIMIT $2",
    )
    .bind(before)
    .bind(limit as i64)
    .fetch_all(pool)
    .await?;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let day: NaiveDate = row.try_get("day")?;
        if let Some(tenant) = tenants(vec![row.try_get("tenant_id")?]).pop() {
            out.push((tenant, day));
        }
    }
    Ok(out)
}

pub(crate) async fn begin(
    pool: &Pool<Postgres>,
    tenant_id: &TenantId,
    day: NaiveDate,
) -> Result<u64> {
    let mut tx = pool.begin().await?;
    let generation: i64 = sqlx::query_scalar(
        "INSERT INTO cdr_archives (tenant_id, day) VALUES ($1, $2)
         ON CONFLICT (tenant_id, day) DO UPDATE SET
             generation = cdr_archives.generation + 1,
             started_at = now(),
             archived_at = NULL,
             locked = false
         RETURNING generation",
    )
    .bind(tenant_id.to_string())
    .bind(day)
    .fetch_one(&mut *tx)
    .await?;
    // Notes for records committed before now are answered by this write,
    // which reads after it; a record committed later notes itself again.
    sqlx::query("DELETE FROM cdr_archive_late WHERE tenant_id = $1 AND day = $2")
        .bind(tenant_id.to_string())
        .bind(day)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(generation as u64)
}

pub(crate) async fn page(
    pool: &Pool<Postgres>,
    tenant_id: &TenantId,
    day: NaiveDate,
    after: Option<CdrCursor>,
    limit: usize,
) -> Result<Vec<CallDetailRecord>> {
    let mut sql = String::from(
        "SELECT * FROM cdrs WHERE tenant_id = $1 AND start_time >= $2 AND start_time < $3",
    );
    if after.is_some() {
        sql.push_str(" AND (start_time, cdr_id) > ($5, $6)");
    }
    sql.push_str(" ORDER BY start_time, cdr_id LIMIT $4");
    let mut query = sqlx::query(&sql)
        .bind(tenant_id.to_string())
        .bind(midnight(day))
        .bind(midnight(next(day)?))
        .bind(limit as i64);
    if let Some(cursor) = after {
        query = query
            .bind(DateTime::from_timestamp_micros(cursor.start_micros).unwrap_or_default())
            .bind(cursor.cdr_id);
    }
    query
        .fetch_all(pool)
        .await?
        .into_iter()
        .map(crate::row_to_cdr)
        .collect()
}

pub(crate) async fn complete(pool: &Pool<Postgres>, write: &CdrArchiveWrite) -> Result<bool> {
    let query = match &write.object_key {
        Some(key) => sqlx::query(
            "UPDATE cdr_archives
             SET archived_at = now(), object_key = $4, records = $5, bytes = $6, sha256 = $7
             WHERE tenant_id = $1 AND day = $2 AND generation = $3",
        )
        .bind(write.tenant_id.to_string())
        .bind(write.day)
        .bind(write.generation as i64)
        .bind(key)
        .bind(write.records as i64)
        .bind(write.bytes as i64)
        .bind(&write.sha256),
        // No records: whatever an earlier write stored stays described.
        None => sqlx::query(
            "UPDATE cdr_archives SET archived_at = now()
             WHERE tenant_id = $1 AND day = $2 AND generation = $3",
        )
        .bind(write.tenant_id.to_string())
        .bind(write.day)
        .bind(write.generation as i64),
    };
    Ok(query.execute(pool).await?.rows_affected() == 1)
}

pub(crate) async fn get(
    pool: &Pool<Postgres>,
    tenant_id: &TenantId,
    day: NaiveDate,
) -> Result<Option<CdrArchiveEntry>> {
    let row = sqlx::query(
        "SELECT generation, object_key, records, bytes, sha256, archived_at,
                EXISTS (SELECT 1 FROM cdr_archive_late l
                        WHERE l.tenant_id = a.tenant_id AND l.day = a.day) AS late
         FROM cdr_archives a WHERE tenant_id = $1 AND day = $2",
    )
    .bind(tenant_id.to_string())
    .bind(day)
    .fetch_optional(pool)
    .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    Ok(Some(CdrArchiveEntry {
        tenant_id: *tenant_id,
        day,
        generation: row.try_get::<i64, _>("generation")? as u64,
        object_key: row.try_get("object_key")?,
        records: row.try_get::<i64, _>("records")? as u64,
        bytes: row.try_get::<i64, _>("bytes")? as u64,
        sha256: row.try_get("sha256")?,
        archived_at: row
            .try_get::<Option<DateTime<Utc>>, _>("archived_at")?
            .map(SystemTime::from),
        late: row.try_get("late")?,
    }))
}

pub(crate) async fn frontier(
    pool: &Pool<Postgres>,
    tenant_id: &TenantId,
) -> Result<Option<SystemTime>> {
    let day: Option<NaiveDate> = sqlx::query_scalar(
        "SELECT CASE WHEN s.next_day IS NULL THEN NULL ELSE LEAST(
                    s.next_day,
                    (SELECT min(day) FROM cdr_archives
                     WHERE tenant_id = $1 AND archived_at IS NULL),
                    (SELECT min(day) FROM cdr_archive_late WHERE tenant_id = $1))
                END
         FROM cdr_archive_state s",
    )
    .bind(tenant_id.to_string())
    .fetch_optional(pool)
    .await?
    .flatten();
    Ok(day.map(|d| midnight(d).into()))
}

pub(crate) async fn status(pool: &Pool<Postgres>) -> Result<CdrArchiveStatus> {
    let row = sqlx::query(
        "SELECT (SELECT next_day FROM cdr_archive_state) AS next_day,
                count(*) FILTER (WHERE archived_at IS NOT NULL) AS archived_days,
                COALESCE(sum(records) FILTER (WHERE archived_at IS NOT NULL), 0)::BIGINT AS records,
                COALESCE(sum(bytes) FILTER (WHERE archived_at IS NOT NULL), 0)::BIGINT AS bytes,
                max(archived_at) AS last_archived_at,
                (SELECT count(*) FROM (
                     SELECT tenant_id, day FROM cdr_archives WHERE archived_at IS NULL
                     UNION
                     SELECT tenant_id, day FROM cdr_archive_late) p) AS pending_days
         FROM cdr_archives",
    )
    .fetch_one(pool)
    .await?;
    Ok(CdrArchiveStatus {
        next_day: row.try_get("next_day")?,
        archived_days: row.try_get::<i64, _>("archived_days")? as u64,
        pending_days: row.try_get::<i64, _>("pending_days")? as u64,
        records: row.try_get::<i64, _>("records")? as u64,
        bytes: row.try_get::<i64, _>("bytes")? as u64,
        last_archived_at: row
            .try_get::<Option<DateTime<Utc>>, _>("last_archived_at")?
            .map(SystemTime::from),
    })
}

pub(crate) async fn lock_changes(
    pool: &Pool<Postgres>,
    limit: usize,
) -> Result<Vec<(TenantId, NaiveDate, String, bool)>> {
    // A day is held when an active hold on records reaches into it; its
    // subjects and calls are not looked at — the object holds the whole day.
    let rows = sqlx::query(
        "SELECT tenant_id, day, object_key, desired FROM (
             SELECT a.tenant_id, a.day, a.object_key, a.locked,
                    EXISTS (
                        SELECT 1 FROM legal_holds h
                        WHERE h.tenant_id = a.tenant_id AND h.released_at IS NULL
                          AND (cardinality(h.classes) = 0 OR 'cdr' = ANY (h.classes))
                          AND (h.held_from IS NULL OR h.held_from < (a.day + 1)::timestamp AT TIME ZONE 'UTC')
                          AND (h.held_to IS NULL OR h.held_to > a.day::timestamp AT TIME ZONE 'UTC')
                    ) AS desired
             FROM cdr_archives a
             WHERE a.archived_at IS NOT NULL AND a.object_key IS NOT NULL
         ) d
         WHERE locked <> desired
         ORDER BY day, tenant_id
         LIMIT $1",
    )
    .bind(limit as i64)
    .fetch_all(pool)
    .await?;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        if let Some(tenant) = tenants(vec![row.try_get("tenant_id")?]).pop() {
            out.push((
                tenant,
                row.try_get("day")?,
                row.try_get("object_key")?,
                row.try_get("desired")?,
            ));
        }
    }
    Ok(out)
}

pub(crate) async fn set_locked(
    pool: &Pool<Postgres>,
    tenant_id: &TenantId,
    day: NaiveDate,
    locked: bool,
) -> Result<()> {
    sqlx::query("UPDATE cdr_archives SET locked = $3 WHERE tenant_id = $1 AND day = $2")
        .bind(tenant_id.to_string())
        .bind(day)
        .bind(locked)
        .execute(pool)
        .await?;
    Ok(())
}
