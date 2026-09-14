//! Reading and keeping the record table at volume (`docs/CDR_PIPELINE.md`
//! §8–9): monthly partitions, counts with a time budget, hourly usage
//! rollups, and the advisory locks that keep a maintenance job to one host.

use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::{DateTime, Datelike, Duration as TimeDelta, DurationRound, NaiveDate, TimeZone, Utc};
use fcp_core::storage::{MaintenanceLock, RollupRun};
use sqlx::pool::PoolConnection;
use sqlx::{Pool, Postgres, Row};
use std::time::Duration;
use tracing::{info, warn};

/// The partition of `cdrs` holding `month` (its first day).
pub(crate) fn partition_name(month: NaiveDate) -> String {
    format!("cdrs_{:04}_{:02}", month.year(), month.month())
}

/// The first day of the month `offset` months from `day`'s.
pub(crate) fn month_start(day: NaiveDate, offset: i32) -> NaiveDate {
    let months = day.year() * 12 + day.month0() as i32 + offset;
    NaiveDate::from_ymd_opt(months.div_euclid(12), months.rem_euclid(12) as u32 + 1, 1)
        .expect("first of a month")
}

fn midnight(day: NaiveDate) -> DateTime<Utc> {
    Utc.from_utc_datetime(&day.and_hms_opt(0, 0, 0).expect("midnight"))
}

/// Whether `cdrs` is the partitioned table migration 059 made.
async fn is_partitioned(pool: &Pool<Postgres>) -> Result<bool> {
    let kind: Option<String> = sqlx::query_scalar(
        "SELECT c.relkind::text FROM pg_class c
         JOIN pg_namespace n ON n.oid = c.relnamespace
         WHERE c.relname = 'cdrs' AND n.nspname = current_schema()",
    )
    .fetch_optional(pool)
    .await?;
    Ok(kind.as_deref() == Some("p"))
}

/// Partitions for last month through three months ahead, created where
/// missing. A new month's rows already in the default partition (a node
/// that ran past the last partition) are moved into it first, since a
/// partition cannot be attached over rows the default partition holds.
pub(crate) async fn ensure_partitions(pool: &Pool<Postgres>) -> Result<Vec<String>> {
    if !is_partitioned(pool).await? {
        return Ok(Vec::new());
    }
    let today = Utc::now().date_naive();
    let mut created = Vec::new();
    for offset in -1..=3 {
        let lo = month_start(today, offset);
        let hi = month_start(today, offset + 1);
        let name = partition_name(lo);
        let exists: bool = sqlx::query_scalar("SELECT to_regclass($1) IS NOT NULL")
            .bind(&name)
            .fetch_one(pool)
            .await?;
        if exists {
            continue;
        }
        let mut tx = pool.begin().await?;
        // One host at a time, and look again under the lock.
        sqlx::query("SELECT pg_advisory_xact_lock(hashtext('fcp:cdr-partitions'))")
            .execute(&mut *tx)
            .await?;
        let exists: bool = sqlx::query_scalar("SELECT to_regclass($1) IS NOT NULL")
            .bind(&name)
            .fetch_one(&mut *tx)
            .await?;
        if exists {
            tx.rollback().await?;
            continue;
        }
        let (lo_ts, hi_ts) = (midnight(lo), midnight(hi));
        sqlx::query(&format!(
            "CREATE TABLE {name} (LIKE cdrs INCLUDING DEFAULTS)"
        ))
        .execute(&mut *tx)
        .await?;
        let moved = sqlx::query(&format!(
            "INSERT INTO {name} SELECT * FROM cdrs_default
             WHERE start_time >= $1 AND start_time < $2"
        ))
        .bind(lo_ts)
        .bind(hi_ts)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        sqlx::query("DELETE FROM cdrs_default WHERE start_time >= $1 AND start_time < $2")
            .bind(lo_ts)
            .bind(hi_ts)
            .execute(&mut *tx)
            .await?;
        sqlx::query(&format!(
            "ALTER TABLE cdrs ATTACH PARTITION {name} FOR VALUES FROM ('{}') TO ('{}')",
            lo_ts.to_rfc3339(),
            hi_ts.to_rfc3339()
        ))
        .execute(&mut *tx)
        .await
        .with_context(|| format!("attach {name}"))?;
        tx.commit().await?;
        info!(partition = %name, moved_from_default = moved, "Created CDR partition");
        created.push(name);
    }
    Ok(created)
}

