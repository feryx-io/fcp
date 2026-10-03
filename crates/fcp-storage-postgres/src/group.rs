//! A tenant's groups and their members (`docs/USERS.md` §5).

use std::str::FromStr;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use fcp_core::group::{Group, GroupId, GroupMember, GroupStore};
use fcp_core::{TenantId, UserId, UserRole};
use sqlx::Row;

use crate::{datetime_to_system_time, system_time_to_datetime, PostgresStorage};

const GROUP_COLS: &str =
    "g.id, g.tenant_id, g.name, g.description, g.role, g.idp_group, g.created_at, g.updated_at";

fn role_to_str(role: UserRole) -> String {
    serde_json::to_value(role)
        .ok()
        .and_then(|v| v.as_str().map(String::from))
        .unwrap_or_default()
}

fn row_to_group(row: &sqlx::postgres::PgRow) -> anyhow::Result<Group> {
    Ok(Group {
        id: row.try_get::<String, _>("id")?.parse()?,
        tenant_id: TenantId::from_str(&row.try_get::<String, _>("tenant_id")?)?,
        name: row.try_get("name")?,
        description: row.try_get("description")?,
        role: row
            .try_get::<Option<String>, _>("role")?
            .and_then(|r| UserRole::from_str(&r).ok()),
        idp_group: row.try_get("idp_group")?,
        created_at: datetime_to_system_time(row.try_get::<DateTime<Utc>, _>("created_at")?),
        updated_at: datetime_to_system_time(row.try_get::<DateTime<Utc>, _>("updated_at")?),
    })
}

fn row_to_member(row: &sqlx::postgres::PgRow) -> anyhow::Result<GroupMember> {
    Ok(GroupMember {
        group_id: row.try_get::<String, _>("group_id")?.parse()?,
        user_id: row.try_get::<String, _>("user_id")?.parse()?,
        tenant_id: TenantId::from_str(&row.try_get::<String, _>("tenant_id")?)?,
        added_at: datetime_to_system_time(row.try_get::<DateTime<Utc>, _>("added_at")?),
    })
}

#[async_trait]
impl GroupStore for PostgresStorage {
    async fn list_groups(&self, tenant_id: &TenantId) -> anyhow::Result<Vec<Group>> {
        let rows = sqlx::query(&format!(
            "SELECT {GROUP_COLS} FROM groups g WHERE g.tenant_id = $1 ORDER BY lower(g.name)"
        ))
        .bind(tenant_id.to_string())
        .fetch_all(self.reads())
        .await?;
        rows.iter().map(row_to_group).collect()
    }

    async fn get_group(&self, id: &GroupId) -> anyhow::Result<Option<Group>> {
        let row = sqlx::query(&format!(
            "SELECT {GROUP_COLS} FROM groups g WHERE g.id = $1"
        ))
        .bind(id.to_string())
        .fetch_optional(self.reads())
        .await?;
        row.as_ref().map(row_to_group).transpose()
    }

