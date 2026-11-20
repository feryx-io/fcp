//! Where a subject erasure and a tenant's deletion act in Postgres
//! (`docs/COMPLIANCE_FOLLOWUPS.md` §6.3–6.6, §4; the decisions are
//! `fcp_core::subject_erasure`).
//!
//! Every statement runs in a transaction attributed to the current deleter
//! (`deletion::attributed`), so the deletion log names the cause, the actor
//! and the request, and a legal hold's trigger still refuses a held row.
//! Classes with no deletion trigger of their own are logged here.

use crate::{deletion, PostgresStorage};
use anyhow::Result;
use fcp_core::subject_erasure::{RowUpdate, SubjectEraseStore};
use fcp_core::subject_request::ExportClass;
use fcp_core::TenantId;
use sqlx::{Postgres, Row, Transaction};

/// How a class's rows are addressed: the table, the tenant condition (`$1`)
/// and the id expression compared with `$2`.
struct Target {
    table: &'static str,
    tenant: &'static str,
    id: &'static str,
    /// Whether a trigger already logs its deletions.
    logged: bool,
}

fn target(class: ExportClass) -> Option<Target> {
    use ExportClass::*;
    let t = |table, tenant, id, logged| {
        Some(Target {
            table,
            tenant,
            id,
            logged,
        })
    };
    match class {
        Profile => t("users", "tenant_id = $1", "id", false),
        ApiTokens => t("api_tokens", "tenant_id IN ($1, '_system')", "id", false),
        Extensions => t("extensions", "tenant_id = $1", "id", false),
        QueueMemberships => t("queue_agents", "tenant_id = $1", "agent_id", false),
        Mailboxes => t("voicemail_mailboxes", "tenant_id = $1", "id", false),
        VoicemailGreetings => t(
            "voicemail_greetings",
            "mailbox_id IN (SELECT id FROM voicemail_mailboxes WHERE tenant_id = $1)",
            "id::text",
            false,
        ),
        VoicemailMessages => t("voicemail_messages", "tenant_id = $1", "id", true),
        Cdrs => t("cdrs", "tenant_id = $1", "cdr_id::text", true),
        CallRecordings => t("call_recordings", "tenant_id = $1", "id", true),
        ConferenceRecordings => t("conference_recordings", "tenant_id = $1", "id", true),
        SipTraces => t(
            "sip_trace_messages",
            "tenant_id IN ($1, '_system')",
            "id::text",
            true,
        ),
        FlowTraces => t(
            "flow_execution_traces",
            "(tenant_id = $1 OR tenant_id IS NULL)",
            "id::text",
            true,
        ),
        Registrations => t(
            "registrations",
            "tenant_id = $1",
            "aor || ' ' || contact",
            false,
        ),
        Presence => t("presence_presentities", "tenant_id = $1", "aor", false),
        PresenceSubscriptions => t(
            "presence_subscriptions",
            "tenant_id = $1",
            "subscription_id",
            false,
        ),
        PresencePolicies => t("presence_auth_policies", "tenant_id = $1", "aor", false),
        ResourceLists => t("resource_lists", "tenant_id = $1", "uri", false),
        PresenceHistory => t("presence_history", "tenant_id = $1", "id::text", false),
        EmergencyCalls => t("emergency_calls", "tenant_id = $1", "id::text", false),
        DeclaredLocations => t("declared_locations", "tenant_id = $1", "user_id", false),
        PersonalContacts => t("personal_contacts", "tenant_id = $1", "id", false),
        ConferenceChat => t("conference_chat", "tenant_id = $1", "id", true),
        WebinarQuestions => t("webinar_questions", "tenant_id = $1", "id", true),
        WebinarRegistrants => t("webinar_registrants", "tenant_id = $1", "id", true),
        AuditEvents => t("admin_audit_log", "tenant_id = $1", "id::text", true),
        ConfigChanges => t("config_audit_log", "tenant_id = $1", "id::text", false),
        WebhookDeliveries => t(
            "webhook_delivery_logs",
            "endpoint_id IN (SELECT id FROM webhook_endpoints WHERE tenant_id = $1)",
            "id",
            false,
        ),
        Devices => t("devices", "tenant_id = $1", "id", false),
        LineBindings => t("line_bindings", "tenant_id = $1", "id", false),
        Meetings => t("conference_meetings", "tenant_id = $1", "id", false),
        PortRequests | NumberLists => None,
    }
}

