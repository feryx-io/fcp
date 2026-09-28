//! What a subject export reads from Postgres (`docs/COMPLIANCE_FOLLOWUPS.md`
//! §6.3; the model and the walk are `fcp_core::subject_request`).
//!
//! Each class is one query returning candidates in a common shape: the
//! item's id, why it may be the subject's (owned, a call's, or identifiers
//! to match), its data as JSON without secrets, and its file. Candidates
//! are narrowed by the subject's canonical keys (`fcp_subject_keys`), which
//! lean towards matching; the export's matcher decides.

use crate::PostgresStorage;
use anyhow::Result;
use chrono::{DateTime, Utc};
use fcp_core::subject_request::{
    ExportClass, RowLink, SubjectDataStore, SubjectPage, SubjectRow, SubjectScope,
};
use fcp_core::TenantId;
use sqlx::Row;
use std::time::SystemTime;

/// The columns every class's query returns.
const SHAPE: &str = "id, link, data, at, call_id, file, ids, others";

/// A class's query. Binds: `$1` tenant, `$2` the cursor (text, NULL for the
/// first page), `$3` limit, `$4` subject keys, `$5` user ids, `$6` mailbox
/// ids, `$7` names, `$8` call ids, `$9` LIKE patterns.
fn class_query(class: ExportClass) -> String {
    let q = match class {
        ExportClass::Profile => {
            "SELECT id, 'owned' AS link,
                    jsonb_build_object(
                        'id', id, 'username', username, 'display_name', display_name,
                        'email', email, 'phone_number', phone_number,
                        'mobile_number', mobile_number, 'extension', extension, 'role', role,
                        'department', department, 'job_title', job_title,
                        'location', location, 'language', language, 'timezone', timezone,
                        'account_status', account_status, 'enabled', enabled,
                        'emergency_contact', emergency_contact_json,
                        'media_preferences', media_preferences,
                        'profile_picture', profile_picture, 'created_at', created_at,
                        'updated_at', updated_at, 'last_login', last_login) AS data,
                    created_at AS at, NULL::text AS call_id, profile_picture AS file,
                    NULL::text[] AS ids, false AS others
             FROM users WHERE tenant_id = $1 AND id = ANY($5)"
        }
        ExportClass::ApiTokens => {
            "SELECT id, 'owned' AS link,
                    jsonb_build_object('id', id, 'name', name, 'prefix', prefix,
                        'permissions', permissions, 'expires_at', expires_at,
                        'created_at', created_at, 'last_used_at', last_used_at) AS data,
                    created_at AS at, NULL::text AS call_id, NULL::text AS file,
                    NULL::text[] AS ids, false AS others
             FROM api_tokens WHERE user_id = ANY($5) AND tenant_id IN ($1, '_system')"
        }
        ExportClass::Devices => {
            "SELECT id, 'owned' AS link,
                    to_jsonb(d) - 'provisioning_token_hash' - 'sip_ha1_hash'
                        - 'sip_provisioning_password' AS data,
                    created_at AS at, NULL::text AS call_id, NULL::text AS file,
                    NULL::text[] AS ids, false AS others
             FROM devices d WHERE tenant_id = $1 AND owner_user_id = ANY($5)"
        }
        ExportClass::LineBindings => {
            "SELECT lb.id, 'owned' AS link,
                    to_jsonb(lb) || jsonb_build_object('number', pn.e164) AS data,
                    lb.created_at AS at, NULL::text AS call_id, NULL::text AS file,
                    NULL::text[] AS ids, false AS others
             FROM line_bindings lb LEFT JOIN phone_numbers pn ON pn.id = lb.phone_number_id
             WHERE lb.tenant_id = $1 AND lb.owner_user_id = ANY($5)"
        }
        ExportClass::Extensions => {
            "SELECT id, 'owned' AS link, to_jsonb(e) AS data,
                    created_at AS at, NULL::text AS call_id, NULL::text AS file,
                    NULL::text[] AS ids, false AS others
             FROM extensions e
             WHERE tenant_id = $1 AND target_type = 'user' AND target_id = ANY($5)"
        }
        ExportClass::QueueMemberships => {
            "SELECT agent_id AS id, 'owned' AS link, to_jsonb(q) AS data,
                    updated_at AS at, NULL::text AS call_id, NULL::text AS file,
                    NULL::text[] AS ids, false AS others
             FROM queue_agents q
             WHERE tenant_id = $1 AND (agent_id = ANY($7) OR extension = ANY($7))"
        }
        ExportClass::Meetings => {
            "SELECT id, 'owned' AS link, to_jsonb(m) - 'guest_pin' - 'host_pin' AS data,
                    created_at AS at, NULL::text AS call_id, NULL::text AS file,
                    NULL::text[] AS ids, false AS others
             FROM conference_meetings m WHERE tenant_id = $1 AND owner_id = ANY($7)"
        }
        ExportClass::Mailboxes => {
            "SELECT id, 'owned' AS link, to_jsonb(v) - 'pin' AS data,
                    created_at AS at, NULL::text AS call_id, NULL::text AS file,
                    NULL::text[] AS ids, false AS others
             FROM voicemail_mailboxes v WHERE tenant_id = $1 AND id = ANY($6)"
        }
        ExportClass::VoicemailGreetings => {
            "SELECT g.id::text AS id, 'owned' AS link, to_jsonb(g) AS data,
                    g.created_at AS at, NULL::text AS call_id, g.audio_path AS file,
                    NULL::text[] AS ids, false AS others
             FROM voicemail_greetings g JOIN voicemail_mailboxes m ON m.id = g.mailbox_id
             WHERE m.tenant_id = $1 AND g.mailbox_id = ANY($6)"
        }
        ExportClass::VoicemailMessages => {
            "SELECT id,
                    CASE WHEN mailbox_id = ANY($6) THEN 'owned' ELSE 'ids' END AS link,
                    to_jsonb(v) AS data, created_at AS at, NULL::text AS call_id,
                    recording_path AS file, ARRAY[caller] AS ids,
                    (mailbox_id = ANY($6) AND NOT fcp_subject_keys(caller) && $4) AS others
             FROM voicemail_messages v
             WHERE tenant_id = $1 AND (mailbox_id = ANY($6) OR fcp_subject_keys(caller) && $4)"
        }
        ExportClass::Cdrs => unreachable!("records are walked by time"),
        ExportClass::CallRecordings => {
            "SELECT id, 'call' AS link, to_jsonb(r) AS data, started_at AS at, call_id,
                    path AS file, NULL::text[] AS ids, true AS others
             FROM call_recordings r WHERE tenant_id = $1 AND call_id = ANY($8)"
        }
        ExportClass::ConferenceRecordings => {
            "SELECT id, 'ids' AS link, to_jsonb(r) AS data, started_at AS at,
                    NULL::text AS call_id, path AS file,
                    ARRAY(SELECT p->>'uri' FROM jsonb_array_elements(participant_list) p
                          WHERE p->>'uri' IS NOT NULL) AS ids,
                    jsonb_array_length(participant_list) > 1 AS others
             FROM conference_recordings r
             WHERE tenant_id = $1 AND fcp_participant_keys(participant_list) && $4"
        }
        ExportClass::SipTraces => {
            "SELECT id::text AS id, 'call' AS link, to_jsonb(t) AS data, timestamp AS at,
                    call_id, NULL::text AS file, NULL::text[] AS ids, false AS others
             FROM sip_trace_messages t
             WHERE tenant_id IN ($1, '_system') AND call_id = ANY($8)"
        }
        ExportClass::FlowTraces => {
            "SELECT id::text AS id, 'call' AS link, to_jsonb(t) AS data, timestamp AS at,
                    call_id, NULL::text AS file, NULL::text[] AS ids, false AS others
             FROM flow_execution_traces t
             WHERE (tenant_id = $1 OR tenant_id IS NULL) AND call_id = ANY($8)"
        }
        ExportClass::Registrations => {
            "SELECT aor || ' ' || contact AS id, 'ids' AS link, to_jsonb(r) AS data,
                    registered_at AS at, NULL::text AS call_id, NULL::text AS file,
                    ARRAY[aor] AS ids, false AS others
             FROM registrations r WHERE tenant_id = $1 AND fcp_subject_keys(aor) && $4"
        }
        ExportClass::Presence => {
            "SELECT aor AS id, 'ids' AS link, to_jsonb(p) AS data,
                    to_timestamp(updated_at_epoch) AS at, NULL::text AS call_id,
                    NULL::text AS file, ARRAY[aor] AS ids, false AS others
             FROM presence_presentities p WHERE tenant_id = $1 AND fcp_subject_keys(aor) && $4"
        }
        ExportClass::PresenceSubscriptions => {
            "SELECT subscription_id AS id, 'ids' AS link, to_jsonb(s) AS data,
                    NULL::timestamptz AS at, NULL::text AS call_id, NULL::text AS file,
                    ARRAY[watcher_aor, presentity_aor] AS ids, false AS others
             FROM presence_subscriptions s
             WHERE tenant_id = $1
               AND (fcp_subject_keys(watcher_aor) || fcp_subject_keys(presentity_aor)) && $4"
        }
        ExportClass::PresencePolicies => {
            "SELECT aor AS id, 'ids' AS link, to_jsonb(p) AS data, NULL::timestamptz AS at,
                    NULL::text AS call_id, NULL::text AS file, ARRAY[aor] AS ids,
                    false AS others
             FROM presence_auth_policies p WHERE tenant_id = $1 AND fcp_subject_keys(aor) && $4"
        }
        ExportClass::ResourceLists => {
            "SELECT uri AS id, 'ids' AS link, to_jsonb(l) AS data, NULL::timestamptz AS at,
                    NULL::text AS call_id, NULL::text AS file, ARRAY[uri] AS ids,
                    false AS others
             FROM resource_lists l WHERE tenant_id = $1 AND fcp_subject_keys(uri) && $4"
        }
        ExportClass::PresenceHistory => {
            "SELECT id::text AS id, 'ids' AS link, to_jsonb(h) AS data,
                    to_timestamp(timestamp_epoch) AS at, NULL::text AS call_id,
                    NULL::text AS file, ARRAY[aor] AS ids, false AS others
             FROM presence_history h WHERE tenant_id = $1 AND fcp_subject_keys(aor) && $4"
        }
        ExportClass::EmergencyCalls => {
            "SELECT id::text AS id, 'ids' AS link, to_jsonb(e) AS data, placed_at AS at,
                    call_id, NULL::text AS file,
                    ARRAY[caller_uri, caller_username, callback_number] AS ids, false AS others
             FROM emergency_calls e
             WHERE tenant_id = $1
               AND (fcp_subject_keys(caller_uri) || fcp_subject_keys(caller_username)
                    || fcp_subject_keys(callback_number)) && $4"
        }
        ExportClass::DeclaredLocations => {
            "SELECT user_id AS id, 'owned' AS link, to_jsonb(d) AS data,
                    declared_at AS at, NULL::text AS call_id, NULL::text AS file,
                    NULL::text[] AS ids, false AS others
             FROM declared_locations d WHERE tenant_id = $1 AND user_id = ANY($5)"
        }
        ExportClass::AuditEvents => {
            "SELECT id::text AS id, 'ids' AS link, to_jsonb(a) AS data, occurred_at AS at,
                    NULL::text AS call_id, NULL::text AS file, ARRAY[actor] AS ids,
                    false AS others
             FROM admin_audit_log a WHERE tenant_id = $1 AND fcp_subject_keys(actor) && $4"
        }
        ExportClass::ConfigChanges => {
            "SELECT id::text AS id, 'ids' AS link, to_jsonb(c) AS data, changed_at AS at,
                    NULL::text AS call_id, NULL::text AS file, ARRAY[changed_by] AS ids,
                    false AS others
             FROM config_audit_log c WHERE tenant_id = $1 AND fcp_subject_keys(changed_by) && $4"
        }
        ExportClass::PortRequests => {
            "SELECT id, 'ids' AS link,
                    jsonb_build_object('id', id, 'status', status, 'created_at', created_at,
                        'updated_at', updated_at, 'request', request_json - 'pin') AS data,
                    created_at AS at, NULL::text AS call_id, NULL::text AS file,
                    ARRAY[request_json->>'authorized_contact_phone',
                          request_json->>'authorized_contact_email'] AS ids,
                    false AS others
             FROM port_requests
             WHERE tenant_id = $1
               AND (fcp_subject_keys(request_json->>'authorized_contact_phone')
                    || fcp_subject_keys(request_json->>'authorized_contact_email')) && $4"
        }
        ExportClass::WebhookDeliveries => {
            "SELECT l.id, 'leaves' AS link, l.delivery_json AS data, l.created_at AS at,
                    NULL::text AS call_id, NULL::text AS file, NULL::text[] AS ids,
                    false AS others
             FROM webhook_delivery_logs l JOIN webhook_endpoints e ON e.id = l.endpoint_id
             WHERE e.tenant_id = $1 AND l.delivery_json::text ILIKE ANY($9)"
        }
        ExportClass::NumberLists => {
            "SELECT pn.id || ':' || x.list || ':' || x.entry AS id, 'ids' AS link,
                    jsonb_build_object('number', pn.e164, 'list', x.list, 'entry', x.entry) AS data,
                    NULL::timestamptz AS at, NULL::text AS call_id, NULL::text AS file,
                    ARRAY[x.entry] AS ids, false AS others
             FROM phone_numbers pn,
                  LATERAL (SELECT 'allow' AS list, e AS entry
                             FROM jsonb_array_elements_text(COALESCE(pn.allow_list, '[]')) e
                           UNION ALL
                           SELECT 'block', e
                             FROM jsonb_array_elements_text(COALESCE(pn.block_list, '[]')) e) x
             WHERE pn.tenant_id = $1 AND fcp_subject_keys(x.entry) && $4"
        }
    };
    format!(
        "SELECT {SHAPE} FROM ({q}) c
         WHERE ($2::text IS NULL OR c.id > $2)
           AND $3::bigint > 0 AND $4::text[] IS NOT NULL AND $5::text[] IS NOT NULL
           AND $6::text[] IS NOT NULL AND $7::text[] IS NOT NULL
           AND $8::text[] IS NOT NULL AND $9::text[] IS NOT NULL
         ORDER BY c.id LIMIT $3"
    )
}

