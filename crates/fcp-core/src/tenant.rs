//! Tenant data structures and types
//!
//! Multi-tenancy is a core design principle of FCP. All resources (users, devices, phone numbers)
//! belong to a tenant.

use crate::identifiers::TenantId;
use serde::{Deserialize, Serialize};
use std::time::SystemTime;

/// Returns `true` when `domain` is a numeric IP literal, `localhost`, or any
/// other host form that should not be treated as a tenant SIP domain.
///
/// IP literals and `localhost` are typically emitted by devices that have not
/// been provisioned with their tenant's SIP domain (legacy phones, softphones
/// pointed at the server IP, test rigs). Matching these against
/// `tenants.sip_domains` will always fail, so the resolver should skip that
/// lookup entirely and fall through to username- and default-based resolution.
/// Real hostnames stay on the strict "must match a tenant" path so typos and
/// lookup misses still fail closed.
///
/// A bracketed IPv6 form such as `[::1]` (used in SIP URIs) is also
/// recognised — the brackets are stripped before parsing.
pub fn is_host_literal(domain: &str) -> bool {
    let trimmed = domain.trim().trim_end_matches('.');
    if trimmed.is_empty() {
        return true;
    }
    if trimmed.eq_ignore_ascii_case("localhost") {
        return true;
    }
    let inner = trimmed
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or(trimmed);
    inner.parse::<std::net::IpAddr>().is_ok()
}

/// Account status for a tenant
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum TenantStatus {
    /// Tenant is active and can use the system
    #[default]
    Active,
    /// Tenant is suspended (cannot make/receive calls)
    Suspended,
    /// Tenant is in trial period
    Trial,
    /// Tenant account is closed
    Closed,
}

impl std::fmt::Display for TenantStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Active => write!(f, "Active"),
            Self::Suspended => write!(f, "Suspended"),
            Self::Trial => write!(f, "Trial"),
            Self::Closed => write!(f, "Closed"),
        }
    }
}

/// A tenant in the FCP system
///
/// Tenants are the top-level organizational unit. All users, devices, phone numbers,
/// and other resources belong to a tenant.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tenant {
    /// Unique tenant identifier
    pub id: TenantId,

    /// Tenant name (e.g., "Acme Corporation")
    pub name: String,

    /// Optional description or notes
    pub description: Option<String>,

    /// Account status
    pub status: TenantStatus,

    /// Contact email for the tenant
    pub contact_email: Option<String>,

    /// Contact phone number
    pub contact_phone: Option<String>,

    /// Maximum number of users allowed
    pub max_users: Option<u32>,

    /// Maximum number of phone numbers allowed
    pub max_phone_numbers: Option<u32>,

    /// Maximum number of concurrent calls allowed
    pub max_concurrent_calls: Option<u32>,

    /// Billing identifier (for external billing system)
    pub billing_id: Option<String>,

    /// When the tenant was created
    pub created_at: SystemTime,

    /// When the tenant was last updated
    pub updated_at: SystemTime,

    /// Optional trial expiration date
    pub trial_expires_at: Option<SystemTime>,

    /// Additional metadata (JSON)
    pub metadata: Option<String>,

    /// SIP domains claimed by this tenant.
    ///
    /// Each entry is a hostname (e.g. `acme.fcp.net`, `sip.acme.com`) that
    /// SIP requests may use in their From/Request-URI domain. The tenant
    /// resolver looks up incoming requests against this list to determine
    /// which tenant they belong to. A single tenant can own multiple
    /// domains — for example one for user phones and one for carrier
    /// trunks.
    ///
    /// Entries are stored as lowercase, trimmed hostnames. Empty or
    /// whitespace-only strings are not allowed at the API boundary.
    /// Numeric IP addresses and `localhost` should NOT be used here —
    /// they are handled as a compatibility fallback at the resolver
    /// level.
    #[serde(default)]
    pub sip_domains: Vec<String>,

    /// Whether this tenant's calls may leave on the platform's trunks —
    /// the ones a node reads from its own `[[trunks]]` — when the tenant
    /// has no trunk of its own that can take them
    /// (`docs/TENANT_TRUNKS.md` §5). True unless a tenant's calls must
    /// never leave on a shared carrier.
    #[serde(default = "default_platform_trunk_fallback")]
    pub platform_trunk_fallback: bool,

    /// How the tenant dials and keeps time (`docs/ENTERPRISE_CALLING.md`
    /// §3.7): its country, its timezone, and its own word on number ranges.
    #[serde(default)]
    pub calling: TenantCalling,

    /// Email domains whose people are this tenant's
    /// (`docs/IDENTITY_AND_DEVICES.md` phase 2): `jane@acme.com` signing in
    /// is sent to the tenant claiming `acme.com`. Lowercase, unique across
    /// tenants, and set only by a platform admin: a domain decides where a
    /// person's sign-in goes.
    #[serde(default)]
    pub login_domains: Vec<String>,

    /// How the tenant's people may sign in.
    #[serde(default)]
    pub sign_in: TenantSignIn,

    /// What happens to a deleted user (`docs/USERS.md` §4).
    #[serde(default)]
    pub offboarding: TenantOffboarding,

    /// How the tenant's phones fetch their configuration
    /// (`docs/PROVISIONING.md` §3).
    #[serde(default)]
    pub provisioning: crate::provisioning::TenantProvisioning,

    /// What the tenant's people see of each other (`docs/SOFTPHONE.md`
    /// §2.7).
    #[serde(default)]
    pub directory: TenantDirectory,
}

