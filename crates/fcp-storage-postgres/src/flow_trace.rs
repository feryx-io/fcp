//! PostgreSQL implementation of the flow execution trace store.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use fcp_core::flow_trace::{FlowTraceEntry, FlowTraceEventType, FlowTraceQuery, FlowTraceStore};
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

fn event_type_from_str(s: &str) -> FlowTraceEventType {
    match s {
        "started" => FlowTraceEventType::Started,
        "node_executed" => FlowTraceEventType::NodeExecuted,
        "waiting_for_input" => FlowTraceEventType::WaitingForInput,
        "completed" => FlowTraceEventType::Completed,
        "failed" => FlowTraceEventType::Failed,
        _ => FlowTraceEventType::NodeExecuted,
    }
}

/// PostgreSQL-backed flow execution trace store.
#[derive(Clone)]
pub struct PostgresFlowTraceStore {
    pool: Pool<Postgres>,
}

impl PostgresFlowTraceStore {
    pub fn new(pool: Pool<Postgres>) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl FlowTraceStore for PostgresFlowTraceStore {
    async fn insert_flow_trace(&self, entry: &FlowTraceEntry) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO flow_execution_traces (
                call_id, flow_id, flow_name, node_id, event_type,
                timestamp, detail, flow_version, elapsed_ms
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
        )
        .bind(&entry.call_id)
        .bind(&entry.flow_id)
        .bind(&entry.flow_name)
        .bind(&entry.node_id)
        .bind(entry.event_type.to_string())
        .bind(system_time_to_datetime(entry.timestamp))
        .bind(&entry.detail)
        .bind(entry.flow_version.map(|v| v as i32))
        .bind(entry.elapsed_ms)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn get_flow_traces_by_call(&self, call_id: &str) -> anyhow::Result<Vec<FlowTraceEntry>> {
        let rows = sqlx::query(
            "SELECT id, call_id, flow_id, flow_name, node_id, event_type,
                    timestamp, detail, flow_version, elapsed_ms
             FROM flow_execution_traces
             WHERE call_id = $1
             ORDER BY timestamp ASC, id ASC",
        )
        .bind(call_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.iter().map(row_to_flow_trace).collect())
    }

    async fn query_flow_traces(
        &self,
        query: &FlowTraceQuery,
    ) -> anyhow::Result<(Vec<FlowTraceEntry>, usize)> {
        // Build WHERE clause dynamically
        let mut conditions = Vec::new();
        let mut param_idx = 1u32;

        if query.call_id.is_some() {
            conditions.push(format!("call_id = ${}", param_idx));
            param_idx += 1;
        }
        if query.flow_id.is_some() {
            conditions.push(format!("flow_id = ${}", param_idx));
            param_idx += 1;
        }
        if query.event_type.is_some() {
            conditions.push(format!("event_type = ${}", param_idx));
            param_idx += 1;
        }
        if query.time_from.is_some() {
            conditions.push(format!("timestamp >= ${}", param_idx));
            param_idx += 1;
        }
        if query.time_to.is_some() {
            conditions.push(format!("timestamp <= ${}", param_idx));
            param_idx += 1;
        }

        let where_clause = if conditions.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", conditions.join(" AND "))
        };

        let limit = query.limit.unwrap_or(100).min(1000) as i64;
        let offset = query.offset.unwrap_or(0) as i64;

        let count_sql = format!(
            "SELECT COUNT(*) as cnt FROM flow_execution_traces {}",
            where_clause
        );
        let data_sql = format!(
            "SELECT id, call_id, flow_id, flow_name, node_id, event_type,
                    timestamp, detail, flow_version, elapsed_ms
             FROM flow_execution_traces {}
             ORDER BY timestamp DESC, id DESC
             LIMIT ${} OFFSET ${}",
            where_clause,
            param_idx,
            param_idx + 1
        );

        // Build and execute count query
        let mut count_q = sqlx::query(&count_sql);
        let mut data_q = sqlx::query(&data_sql);

        // Bind parameters in the same order for both queries
        if let Some(ref call_id) = query.call_id {
            count_q = count_q.bind(call_id);
            data_q = data_q.bind(call_id);
        }
        if let Some(ref flow_id) = query.flow_id {
            count_q = count_q.bind(flow_id);
            data_q = data_q.bind(flow_id);
        }
        if let Some(ref event_type) = query.event_type {
            count_q = count_q.bind(event_type.to_string());
            data_q = data_q.bind(event_type.to_string());
        }
        if let Some(ref time_from) = query.time_from {
            count_q = count_q.bind(system_time_to_datetime(*time_from));
            data_q = data_q.bind(system_time_to_datetime(*time_from));
        }
        if let Some(ref time_to) = query.time_to {
            count_q = count_q.bind(system_time_to_datetime(*time_to));
            data_q = data_q.bind(system_time_to_datetime(*time_to));
        }

        // Bind limit/offset (data query only)
        data_q = data_q.bind(limit).bind(offset);

        let count_row = count_q.fetch_one(&self.pool).await?;
        let total: i64 = count_row.get("cnt");

        let rows = data_q.fetch_all(&self.pool).await?;
        let entries: Vec<FlowTraceEntry> = rows.iter().map(row_to_flow_trace).collect();

        Ok((entries, total as usize))
    }

    async fn delete_flow_traces_older_than(&self, days: u32) -> anyhow::Result<u64> {
        let result =
            sqlx::query("DELETE FROM flow_execution_traces WHERE timestamp < NOW() - $1::interval")
                .bind(format!("{} days", days))
                .execute(&self.pool)
                .await?;
        Ok(result.rows_affected())
    }
}

fn row_to_flow_trace(row: &sqlx::postgres::PgRow) -> FlowTraceEntry {
    let dt: DateTime<Utc> = row.get("timestamp");
    let event_type_str: String = row.get("event_type");
    FlowTraceEntry {
        id: row.get("id"),
        call_id: row.get("call_id"),
        flow_id: row.get("flow_id"),
        flow_name: row.get("flow_name"),
        node_id: row.get("node_id"),
        event_type: event_type_from_str(&event_type_str),
        timestamp: datetime_to_system_time(dt),
        detail: row.get("detail"),
        flow_version: row
            .try_get::<Option<i32>, _>("flow_version")
            .unwrap_or(None)
            .map(|v| v as u32),
        elapsed_ms: row.try_get("elapsed_ms").unwrap_or(None),
    }
}