/// The columns an erasure may rewrite, by class, and each one's type.
fn writable(class: ExportClass, column: &str) -> Option<&'static str> {
    use ExportClass::*;
    let text = Some("text");
    match (class, column) {
        (Cdrs, "from_uri" | "to_uri" | "sip_call_id") => text,
        (Cdrs, "from_display_name" | "to_display_name") => text,
        (Cdrs, "custom_data") => Some("jsonb"),
        (EmergencyCalls, "caller_uri" | "caller_username" | "callback_number") => text,
        (AuditEvents, "actor" | "path") => text,
        (ConfigChanges, "changed_by") => text,
        (Devices | LineBindings, "owner_user_id") => text,
        (Meetings, "owner_id") => text,
        _ => None,
    }
}

/// Note the archived days of these records for the archiver to write again:
/// the late-record note the archive already acts on.
async fn note_archive_days(
    tx: &mut Transaction<'static, Postgres>,
    tenant: &TenantId,
    cdr_ids: &[String],
) -> Result<()> {
    sqlx::query(
        "INSERT INTO cdr_archive_late (tenant_id, day)
         SELECT DISTINCT c.tenant_id, (c.start_time AT TIME ZONE 'UTC')::date
         FROM cdrs c, cdr_archive_state s
         WHERE c.tenant_id = $1 AND c.cdr_id::text = ANY($2)
           AND (c.start_time AT TIME ZONE 'UTC')::date <= s.next_day",
    )
    .bind(tenant.to_string())
    .bind(cdr_ids)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// One deletion-log entry for a batch no trigger logs.
async fn log_batch(
    tx: &mut Transaction<'static, Postgres>,
    tenant: &TenantId,
    class: &str,
    what: &str,
    ids: &[String],
    count: u64,
) -> Result<()> {
    if count == 0 {
        return Ok(());
    }
    let shown: Vec<&String> = ids.iter().take(100).collect();
    sqlx::query(
        "INSERT INTO deletion_log (tenant_id, class, item, detail, cause, actor)
         VALUES ($1, $2, $3, $4, fcp_deletion_cause(), fcp_deletion_actor())",
    )
    .bind(tenant.to_string())
    .bind(class)
    .bind(format!("{count} {what}"))
    .bind(serde_json::json!({ "count": count, "ids": shown, what: true }))
    .execute(&mut **tx)
    .await?;
    Ok(())
}

#[async_trait::async_trait]
impl SubjectEraseStore for PostgresStorage {
    async fn pseudonym_secret(&self, tenant: &TenantId) -> Result<Vec<u8>> {
        let fresh = format!(
            "{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        );
        let sealed = self
            .seal_opt(Some(&fresh))?
            .expect("a value seals to a value");
        sqlx::query(
            "INSERT INTO subject_pseudonym_keys (id, secret) VALUES ($1, $2)
             ON CONFLICT (id) DO NOTHING",
        )
        .bind(tenant.to_string())
        .bind(sealed)
        .execute(self.writes())
        .await?;
        let stored: String =
            sqlx::query_scalar("SELECT secret FROM subject_pseudonym_keys WHERE id = $1")
                .bind(tenant.to_string())
                .fetch_one(self.writes())
                .await?;
        let secret = self
            .open_opt(Some(stored))
            .await?
            .expect("a value opens to a value");
        Ok(secret.into_bytes())
    }

    async fn delete_subject_rows(
        &self,
        tenant: &TenantId,
        class: ExportClass,
        ids: &[String],
    ) -> Result<u64> {
        if ids.is_empty() {
            return Ok(0);
        }
        let mut tx = deletion::attributed(self.writes()).await?;
        let removed = if class == ExportClass::NumberLists {
            let mut n = 0;
            for id in ids {
                let mut parts = id.splitn(3, ':');
                let (Some(number), Some(list), Some(entry)) =
                    (parts.next(), parts.next(), parts.next())
                else {
                    continue;
                };
                let column = match list {
                    "allow" => "allow_list",
                    "block" => "block_list",
                    _ => continue,
                };
                n += sqlx::query(&format!(
                    "UPDATE phone_numbers SET {column} = {column} - $3
                     WHERE tenant_id = $1 AND id = $2 AND {column} ? $3"
                ))
                .bind(tenant.to_string())
                .bind(number)
                .bind(entry)
                .execute(&mut *tx)
                .await?
                .rows_affected();
            }
            log_batch(&mut tx, tenant, class.as_str(), "entries removed", ids, n).await?;
            n
        } else {
            let t = target(class)
                .ok_or_else(|| anyhow::anyhow!("{} rows are not deleted", class.as_str()))?;
            if class == ExportClass::Cdrs {
                note_archive_days(&mut tx, tenant, ids).await?;
            }
            let n = sqlx::query(&format!(
                "DELETE FROM {} WHERE {} AND {} = ANY($2)",
                t.table, t.tenant, t.id
            ))
            .bind(tenant.to_string())
            .bind(ids)
            .execute(&mut *tx)
            .await
            .map_err(deletion::held)?
            .rows_affected();
            if !t.logged {
                log_batch(&mut tx, tenant, class.as_str(), "deleted", ids, n).await?;
            }
            n
        };
        tx.commit().await.map_err(deletion::held)?;
        Ok(removed)
    }

    async fn update_subject_rows(
        &self,
        tenant: &TenantId,
        class: ExportClass,
        updates: &[RowUpdate],
    ) -> Result<u64> {
        if updates.is_empty() {
            return Ok(0);
        }
        let t = target(class)
            .ok_or_else(|| anyhow::anyhow!("{} rows are not rewritten", class.as_str()))?;
        let mut tx = deletion::attributed(self.writes()).await?;
        if class == ExportClass::Cdrs {
            let ids: Vec<String> = updates.iter().map(|u| u.id.clone()).collect();
            note_archive_days(&mut tx, tenant, &ids).await?;
        }
        let mut n = 0;
        for update in updates {
            let mut sets = Vec::new();
            let mut values: Vec<(&'static str, &serde_json::Value)> = Vec::new();
            for (column, value) in &update.fields {
                let kind = writable(class, column).ok_or_else(|| {
                    anyhow::anyhow!("{column} of {} is not rewritten", class.as_str())
                })?;
                sets.push(format!("{column} = ${}", values.len() + 3));
                values.push((kind, value));
            }
            let sql = format!(
                "UPDATE {} SET {} WHERE {} AND {} = $2",
                t.table,
                sets.join(", "),
                t.tenant,
                t.id
            );
            let mut query = sqlx::query(&sql).bind(tenant.to_string()).bind(&update.id);
            for (kind, value) in values {
                query = match (kind, value) {
                    ("jsonb", v) => query.bind(v.clone()),
                    (_, serde_json::Value::Null) => query.bind(None::<String>),
                    (_, serde_json::Value::String(s)) => query.bind(Some(s.clone())),
                    (_, other) => query.bind(Some(other.to_string())),
                };
            }
            n += query.execute(&mut *tx).await?.rows_affected();
        }
        let ids: Vec<String> = updates.iter().map(|u| u.id.clone()).collect();
        let what = match class {
            ExportClass::Devices | ExportClass::LineBindings | ExportClass::Meetings => {
                "owner cleared"
            }
            _ => "pseudonymised",
        };
        log_batch(&mut tx, tenant, class.as_str(), what, &ids, n).await?;
        tx.commit().await?;
        Ok(n)
    }

    async fn note_tenant_archive_rewrite(&self, tenant: &TenantId) -> Result<u64> {
        Ok(sqlx::query(
            "INSERT INTO cdr_archive_late (tenant_id, day)
             SELECT tenant_id, day FROM cdr_archives
             WHERE tenant_id = $1 AND object_key IS NOT NULL",
        )
        .bind(tenant.to_string())
        .execute(self.writes())
        .await?
        .rows_affected())
    }

    async fn delete_tenant_data(&self, tenant: &TenantId) -> Result<Vec<(String, u64)>> {
        let pool = self.writes();
        let tenant_s = tenant.to_string();
        let mut out = Vec::new();
        let run = |table: String, sql: String| {
            let tenant_s = tenant_s.clone();
            async move {
                let mut tx = deletion::attributed(pool).await?;
                let n = sqlx::query(&sql)
                    .bind(&tenant_s)
                    .execute(&mut *tx)
                    .await
                    .map_err(deletion::held)?
                    .rows_affected();
                tx.commit().await.map_err(deletion::held)?;
                anyhow::Ok((table, n))
            }
        };

        // What belongs to the tenant through its calls, before the calls go.
        out.push(
            run(
                "sip_trace_messages (_system)".into(),
                "DELETE FROM sip_trace_messages WHERE tenant_id = '_system'
                 AND call_id IN (SELECT call_id FROM cdrs WHERE tenant_id = $1)"
                    .into(),
            )
            .await?,
        );
        out.push(
            run(
                "flow_execution_traces (untenanted)".into(),
                "DELETE FROM flow_execution_traces WHERE tenant_id IS NULL
                 AND call_id IN (SELECT call_id FROM cdrs WHERE tenant_id = $1)"
                    .into(),
            )
            .await?,
        );

        // Every table with a tenant, but the records of the deletion itself,
        // what the archive still has to rewrite, and the tenant.
        const KEEP: &[&str] = &[
            "tenants",
            "subject_requests",
            "legal_holds",
            "deletion_log",
            "pending_file_deletions",
            "cdr_archives",
            "cdr_archive_late",
            "retention_runs",
        ];
        const FIRST: &[&str] = &[
            "call_recordings",
            "conference_recordings",
            "voicemail_messages",
            "sip_trace_messages",
            "flow_execution_traces",
        ];
        let mut tables: Vec<String> = sqlx::query(
            "SELECT c.relname AS name
             FROM pg_attribute a
             JOIN pg_class c ON c.oid = a.attrelid
             JOIN pg_namespace n ON n.oid = c.relnamespace
             WHERE a.attname = 'tenant_id' AND NOT a.attisdropped AND a.attnum > 0
               AND c.relkind IN ('r', 'p') AND NOT c.relispartition
               AND n.nspname = current_schema()",
        )
        .fetch_all(pool)
        .await?
        .into_iter()
        .map(|r| r.get::<String, _>("name"))
        .filter(|t| !KEEP.contains(&t.as_str()))
        .collect();
        tables.sort_by_key(|t| {
            (
                FIRST.iter().position(|f| f == t).unwrap_or(FIRST.len()),
                t.clone(),
            )
        });
        for table in tables {
            let sql = format!("DELETE FROM {table} WHERE tenant_id::text = $1");
            out.push(run(table, sql).await?);
        }
        out.push(
            run(
                "subject_pseudonym_keys".into(),
                "DELETE FROM subject_pseudonym_keys WHERE id = $1".into(),
            )
            .await?,
        );
        out.push(run("tenants".into(), "DELETE FROM tenants WHERE id = $1".into()).await?);
        out.retain(|(_, n)| *n > 0);
        Ok(out)
    }
}
