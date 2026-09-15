//! PostgreSQL implementation of the SIP trace message store.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use fcp_core::sip_trace::{SipTraceMessage, SipTraceStore};
use sqlx::{Pool, Postgres, Row};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

fn system_time_to_datetime(st: SystemTime) -> DateTime<Utc> {
    let duration = st.duration_since(UNIX_EPOCH).unwrap_or_default();
    DateTime::from_timestamp(duration.as_secs() as i64, duration.subsec_nanos()).unwrap_or_default()
}

fn datetime_to_system_time(dt: DateTime<Utc>) -> SystemTime {
    UNIX_EPOCH
        + Duration::from_secs(dt.timestamp() as u64)
        + Duration::from_nanos(dt.timestamp_subsec_nanos() as u64)
}

/// PostgreSQL-backed SIP trace store.
#[derive(Clone)]
pub struct PostgresSipTraceStore {
    pool: Pool<Postgres>,
}

impl PostgresSipTraceStore {
    pub fn new(pool: Pool<Postgres>) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl SipTraceStore for PostgresSipTraceStore {
    async fn insert_sip_trace(&self, msg: &SipTraceMessage) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO sip_trace_messages (
                call_id, sip_call_id, tenant_id, timestamp,
                direction, method_or_status, cseq,
                from_uri, to_uri, source_addr, dest_addr,
                transport, raw_message, node_id
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)",
        )
        .bind(&msg.call_id)
        .bind(&msg.sip_call_id)
        .bind(&msg.tenant_id)
        .bind(system_time_to_datetime(msg.timestamp))
        .bind(&msg.direction)
        .bind(&msg.method_or_status)
        .bind(&msg.cseq)
        .bind(&msg.from_uri)
        .bind(&msg.to_uri)
        .bind(&msg.source_addr)
        .bind(&msg.dest_addr)
        .bind(&msg.transport)
        .bind(&msg.raw_message)
        .bind(&msg.node_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn insert_sip_traces_batch(&self, msgs: &[SipTraceMessage]) -> anyhow::Result<()> {
        if msgs.is_empty() {
            return Ok(());
        }

        // Use a transaction for batch inserts
        let mut tx = self.pool.begin().await?;
        for msg in msgs {
            sqlx::query(
                "INSERT INTO sip_trace_messages (
                    call_id, sip_call_id, tenant_id, timestamp,
                    direction, method_or_status, cseq,
                    from_uri, to_uri, source_addr, dest_addr,
                    transport, raw_message, node_id
                ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)",
            )
            .bind(&msg.call_id)
            .bind(&msg.sip_call_id)
            .bind(&msg.tenant_id)
            .bind(system_time_to_datetime(msg.timestamp))
            .bind(&msg.direction)
            .bind(&msg.method_or_status)
            .bind(&msg.cseq)
            .bind(&msg.from_uri)
            .bind(&msg.to_uri)
            .bind(&msg.source_addr)
            .bind(&msg.dest_addr)
            .bind(&msg.transport)
            .bind(&msg.raw_message)
            .bind(&msg.node_id)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    async fn get_sip_traces(&self, call_id: &str) -> anyhow::Result<Vec<SipTraceMessage>> {
        let rows = sqlx::query(
            "SELECT id, call_id, sip_call_id, tenant_id, timestamp,
                    direction, method_or_status, cseq,
                    from_uri, to_uri, source_addr, dest_addr,
                    transport, raw_message, node_id
             FROM sip_trace_messages
             WHERE call_id = $1
             ORDER BY timestamp ASC, id ASC",
        )
        .bind(call_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.iter().map(row_to_sip_trace).collect())
    }

    async fn get_sip_traces_by_sip_call_id(
        &self,
        sip_call_id: &str,
    ) -> anyhow::Result<Vec<SipTraceMessage>> {
        let rows = sqlx::query(
            "SELECT id, call_id, sip_call_id, tenant_id, timestamp,
                    direction, method_or_status, cseq,
                    from_uri, to_uri, source_addr, dest_addr,
                    transport, raw_message, node_id
             FROM sip_trace_messages
             WHERE sip_call_id = $1
             ORDER BY timestamp ASC, id ASC",
        )
        .bind(sip_call_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.iter().map(row_to_sip_trace).collect())
    }

    async fn delete_sip_traces_for_tenant_before(
        &self,
        tenant_id: &str,
        before: SystemTime,
    ) -> anyhow::Result<u64> {
        Ok(crate::deletion::execute(
            &self.pool,
            sqlx::query(
                "DELETE FROM sip_trace_messages
                 WHERE timestamp < $2
                   AND (tenant_id = $1
                        OR (tenant_id = '_system'
                            AND call_id IN (SELECT call_id FROM cdrs WHERE tenant_id = $1)))
                   AND fcp_sip_trace_hold($1, timestamp, from_uri, to_uri, call_id) IS NULL",
            )
            .bind(tenant_id)
            .bind(system_time_to_datetime(before)),
        )
        .await?)
    }

    async fn adopt_system_traces(&self) -> anyhow::Result<u64> {
        // A call's messages are captured as they cross the wire, before
        // anything knows the call's tenant; its stored record does.
        let result = sqlx::query(
            "UPDATE sip_trace_messages t SET tenant_id = c.tenant_id
             FROM cdrs c
             WHERE t.tenant_id = '_system' AND c.call_id = t.call_id",
        )
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
    }

    async fn delete_system_traces_before(&self, before: SystemTime) -> anyhow::Result<u64> {
        Ok(crate::deletion::execute(
            &self.pool,
            sqlx::query(
                "DELETE FROM sip_trace_messages WHERE tenant_id = '_system' AND timestamp < $1",
            )
            .bind(system_time_to_datetime(before)),
        )
        .await?)
    }

    async fn delete_sip_traces_older_than(&self, days: u32) -> anyhow::Result<u64> {
        let result =
            sqlx::query("DELETE FROM sip_trace_messages WHERE timestamp < NOW() - $1::interval")
                .bind(format!("{} days", days))
                .execute(&self.pool)
                .await?;
        Ok(result.rows_affected())
    }
}

fn row_to_sip_trace(row: &sqlx::postgres::PgRow) -> SipTraceMessage {
    let dt: DateTime<Utc> = row.get("timestamp");
    SipTraceMessage {
        id: row.get("id"),
        call_id: row.get("call_id"),
        sip_call_id: row.get("sip_call_id"),
        tenant_id: row.get("tenant_id"),
        timestamp: datetime_to_system_time(dt),
        direction: row.get("direction"),
        method_or_status: row.get("method_or_status"),
        cseq: row.get("cseq"),
        from_uri: row.get("from_uri"),
        to_uri: row.get("to_uri"),
        source_addr: row.get("source_addr"),
        dest_addr: row.get("dest_addr"),
        transport: row.get("transport"),
        raw_message: row.get("raw_message"),
        node_id: row.get("node_id"),
    }
}