/// What a tenant's people see of each other through `/me/directory`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TenantDirectory {
    /// Whether its users may list the tenant's users, extensions and
    /// presence; on unless a tenant turns it off.
    #[serde(default = "default_visible_to_users")]
    pub visible_to_users: bool,
}

fn default_visible_to_users() -> bool {
    true
}

impl Default for TenantDirectory {
    fn default() -> Self {
        Self {
            visible_to_users: true,
        }
    }
}

/// What happens to a tenant's deleted user (`docs/USERS.md` §4): they are
/// suspended at once and purged `grace_days` later.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TenantOffboarding {
    /// Days between a delete and the purge, so a mistake can be undone and
    /// a mailbox collected. 0 purges at once.
    #[serde(default = "default_grace_days")]
    pub grace_days: u32,
    /// The username a purged user's meetings move to; without one they
    /// are left without an owner.
    #[serde(default)]
    pub meetings_owner: Option<String>,
}

/// The grace period when a tenant names none.
pub const DEFAULT_GRACE_DAYS: u32 = 30;
/// The longest grace period a tenant may set.
pub const MAX_GRACE_DAYS: u32 = 3650;

fn default_grace_days() -> u32 {
    DEFAULT_GRACE_DAYS
}

impl Default for TenantOffboarding {
    fn default() -> Self {
        Self {
            grace_days: DEFAULT_GRACE_DAYS,
            meetings_owner: None,
        }
    }
}

impl TenantOffboarding {
    /// The grace period.
    pub fn grace(&self) -> std::time::Duration {
        std::time::Duration::from_secs(u64::from(self.grace_days) * 86_400)
    }
}

/// How a tenant's people may sign in (`docs/IDENTITY_AND_DEVICES.md` §2).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TenantSignIn {
    /// Whether a password signs in.
    #[serde(default)]
    pub password_login: PasswordLogin,
    /// Who must add a one-time code to a password (phase 2b).
    #[serde(default)]
    pub require_totp: RequireTotp,
    /// The tenant's users come from its SCIM provisioner (`docs/SCIM.md`):
    /// single sign-on makes nobody just in time and signs in only users
    /// SCIM made.
    #[serde(default)]
    pub scim: bool,
    /// How many days an app session (the softphone's) lasts after its last
    /// refresh (`docs/SOFTPHONE.md` §2.3); absent, 90.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_session_days: Option<u32>,
}

impl TenantSignIn {
    /// How long an app session lasts after its last refresh.
    pub fn app_session_ttl(&self) -> std::time::Duration {
        let days = self
            .app_session_days
            .unwrap_or(crate::session::DEFAULT_APP_SESSION_DAYS)
            .clamp(1, 365);
        std::time::Duration::from_secs(u64::from(days) * 86_400)
    }
}

/// Who must add a one-time code to a password sign-in.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum RequireTotp {
    /// Nobody; a user may still choose to.
    #[default]
    None,
    /// Administrators: every role but end user.
    Admins,
    /// Everyone who signs in with a password.
    Everyone,
}

impl RequireTotp {
    /// Whether a user of `role` must.
    pub fn applies(self, role: &crate::UserRole) -> bool {
        match self {
            Self::None => false,
            Self::Admins => *role != crate::UserRole::EndUser,
            Self::Everyone => true,
        }
    }
}