/// Drop the monthly partitions that end at or before `before` and hold no
/// rows. A partition with rows left is kept and logged: retention removes
/// rows tenant by tenant first, so a row still there is one it has not
/// reached, and is never dropped here.
pub(crate) async fn drop_partitions_before(
    pool: &Pool<Postgres>,
    before: DateTime<Utc>,
) -> Result<Vec<String>> {
    if !is_partitioned(pool).await? {
        return Ok(Vec::new());
    }
    let names: Vec<String> = sqlx::query_scalar(
        "SELECT c.relname::text FROM pg_inherits i
         JOIN pg_class c ON c.oid = i.inhrelid
         JOIN pg_class p ON p.oid = i.inhparent
         WHERE p.relname = 'cdrs' AND c.relname ~ '^cdrs_[0-9]{4}_[0-9]{2}$'
         ORDER BY c.relname",
    )
    .fetch_all(pool)
    .await?;
    let mut dropped = Vec::new();
    for name in names {
        let Some(month) = NaiveDate::parse_from_str(&format!("{}_01", &name[5..]), "%Y_%m_%d").ok()
        else {
            continue;
        };
        if midnight(month_start(month, 1)) > before {
            continue;
        }
        let empty: bool = sqlx::query_scalar(&format!("SELECT NOT EXISTS (SELECT 1 FROM {name})"))
            .fetch_one(pool)
            .await?;
        if !empty {
            warn!(partition = %name, "CDR partition past retention still holds rows; kept");
            continue;
        }
        sqlx::query(&format!("DROP TABLE {name}"))
            .execute(pool)
            .await?;
        info!(partition = %name, "Dropped CDR partition past retention");
        dropped.push(name);
    }
    Ok(dropped)
}

/// The hour `t` falls in, and the first hour starting at or after `t`.
pub(crate) fn floor_hour(t: DateTime<Utc>) -> DateTime<Utc> {
    t.duration_trunc(TimeDelta::hours(1)).unwrap_or(t)
}

pub(crate) fn ceil_hour(t: DateTime<Utc>) -> DateTime<Utc> {
    let floor = floor_hour(t);
    if floor < t {
        floor + TimeDelta::hours(1)
    } else {
        floor
    }
}

