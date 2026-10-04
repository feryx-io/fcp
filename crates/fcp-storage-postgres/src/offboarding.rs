//! A deprovisioned user's purge (`docs/USERS.md` §4): one transaction
//! over every store that names the user.

use std::time::SystemTime;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use fcp_core::offboarding::{OffboardingStore, PurgeOutcome, PurgeReport, PurgeRequest};
use fcp_core::{TenantId, User};
use sqlx::Row;

use crate::{deletion, row_to_user, system_time_to_datetime, PostgresStorage};

/// The user part of an address of record, lower-cased, in SQL.
fn user_part(column: &str) -> String {
    format!("lower(split_part(regexp_replace({column}, '^sips?:', ''), '@', 1))")
}

#[async_trait]
impl OffboardingStore for PostgresStorage {
    async fn purge_user(&self, request: &PurgeRequest) -> anyhow::Result<PurgeOutcome> {
        let tenant = request.tenant_id.to_string();
        let id = request.user_id.to_string();
        let mut tx = deletion::attributed(self.writes()).await?;

        let Some(row) =
            sqlx::query("SELECT * FROM users WHERE tenant_id = $1 AND id = $2 FOR UPDATE")
                .bind(&tenant)
                .bind(&id)
                .fetch_optional(&mut *tx)
                .await?
        else {
            return Ok(PurgeOutcome::Gone);
        };
        let user = self.open_user(row_to_user(row)?).await?;
        if !request.mirror {
            if !user.is_deprovisioned() {
                return Ok(PurgeOutcome::NotDeprovisioned);
            }
            if let Some(before) = request.deprovisioned_before {
                if user.deprovisioned_at.is_none_or(|at| at > before) {
                    return Ok(PurgeOutcome::NotDue);
                }
            }
            // An erasure asked for the user runs first.
            let erasure_open: bool = sqlx::query_scalar(
                "SELECT EXISTS (
                     SELECT 1 FROM subject_requests
                     WHERE tenant_id = $1 AND kind = 'erasure'
                       AND status IN ('received', 'running', 'preview', 'confirmed')
                       AND subjects @> jsonb_build_array(
                           jsonb_build_object('type', 'user', 'username', $2::TEXT)))",
            )
            .bind(&tenant)
            .bind(&user.username)
            .fetch_one(&mut *tx)
            .await?;
            if erasure_open {
                return Ok(PurgeOutcome::ErasureOpen);
            }
        }

        let mut report = PurgeReport::default();

        // The names the user's phones register under: theirs, their
        // extension's, and their devices'.
        let mut names: Vec<String> =
            vec![user.username.to_lowercase(), user.sip_name().to_lowercase()];
        if let Some(ext) = user.extension.as_deref().filter(|e| !e.is_empty()) {
            names.push(ext.to_lowercase());
        }
        let device_names: Vec<String> = sqlx::query_scalar(
            "SELECT lower(sip_username) FROM devices
             WHERE tenant_id = $1 AND owner_user_id = $2 AND sip_username IS NOT NULL",
        )
        .bind(&tenant)
        .bind(&id)
        .fetch_all(&mut *tx)
        .await?;
        names.extend(device_names);
        names.sort();
        names.dedup();

        report.devices_unassigned = sqlx::query(
            "UPDATE devices SET owner_user_id = NULL, updated_at = now()
             WHERE tenant_id = $1 AND owner_user_id = $2",
        )
        .bind(&tenant)
        .bind(&id)
        .execute(&mut *tx)
        .await?
        .rows_affected();

        report.hoteling_sessions_ended =
            sqlx::query("DELETE FROM hoteling_sessions WHERE tenant_id = $1 AND user_id = $2")
                .bind(&tenant)
                .bind(&id)
                .execute(&mut *tx)
                .await?
                .rows_affected();

        // The mailbox is kept, closed and off its line, for retention to
        // empty; its old name is freed for whoever has it next.
        report.mailboxes_kept = sqlx::query(
            "UPDATE voicemail_mailboxes
             SET line_binding_id = NULL, enabled = false,
                 extension = 'offboarded-' || id, updated_at = now()
             WHERE tenant_id = $1 AND user_id = $2",
        )
        .bind(&tenant)
        .bind(&id)
        .execute(&mut *tx)
        .await?
        .rows_affected();