/// Who may sign in with a password.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum PasswordLogin {
    /// Everyone with a password.
    #[default]
    Allowed,
    /// Administrators only; everyone else signs in through the tenant's
    /// identity provider.
    AdminsOnly,
    /// Nobody; only a platform super-admin, whatever the tenant says
    /// (break-glass), and the platform's static token.
    Disabled,
}

impl PasswordLogin {
    /// Whether a user of `role` may sign in with a password.
    pub fn permits(self, role: &crate::UserRole) -> bool {
        use crate::UserRole;
        match self {
            Self::Allowed => true,
            // A super admin is the platform's, never shut out by a tenant.
            _ if *role == UserRole::SuperAdmin => true,
            Self::AdminsOnly => *role != UserRole::EndUser,
            Self::Disabled => false,
        }
    }
}

/// A login domain as stored: trimmed and lowercased; an error for one
/// that is not a domain.
pub fn normalize_login_domain(raw: &str) -> Result<String, String> {
    let domain = raw.trim().trim_start_matches('@').to_ascii_lowercase();
    let valid = domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && domain.len() <= 253
        && domain.parse::<std::net::IpAddr>().is_err()
        && domain.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        });
    if valid {
        Ok(domain)
    } else {
        Err(format!("`{raw}` is not an email domain"))
    }
}

/// The domain of a login that is an email address.
pub fn login_domain(login: &str) -> Option<String> {
    let (local, domain) = login.trim().rsplit_once('@')?;
    if local.is_empty() {
        return None;
    }
    normalize_login_domain(domain).ok()
}

/// A tenant's calling defaults.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TenantCalling {
    /// ISO 3166-1 alpha-2 (`US`, `GB`, …), for classifying the numbers the
    /// tenant's users dial; unset reads numbers as North American.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub country: Option<String>,
    /// IANA timezone, the default for the tenant's new schedules.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timezone: Option<String>,
    /// The tenant's own classification of number ranges, over the
    /// built-in table.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub destination_overrides: Vec<crate::class_of_service::DestinationOverride>,
    /// How long a hoteling guest stays logged in on a host phone
    /// (`docs/ENTERPRISE_CALLING.md` §3.8); unset is
    /// [`crate::hoteling::DEFAULT_SESSION_HOURS`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hoteling_session_hours: Option<u32>,
}

impl TenantCalling {
    /// Refuse a country the table does not know, a timezone that is not
    /// IANA's, an override that is not a prefix.
    pub fn validate(&self) -> Result<(), String> {
        if let Some(country) = &self.country {
            let known = crate::class_of_service::known_countries();
            if !known.iter().any(|k| k.eq_ignore_ascii_case(country)) {
                return Err(format!(
                    "country `{country}` is not one the platform classifies numbers for ({})",
                    known.join(", ")
                ));
            }
        }
        if let Some(tz) = &self.timezone {
            if tz.parse::<chrono_tz::Tz>().is_err() {
                return Err(format!("timezone `{tz}` is not an IANA timezone"));
            }
        }
        for o in &self.destination_overrides {
            o.validate()?;
        }
        if let Some(hours) = self.hoteling_session_hours {
            if !(1..=crate::hoteling::MAX_SESSION_HOURS).contains(&hours) {
                return Err(format!(
                    "hoteling_session_hours must be 1 to {}",
                    crate::hoteling::MAX_SESSION_HOURS
                ));
            }
        }
        Ok(())
    }

    /// How long a hoteling session lasts for this tenant.
    pub fn hoteling_session(&self) -> std::time::Duration {
        let hours = self
            .hoteling_session_hours
            .unwrap_or(crate::hoteling::DEFAULT_SESSION_HOURS);
        std::time::Duration::from_secs(u64::from(hours) * 3600)
    }
}

fn default_platform_trunk_fallback() -> bool {
    true
}

impl Tenant {
    /// Create a new tenant
    pub fn new(name: String) -> Self {
        let now = SystemTime::now();
        Self {
            id: TenantId::new(),
            name,
            description: None,
            status: TenantStatus::Active,
            contact_email: None,
            contact_phone: None,
            max_users: None,
            max_phone_numbers: None,
            max_concurrent_calls: None,
            billing_id: None,
            created_at: now,
            updated_at: now,
            trial_expires_at: None,
            metadata: None,
            sip_domains: Vec::new(),
            platform_trunk_fallback: true,
            calling: TenantCalling::default(),
            login_domains: Vec::new(),
            sign_in: TenantSignIn::default(),
            offboarding: TenantOffboarding::default(),
            provisioning: Default::default(),
            directory: TenantDirectory::default(),
        }
    }