/// The part of `[from, to)` the rollups can answer: whole hours inside it
/// that the last rebuild covered. `None` when there is none (no rebuild
/// yet, or a range inside an hour).
pub(crate) async fn rollup_span(
    pool: &Pool<Postgres>,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> Result<Option<(DateTime<Utc>, DateTime<Utc>)>> {
    let state =
        sqlx::query("SELECT covers_from, covers_until FROM cdr_rollup_state WHERE name = 'usage'")
            .fetch_optional(pool)
            .await?;
    let Some(state) = state else {
        return Ok(None);
    };
    let covers_from: DateTime<Utc> = state.get("covers_from");
    let covers_until: DateTime<Utc> = state.get("covers_until");
    let lo = ceil_hour(from).max(covers_from);
    let hi = floor_hour(to).min(covers_until);
    Ok((lo < hi).then_some((lo, hi)))
}

/// Recompute the usage rollups for `[max(now - window, …), now)` — from the
/// earliest record on the first run, and from where the last run stopped
/// when it stopped longer ago than `window` — in one transaction, so a
/// reader sees the old rollups or the new ones and never half of each.
pub(crate) async fn rebuild_rollups(
    pool: &Pool<Postgres>,
    window: Duration,
    inbound_match: &str,
    outbound_match: &str,
) -> Result<RollupRun> {
    let mut tx = pool.begin().await?;
    // One rebuild at a time, whichever hosts run the job.
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext('fcp:cdr-rollups'))")
        .execute(&mut *tx)
        .await?;
    let now_hour = floor_hour(Utc::now());
    let state = sqlx::query(
        "SELECT covers_from, covers_until FROM cdr_rollup_state WHERE name = 'usage' FOR UPDATE",
    )
    .fetch_optional(&mut *tx)
    .await?;
    let window = TimeDelta::from_std(window).unwrap_or(TimeDelta::hours(48));
    let (lo, covers_from) = match state {
        Some(state) => {
            let covers_from: DateTime<Utc> = state.get("covers_from");
            let covers_until: DateTime<Utc> = state.get("covers_until");
            let lo = (now_hour - window).min(covers_until).max(covers_from);
            (lo, covers_from)
        }
        None => {
            let earliest: Option<DateTime<Utc>> =
                sqlx::query_scalar("SELECT min(start_time) FROM cdrs")
                    .fetch_one(&mut *tx)
                    .await?;
            let lo = earliest.map(floor_hour).unwrap_or(now_hour).min(now_hour);
            (lo, lo)
        }
    };
    let hi = now_hour;
    let mut rows = 0;
    if lo < hi {
        sqlx::query("DELETE FROM cdr_usage_hourly WHERE hour >= $1 AND hour < $2")
            .bind(lo)
            .bind(hi)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM cdr_number_usage_hourly WHERE hour >= $1 AND hour < $2")
            .bind(lo)
            .bind(hi)
            .execute(&mut *tx)
            .await?;
        rows += sqlx::query(&format!(
            "INSERT INTO cdr_usage_hourly
                 (tenant_id, hour, kind, direction, calls, answered, duration_ms, cost)
             SELECT tenant_id, date_trunc('hour', start_time), kind, dir,
                    COUNT(*),
                    COUNT(*) FILTER (WHERE answer_time IS NOT NULL),
                    COALESCE(SUM(call_duration_ms) FILTER (WHERE answer_time IS NOT NULL), 0)::BIGINT,
                    COALESCE(SUM(cost), 0)
             FROM (
                 SELECT c.tenant_id, c.start_time, c.kind, c.answer_time, c.call_duration_ms, c.cost,
                        COALESCE(c.direction,
                            CASE
                                WHEN EXISTS (SELECT 1 FROM phone_numbers pn
                                             WHERE pn.tenant_id = c.tenant_id AND ({inbound_match}))
                                    THEN 'inbound'
                                WHEN EXISTS (SELECT 1 FROM phone_numbers pn
                                             WHERE pn.tenant_id = c.tenant_id AND ({outbound_match}))
                                    THEN 'outbound'
                                ELSE 'internal'
                            END) AS dir
                 FROM cdrs c
                 WHERE c.start_time >= $1 AND c.start_time < $2
             ) r
             GROUP BY tenant_id, date_trunc('hour', start_time), kind, dir"
        ))
        .bind(lo)
        .bind(hi)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        rows += sqlx::query(&format!(
            "WITH matched AS (
                 SELECT DISTINCT c.tenant_id, date_trunc('hour', c.start_time) AS hour, c.cdr_id,
                        pn.e164 AS number, 'outbound' AS direction, c.call_duration_ms, c.answer_time
                 FROM cdrs c
                 JOIN phone_numbers pn ON pn.tenant_id = c.tenant_id AND ({outbound_match})
                 WHERE c.kind = 'call' AND c.start_time >= $1 AND c.start_time < $2
                 UNION ALL
                 SELECT DISTINCT c.tenant_id, date_trunc('hour', c.start_time) AS hour, c.cdr_id,
                        pn.e164 AS number, 'inbound' AS direction, c.call_duration_ms, c.answer_time
                 FROM cdrs c
                 JOIN phone_numbers pn ON pn.tenant_id = c.tenant_id AND ({inbound_match})
                 WHERE c.kind = 'call' AND c.start_time >= $1 AND c.start_time < $2
             ),
             per_call AS (
                 SELECT tenant_id, hour, number, cdr_id,
                        BOOL_OR(answer_time IS NOT NULL) AS answered,
                        MAX(COALESCE(call_duration_ms, 0)) AS duration_ms,
                        BOOL_OR(direction = 'inbound') AS has_inbound,
                        BOOL_OR(direction = 'outbound') AS has_outbound
                 FROM matched
                 GROUP BY tenant_id, hour, number, cdr_id
             )
             INSERT INTO cdr_number_usage_hourly
                 (tenant_id, hour, number, calls, answered, duration_ms, inbound, outbound)
             SELECT tenant_id, hour, number,
                    COUNT(*),
                    COUNT(*) FILTER (WHERE answered),
                    COALESCE(SUM(CASE WHEN answered THEN duration_ms ELSE 0 END), 0)::BIGINT,
                    COUNT(*) FILTER (WHERE has_inbound),
                    COUNT(*) FILTER (WHERE has_outbound)
             FROM per_call
             GROUP BY tenant_id, hour, number"
        ))
        .bind(lo)
        .bind(hi)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    }
    sqlx::query(
        "INSERT INTO cdr_rollup_state (name, covers_from, covers_until, rebuilt_at)
         VALUES ('usage', $1, $2, now())
         ON CONFLICT (name) DO UPDATE SET
             covers_from = LEAST(cdr_rollup_state.covers_from, EXCLUDED.covers_from),
             covers_until = GREATEST(cdr_rollup_state.covers_until, EXCLUDED.covers_until),
             rebuilt_at = now()",
    )
    .bind(covers_from.min(lo))
    .bind(hi)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    let hours = if lo < hi {
        ((hi - lo).num_hours()).max(0) as u64
    } else {
        0
    };
    Ok(RollupRun { hours, rows })
}

