//! Legal holds in Postgres (`docs/COMPLIANCE_FOLLOWUPS.md` §5; the model is
//! `fcp_core::legal_hold`). Migration 063 holds the rules as SQL functions
//! and the triggers that refuse a held row's deletion; this is the store's
//! side: placing, listing and releasing holds, and asking what they hold.

use anyhow::Result;
use chrono::{DateTime, Utc};
use fcp_core::legal_hold::{DataClass, HoldRelease, HoldScope, LegalHold};
use fcp_core::{LegalHoldId, TenantId};
use sqlx::{Pool, Postgres, Row};
use std::str::FromStr;
use std::time::{Duration, SystemTime};

/// How long counting what a hold holds may take, per class.
const COUNT_BUDGET: Duration = Duration::from_secs(2);

/// How each class's rows are read: `$1` is always the tenant.
struct ClassSql {
    table: &'static str,
    id: &'static str,
    at: &'static str,
    tenant: &'static str,
    /// The hold covering a row, NULL when none does.
    hold: &'static str,
    keys: &'static str,
    call: &'static str,
}

fn class_sql(class: DataClass) -> ClassSql {
    match class {
        DataClass::Cdr => ClassSql {
            table: "cdrs",
            id: "cdr_id::text",
            at: "start_time",
            tenant: "tenant_id = $1",
            hold: "fcp_cdr_hold(tenant_id, start_time, from_uri, to_uri, call_id)",
            keys: "fcp_subject_keys(from_uri) || fcp_subject_keys(to_uri)",
            call: "call_id",
        },
        DataClass::SipTrace => ClassSql {
            table: "sip_trace_messages",
            id: "id::text",
            at: "timestamp",
            tenant: "(tenant_id = $1 OR (tenant_id = '_system' \
                     AND call_id IN (SELECT call_id FROM cdrs WHERE tenant_id = $1)))",
            hold: "fcp_sip_trace_hold($1, timestamp, from_uri, to_uri, call_id)",
            keys: "fcp_subject_keys(from_uri) || fcp_subject_keys(to_uri)",
            call: "call_id",
        },
        DataClass::FlowTrace => ClassSql {
            table: "flow_execution_traces",
            id: "id::text",
            at: "timestamp",
            tenant: "(tenant_id = $1 OR (tenant_id IS NULL \
                     AND call_id IN (SELECT call_id FROM cdrs WHERE tenant_id = $1)))",
            hold: "fcp_flow_trace_hold($1, timestamp, call_id)",
            keys: "fcp_call_keys(call_id)",
            call: "call_id",
        },
        DataClass::CallRecording => ClassSql {
            table: "call_recordings",
            id: "id",
            at: "started_at",
            tenant: "tenant_id = $1",
            hold: "fcp_call_recording_hold(tenant_id, started_at, call_id)",
            keys: "fcp_call_keys(call_id)",
            call: "call_id",
        },
        DataClass::ConferenceRecording => ClassSql {
            table: "conference_recordings",
            id: "id",
            at: "started_at",
            tenant: "tenant_id = $1",
            hold: "fcp_conference_recording_hold(tenant_id, started_at, participant_list)",
            keys: "fcp_participant_keys(participant_list)",
            call: "NULL::text",
        },
        DataClass::Voicemail => ClassSql {
            table: "voicemail_messages",
            id: "id",
            at: "created_at",
            tenant: "tenant_id = $1",
            hold: "fcp_voicemail_hold(tenant_id, created_at, caller, mailbox_id)",
            keys: "fcp_subject_keys(caller) || ('mailbox:' || mailbox_id)",
            call: "NULL::text",
        },
        DataClass::AuditEvent => ClassSql {
            table: "admin_audit_log",
            id: "id::text",
            at: "occurred_at",
            tenant: "tenant_id = $1",
            hold: "fcp_audit_event_hold(tenant_id, occurred_at, actor)",
            keys: "fcp_subject_keys(actor)",
            call: "NULL::text",
        },
    }
}