/// LIKE patterns for the payloads no column names a party in: each
/// identifier's user part, or its last ten digits.
fn like_patterns(scope: &SubjectScope) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for identifier in &scope.identifiers {
        for key in fcp_core::subject::keys(identifier) {
            let needle = if let Some(d) = key.strip_prefix("num:") {
                if d.len() < 4 {
                    continue;
                }
                d[d.len().saturating_sub(10)..].to_string()
            } else if let Some(n) = key.strip_prefix("user:") {
                if n.len() < 3 {
                    continue;
                }
                n.to_string()
            } else {
                continue;
            };
            let escaped = needle
                .replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_");
            let pattern = format!("%{escaped}%");
            if !out.contains(&pattern) {
                out.push(pattern);
            }
        }
    }
    out
}

/// Every string in a JSON value, and in JSON held as a string inside it.
fn leaves(value: &serde_json::Value, out: &mut Vec<String>, depth: u8) {
    match value {
        serde_json::Value::String(s) => {
            if depth < 3 && (s.starts_with('{') || s.starts_with('[')) {
                if let Ok(inner) = serde_json::from_str::<serde_json::Value>(s) {
                    leaves(&inner, out, depth + 1);
                    return;
                }
            }
            if s.len() <= 512 {
                out.push(s.clone());
            }
        }
        serde_json::Value::Array(items) => items.iter().for_each(|v| leaves(v, out, depth)),
        serde_json::Value::Object(map) => map.values().for_each(|v| leaves(v, out, depth)),
        _ => {}
    }
}