/// An advisory lock held on one pooled connection.
pub(crate) struct PgMaintenanceLock {
    conn: Option<PoolConnection<Postgres>>,
    name: String,
}

pub(crate) async fn try_lock(
    pool: &Pool<Postgres>,
    name: &str,
) -> Result<Option<Box<dyn MaintenanceLock>>> {
    let mut conn = pool.acquire().await?;
    let got: bool = sqlx::query_scalar("SELECT pg_try_advisory_lock(hashtext($1))")
        .bind(name)
        .fetch_one(&mut *conn)
        .await?;
    if !got {
        return Ok(None);
    }
    Ok(Some(Box::new(PgMaintenanceLock {
        conn: Some(conn),
        name: name.to_string(),
    })))
}

#[async_trait]
impl MaintenanceLock for PgMaintenanceLock {
    async fn release(mut self: Box<Self>) {
        if let Some(mut conn) = self.conn.take() {
            if let Err(e) = sqlx::query("SELECT pg_advisory_unlock(hashtext($1))")
                .bind(&self.name)
                .execute(&mut *conn)
                .await
            {
                warn!(lock = %self.name, error = %e, "Could not release a maintenance lock; closing its connection");
                drop(conn.detach());
            }
        }
    }
}

impl Drop for PgMaintenanceLock {
    fn drop(&mut self) {
        // Not released: close the connection, which releases the lock,
        // rather than return a locked session to the pool.
        if let Some(conn) = self.conn.take() {
            drop(conn.detach());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn months_step_across_years_and_partitions_are_named_by_month() {
        let d = NaiveDate::from_ymd_opt(2026, 1, 17).unwrap();
        assert_eq!(
            month_start(d, -1),
            NaiveDate::from_ymd_opt(2025, 12, 1).unwrap()
        );
        assert_eq!(
            month_start(d, 0),
            NaiveDate::from_ymd_opt(2026, 1, 1).unwrap()
        );
        assert_eq!(
            month_start(d, 12),
            NaiveDate::from_ymd_opt(2027, 1, 1).unwrap()
        );
        assert_eq!(partition_name(month_start(d, -1)), "cdrs_2025_12");
    }

    #[test]
    fn hours_round_down_and_up() {
        let t = Utc.with_ymd_and_hms(2026, 9, 14, 10, 30, 0).unwrap();
        assert_eq!(
            floor_hour(t),
            Utc.with_ymd_and_hms(2026, 9, 14, 10, 0, 0).unwrap()
        );
        assert_eq!(
            ceil_hour(t),
            Utc.with_ymd_and_hms(2026, 9, 14, 11, 0, 0).unwrap()
        );
        let on = Utc.with_ymd_and_hms(2026, 9, 14, 10, 0, 0).unwrap();
        assert_eq!(ceil_hour(on), on);
    }
}
