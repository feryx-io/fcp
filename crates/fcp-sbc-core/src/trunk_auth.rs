//! Trunk authentication credential provider for SIP digest challenges.
//!
//! Maps realm strings to (username, password) pairs so the `IntegratedUAC`
//! can automatically retry on 401/407 challenges from upstream trunks.

use crate::trunk::TrunkConfig;
use async_trait::async_trait;
use sip_uac::CredentialProvider;
use std::collections::HashMap;

/// Provides trunk authentication credentials for SIP digest challenges.
///
/// Lookup is by realm string. Credentials are indexed at construction time
/// from the configured trunks:
///
/// 1. Explicit `credentials.realm` (highest priority)
/// 2. Each trunk host name (SIP servers commonly use their domain as realm)
pub struct TrunkCredentialProvider {
    /// realm/host -> (username, password)
    credentials: HashMap<String, (String, String)>,
}

impl TrunkCredentialProvider {
    pub fn from_trunks(trunks: &[TrunkConfig]) -> Self {
        let mut credentials = HashMap::new();

        for trunk in trunks {
            if let Some(creds) = &trunk.credentials {
                let pair = (creds.username.clone(), creds.password.clone());

                // Index by explicit realm if configured
                if let Some(realm) = &creds.realm {
                    credentials.insert(realm.clone(), pair.clone());
                }

                // Also index by each trunk host (common SIP pattern:
                // the trunk's domain is used as the digest realm).
                // entry() ensures explicit realm takes precedence on collision.
                for host in &trunk.hosts {
                    credentials
                        .entry(host.clone())
                        .or_insert_with(|| pair.clone());
                }
            }
        }

        Self { credentials }
    }
}

#[async_trait]
impl CredentialProvider for TrunkCredentialProvider {
    async fn credentials(&self, realm: &str) -> Option<(String, String)> {
        self.credentials.get(realm).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trunk::TrunkCredentials;

    fn make_trunk(
        name: &str,
        hosts: Vec<&str>,
        credentials: Option<TrunkCredentials>,
    ) -> TrunkConfig {
        TrunkConfig {
            name: name.into(),
            hosts: hosts.into_iter().map(String::from).collect(),
            credentials,
            ..TrunkConfig::default()
        }
    }

    #[tokio::test]
    async fn empty_trunks_returns_none() {
        let provider = TrunkCredentialProvider::from_trunks(&[]);
        assert!(provider.credentials("any-realm").await.is_none());
    }

    #[tokio::test]
    async fn trunk_without_credentials_skipped() {
        let trunk = make_trunk("no-creds", vec!["sip.example.com"], None);
        let provider = TrunkCredentialProvider::from_trunks(&[trunk]);
        assert!(provider.credentials("sip.example.com").await.is_none());
    }

    #[tokio::test]
    async fn lookup_by_explicit_realm() {
        let trunk = make_trunk(
            "carrier-a",
            vec!["sip.carrier.com"],
            Some(TrunkCredentials {
                username: "user1".into(),
                password: "pass1".into(),
                realm: Some("auth.carrier.com".into()),
            }),
        );
        let provider = TrunkCredentialProvider::from_trunks(&[trunk]);

        let creds = provider.credentials("auth.carrier.com").await.unwrap();
        assert_eq!(creds, ("user1".into(), "pass1".into()));
    }

    #[tokio::test]
    async fn lookup_by_host() {
        let trunk = make_trunk(
            "carrier-b",
            vec!["sip.carrier.com"],
            Some(TrunkCredentials {
                username: "user2".into(),
                password: "pass2".into(),
                realm: None,
            }),
        );
        let provider = TrunkCredentialProvider::from_trunks(&[trunk]);

        let creds = provider.credentials("sip.carrier.com").await.unwrap();
        assert_eq!(creds, ("user2".into(), "pass2".into()));
    }

    #[tokio::test]
    async fn explicit_realm_takes_precedence_over_host() {
        // Trunk A has explicit realm "sip.carrier.com"
        let trunk_a = make_trunk(
            "trunk-a",
            vec!["other.host.com"],
            Some(TrunkCredentials {
                username: "realm-user".into(),
                password: "realm-pass".into(),
                realm: Some("sip.carrier.com".into()),
            }),
        );
        // Trunk B has host "sip.carrier.com" (no explicit realm)
        let trunk_b = make_trunk(
            "trunk-b",
            vec!["sip.carrier.com"],
            Some(TrunkCredentials {
                username: "host-user".into(),
                password: "host-pass".into(),
                realm: None,
            }),
        );
        let provider = TrunkCredentialProvider::from_trunks(&[trunk_a, trunk_b]);

        // Explicit realm insert wins because it was inserted first
        let creds = provider.credentials("sip.carrier.com").await.unwrap();
        assert_eq!(creds, ("realm-user".into(), "realm-pass".into()));
    }

    #[tokio::test]
    async fn multiple_hosts_indexed() {
        let trunk = make_trunk(
            "multi-host",
            vec!["primary.carrier.com", "secondary.carrier.com"],
            Some(TrunkCredentials {
                username: "user".into(),
                password: "pass".into(),
                realm: None,
            }),
        );
        let provider = TrunkCredentialProvider::from_trunks(&[trunk]);

        assert!(provider.credentials("primary.carrier.com").await.is_some());
        assert!(provider
            .credentials("secondary.carrier.com")
            .await
            .is_some());
        assert!(provider.credentials("unknown.com").await.is_none());
    }
}
