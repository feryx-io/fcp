//! A tenant's identity providers and the accounts its users hold at them
//! (`docs/IDENTITY_AND_DEVICES.md` phase 3).
//!
//! A provider is OpenID Connect today (SAML is phase 4): an issuer, a
//! client registered there, and how the ID token's claims become an FCP
//! user — the username, email and name claims, groups mapped to roles,
//! whether a stranger is created on first sign-in (just in time) and
//! whether a verified email links an existing user. The client secret is
//! sealed at rest with the tenant's other secrets.
//!
//! A user's identity is the provider's subject (`sub`) for them, so a
//! renamed or re-mailed account stays the same user. A sign-in in flight
//! is an [`SsoLogin`]: the state, nonce and PKCE verifier the browser's
//! round trip through the provider must match, then a one-time code the
//! UI exchanges for a session.

pub use crate::identifiers::IdentityProviderId;
use crate::permissions::UserRole;
use crate::{TenantId, UserId};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::time::SystemTime;

/// The protocol a provider speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    #[default]
    Oidc,
    /// SAML 2.0 (phase 4): the browser SSO profile, HTTP-Redirect out and
    /// HTTP-POST back.
    Saml,
}

impl ProviderKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Oidc => "oidc",
            Self::Saml => "saml",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "oidc" => Some(Self::Oidc),
            "saml" => Some(Self::Saml),
            _ => None,
        }
    }
}

/// A tenant's identity provider. Serialized whole only for a site's link,
/// over its mutual TLS, which stores it sealed with the site's own key
/// (`docs/IDENTITY_AND_DEVICES.md` Q11); the API answers without the secret.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdentityProvider {
    pub id: IdentityProviderId,
    pub tenant_id: TenantId,
    /// What the login page's button says: "Continue with <name>".
    pub name: String,
    pub kind: ProviderKind,
    /// Offered at sign-in.
    pub enabled: bool,
    pub settings: ProviderSettings,
    /// The client secret; `None` for a public client (PKCE alone).
    pub client_secret: Option<String>,
    pub created_at: SystemTime,
    pub updated_at: SystemTime,
}

/// A provider's configuration, kept as one JSON document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(deny_unknown_fields)]
pub struct ProviderSettings {
    /// OIDC: the issuer, as its ID tokens name it; discovery is read from
    /// `<issuer>/.well-known/openid-configuration`.
    #[serde(default)]
    pub issuer: String,
    /// OIDC: the client registered at the issuer.
    #[serde(default)]
    pub client_id: String,
    /// SAML: the identity provider's metadata and what to trust of it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub saml: Option<SamlSettings>,
    /// Scopes asked for besides `openid`.
    #[serde(default = "default_scopes")]
    pub scopes: Vec<String>,
    #[serde(default)]
    pub claims: ClaimMapping,
    /// Groups the provider asserts, and the role each gives. The most
    /// privileged role a user's groups give wins, and is set at every
    /// sign-in; with no mapping, a user's role is left as an admin set it.
    #[serde(default)]
    pub group_roles: Vec<GroupRole>,
    /// Create a user the first time someone the provider vouches for
    /// signs in, with no FCP account yet.
    #[serde(default)]
    pub jit: bool,
    /// Link an existing user whose email is the one the provider asserts
    /// as verified, on their first sign-in.
    #[serde(default = "yes")]
    pub link_by_email: bool,
    /// The claim that says the email is verified: `email_verified`, the
    /// standard one; `xms_edov` for Entra ID, whose tokens carry no
    /// `email_verified`.
    #[serde(default = "default_email_verified_claim")]
    pub email_verified_claim: String,
    /// Claims a token must carry to sign anyone in: Google's `hd` (the
    /// Workspace domain), so a personal Google account is turned away;
    /// Entra's `tid`. A claim that is a list must contain the value.
    #[serde(default)]
    pub required_claims: Vec<RequiredClaim>,
}

fn default_email_verified_claim() -> String {
    "email_verified".to_string()
}

/// A claim a token must carry, with this value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(deny_unknown_fields)]
pub struct RequiredClaim {
    pub claim: String,
    pub value: String,
}

/// A SAML identity provider, from its metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(deny_unknown_fields)]
pub struct SamlSettings {
    /// The provider's metadata XML, as it publishes it: its entity id, its
    /// single sign-on address and the certificates it signs with.
    pub idp_metadata_xml: String,
    /// Take the email attribute as the person's own, for linking by
    /// email. SAML has no `email_verified`; turn this on only for a
    /// provider whose directory owns its users' addresses.
    #[serde(default)]
    pub trust_email: bool,
}

/// The attribute name that means the assertion's subject (`NameID`), in a
/// SAML provider's claim mapping.
pub const SAML_NAME_ID: &str = "NameID";

fn default_scopes() -> Vec<String> {
    vec!["profile".to_string(), "email".to_string()]
}

fn yes() -> bool {
    true
}