fn to_datetime(t: SystemTime) -> DateTime<Utc> {
    DateTime::<Utc>::from(t)
}

const COLUMNS: &str = "id, tenant_id, matter, reason, held_from, held_to, subjects, subject_keys, \
                       call_ids, classes, created_by, created_at, released_by, released_at, \
                       release_reason";

fn row_to_hold(row: sqlx::postgres::PgRow) -> Result<LegalHold> {
    let id: String = row.try_get("id")?;
    let tenant: String = row.try_get("tenant_id")?;
    let classes: Vec<String> = row.try_get("classes")?;
    let released_at: Option<DateTime<Utc>> = row.try_get("released_at")?;
    Ok(LegalHold {
        id: LegalHoldId::from_str(&id)?,
        tenant_id: TenantId::from_str(&tenant)?,
        matter: row.try_get("matter")?,
        reason: row.try_get("reason")?,
        scope: HoldScope {
            from: row
                .try_get::<Option<DateTime<Utc>>, _>("held_from")?
                .map(SystemTime::from),
            to: row
                .try_get::<Option<DateTime<Utc>>, _>("held_to")?
                .map(SystemTime::from),
            subjects: serde_json::from_value(row.try_get("subjects")?)?,
            call_ids: row.try_get("call_ids")?,
            classes: classes.iter().filter_map(|c| DataClass::parse(c)).collect(),
        },
        subject_keys: row.try_get("subject_keys")?,
        created_by: row.try_get("created_by")?,
        created_at: SystemTime::from(row.try_get::<DateTime<Utc>, _>("created_at")?),
        released: match released_at {
            Some(at) => Some(HoldRelease {
                by: row
                    .try_get::<Option<String>, _>("released_by")?
                    .unwrap_or_default(),
                at: SystemTime::from(at),
                reason: row
                    .try_get::<Option<String>, _>("release_reason")?
                    .unwrap_or_default(),
            }),
            None => None,
        },
    })
}