fn row_to_subject_row(row: sqlx::postgres::PgRow) -> Result<SubjectRow> {
    let link: String = row.try_get("link")?;
    let data: serde_json::Value = row.try_get("data")?;
    let ids: Option<Vec<Option<String>>> = row.try_get("ids")?;
    let call_id: Option<String> = row.try_get("call_id")?;
    let link = match link.as_str() {
        "owned" => RowLink::Owned,
        "call" => RowLink::Call(call_id.clone().unwrap_or_default()),
        "leaves" => {
            let mut found = Vec::new();
            leaves(&data, &mut found, 0);
            RowLink::Identifiers(found)
        }
        _ => RowLink::Identifiers(ids.unwrap_or_default().into_iter().flatten().collect()),
    };
    Ok(SubjectRow {
        id: row.try_get("id")?,
        link,
        data,
        at: row
            .try_get::<Option<DateTime<Utc>>, _>("at")?
            .map(SystemTime::from),
        call_id,
        file: row
            .try_get::<Option<String>, _>("file")?
            .filter(|f| !f.is_empty()),
        others_heard: row.try_get("others")?,
    })
}

#[async_trait::async_trait]
impl SubjectDataStore for PostgresStorage {
    async fn subject_rows(
        &self,
        tenant: &TenantId,
        class: ExportClass,
        scope: &SubjectScope,
        after: Option<&str>,
        limit: usize,
    ) -> Result<SubjectPage> {
        if class == ExportClass::Cdrs {
            return cdr_page(self, tenant, scope, after, limit).await;
        }
        if class == ExportClass::WebhookDeliveries && like_patterns(scope).is_empty() {
            return Ok(SubjectPage::default());
        }
        let rows = sqlx::query(&class_query(class))
            .bind(tenant.to_string())
            .bind(after)
            .bind(limit.max(1) as i64)
            .bind(&scope.keys)
            .bind(&scope.user_ids)
            .bind(&scope.mailbox_ids)
            .bind(&scope.names)
            .bind(&scope.call_ids)
            .bind(like_patterns(scope))
            .fetch_all(self.writes())
            .await?;
        let rows = rows
            .into_iter()
            .map(row_to_subject_row)
            .collect::<Result<Vec<_>>>()?;
        let next = (rows.len() >= limit.max(1))
            .then(|| rows.last().map(|r| r.id.clone()))
            .flatten();
        Ok(SubjectPage { rows, next })
    }
}

