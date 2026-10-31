//! Calls' quality as phones reported it (`docs/CALL_DIAGNOSTICS.md` §2).

use std::str::FromStr;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use fcp_core::call_quality::{CallQuality, CallQualityStore};
use fcp_core::{CallId, TenantId, UserId};
use sqlx::postgres::PgRow;
use sqlx::Row;

use crate::{datetime_to_system_time, system_time_to_datetime, PostgresStorage};

const COLUMNS: &str = "tenant_id, call_id, user_id, device_id, direction, codec, srtp,
    duration_seconds, packets_sent, packets_received, packets_expected, packets_lost,
    loss_pct, jitter_ms, rtt_ms, far_loss_pct, far_jitter_ms, far_lost, mos, far_mos,
    local_rtp, remote_rtp, reported_at";

fn from_row(row: &PgRow) -> anyhow::Result<CallQuality> {
    let count =
        |name: &str| -> anyhow::Result<u64> { Ok(row.try_get::<i64, _>(name)?.max(0) as u64) };
    let device: String = row.try_get("device_id")?;
    Ok(CallQuality {
        tenant_id: TenantId::from_str(&row.try_get::<String, _>("tenant_id")?)?,
        call_id: CallId::from_str(&row.try_get::<String, _>("call_id")?)?,
        user_id: UserId::from_str(&row.try_get::<String, _>("user_id")?)?,
        device_id: (!device.is_empty()).then_some(device),
        direction: row.try_get("direction")?,
        codec: row.try_get("codec")?,
        srtp: row.try_get("srtp")?,
        duration_seconds: row
            .try_get::<Option<i32>, _>("duration_seconds")?
            .map(|d| d.max(0) as u32),
        packets_sent: count("packets_sent")?,
        packets_received: count("packets_received")?,
        packets_expected: count("packets_expected")?,
        packets_lost: count("packets_lost")?,
        loss_pct: row.try_get("loss_pct")?,
        jitter_ms: row.try_get("jitter_ms")?,
        rtt_ms: row.try_get("rtt_ms")?,
        far_loss_pct: row.try_get("far_loss_pct")?,
        far_jitter_ms: row.try_get("far_jitter_ms")?,
        far_lost: row.try_get("far_lost")?,
        mos: row.try_get("mos")?,
        far_mos: row.try_get("far_mos")?,
        local_rtp: row.try_get("local_rtp")?,
        remote_rtp: row.try_get("remote_rtp")?,
        reported_at: datetime_to_system_time(row.try_get::<DateTime<Utc>, _>("reported_at")?),
    })
}

#[async_trait]
impl CallQualityStore for PostgresStorage {
    async fn put_call_quality(&self, q: &CallQuality) -> anyhow::Result<()> {
        let count = |n: u64| n.min(i64::MAX as u64) as i64;
        sqlx::query(&format!(
            "INSERT INTO call_quality ({COLUMNS})
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16,
                     $17, $18, $19, $20, $21, $22, $23)
             ON CONFLICT (tenant_id, call_id, user_id, device_id) DO UPDATE SET
                direction = EXCLUDED.direction, codec = EXCLUDED.codec, srtp = EXCLUDED.srtp,
                duration_seconds = EXCLUDED.duration_seconds,
                packets_sent = EXCLUDED.packets_sent,
                packets_received = EXCLUDED.packets_received,
                packets_expected = EXCLUDED.packets_expected,
                packets_lost = EXCLUDED.packets_lost, loss_pct = EXCLUDED.loss_pct,
                jitter_ms = EXCLUDED.jitter_ms, rtt_ms = EXCLUDED.rtt_ms,
                far_loss_pct = EXCLUDED.far_loss_pct, far_jitter_ms = EXCLUDED.far_jitter_ms,
                far_lost = EXCLUDED.far_lost, mos = EXCLUDED.mos, far_mos = EXCLUDED.far_mos,
                local_rtp = EXCLUDED.local_rtp, remote_rtp = EXCLUDED.remote_rtp,
                reported_at = EXCLUDED.reported_at"
        ))
        .bind(q.tenant_id.to_string())
        .bind(q.call_id.to_string())
        .bind(q.user_id.to_string())
        .bind(q.device_id.clone().unwrap_or_default())
        .bind(&q.direction)
        .bind(&q.codec)
        .bind(q.srtp)
        .bind(q.duration_seconds.map(|d| d.min(i32::MAX as u32) as i32))
        .bind(count(q.packets_sent))
        .bind(count(q.packets_received))
        .bind(count(q.packets_expected))
        .bind(count(q.packets_lost))
        .bind(q.loss_pct)
        .bind(q.jitter_ms)
        .bind(q.rtt_ms)
        .bind(q.far_loss_pct)
        .bind(q.far_jitter_ms)
        .bind(q.far_lost)
        .bind(q.mos)
        .bind(q.far_mos)
        .bind(&q.local_rtp)
        .bind(&q.remote_rtp)
        .bind(system_time_to_datetime(q.reported_at))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn call_quality(
        &self,
        tenant: &TenantId,
        call: &CallId,
    ) -> anyhow::Result<Vec<CallQuality>> {
        let rows = sqlx::query(&format!(
            "SELECT {COLUMNS} FROM call_quality WHERE tenant_id = $1 AND call_id = $2
             ORDER BY reported_at"
        ))
        .bind(tenant.to_string())
        .bind(call.to_string())
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(from_row).collect()
    }

    async fn call_qualities_of_user(
        &self,
        user: &UserId,
        calls: &[CallId],
    ) -> anyhow::Result<Vec<CallQuality>> {
        if calls.is_empty() {
            return Ok(Vec::new());
        }
        let ids: Vec<String> = calls.iter().map(ToString::to_string).collect();
        let rows = sqlx::query(&format!(
            "SELECT {COLUMNS} FROM call_quality WHERE user_id = $1 AND call_id = ANY($2)
             ORDER BY reported_at"
        ))
        .bind(user.to_string())
        .bind(&ids)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(from_row).collect()
    }
}
