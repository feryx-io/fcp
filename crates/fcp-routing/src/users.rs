//! The routing engine's users from the database
//! (`docs/PRODUCTION_STACK.md` §1.5, phase 5).
//!
//! The YAML rules file is the right home for a node's policies and the
//! wrong one for its users, which the database has: every other lookup
//! finds a caller there. [`DatabaseRoutingStore`] is a [`RoutingStore`]
//! whose users come from the platform's [`CredentialStore`] — the same
//! store the registrar authenticates against — and whose policies come
//! from whatever store it wraps (the YAML file). A caller is identified
//! the way the SIP handler names one, by the URI in its `From`:
//! `sip:alice@fcp.local` is the user `alice`, looked up within the tenant
//! the handler resolved for the call.

use crate::{RoutingStore, TenantFeatures, User};
use anyhow::Result;
use async_trait::async_trait;
use fcp_core::storage::CredentialStore;
use fcp_core::TenantId;
use fcp_policy::models::PolicyRule;
use std::collections::HashSet;
use std::sync::Arc;

/// Users from the credential store, policies from another routing store.
pub struct DatabaseRoutingStore {
    users: Arc<dyn CredentialStore>,
    policies: Arc<dyn RoutingStore>,
}

/// The user part of a caller id as the handler builds it — a SIP or SIPS
/// URI, or a bare `user@host`, or a bare user — with any parameters and
/// headers dropped: `sip:alice@fcp.local;transport=tls` is `alice`.
pub fn username_of(caller: &str) -> &str {
    let rest = caller.trim().trim_start_matches('<').trim_end_matches('>');
    let rest = rest
        .strip_prefix("sips:")
        .or_else(|| rest.strip_prefix("sip:"))
        .unwrap_or(rest);
    let rest = rest.split([';', '?']).next().unwrap_or(rest);
    rest.split('@').next().unwrap_or(rest)
}

/// The engine's view of a platform user: its id is the caller id it was
/// asked for, its role is its one role, its department its one group.
fn engine_user(id: &str, user: fcp_core::User) -> User {
    let enabled = user.may_use_services();
    let mut roles = HashSet::new();
    roles.insert(
        serde_json::to_value(user.role)
            .ok()
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_else(|| format!("{:?}", user.role).to_lowercase()),
    );
    let mut groups = HashSet::new();
    if let Some(department) = user.department_id {
        groups.insert(department);
    }
    User {
        id: id.to_string(),
        tenant_id: user.tenant_id,
        display_name: user.display_name,
        roles,
        groups,
        endpoints: Vec::new(),
        enabled,
    }
}

impl DatabaseRoutingStore {
    /// Users from `users`, policies (and tenant features) from `policies`.
    pub fn new(users: Arc<dyn CredentialStore>, policies: Arc<dyn RoutingStore>) -> Self {
        Self { users, policies }
    }
}

#[async_trait]
impl RoutingStore for DatabaseRoutingStore {
    /// The user with this caller's username, in whichever tenant has it;
    /// an error when several do, since the caller is then one of several
    /// and only the tenant the call belongs to can say which
    /// ([`RoutingStore::get_user`]).
    async fn find_user(&self, user_id: &str) -> Result<Option<User>> {
        let username = username_of(user_id);
        let mut found = self.users.list_users_by_sip_name(username).await?;
        match found.len() {
            0 => Ok(None),
            1 => Ok(Some(engine_user(user_id, found.remove(0)))),
            n => anyhow::bail!(
                "user '{username}' exists in {n} tenants; look the caller up within the call's tenant"
            ),
        }
    }

    async fn get_user(&self, user_id: &str, tenant_id: &TenantId) -> Result<Option<User>> {
        let username = username_of(user_id);
        Ok(self
            .users
            .get_user_by_sip_name(username, tenant_id)
            .await?
            .map(|u| engine_user(user_id, u)))
    }