/// The tenant's records that may be the subject's, oldest first, by
/// `(start_time, cdr_id)`: the cursor is `<micros>/<cdr id>`.
async fn cdr_page(
    storage: &PostgresStorage,
    tenant: &TenantId,
    scope: &SubjectScope,
    after: Option<&str>,
    limit: usize,
) -> Result<SubjectPage> {
    let cursor = after.and_then(|c| {
        let (micros, id) = c.split_once('/')?;
        Some((
            DateTime::<Utc>::from_timestamp_micros(micros.parse().ok()?)?,
            uuid::Uuid::parse_str(id).ok()?,
        ))
    });
    let rows = sqlx::query(
        "SELECT * FROM cdrs
         WHERE tenant_id = $1
           AND ($2::timestamptz IS NULL OR (start_time, cdr_id) > ($2, $3))
           AND (fcp_subject_keys(from_uri) || fcp_subject_keys(to_uri)) && $4
         ORDER BY start_time, cdr_id LIMIT $5",
    )
    .bind(tenant.to_string())
    .bind(cursor.map(|c| c.0))
    .bind(cursor.map(|c| c.1).unwrap_or_else(uuid::Uuid::nil))
    .bind(&scope.keys)
    .bind(limit.max(1) as i64)
    .fetch_all(storage.writes())
    .await?;
    let mut out = Vec::with_capacity(rows.len());
    let mut last = None;
    for row in rows {
        let start: DateTime<Utc> = row.try_get("start_time")?;
        let cdr = crate::row_to_cdr(row)?;
        last = Some(format!("{}/{}", start.timestamp_micros(), cdr.cdr_id));
        out.push(SubjectRow {
            id: cdr.cdr_id.to_string(),
            link: RowLink::Identifiers(vec![cdr.from_uri.clone(), cdr.to_uri.clone()]),
            at: Some(cdr.start_time),
            call_id: Some(cdr.call_id.to_string()),
            file: None,
            others_heard: false,
            data: serde_json::to_value(&cdr)?,
        });
    }
    let next = (out.len() >= limit.max(1)).then_some(last).flatten();
    Ok(SubjectPage { rows: out, next })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_patterns_are_escaped_user_parts_and_number_tails() {
        let scope = SubjectScope {
            identifiers: vec![
                "al_ice@pbx.example".into(),
                "+14155550100".into(),
                "12".into(),
            ],
            ..Default::default()
        };
        assert_eq!(
            like_patterns(&scope),
            vec!["%al\\_ice%".to_string(), "%4155550100%".to_string()]
        );
    }

    #[test]
    fn leaves_reach_into_json_held_as_a_string() {
        let v = serde_json::json!({
            "payload": "{\"from\":\"sip:alice@pbx.example\",\"n\":[\"+14155550100\"]}",
            "status_code": 200
        });
        let mut found = Vec::new();
        leaves(&v, &mut found, 0);
        assert!(found.contains(&"sip:alice@pbx.example".to_string()));
        assert!(found.contains(&"+14155550100".to_string()));
    }
}
