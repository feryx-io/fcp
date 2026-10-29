//! Presence a person sets (`docs/PRESENCE.md`): users' chosen statuses and
//! their own statuses.

use std::str::FromStr;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use fcp_core::presence_status::{ChosenStatus, OwnStatus, PresenceStatusStore, StatusState};
use fcp_core::{TenantId, UserId};
use sqlx::Row;

use crate::{datetime_to_system_time_micros, system_time_to_datetime, PostgresStorage};

fn row_to_chosen(row: &sqlx::postgres::PgRow) -> anyhow::Result<ChosenStatus> {
    Ok(ChosenStatus {
        tenant_id: TenantId::from_str(&row.try_get::<String, _>("tenant_id")?)?,
        user_id: UserId::from_str(&row.try_get::<String, _>("user_id")?)?,
        status: row.try_get("status")?,
        note: row.try_get("note")?,
        until: row
            .try_get::<Option<DateTime<Utc>>, _>("until")?
            .map(datetime_to_system_time_micros),
        updated_at: datetime_to_system_time_micros(row.try_get::<DateTime<Utc>, _>("updated_at")?),
    })
}

fn row_to_own(row: &sqlx::postgres::PgRow) -> anyhow::Result<OwnStatus> {
    let state: String = row.try_get("state")?;
    Ok(OwnStatus {
        id: row.try_get("id")?,
        tenant_id: TenantId::from_str(&row.try_get::<String, _>("tenant_id")?)?,
        user_id: UserId::from_str(&row.try_get::<String, _>("user_id")?)?,
        label: row.try_get("label")?,
        state: StatusState::parse(&state)
            .ok_or_else(|| anyhow::anyhow!("unknown status state {state}"))?,
        position: row.try_get("position")?,
        updated_at: datetime_to_system_time_micros(row.try_get::<DateTime<Utc>, _>("updated_at")?),
    })
}

const CHOSEN: &str = "tenant_id, user_id, status, note, until, updated_at";
const OWN: &str = "id, tenant_id, user_id, label, state, position, updated_at";

#[async_trait]
impl PresenceStatusStore for PostgresStorage {
    async fn get_chosen_status(&self, user: &UserId) -> anyhow::Result<Option<ChosenStatus>> {
        let row = sqlx::query(&format!(
            "SELECT {CHOSEN} FROM user_presence WHERE user_id = $1"
        ))
        .bind(user.to_string())
        .fetch_optional(&self.pool)
        .await?;
        row.as_ref().map(row_to_chosen).transpose()
    }

    async fn list_chosen_statuses(&self) -> anyhow::Result<Vec<ChosenStatus>> {
        let rows = sqlx::query(&format!("SELECT {CHOSEN} FROM user_presence"))
            .fetch_all(&self.pool)
            .await?;
        rows.iter().map(row_to_chosen).collect()
    }

    async fn put_chosen_status(&self, c: &ChosenStatus) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO user_presence (user_id, tenant_id, status, note, until, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6)
             ON CONFLICT (user_id) DO UPDATE SET
                tenant_id = EXCLUDED.tenant_id, status = EXCLUDED.status,
                note = EXCLUDED.note, until = EXCLUDED.until,
                updated_at = EXCLUDED.updated_at",
        )
        .bind(c.user_id.to_string())
        .bind(c.tenant_id.to_string())
        .bind(&c.status)
        .bind(&c.note)
        .bind(c.until.map(system_time_to_datetime))
        .bind(system_time_to_datetime(c.updated_at))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn delete_chosen_status(&self, user: &UserId) -> anyhow::Result<()> {
        sqlx::query("DELETE FROM user_presence WHERE user_id = $1")
            .bind(user.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn list_own_statuses(&self, user: &UserId) -> anyhow::Result<Vec<OwnStatus>> {
        let rows = sqlx::query(&format!(
            "SELECT {OWN} FROM presence_statuses WHERE user_id = $1 ORDER BY position, id"
        ))
        .bind(user.to_string())
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(row_to_own).collect()
    }

    async fn put_own_status(&self, s: &OwnStatus) -> anyhow::Result<()> {
        // A status stays its first owner's: an id another user holds is
        // left alone.
        sqlx::query(
            "INSERT INTO presence_statuses (id, tenant_id, user_id, label, state, position, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7)
             ON CONFLICT (id) DO UPDATE SET
                label = EXCLUDED.label, state = EXCLUDED.state,
                position = EXCLUDED.position, updated_at = EXCLUDED.updated_at
             WHERE presence_statuses.user_id = EXCLUDED.user_id",
        )
        .bind(&s.id)
        .bind(s.tenant_id.to_string())
        .bind(s.user_id.to_string())
        .bind(&s.label)
        .bind(s.state.as_str())
        .bind(s.position)
        .bind(system_time_to_datetime(s.updated_at))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn delete_own_status(&self, user: &UserId, id: &str) -> anyhow::Result<bool> {
        let done = sqlx::query("DELETE FROM presence_statuses WHERE id = $1 AND user_id = $2")
            .bind(id)
            .bind(user.to_string())
            .execute(&self.pool)
            .await?;
        Ok(done.rows_affected() > 0)
    }
}
