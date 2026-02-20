//! Presence federation for inter-domain presence subscriptions.
//!
//! When a local user subscribes to a presentity on a remote domain,
//! the `FederationManager` creates an outbound SUBSCRIBE to the remote
//! server, caches the resulting NOTIFY state, and relays it to local watchers.

use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::{debug, info, warn};

use crate::types::PresenceState;

/// Federation configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FederationConfig {
    /// Whether federation is enabled.
    #[serde(default)]
    pub enabled: bool,

    /// Domains considered "local" (subscriptions to these are handled locally).
    pub local_domains: Vec<String>,

    /// Explicit allow-list of remote domains. Empty = allow all.
    #[serde(default)]
    pub allowed_domains: Vec<String>,

    /// Explicit deny-list of remote domains. Checked before allow.
    #[serde(default)]
    pub denied_domains: Vec<String>,

    /// Default SUBSCRIBE expiry for remote subscriptions (seconds).
    #[serde(default = "default_expiry")]
    pub default_expiry_secs: u32,

    /// Maximum number of simultaneous remote subscriptions.
    #[serde(default = "default_max_remote")]
    pub max_remote_subscriptions: usize,
}

fn default_expiry() -> u32 {
    3600
}

fn default_max_remote() -> usize {
    1000
}

impl Default for FederationConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            local_domains: Vec::new(),
            allowed_domains: Vec::new(),
            denied_domains: Vec::new(),
            default_expiry_secs: default_expiry(),
            max_remote_subscriptions: default_max_remote(),
        }
    }
}

/// Lifecycle state of a remote subscription.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SubscriptionLifecycle {
    /// SUBSCRIBE sent, waiting for 200 OK + NOTIFY.
    Pending,
    /// Active subscription with periodic refresh.
    Active,
    /// Subscription has been terminated.
    Terminated,
}

/// A subscription to a presentity on a remote domain.
#[derive(Debug, Clone)]
pub struct RemoteSubscription {
    /// Local watcher who requested this.
    pub local_watcher_aor: String,
    /// Remote presentity AOR (e.g., bob@remote.com).
    pub remote_aor: String,
    /// Domain of the remote server.
    pub remote_domain: String,
    /// Current lifecycle state.
    pub subscription_state: SubscriptionLifecycle,
    /// When this subscription expires.
    pub expires_at: Instant,
    /// Last NOTIFY presence state received from remote.
    pub last_notify_state: Option<PresenceState>,
}

/// Manages federated (inter-domain) presence subscriptions.
///
/// When a local watcher subscribes to a remote AOR, the FederationManager
/// tracks the remote subscription and caches the last known state.
/// The actual SIP SUBSCRIBE/NOTIFY signaling is delegated to the SIP layer.
pub struct FederationManager {
    config: FederationConfig,
    /// Remote AOR → RemoteSubscription
    remote_subscriptions: Arc<DashMap<String, RemoteSubscription>>,
    /// Cache of federated presence state: remote AOR → PresenceState
    federated_cache: Arc<DashMap<String, PresenceState>>,
}

impl FederationManager {
    /// Create a new federation manager.
    pub fn new(config: FederationConfig) -> Self {
        Self {
            config,
            remote_subscriptions: Arc::new(DashMap::new()),
            federated_cache: Arc::new(DashMap::new()),
        }
    }

    /// Check if a domain is local (handled by this node).
    pub fn is_local_domain(&self, domain: &str) -> bool {
        self.config
            .local_domains
            .iter()
            .any(|d| d.eq_ignore_ascii_case(domain))
    }

    /// Check if a remote domain is allowed for federation.
    pub fn is_domain_allowed(&self, domain: &str) -> bool {
        // Deny list takes precedence
        if self
            .config
            .denied_domains
            .iter()
            .any(|d| d.eq_ignore_ascii_case(domain))
        {
            return false;
        }

        // If allow list is empty, allow everything not denied
        if self.config.allowed_domains.is_empty() {
            return true;
        }

        // Must be in allow list
        self.config
            .allowed_domains
            .iter()
            .any(|d| d.eq_ignore_ascii_case(domain))
    }

    /// Extract the domain part from an AOR (user@domain).
    pub fn extract_domain(aor: &str) -> Option<&str> {
        aor.split('@').nth(1)
    }

    /// Check if an AOR is on a remote (federated) domain.
    pub fn is_remote_aor(&self, aor: &str) -> bool {
        if let Some(domain) = Self::extract_domain(aor) {
            !self.is_local_domain(domain)
        } else {
            false
        }
    }

