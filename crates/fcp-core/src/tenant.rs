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
}

/// A tenant's calling defaults.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
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