    async fn put_group(&self, group: &Group) -> anyhow::Result<()> {
        let taken: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM groups
                            WHERE tenant_id = $1 AND lower(name) = lower($2) AND id <> $3)",
        )
        .bind(group.tenant_id.to_string())
        .bind(&group.name)
        .bind(group.id.to_string())
        .fetch_one(self.writes())
        .await?;
        if taken {
            anyhow::bail!("the tenant already has a group named '{}'", group.name);
        }
        sqlx::query(
            "INSERT INTO groups (id, tenant_id, name, description, role, idp_group,
                                 created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
             ON CONFLICT (id) DO UPDATE SET
                 name = EXCLUDED.name,
                 description = EXCLUDED.description,
                 role = EXCLUDED.role,
                 idp_group = EXCLUDED.idp_group,
                 updated_at = EXCLUDED.updated_at",
        )
        .bind(group.id.to_string())
        .bind(group.tenant_id.to_string())
        .bind(&group.name)
        .bind(&group.description)
        .bind(group.role.map(role_to_str))
        .bind(&group.idp_group)
        .bind(system_time_to_datetime(group.created_at))
        .bind(system_time_to_datetime(group.updated_at))
        .execute(self.writes())
        .await?;
        Ok(())
    }

    async fn delete_group(&self, id: &GroupId) -> anyhow::Result<bool> {
        let mut tx = self.writes().begin().await?;
        // Queues that took their agents from the group take them from
        // nobody now.
        sqlx::query("UPDATE queue_configs SET agent_group_id = NULL WHERE agent_group_id = $1")
            .bind(id.to_string())
            .execute(&mut *tx)
            .await?;
        let gone = sqlx::query("DELETE FROM groups WHERE id = $1")
            .bind(id.to_string())
            .execute(&mut *tx)
            .await?
            .rows_affected()
            > 0;
        tx.commit().await?;
        Ok(gone)
    }

    async fn list_members(&self, group_id: &GroupId) -> anyhow::Result<Vec<GroupMember>> {
        let rows = sqlx::query(
            "SELECT m.group_id, m.user_id, m.tenant_id, m.added_at
             FROM group_members m JOIN users u ON u.id = m.user_id
             WHERE m.group_id = $1 ORDER BY lower(u.username)",
        )
        .bind(group_id.to_string())
        .fetch_all(self.reads())
        .await?;
        rows.iter().map(row_to_member).collect()
    }

    async fn list_tenant_members(&self, tenant_id: &TenantId) -> anyhow::Result<Vec<GroupMember>> {
        let rows = sqlx::query(
            "SELECT group_id, user_id, tenant_id, added_at FROM group_members
             WHERE tenant_id = $1 ORDER BY group_id, user_id",
        )
        .bind(tenant_id.to_string())
        .fetch_all(self.reads())
        .await?;
        rows.iter().map(row_to_member).collect()
    }

    async fn get_member(
        &self,
        group_id: &GroupId,
        user_id: &UserId,
    ) -> anyhow::Result<Option<GroupMember>> {
        let row = sqlx::query(
            "SELECT group_id, user_id, tenant_id, added_at FROM group_members
             WHERE group_id = $1 AND user_id = $2",
        )
        .bind(group_id.to_string())
        .bind(user_id.to_string())
        .fetch_optional(self.reads())
        .await?;
        row.as_ref().map(row_to_member).transpose()
    }

    async fn add_member(&self, member: &GroupMember) -> anyhow::Result<bool> {
        let added = sqlx::query(
            "INSERT INTO group_members (group_id, user_id, tenant_id, added_at)
             VALUES ($1, $2, $3, $4)
             ON CONFLICT (group_id, user_id) DO NOTHING",
        )
        .bind(member.group_id.to_string())
        .bind(member.user_id.to_string())
        .bind(member.tenant_id.to_string())
        .bind(system_time_to_datetime(member.added_at))
        .execute(self.writes())
        .await?
        .rows_affected();
        Ok(added > 0)
    }

    async fn remove_member(&self, group_id: &GroupId, user_id: &UserId) -> anyhow::Result<bool> {
        let removed = sqlx::query("DELETE FROM group_members WHERE group_id = $1 AND user_id = $2")
            .bind(group_id.to_string())
            .bind(user_id.to_string())
            .execute(self.writes())
            .await?
            .rows_affected();
        Ok(removed > 0)
    }

    async fn groups_of_user(&self, user_id: &UserId) -> anyhow::Result<Vec<Group>> {
        let rows = sqlx::query(&format!(
            "SELECT {GROUP_COLS} FROM groups g
             JOIN group_members m ON m.group_id = g.id
             WHERE m.user_id = $1 ORDER BY lower(g.name)"
        ))
        .bind(user_id.to_string())
        .fetch_all(self.reads())
        .await?;
        rows.iter().map(row_to_group).collect()
    }
}

/// The roles a user's groups give them, in one query.
pub(crate) async fn group_roles_of_user(
    storage: &PostgresStorage,
    user_id: &UserId,
) -> anyhow::Result<Vec<UserRole>> {
    let roles: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT g.role FROM groups g
         JOIN group_members m ON m.group_id = g.id
         WHERE m.user_id = $1 AND g.role IS NOT NULL",
    )
    .bind(user_id.to_string())
    .fetch_all(storage.reads())
    .await?;
    let mut roles: Vec<UserRole> = roles
        .iter()
        .filter_map(|r| UserRole::from_str(r).ok())
        // A row written around the API cannot make a platform admin.
        .filter(UserRole::may_be_given_by_a_group)
        .collect();
    roles.sort();
    Ok(roles)
}