    /// Request a subscription to a remote presentity.
    ///
    /// Returns `Ok(true)` if the subscription was created (or already exists),
    /// `Ok(false)` if the domain is denied or limits reached,
    /// `Err` on other failures.
    pub fn subscribe_remote(
        &self,
        local_watcher_aor: &str,
        remote_aor: &str,
    ) -> anyhow::Result<bool> {
        let domain = Self::extract_domain(remote_aor)
            .ok_or_else(|| anyhow::anyhow!("Invalid AOR: {}", remote_aor))?;

        if !self.is_domain_allowed(domain) {
            warn!(
                remote_aor = %remote_aor,
                domain = %domain,
                "Federation denied: domain not allowed"
            );
            return Ok(false);
        }

        if self.remote_subscriptions.len() >= self.config.max_remote_subscriptions {
            warn!(
                limit = self.config.max_remote_subscriptions,
                "Federation subscription limit reached"
            );
            return Ok(false);
        }

        // Check if already subscribed
        if self.remote_subscriptions.contains_key(remote_aor) {
            debug!(remote_aor = %remote_aor, "Already subscribed to remote presentity");
            return Ok(true);
        }

        let sub = RemoteSubscription {
            local_watcher_aor: local_watcher_aor.to_string(),
            remote_aor: remote_aor.to_string(),
            remote_domain: domain.to_string(),
            subscription_state: SubscriptionLifecycle::Pending,
            expires_at: Instant::now() + Duration::from_secs(self.config.default_expiry_secs as u64),
            last_notify_state: None,
        };

        self.remote_subscriptions
            .insert(remote_aor.to_string(), sub);

        info!(
            remote_aor = %remote_aor,
            watcher = %local_watcher_aor,
            domain = %domain,
            "Created federated subscription (pending outbound SUBSCRIBE)"
        );

        Ok(true)
    }

    /// Handle a NOTIFY received from a remote server for a federated subscription.
    ///
    /// Updates the cached state and marks the subscription as active.
    pub fn handle_remote_notify(&self, remote_aor: &str, state: PresenceState) {
        // Update the subscription lifecycle
        if let Some(mut sub) = self.remote_subscriptions.get_mut(remote_aor) {
            sub.subscription_state = SubscriptionLifecycle::Active;
            sub.last_notify_state = Some(state);
            debug!(
                remote_aor = %remote_aor,
                state = %state,
                "Updated federated presence from remote NOTIFY"
            );
        }

        // Update the cache
        self.federated_cache.insert(remote_aor.to_string(), state);
    }

    /// Get the cached presence state for a remote AOR.
    pub fn get_federated_presence(&self, remote_aor: &str) -> Option<PresenceState> {
        self.federated_cache
            .get(remote_aor)
            .map(|entry| *entry.value())
    }

    /// Remove a remote subscription (e.g., when all local watchers unsubscribe).
    pub fn unsubscribe_remote(&self, remote_aor: &str) -> bool {
        let removed = self.remote_subscriptions.remove(remote_aor).is_some();
        if removed {
            self.federated_cache.remove(remote_aor);
            info!(remote_aor = %remote_aor, "Removed federated subscription");
        }
        removed
    }

    /// Get subscriptions that need refreshing (within 10% of expiry).
    pub fn subscriptions_needing_refresh(&self) -> Vec<RemoteSubscription> {
        let now = Instant::now();
        self.remote_subscriptions
            .iter()
            .filter(|entry| {
                let sub = entry.value();
                sub.subscription_state == SubscriptionLifecycle::Active
                    && sub.expires_at.saturating_duration_since(now)
                        < Duration::from_secs(self.config.default_expiry_secs as u64 / 10)
            })
            .map(|entry| entry.value().clone())
            .collect()
    }

    /// Get all expired subscriptions and remove them.
    pub fn cleanup_expired(&self) -> Vec<RemoteSubscription> {
        let now = Instant::now();
        let mut expired = Vec::new();

        self.remote_subscriptions.retain(|_key, sub| {
            if sub.expires_at <= now {
                expired.push(sub.clone());
                false
            } else {
                true
            }
        });

        for sub in &expired {
            self.federated_cache.remove(&sub.remote_aor);
        }

        if !expired.is_empty() {
            info!(count = expired.len(), "Cleaned up expired federated subscriptions");
        }

        expired
    }

    /// Get the number of active remote subscriptions.
    pub fn remote_subscription_count(&self) -> usize {
        self.remote_subscriptions.len()
    }

    /// Get all remote subscriptions (for diagnostics/API).
    pub fn get_all_remote_subscriptions(&self) -> Vec<RemoteSubscription> {
        self.remote_subscriptions
            .iter()
            .map(|entry| entry.value().clone())
            .collect()
    }

    /// Refresh a remote subscription's expiry time.
    pub fn refresh_remote_subscription(&self, remote_aor: &str, new_expiry_secs: u32) {
        if let Some(mut sub) = self.remote_subscriptions.get_mut(remote_aor) {
            sub.expires_at = Instant::now() + Duration::from_secs(new_expiry_secs as u64);
            debug!(
                remote_aor = %remote_aor,
                new_expiry_secs = new_expiry_secs,
                "Refreshed federated subscription expiry"
            );
        }
    }

    /// Mark a remote subscription as terminated.
    pub fn terminate_remote_subscription(&self, remote_aor: &str) {
        if let Some(mut sub) = self.remote_subscriptions.get_mut(remote_aor) {
            sub.subscription_state = SubscriptionLifecycle::Terminated;
        }
    }