/// Which claims become the user's names.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(deny_unknown_fields)]
pub struct ClaimMapping {
    /// The username of a user made just in time; the part before the `@`
    /// when the claim is an address. Falls back to the email's.
    #[serde(default = "default_username_claim")]
    pub username: String,
    #[serde(default = "default_email_claim")]
    pub email: String,
    #[serde(default = "default_name_claim")]
    pub display_name: String,
    /// A list of strings: the user's groups (Entra ID sends ids, Okta
    /// and Google Workspace names, when asked to).
    #[serde(default = "default_groups_claim")]
    pub groups: String,
}

fn default_username_claim() -> String {
    "preferred_username".to_string()
}
fn default_email_claim() -> String {
    "email".to_string()
}
fn default_name_claim() -> String {
    "name".to_string()
}
fn default_groups_claim() -> String {
    "groups".to_string()
}

impl Default for ClaimMapping {
    fn default() -> Self {
        Self {
            username: default_username_claim(),
            email: default_email_claim(),
            display_name: default_name_claim(),
            groups: default_groups_claim(),
        }
    }
}

/// A provider group, and the role its members get.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(deny_unknown_fields)]
pub struct GroupRole {
    pub group: String,
    /// Any role but `super_admin`, which no tenant's provider grants.
    #[cfg_attr(feature = "openapi", schema(value_type = String))]
    pub role: UserRole,
}

impl ProviderSettings {
    /// The role `groups` give: the most privileged mapped, or `None`
    /// where none is (the user's role is then left alone).
    pub fn role_for(&self, groups: &[String]) -> Option<UserRole> {
        self.group_roles
            .iter()
            .filter(|m| groups.iter().any(|g| g == &m.group))
            .map(|m| m.role)
            // `UserRole` orders the most privileged first.
            .min()
    }

    /// Whether `claims` carry every required claim.
    pub fn carries_required(&self, claims: &serde_json::Map<String, serde_json::Value>) -> bool {
        self.required_claims
            .iter()
            .all(|r| match claims.get(&r.claim) {
                Some(serde_json::Value::String(s)) => s == &r.value,
                Some(serde_json::Value::Array(list)) => {
                    list.iter().any(|v| v.as_str() == Some(r.value.as_str()))
                }
                Some(serde_json::Value::Bool(b)) => b.to_string() == r.value,
                Some(serde_json::Value::Number(n)) => n.to_string() == r.value,
                _ => false,
            })
    }

    /// What is wrong with it for a provider of `kind`, if anything.
    pub fn problems(&self, kind: ProviderKind) -> Option<String> {
        match kind {
            ProviderKind::Oidc => {
                if self.issuer.trim().is_empty() || self.client_id.trim().is_empty() {
                    return Some("issuer and client_id are required".to_string());
                }
                if self.saml.is_some() {
                    return Some("an OIDC provider has no saml settings".to_string());
                }
            }
            ProviderKind::Saml => match &self.saml {
                Some(s) if !s.idp_metadata_xml.trim().is_empty() => {}
                _ => return Some("saml.idp_metadata_xml is required".to_string()),
            },
        }
        if self
            .group_roles
            .iter()
            .any(|m| m.role == UserRole::SuperAdmin)
        {
            return Some("a provider cannot grant super_admin".to_string());
        }
        None
    }
}

/// A user's account at a provider.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserIdentity {
    pub provider_id: IdentityProviderId,
    /// The provider's `sub` for the user: stable, never reassigned.
    pub subject: String,
    pub user_id: UserId,
    pub tenant_id: TenantId,
    /// The email the provider last asserted, for the admin to read.
    pub email: Option<String>,
    pub created_at: SystemTime,
    pub last_sign_in_at: Option<SystemTime>,
}

impl UserIdentity {
    /// The one string the sync journal names it by: `<provider>:<subject>`
    /// (migration 094's generated `id`; a provider id has no colon).
    pub fn row_id(&self) -> String {
        format!("{}:{}", self.provider_id, self.subject)
    }

    /// The provider and subject a journal row id names.
    pub fn parse_row_id(id: &str) -> Option<(IdentityProviderId, String)> {
        let (provider, subject) = id.split_once(':')?;
        Some((provider.parse().ok()?, subject.to_string()))
    }
}

/// A sign-in through a provider, in flight.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SsoLogin {
    /// The `state` parameter: random, and the browser's cookie.
    pub state: String,
    pub provider_id: IdentityProviderId,
    pub tenant_id: TenantId,
    pub nonce: String,
    pub code_verifier: String,
    pub created_at: SystemTime,
}

/// A finished sign-in, waiting for the UI to collect it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SsoHandoff {
    /// SHA-256 of the one-time code.
    pub code_hash: String,
    pub user_id: UserId,
    pub provider_id: IdentityProviderId,
    /// The provider's session (`sid`), when its ID token named one.
    pub idp_sid: Option<String>,
}

/// How long a sign-in may take at the provider.
pub const SSO_LOGIN_TTL: std::time::Duration = std::time::Duration::from_secs(600);
/// How long the UI has to exchange its one-time code.
pub const SSO_HANDOFF_TTL: std::time::Duration = std::time::Duration::from_secs(120);

