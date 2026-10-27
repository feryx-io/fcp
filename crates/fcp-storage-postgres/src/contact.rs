//! A user's own contacts (`docs/RECORD_AND_CONTACTS.md` §3).

use std::str::FromStr;
use std::time::SystemTime;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use fcp_core::contact::{ContactStore, PersonalContact};
use fcp_core::{TenantId, UserId};
use sqlx::Row;

use crate::{datetime_to_system_time_micros, system_time_to_datetime, PostgresStorage};

const COLS: &str = "id, tenant_id, owner_user_id, name, company, numbers, email, notes, \
                    created_at, updated_at, deleted_at, ship_seq";

/// Times to the microsecond, as stored: which copy is newer is decided on
/// them.
fn row_to_contact(row: &sqlx::postgres::PgRow) -> anyhow::Result<PersonalContact> {
    Ok(PersonalContact {
        id: row.try_get("id")?,
        tenant_id: TenantId::from_str(&row.try_get::<String, _>("tenant_id")?)?,
        owner_user_id: UserId::from_str(&row.try_get::<String, _>("owner_user_id")?)?,
        name: row.try_get("name")?,
        company: row.try_get("company")?,
        numbers: serde_json::from_value(row.try_get("numbers")?)?,
        email: row.try_get("email")?,
        notes: row.try_get("notes")?,
        created_at: datetime_to_system_time_micros(row.try_get::<DateTime<Utc>, _>("created_at")?),
        updated_at: datetime_to_system_time_micros(row.try_get::<DateTime<Utc>, _>("updated_at")?),
        deleted_at: row
            .try_get::<Option<DateTime<Utc>>, _>("deleted_at")?
            .map(datetime_to_system_time_micros),
    })
}

#[async_trait]
impl ContactStore for PostgresStorage {
    async fn list_contacts(
        &self,
        owner: &UserId,
        q: Option<&str>,
        after: Option<&str>,
        limit: usize,
    ) -> anyhow::Result<Vec<PersonalContact>> {
        // Keyset by (lower(name), id); the cursor names the last contact.
        let pattern = q.map(|q| {
            format!(
                "%{}%",
                q.to_lowercase()
                    .replace('\\', "\\\\")
                    .replace('%', "\\%")
                    .replace('_', "\\_")
            )
        });
        let rows = sqlx::query(&format!(
            "SELECT {COLS} FROM personal_contacts c
             WHERE owner_user_id = $1 AND deleted_at IS NULL
               AND ($2::text IS NULL
                    OR lower(name) LIKE $2 OR lower(coalesce(company, '')) LIKE $2
                    OR lower(coalesce(email, '')) LIKE $2 OR lower(numbers::text) LIKE $2)
               AND ($3::text IS NULL OR (lower(name), id) >
                    (SELECT lower(name), id FROM personal_contacts WHERE id = $3))
             ORDER BY lower(name), id
             LIMIT $4"
        ))
        .bind(owner.to_string())
        .bind(pattern)
        .bind(after)
        .bind(limit as i64)
        .fetch_all(self.reads())
        .await?;
        rows.iter().map(row_to_contact).collect()
    }