    /// Get the federation config.
    pub fn config(&self) -> &FederationConfig {
        &self.config
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> FederationConfig {
        FederationConfig {
            enabled: true,
            local_domains: vec!["example.com".to_string(), "fcp.local".to_string()],
            allowed_domains: vec!["partner.com".to_string()],
            denied_domains: vec!["blocked.com".to_string()],
            default_expiry_secs: 3600,
            max_remote_subscriptions: 100,
        }
    }

    #[test]
    fn test_local_domain_detection() {
        let fm = FederationManager::new(test_config());
        assert!(fm.is_local_domain("example.com"));
        assert!(fm.is_local_domain("EXAMPLE.COM")); // case insensitive
        assert!(fm.is_local_domain("fcp.local"));
        assert!(!fm.is_local_domain("remote.com"));
    }

    #[test]
    fn test_domain_allow_deny() {
        let fm = FederationManager::new(test_config());
        assert!(fm.is_domain_allowed("partner.com"));
        assert!(!fm.is_domain_allowed("blocked.com")); // denied takes precedence
        assert!(!fm.is_domain_allowed("unknown.com")); // not in allow list
    }

    #[test]
    fn test_domain_allow_deny_open_policy() {
        let config = FederationConfig {
            enabled: true,
            local_domains: vec!["example.com".to_string()],
            allowed_domains: vec![], // empty = allow all
            denied_domains: vec!["blocked.com".to_string()],
            ..Default::default()
        };
        let fm = FederationManager::new(config);
        assert!(fm.is_domain_allowed("anyone.com"));
        assert!(!fm.is_domain_allowed("blocked.com"));
    }

    #[test]
    fn test_remote_aor_detection() {
        let fm = FederationManager::new(test_config());
        assert!(!fm.is_remote_aor("alice@example.com")); // local
        assert!(fm.is_remote_aor("bob@partner.com")); // remote
        assert!(fm.is_remote_aor("carol@unknown.com")); // remote (even if not allowed)
        assert!(!fm.is_remote_aor("invalid-no-at")); // not a valid AOR
    }

    #[test]
    fn test_subscribe_remote() {
        let fm = FederationManager::new(test_config());

        // Allowed domain
        assert!(fm
            .subscribe_remote("alice@example.com", "bob@partner.com")
            .unwrap());
        assert_eq!(fm.remote_subscription_count(), 1);

        // Duplicate → returns true without adding
        assert!(fm
            .subscribe_remote("alice@example.com", "bob@partner.com")
            .unwrap());
        assert_eq!(fm.remote_subscription_count(), 1);

        // Denied domain
        assert!(!fm
            .subscribe_remote("alice@example.com", "eve@blocked.com")
            .unwrap());

        // Not in allow list
        assert!(!fm
            .subscribe_remote("alice@example.com", "charlie@unknown.com")
            .unwrap());
    }

    #[test]
    fn test_subscribe_limit() {
        let config = FederationConfig {
            enabled: true,
            local_domains: vec!["example.com".to_string()],
            allowed_domains: vec![], // allow all
            denied_domains: vec![],
            max_remote_subscriptions: 2,
            ..Default::default()
        };
        let fm = FederationManager::new(config);

        assert!(fm.subscribe_remote("a@example.com", "b@x.com").unwrap());
        assert!(fm.subscribe_remote("a@example.com", "c@y.com").unwrap());
        assert!(!fm.subscribe_remote("a@example.com", "d@z.com").unwrap()); // limit reached
    }

    #[test]
    fn test_handle_remote_notify() {
        let fm = FederationManager::new(test_config());
        fm.subscribe_remote("alice@example.com", "bob@partner.com")
            .unwrap();

        fm.handle_remote_notify("bob@partner.com", PresenceState::Open);

        assert_eq!(
            fm.get_federated_presence("bob@partner.com"),
            Some(PresenceState::Open)
        );

        let sub = fm.remote_subscriptions.get("bob@partner.com").unwrap();
        assert_eq!(sub.subscription_state, SubscriptionLifecycle::Active);
        assert_eq!(sub.last_notify_state, Some(PresenceState::Open));
    }

    #[test]
    fn test_unsubscribe_remote() {
        let fm = FederationManager::new(test_config());
        fm.subscribe_remote("alice@example.com", "bob@partner.com")
            .unwrap();
        fm.handle_remote_notify("bob@partner.com", PresenceState::Busy);

        assert!(fm.unsubscribe_remote("bob@partner.com"));
        assert_eq!(fm.remote_subscription_count(), 0);
        assert_eq!(fm.get_federated_presence("bob@partner.com"), None);
        assert!(!fm.unsubscribe_remote("bob@partner.com")); // already gone
    }

    #[test]
    fn test_extract_domain() {
        assert_eq!(FederationManager::extract_domain("alice@example.com"), Some("example.com"));
        assert_eq!(FederationManager::extract_domain("noat"), None);
    }
}