/// Where providers, identities and sign-ins in flight are kept.
#[async_trait]
pub trait IdentityStore: Send + Sync {
    async fn list_identity_providers(
        &self,
        tenant_id: &TenantId,
    ) -> crate::Result<Vec<IdentityProvider>>;
    async fn get_identity_provider(
        &self,
        id: &IdentityProviderId,
    ) -> crate::Result<Option<IdentityProvider>>;
    async fn put_identity_provider(&self, provider: &IdentityProvider) -> crate::Result<()>;
    /// Delete a provider and the identities at it.
    async fn delete_identity_provider(&self, id: &IdentityProviderId) -> crate::Result<bool>;

    async fn get_user_identity(
        &self,
        provider_id: &IdentityProviderId,
        subject: &str,
    ) -> crate::Result<Option<UserIdentity>>;
    /// Create or update (by provider and subject).
    async fn put_user_identity(&self, identity: &UserIdentity) -> crate::Result<()>;
    async fn list_user_identities(&self, user_id: &UserId) -> crate::Result<Vec<UserIdentity>>;
    /// Every identity of a tenant's users (a site's slice).
    async fn list_tenant_identities(
        &self,
        tenant_id: &TenantId,
    ) -> crate::Result<Vec<UserIdentity>>;
    async fn delete_user_identity(
        &self,
        provider_id: &IdentityProviderId,
        subject: &str,
    ) -> crate::Result<bool>;

    async fn create_sso_login(&self, login: &SsoLogin) -> crate::Result<()>;
    /// The sign-in `state` names, taken: a second callback with the same
    /// state finds nothing. Sign-ins older than [`SSO_LOGIN_TTL`] are not
    /// returned.
    async fn take_sso_login(&self, state: &str, now: SystemTime)
        -> crate::Result<Option<SsoLogin>>;
    /// Record a finished sign-in for the UI to collect by the hash of its
    /// one-time code.
    async fn put_sso_handoff(&self, handoff: &SsoHandoff, now: SystemTime) -> crate::Result<()>;
    /// Remember a SAML assertion's id until `until`: false when it was
    /// already seen — a replay (phase 4).
    async fn remember_saml_assertion(
        &self,
        provider_id: &IdentityProviderId,
        assertion_id: &str,
        until: SystemTime,
    ) -> crate::Result<bool>;
    /// Collect it, once, within [`SSO_HANDOFF_TTL`].
    async fn take_sso_handoff(
        &self,
        code_hash: &str,
        now: SystemTime,
    ) -> crate::Result<Option<SsoHandoff>>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(group_roles: Vec<(&str, UserRole)>) -> ProviderSettings {
        ProviderSettings {
            issuer: "https://idp.example.com".to_string(),
            client_id: "fcp".to_string(),
            saml: None,
            scopes: default_scopes(),
            claims: ClaimMapping::default(),
            group_roles: group_roles
                .into_iter()
                .map(|(g, role)| GroupRole {
                    group: g.to_string(),
                    role,
                })
                .collect(),
            jit: false,
            link_by_email: true,
            email_verified_claim: default_email_verified_claim(),
            required_claims: Vec::new(),
        }
    }

    #[test]
    fn the_most_privileged_mapped_role_wins() {
        let s = settings(vec![
            ("staff", UserRole::EndUser),
            ("it", UserRole::TenantAdmin),
            ("ops", UserRole::ReadOnly),
        ]);
        let groups = |g: &[&str]| g.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            s.role_for(&groups(&["staff", "it"])),
            Some(UserRole::TenantAdmin)
        );
        assert_eq!(
            s.role_for(&groups(&["staff", "ops"])),
            Some(UserRole::ReadOnly)
        );
        assert_eq!(s.role_for(&groups(&["visitors"])), None);
    }

    #[test]
    fn no_provider_grants_super_admin() {
        assert!(settings(vec![("root", UserRole::SuperAdmin)])
            .problems(ProviderKind::Oidc)
            .is_some());
        assert!(settings(vec![]).problems(ProviderKind::Oidc).is_none());
        // A SAML provider is its metadata.
        assert!(settings(vec![]).problems(ProviderKind::Saml).is_some());
    }

    #[test]
    fn a_token_must_carry_every_required_claim() {
        let mut s = settings(vec![]);
        s.required_claims = vec![RequiredClaim {
            claim: "hd".to_string(),
            value: "acme.com".to_string(),
        }];
        let claims = |v: serde_json::Value| v.as_object().unwrap().clone();
        assert!(s.carries_required(&claims(serde_json::json!({"hd": "acme.com"}))));
        assert!(!s.carries_required(&claims(serde_json::json!({"hd": "gmail.com"}))));
        assert!(!s.carries_required(&claims(serde_json::json!({}))));
        s.required_claims[0].claim = "groups".to_string();
        assert!(s.carries_required(&claims(serde_json::json!({"groups": ["x", "acme.com"]}))));
    }

    #[test]
    fn settings_read_with_defaults() {
        let s: ProviderSettings =
            serde_json::from_str(r#"{"issuer":"https://i","client_id":"c"}"#).unwrap();
        assert_eq!(s.claims.username, "preferred_username");
        assert!(s.link_by_email);
        assert!(!s.jit);
        assert_eq!(s.scopes, vec!["profile", "email"]);
    }
}