        report.lines_released =
            sqlx::query("DELETE FROM line_bindings WHERE tenant_id = $1 AND owner_user_id = $2")
                .bind(&tenant)
                .bind(&id)
                .execute(&mut *tx)
                .await?
                .rows_affected();

        report.extensions_released = sqlx::query(
            "DELETE FROM extensions
             WHERE tenant_id = $1 AND target_type = 'user' AND target_id = $2",
        )
        .bind(&tenant)
        .bind(&id)
        .execute(&mut *tx)
        .await?
        .rows_affected();

        report.calling_settings_removed =
            sqlx::query("DELETE FROM user_calling_settings WHERE tenant_id = $1 AND user_id = $2")
                .bind(&tenant)
                .bind(&id)
                .execute(&mut *tx)
                .await?
                .rows_affected();

        // Everyone else's forwards, find-me steps and assistants that name
        // the user. The settings hold the bare uuid.
        let naming = sqlx::query(
            "SELECT user_id, settings_json FROM user_calling_settings
             WHERE tenant_id = $1 AND settings_json::TEXT LIKE '%' || $2 || '%'
             FOR UPDATE",
        )
        .bind(&tenant)
        .bind(request.user_id.as_uuid().to_string())
        .fetch_all(&mut *tx)
        .await?;
        for row in naming {
            let other: String = row.try_get("user_id")?;
            let mut settings: fcp_core::CallingSettings =
                serde_json::from_value(row.try_get("settings_json")?)?;
            if settings.forget_user(&request.user_id) {
                sqlx::query(
                    "UPDATE user_calling_settings SET settings_json = $2, updated_at = now()
                     WHERE user_id = $1",
                )
                .bind(&other)
                .bind(serde_json::to_value(&settings)?)
                .execute(&mut *tx)
                .await?;
                report.others_settings_changed += 1;
            }
        }

        // Tokens the user made; a legacy maker is named by username.
        report.api_tokens_removed = sqlx::query(
            "DELETE FROM api_tokens
             WHERE user_id = $2 OR (tenant_id = $1 AND user_id = $3)",
        )
        .bind(&tenant)
        .bind(&id)
        .bind(&user.username)
        .execute(&mut *tx)
        .await?
        .rows_affected();

        report.queue_memberships_removed = sqlx::query(
            "DELETE FROM queue_agents
             WHERE tenant_id = $1
               AND (agent_id = $2 OR lower(agent_id) = ANY($3) OR lower(extension) = ANY($3))",
        )
        .bind(&tenant)
        .bind(&id)
        .bind(&names)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        report.queue_memberships_removed += sqlx::query(
            "UPDATE queue_configs
             SET agents = ARRAY(SELECT a FROM unnest(agents) AS a WHERE lower(a) <> ALL($2))
             WHERE tenant_id = $1
               AND EXISTS (SELECT 1 FROM unnest(agents) AS a WHERE lower(a) = ANY($2))",
        )
        .bind(&tenant)
        .bind(&names)
        .execute(&mut *tx)
        .await?
        .rows_affected();

        report.declared_locations_removed =
            sqlx::query("DELETE FROM declared_locations WHERE tenant_id = $1 AND user_id = $2")
                .bind(&tenant)
                .bind(&id)
                .execute(&mut *tx)
                .await?
                .rows_affected();

        // Meetings to the owner the tenant names, if they are still a user
        // who may own one; otherwise to nobody.
        let owner: Option<String> = match request.meetings_owner.as_deref() {
            Some(owner) if !owner.eq_ignore_ascii_case(&user.username) => {
                sqlx::query_scalar(
                    "SELECT username FROM users
                 WHERE tenant_id = $1 AND username = $2 AND account_status = 'active'",
                )
                .bind(&tenant)
                .bind(owner)
                .fetch_optional(&mut *tx)
                .await?
            }
            _ => None,
        };
        let meetings = sqlx::query(
            "UPDATE conference_meetings SET owner_id = $4
             WHERE tenant_id = $1 AND owner_id IN ($2, $3)",
        )
        .bind(&tenant)
        .bind(&user.username)
        .bind(&id)
        .bind(&owner)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if owner.is_some() {
            report.meetings_moved = meetings;
        } else {
            report.meetings_unowned = meetings;
        }