    async fn count_contacts(&self, owner: &UserId) -> anyhow::Result<u64> {
        let n: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM personal_contacts
             WHERE owner_user_id = $1 AND deleted_at IS NULL",
        )
        .bind(owner.to_string())
        .fetch_one(self.reads())
        .await?;
        Ok(n as u64)
    }

    async fn get_contact(&self, id: &str) -> anyhow::Result<Option<PersonalContact>> {
        let row = sqlx::query(&format!(
            "SELECT {COLS} FROM personal_contacts WHERE id = $1"
        ))
        .bind(id)
        .fetch_optional(self.writes())
        .await?;
        row.as_ref().map(row_to_contact).transpose()
    }

    async fn put_contact_if_newer(&self, c: &PersonalContact) -> anyhow::Result<bool> {
        // A tombstone only marks a contact kept here: one for a contact
        // already gone (swept, or never here) is not kept, so a late echo
        // brings nothing back.
        if let Some(deleted_at) = c.deleted_at {
            let done = sqlx::query(
                "UPDATE personal_contacts SET deleted_at = $2, updated_at = $3
                 WHERE id = $1 AND updated_at < $3 AND tenant_id = $4 AND owner_user_id = $5",
            )
            .bind(&c.id)
            .bind(system_time_to_datetime(deleted_at))
            .bind(system_time_to_datetime(c.updated_at))
            .bind(c.tenant_id.to_string())
            .bind(c.owner_user_id.to_string())
            .execute(self.writes())
            .await?;
            return Ok(done.rows_affected() > 0);
        }
        let done = sqlx::query(
            "INSERT INTO personal_contacts
                 (id, tenant_id, owner_user_id, name, company, numbers, email, notes,
                  created_at, updated_at, deleted_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
             ON CONFLICT (id) DO UPDATE SET
                 name = EXCLUDED.name, company = EXCLUDED.company,
                 numbers = EXCLUDED.numbers, email = EXCLUDED.email,
                 notes = EXCLUDED.notes, updated_at = EXCLUDED.updated_at,
                 deleted_at = NULL
             WHERE personal_contacts.updated_at < EXCLUDED.updated_at
               AND personal_contacts.tenant_id = EXCLUDED.tenant_id
               AND personal_contacts.owner_user_id = EXCLUDED.owner_user_id",
        )
        .bind(&c.id)
        .bind(c.tenant_id.to_string())
        .bind(c.owner_user_id.to_string())
        .bind(&c.name)
        .bind(&c.company)
        .bind(serde_json::to_value(&c.numbers)?)
        .bind(&c.email)
        .bind(&c.notes)
        .bind(system_time_to_datetime(c.created_at))
        .bind(system_time_to_datetime(c.updated_at))
        .bind(None::<chrono::DateTime<Utc>>)
        .execute(self.writes())
        .await?;
        Ok(done.rows_affected() > 0)
    }

    async fn delete_contact_row(&self, id: &str) -> anyhow::Result<bool> {
        let done = sqlx::query("DELETE FROM personal_contacts WHERE id = $1")
            .bind(id)
            .execute(self.writes())
            .await?;
        Ok(done.rows_affected() > 0)
    }

    async fn sweep_contact_tombstones(&self, before: SystemTime) -> anyhow::Result<u64> {
        let done = sqlx::query(
            "DELETE FROM personal_contacts WHERE deleted_at IS NOT NULL AND deleted_at < $1",
        )
        .bind(system_time_to_datetime(before))
        .execute(self.writes())
        .await?;
        Ok(done.rows_affected())
    }

    async fn contacts_after(
        &self,
        after: i64,
        limit: usize,
    ) -> anyhow::Result<Vec<(i64, PersonalContact)>> {
        let rows = sqlx::query(&format!(
            "SELECT {COLS} FROM personal_contacts
             WHERE ship_seq > $1 ORDER BY ship_seq LIMIT $2"
        ))
        .bind(after)
        .bind(limit as i64)
        .fetch_all(self.writes())
        .await?;
        rows.iter()
            .map(|row| {
                let seq: i64 = row.try_get("ship_seq")?;
                Ok((seq, row_to_contact(row)?))
            })
            .collect()
    }

    async fn tenant_contacts(&self, tenant: &TenantId) -> anyhow::Result<Vec<PersonalContact>> {
        let rows = sqlx::query(&format!(
            "SELECT {COLS} FROM personal_contacts WHERE tenant_id = $1 ORDER BY id"
        ))
        .bind(tenant.to_string())
        .fetch_all(self.reads())
        .await?;
        rows.iter().map(row_to_contact).collect()
    }
}