    async fn list_users(&self, tenant_id: &TenantId) -> Result<Vec<User>> {
        Ok(self
            .users
            .list_users_in_tenant(tenant_id, &fcp_core::storage::UserFilter::default())
            .await?
            .into_iter()
            .map(|u| {
                let id = u.username.clone();
                engine_user(&id, u)
            })
            .collect())
    }

    async fn list_policy_rules(&self, tenant_id: &TenantId) -> Result<Vec<PolicyRule>> {
        self.policies.list_policy_rules(tenant_id).await
    }

    async fn get_policy_rule(
        &self,
        name: &str,
        tenant_id: &TenantId,
    ) -> Result<Option<PolicyRule>> {
        self.policies.get_policy_rule(name, tenant_id).await
    }

    async fn get_tenant_features(&self, tenant_id: &TenantId) -> Result<Option<TenantFeatures>> {
        self.policies.get_tenant_features(tenant_id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fcp_core::storage::{AccountStatus, EmergencyContact, MediaPreferences};
    use fcp_core::{TenantId, UserRole};
    use std::time::SystemTime;

    fn platform_user(username: &str, tenant: TenantId, role: UserRole) -> fcp_core::User {
        fcp_core::User {
            id: fcp_core::UserId::new(),
            username: username.to_string(),
            password: "x".to_string(),
            digest_password: None,
            sip_ha1_hash: None,
            sip_provisioning_password: None,
            display_name: Some(username.to_uppercase()),
            directory_listed: true,
            tenant_id: tenant,
            enabled: true,
            role,
            department_id: Some("sales".to_string()),
            created_at: SystemTime::now(),
            updated_at: SystemTime::now(),
            media_preferences: MediaPreferences::default(),
            email: None,
            password_last_reset: None,
            phone_number: None,
            mobile_number: None,
            pin: None,
            extension: None,
            department: None,
            job_title: None,
            location: None,
            location_id: None,
            language: "en-US".to_string(),
            timezone: "UTC".to_string(),
            account_status: AccountStatus::Active,
            last_login: None,
            emergency_contact: EmergencyContact::default(),
            primary_phone_number_id: None,
            voicemail_box_id: None,
            profile_picture: None,
            deprovisioned_at: None,
            sip_username: None,
            scim_managed: false,
            scim_external_id: None,
            failed_login_attempts: 0,
            account_locked_until: None,
        }
    }

    /// The platform's users, as a credential store answers for them.
    struct Users(Vec<fcp_core::User>);

    #[async_trait]
    impl CredentialStore for Users {
        async fn get_user(&self, username: &str) -> Result<Option<fcp_core::User>> {
            let mut found = self.list_users_by_username(username).await?;
            anyhow::ensure!(found.len() <= 1, "ambiguous");
            Ok(found.pop())
        }
        async fn get_user_in_tenant(
            &self,
            username: &str,
            tenant_id: &TenantId,
        ) -> Result<Option<fcp_core::User>> {
            Ok(self
                .0
                .iter()
                .find(|u| u.username == username && &u.tenant_id == tenant_id)
                .cloned())
        }
        async fn list_users_by_username(&self, username: &str) -> Result<Vec<fcp_core::User>> {
            Ok(self
                .0
                .iter()
                .filter(|u| u.username == username)
                .cloned()
                .collect())
        }
        async fn put_user(&self, _user: fcp_core::User) -> Result<()> {
            unimplemented!()
        }
        async fn delete_user(&self, _username: &str) -> Result<()> {
            unimplemented!()
        }
        async fn delete_user_in_tenant(
            &self,
            _username: &str,
            _tenant_id: &TenantId,
        ) -> Result<()> {
            unimplemented!()
        }
        async fn list_users(&self) -> Result<Vec<fcp_core::User>> {
            Ok(self.0.clone())
        }
    }

    /// A rules file with no users and no policies.
    struct NoPolicies;

    #[async_trait]
    impl RoutingStore for NoPolicies {
        async fn find_user(&self, _user_id: &str) -> Result<Option<User>> {
            Ok(None)
        }
        async fn get_user(&self, _user_id: &str, _tenant_id: &TenantId) -> Result<Option<User>> {
            Ok(None)
        }
        async fn list_users(&self, _tenant_id: &TenantId) -> Result<Vec<User>> {
            Ok(Vec::new())
        }
        async fn list_policy_rules(&self, _tenant_id: &TenantId) -> Result<Vec<PolicyRule>> {
            Ok(Vec::new())
        }
        async fn get_policy_rule(
            &self,
            _name: &str,
            _tenant_id: &TenantId,
        ) -> Result<Option<PolicyRule>> {
            Ok(None)
        }
    }

    fn store(users: Vec<fcp_core::User>) -> DatabaseRoutingStore {
        DatabaseRoutingStore::new(Arc::new(Users(users)), Arc::new(NoPolicies))
    }

    #[tokio::test]
    async fn a_caller_is_looked_up_within_the_tenant_and_seen_as_the_engine_needs() {
        let t1 = TenantId::new();
        let t2 = TenantId::new();
        let store = store(vec![
            platform_user("alice", t1, UserRole::EndUser),
            platform_user("alice", t2, UserRole::TenantAdmin),
        ]);

        let alice = store
            .get_user("sip:alice@fcp.local;transport=tls", &t1)
            .await
            .unwrap()
            .expect("alice in t1");
        assert_eq!(alice.id, "sip:alice@fcp.local;transport=tls");
        assert_eq!(alice.tenant_id, t1);
        assert!(alice.roles.contains("end_user"), "{:?}", alice.roles);
        assert!(alice.groups.contains("sales"));
        assert!(alice.enabled);
        assert_eq!(alice.display_name.as_deref(), Some("ALICE"));

        let other = store
            .get_user("sip:alice@fcp.local", &t2)
            .await
            .unwrap()
            .unwrap();
        assert!(other.roles.contains("tenant_admin"), "{:?}", other.roles);

        assert!(store
            .get_user("sip:nobody@fcp.local", &t1)
            .await
            .unwrap()
            .is_none());
        assert!(store
            .get_user("sip:alice@fcp.local", &TenantId::new())
            .await
            .unwrap()
            .is_none());
    }

    #[test]
    fn a_disabled_user_is_returned_disabled() {
        let mut u = platform_user("carol", TenantId::nil(), UserRole::EndUser);
        u.enabled = false;
        assert!(!engine_user("carol", u).enabled);
    }

    #[tokio::test]
    async fn without_a_tenant_a_username_shared_by_tenants_is_an_error_not_a_guess() {
        let t1 = TenantId::new();
        let t2 = TenantId::new();
        let store = store(vec![
            platform_user("alice", t1, UserRole::EndUser),
            platform_user("alice", t2, UserRole::EndUser),
            platform_user("bob", t1, UserRole::EndUser),
        ]);
        let bob = store.find_user("bob@fcp.local").await.unwrap().unwrap();
        assert_eq!(bob.tenant_id, t1);
        assert!(store.find_user("nobody").await.unwrap().is_none());
        let e = store.find_user("alice@fcp.local").await.unwrap_err();
        assert!(e.to_string().contains("2 tenants"), "{e}");

        let all = store.list_users(&t1).await.unwrap();
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn the_username_is_taken_from_however_the_caller_is_named() {
        assert_eq!(username_of("sip:alice@fcp.local"), "alice");
        assert_eq!(username_of("sips:alice@fcp.local;transport=tls"), "alice");
        assert_eq!(username_of("<sip:alice@fcp.local>"), "alice");
        assert_eq!(username_of("alice@fcp.local"), "alice");
        assert_eq!(username_of("alice"), "alice");
        assert_eq!(username_of("sip:1001@10.0.0.1:5060?x=y"), "1001");
    }
}