        let mut aors: Vec<String> = sqlx::query_scalar(&format!(
            "DELETE FROM registrations WHERE tenant_id = $1 AND {} = ANY($2) RETURNING aor",
            user_part("aor")
        ))
        .bind(&tenant)
        .bind(&names)
        .fetch_all(&mut *tx)
        .await?;
        report.registrations_removed = aors.len() as u64;
        aors.sort();
        aors.dedup();
        report.removed_aors = aors;

        report.presence_removed = sqlx::query(&format!(
            "DELETE FROM presence_subscriptions
             WHERE tenant_id = $1 AND ({} = ANY($2) OR {} = ANY($2))",
            user_part("watcher_aor"),
            user_part("presentity_aor")
        ))
        .bind(&tenant)
        .bind(&names)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        report.presence_removed += sqlx::query(&format!(
            "DELETE FROM presence_presentities WHERE tenant_id = $1 AND {} = ANY($2)",
            user_part("aor")
        ))
        .bind(&tenant)
        .bind(&names)
        .execute(&mut *tx)
        .await?
        .rows_affected();

        if let Some(path) = &request.profile_picture_path {
            sqlx::query(
                "INSERT INTO pending_file_deletions (tenant_id, class, path)
                 VALUES ($1, 'profile_picture', $2)",
            )
            .bind(&tenant)
            .bind(path)
            .execute(&mut *tx)
            .await?;
            report.files_queued = 1;
        }

        // The user goes last, and is logged as gone: their sessions,
        // second factor and identities follow by cascade.
        sqlx::query(
            "INSERT INTO deletion_log (tenant_id, class, item, detail, cause, actor, request_id)
             VALUES ($1, 'user', $2, $3, fcp_deletion_cause(), fcp_deletion_actor(),
                     NULLIF(current_setting('fcp.deletion_request', true), ''))",
        )
        .bind(&tenant)
        .bind(&id)
        .bind(serde_json::json!({
            "username": user.username,
            "deprovisioned_at": user.deprovisioned_at.map(system_time_to_datetime),
            "report": &report,
        }))
        .execute(&mut *tx)
        .await?;
        sqlx::query("DELETE FROM users WHERE tenant_id = $1 AND id = $2")
            .bind(&tenant)
            .bind(&id)
            .execute(&mut *tx)
            .await?;

        tx.commit().await?;
        Ok(PurgeOutcome::Purged {
            user: Box::new(user),
            report,
        })
    }

    async fn deprovisioned_users(
        &self,
        tenant_id: &TenantId,
        before: SystemTime,
    ) -> anyhow::Result<Vec<User>> {
        let rows = sqlx::query(
            "SELECT * FROM users
             WHERE tenant_id = $1 AND account_status = 'deprovisioned'
               AND deprovisioned_at <= $2
             ORDER BY deprovisioned_at",
        )
        .bind(tenant_id.to_string())
        .bind(DateTime::<Utc>::from(before))
        .fetch_all(self.reads())
        .await?;
        let mut users = Vec::with_capacity(rows.len());
        for row in rows {
            users.push(self.open_user(row_to_user(row)?).await?);
        }
        Ok(users)
    }

    async fn remove_emptied_mailboxes(&self, tenant_id: &TenantId) -> anyhow::Result<u64> {
        let mut tx = deletion::attributed(self.writes()).await?;
        let removed = sqlx::query(
            "DELETE FROM voicemail_mailboxes m
             WHERE m.tenant_id = $1
               AND NOT EXISTS (SELECT 1 FROM users u WHERE u.id = m.user_id)
               AND NOT EXISTS (SELECT 1 FROM voicemail_messages v WHERE v.mailbox_id = m.id)",
        )
        .bind(tenant_id.to_string())
        .execute(&mut *tx)
        .await?
        .rows_affected();
        tx.commit().await?;
        Ok(removed)
    }
}