    /// Returns true if this tenant claims the given SIP domain.
    ///
    /// The comparison is case-insensitive and ignores surrounding
    /// whitespace, matching the normalization performed at write time.
    pub fn claims_sip_domain(&self, domain: &str) -> bool {
        let needle = domain.trim().to_ascii_lowercase();
        if needle.is_empty() {
            return false;
        }
        self.sip_domains
            .iter()
            .any(|d| d.trim().eq_ignore_ascii_case(&needle))
    }

    /// Check if tenant is active
    pub fn is_active(&self) -> bool {
        self.status == TenantStatus::Active || self.status == TenantStatus::Trial
    }

    /// Check if tenant is in trial
    pub fn is_trial(&self) -> bool {
        self.status == TenantStatus::Trial
    }

    /// Check if trial has expired
    pub fn is_trial_expired(&self) -> bool {
        if let Some(expires_at) = self.trial_expires_at {
            SystemTime::now() > expires_at
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tenant_creation() {
        let tenant = Tenant::new("Test Corp".to_string());
        assert_eq!(tenant.name, "Test Corp");
        assert_eq!(tenant.status, TenantStatus::Active);
        assert!(tenant.is_active());
        assert!(!tenant.is_trial());
    }

    #[test]
    fn test_tenant_status() {
        let mut tenant = Tenant::new("Test Corp".to_string());

        tenant.status = TenantStatus::Suspended;
        assert!(!tenant.is_active());

        tenant.status = TenantStatus::Trial;
        assert!(tenant.is_active());
        assert!(tenant.is_trial());
    }

    #[test]
    fn test_claims_sip_domain() {
        let mut t = Tenant::new("Acme".to_string());
        t.sip_domains = vec!["acme.fcp.net".to_string(), "sip.acme.com".to_string()];

        assert!(t.claims_sip_domain("acme.fcp.net"));
        assert!(t.claims_sip_domain("sip.acme.com"));
        // Case-insensitive.
        assert!(t.claims_sip_domain("ACME.FCP.NET"));
        // Trims whitespace on the input.
        assert!(t.claims_sip_domain("  acme.fcp.net  "));
        // Not a claimed domain.
        assert!(!t.claims_sip_domain("coke.fcp.net"));
        // Empty input never matches.
        assert!(!t.claims_sip_domain(""));
        assert!(!t.claims_sip_domain("   "));
    }

    #[test]
    fn test_trial_expiration() {
        let mut tenant = Tenant::new("Test Corp".to_string());
        tenant.status = TenantStatus::Trial;

        // No expiration set
        assert!(!tenant.is_trial_expired());

        // Set expiration in the past
        tenant.trial_expires_at = Some(SystemTime::UNIX_EPOCH);
        assert!(tenant.is_trial_expired());

        // Set expiration in the future
        tenant.trial_expires_at = Some(SystemTime::now() + std::time::Duration::from_secs(86400));
        assert!(!tenant.is_trial_expired());
    }
}

#[cfg(test)]
mod sign_in_tests {
    use super::*;
    use crate::UserRole;

    #[test]
    fn a_login_domain_is_a_domain() {
        assert_eq!(normalize_login_domain(" @ACME.com ").unwrap(), "acme.com");
        assert_eq!(
            normalize_login_domain("mail.acme.co.uk").unwrap(),
            "mail.acme.co.uk"
        );
        for bad in [
            "acme",
            "192.168.1.1",
            ".acme.com",
            "acme..com",
            "-acme.com",
            "ac me.com",
            "",
        ] {
            assert!(normalize_login_domain(bad).is_err(), "{bad}");
        }
        assert_eq!(login_domain("Jane@Acme.com").as_deref(), Some("acme.com"));
        assert_eq!(login_domain("jane"), None);
        assert_eq!(login_domain("@acme.com"), None);
    }

    #[test]
    fn a_password_policy_never_shuts_out_a_super_admin() {
        use PasswordLogin::*;
        assert!(Allowed.permits(&UserRole::EndUser));
        assert!(!AdminsOnly.permits(&UserRole::EndUser));
        assert!(AdminsOnly.permits(&UserRole::TenantAdmin));
        assert!(!Disabled.permits(&UserRole::TenantAdmin));
        assert!(Disabled.permits(&UserRole::SuperAdmin));
    }
}