pub(crate) async fn place(pool: &Pool<Postgres>, hold: &LegalHold) -> Result<()> {
    sqlx::query(
        "INSERT INTO legal_holds
            (id, tenant_id, matter, reason, held_from, held_to, subjects, subject_keys,
             call_ids, classes, created_by, created_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
    )
    .bind(hold.id.to_string())
    .bind(hold.tenant_id.to_string())
    .bind(&hold.matter)
    .bind(&hold.reason)
    .bind(hold.scope.from.map(to_datetime))
    .bind(hold.scope.to.map(to_datetime))
    .bind(serde_json::to_value(&hold.scope.subjects)?)
    .bind(&hold.subject_keys)
    .bind(&hold.scope.call_ids)
    .bind(
        hold.scope
            .classes
            .iter()
            .map(|c| c.as_str().to_string())
            .collect::<Vec<_>>(),
    )
    .bind(&hold.created_by)
    .bind(to_datetime(hold.created_at))
    .execute(pool)
    .await?;
    Ok(())
}

pub(crate) async fn get(
    pool: &Pool<Postgres>,
    tenant: &TenantId,
    id: &LegalHoldId,
) -> Result<Option<LegalHold>> {
    sqlx::query(&format!(
        "SELECT {COLUMNS} FROM legal_holds WHERE tenant_id = $1 AND id = $2"
    ))
    .bind(tenant.to_string())
    .bind(id.to_string())
    .fetch_optional(pool)
    .await?
    .map(row_to_hold)
    .transpose()
}

pub(crate) async fn list(
    pool: &Pool<Postgres>,
    tenant: &TenantId,
    include_released: bool,
) -> Result<Vec<LegalHold>> {
    let released = if include_released {
        ""
    } else {
        " AND released_at IS NULL"
    };
    sqlx::query(&format!(
        "SELECT {COLUMNS} FROM legal_holds WHERE tenant_id = $1{released}
         ORDER BY created_at DESC, id"
    ))
    .bind(tenant.to_string())
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(row_to_hold)
    .collect()
}

pub(crate) async fn release(
    pool: &Pool<Postgres>,
    tenant: &TenantId,
    id: &LegalHoldId,
    release: &HoldRelease,
) -> Result<bool> {
    let done = sqlx::query(
        "UPDATE legal_holds SET released_by = $3, released_at = $4, release_reason = $5
         WHERE tenant_id = $1 AND id = $2 AND released_at IS NULL",
    )
    .bind(tenant.to_string())
    .bind(id.to_string())
    .bind(&release.by)
    .bind(to_datetime(release.at))
    .bind(&release.reason)
    .execute(pool)
    .await?;
    Ok(done.rows_affected() == 1)
}

pub(crate) async fn counts(
    pool: &Pool<Postgres>,
    tenant: &TenantId,
    id: &LegalHoldId,
) -> Result<Vec<(DataClass, Option<u64>)>> {
    let Some(hold) = get(pool, tenant, id).await? else {
        return Ok(Vec::new());
    };
    let classes: Vec<DataClass> = if hold.scope.classes.is_empty() {
        DataClass::ALL.to_vec()
    } else {
        hold.scope.classes.clone()
    };
    let mut out = Vec::new();
    for class in classes {
        let c = class_sql(class);
        let sql = format!(
            "SELECT count(*) FROM {table}
             WHERE {tenant}
               AND ($3::timestamptz IS NULL OR {at} >= $3)
               AND ($4::timestamptz IS NULL OR {at} < $4)
               AND fcp_hold_covers($2, '{class}', {at}, {keys}, {call})",
            table = c.table,
            tenant = c.tenant,
            at = c.at,
            keys = c.keys,
            call = c.call,
            class = class.as_str(),
        );
        let mut tx = pool.begin().await?;
        sqlx::query(&format!(
            "SET LOCAL statement_timeout = {}",
            COUNT_BUDGET.as_millis()
        ))
        .execute(&mut *tx)
        .await?;
        let counted: Result<i64, sqlx::Error> = sqlx::query_scalar(&sql)
            .bind(tenant.to_string())
            .bind(id.to_string())
            .bind(hold.scope.from.map(to_datetime))
            .bind(hold.scope.to.map(to_datetime))
            .fetch_one(&mut *tx)
            .await;
        match counted {
            Ok(n) => {
                tx.commit().await?;
                out.push((class, Some(n.max(0) as u64)));
            }
            Err(e)
                if e.as_database_error()
                    .and_then(|d| d.code())
                    .is_some_and(|code| code == "57014") =>
            {
                out.push((class, None));
            }
            Err(e) => return Err(e.into()),
        }
    }
    Ok(out)
}

pub(crate) async fn hold_on(
    pool: &Pool<Postgres>,
    tenant: &TenantId,
    class: DataClass,
    item: &str,
) -> Result<Option<LegalHoldId>> {
    let c = class_sql(class);
    let id: Option<Option<String>> = sqlx::query_scalar(&format!(
        "SELECT {hold} FROM {table} WHERE {tenant} AND {id} = $2 LIMIT 1",
        hold = c.hold,
        table = c.table,
        tenant = c.tenant,
        id = c.id,
    ))
    .bind(tenant.to_string())
    .bind(item)
    .fetch_optional(pool)
    .await?;
    Ok(id.flatten().and_then(|h| LegalHoldId::from_str(&h).ok()))
}

pub(crate) async fn held_before(
    pool: &Pool<Postgres>,
    tenant: &TenantId,
    class: DataClass,
    before: SystemTime,
) -> Result<u64> {
    let c = class_sql(class);
    let n: i64 = sqlx::query_scalar(&format!(
        "SELECT count(*) FROM {table} WHERE {tenant} AND {at} < $2 AND {hold} IS NOT NULL",
        table = c.table,
        tenant = c.tenant,
        at = c.at,
        hold = c.hold,
    ))
    .bind(tenant.to_string())
    .bind(to_datetime(before))
    .fetch_one(pool)
    .await?;
    Ok(n.max(0) as u64)
}
