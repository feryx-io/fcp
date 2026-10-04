//! A tenant's groups of users (`docs/USERS.md` §5,
//! `docs/IDENTITY_AND_DEVICES.md` phase 5c).
//!
//! A group holds members by user id. It may carry a role, which its
//! members hold besides their own: a user's permissions are what their
//! own role and their groups' roles hold together. A group may also name
//! a group at the tenant's identity providers (`idp_group`): a sign-in
//! through one sets the user's membership of such groups from the groups
//! the provider says they are in. A queue may take its agents from a
//! group.

pub use crate::identifiers::GroupId;
use crate::permissions::UserRole;
use crate::{TenantId, UserId};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::time::SystemTime;

/// The longest name a group may have.
pub const MAX_NAME_LEN: usize = 100;

/// A tenant's group.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Group {
    pub id: GroupId,
    pub tenant_id: TenantId,
    /// Unique in the tenant.
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    /// The role the group's members hold besides their own.
    #[serde(default)]
    pub role: Option<UserRole>,
    /// The group of this name at the tenant's identity providers: a
    /// sign-in through one adds the user to this group or takes them out.
    #[serde(default)]
    pub idp_group: Option<String>,
    /// The tenant's SCIM provisioner's id for the group (`externalId`,
    /// `docs/SCIM.md`).
    #[serde(default)]
    pub scim_external_id: Option<String>,
    pub created_at: SystemTime,
    pub updated_at: SystemTime,
}

impl Group {
    /// A new group, as made.
    pub fn new(tenant_id: TenantId, name: impl Into<String>) -> Self {
        let now = SystemTime::now();
        Self {
            id: GroupId::new(),
            tenant_id,
            name: name.into(),
            description: None,
            role: None,
            idp_group: None,
            scim_external_id: None,
            created_at: now,
            updated_at: now,
        }
    }

    /// The group as stored: names trimmed, blanks cleared, and checked.
    pub fn normalized(mut self) -> Result<Self, String> {
        self.name = self.name.trim().to_string();
        if self.name.is_empty() || self.name.chars().count() > MAX_NAME_LEN {
            return Err(format!("a group's name is 1 to {MAX_NAME_LEN} characters"));
        }
        self.description = self
            .description
            .map(|d| d.trim().to_string())
            .filter(|d| !d.is_empty());
        self.idp_group = self
            .idp_group
            .map(|g| g.trim().to_string())
            .filter(|g| !g.is_empty());
        if let Some(role) = self.role {
            if !role.may_be_given_by_a_group() {
                return Err(format!(
                    "a group cannot give the role '{}'",
                    serde_json::to_value(role)
                        .ok()
                        .and_then(|v| v.as_str().map(String::from))
                        .unwrap_or_default()
                ));
            }
        }
        Ok(self)
    }
}

/// A user's membership of a group.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupMember {
    pub group_id: GroupId,
    pub user_id: UserId,
    pub tenant_id: TenantId,
    pub added_at: SystemTime,
}

impl GroupMember {
    /// The id the sync journal names a membership by.
    pub fn row_id(&self) -> String {
        format!("{}:{}", self.group_id, self.user_id)
    }

    /// The group and user a journal row id names.
    pub fn parse_row_id(id: &str) -> Option<(GroupId, UserId)> {
        let (group, user) = id.split_once(':')?;
        Some((group.parse().ok()?, user.parse().ok()?))
    }
}

/// Where groups are kept.
#[async_trait]
pub trait GroupStore: Send + Sync {
    /// A tenant's groups, by name.
    async fn list_groups(&self, tenant_id: &TenantId) -> anyhow::Result<Vec<Group>>;
    async fn get_group(&self, id: &GroupId) -> anyhow::Result<Option<Group>>;
    /// Write a group; a name another group of the tenant has is refused.
    async fn put_group(&self, group: &Group) -> anyhow::Result<()>;
    /// Remove a group and its memberships.
    async fn delete_group(&self, id: &GroupId) -> anyhow::Result<bool>;

    /// A group's members.
    async fn list_members(&self, group_id: &GroupId) -> anyhow::Result<Vec<GroupMember>>;
    /// Every membership in a tenant (a site's snapshot).
    async fn list_tenant_members(&self, tenant_id: &TenantId) -> anyhow::Result<Vec<GroupMember>>;
    /// One membership, when it exists.
    async fn get_member(
        &self,
        group_id: &GroupId,
        user_id: &UserId,
    ) -> anyhow::Result<Option<GroupMember>>;
    /// Add a member; adding one already there changes nothing. Whether
    /// they were added.
    async fn add_member(&self, member: &GroupMember) -> anyhow::Result<bool>;
    /// Whether they were there to remove.
    async fn remove_member(&self, group_id: &GroupId, user_id: &UserId) -> anyhow::Result<bool>;

    /// The groups a user is in.
    async fn groups_of_user(&self, user_id: &UserId) -> anyhow::Result<Vec<Group>>;
}

/// The roles a user's groups give them.
pub fn roles_of(groups: &[Group]) -> Vec<UserRole> {
    let mut roles: Vec<UserRole> = groups.iter().filter_map(|g| g.role).collect();
    roles.sort();
    roles.dedup();
    roles
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_group_is_checked_and_trimmed() {
        let mut g = Group::new(TenantId::new(), "  Sales  ");
        g.description = Some("  ".into());
        g.idp_group = Some(" sales-team ".into());
        g.role = Some(UserRole::TelecomAdmin);
        let g = g.normalized().unwrap();
        assert_eq!(g.name, "Sales");
        assert_eq!(g.description, None);
        assert_eq!(g.idp_group.as_deref(), Some("sales-team"));

        for role in [UserRole::SuperAdmin, UserRole::DepartmentAdmin] {
            let mut g = Group::new(TenantId::new(), "x");
            g.role = Some(role);
            assert!(g.normalized().is_err(), "{role:?}");
        }
        assert!(Group::new(TenantId::new(), " ").normalized().is_err());
    }

    #[test]
    fn a_membership_round_trips_through_its_row_id() {
        let m = GroupMember {
            group_id: GroupId::new(),
            user_id: UserId::new(),
            tenant_id: TenantId::new(),
            added_at: SystemTime::now(),
        };
        assert_eq!(
            GroupMember::parse_row_id(&m.row_id()),
            Some((m.group_id, m.user_id))
        );
    }

    #[test]
    fn roles_together_hold_what_each_holds() {
        use crate::permissions::{roles_have_permission, DeploymentMode, Permission};
        let mode = DeploymentMode::default();
        // A user admin manages users and a telecom admin queues; the two
        // together, both — where "the highest" would have lost one.
        let both = [UserRole::TelecomAdmin, UserRole::UserAdmin];
        assert!(roles_have_permission(&both, Permission::UserCreate, mode));
        assert!(roles_have_permission(&both, Permission::QueueManage, mode));
        assert!(!roles_have_permission(
            &[UserRole::TelecomAdmin],
            Permission::UserCreate,
            mode
        ));
        assert_eq!(
            UserRole::most_privileged(both),
            Some(UserRole::TelecomAdmin)
        );
    }
}
