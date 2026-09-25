//! A site: a tenant's own stack on the tenant's own network, enrolled to
//! this cloud and kept by `fcp-link` (`docs/HYBRID_CONTROL_PLANE.md` §2,
//! phase 1). The cloud is the system of record for the site's
//! configuration; the site is the system of record for its calls. This
//! module holds what the cloud knows of a site: its record, the one-time
//! token it enrolls with, and what its link last reported.

use crate::{SiteId, TenantId};
use anyhow::Result as AnyhowResult;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::time::{Duration, SystemTime};

/// Where a site is in its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SiteStatus {
    /// Made in the cloud; no link has enrolled yet.
    Pending,
    /// A link holds a certificate for it.
    Enrolled,
    /// Refused: its certificate and secret are no longer accepted.
    Disabled,
}

impl SiteStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Enrolled => "enrolled",
            Self::Disabled => "disabled",
        }
    }
}

/// One node of the site, as its link last reported it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SiteNode {
    pub node_id: String,
    pub node_type: String,
    pub version: String,
    /// Whether the node answered its health check when the link looked.
    pub healthy: bool,
}

/// What a link reports on every heartbeat.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SiteHeartbeat {
    /// The link's own release.
    pub link_version: String,
    /// The link protocol it speaks.
    pub link_protocol: u32,
    /// The site's nodes, as the link found them.
    #[serde(default)]
    pub nodes: Vec<SiteNode>,
}

/// A site as the cloud keeps it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Site {
    pub id: SiteId,
    pub tenant_id: TenantId,
    pub name: String,
    pub status: SiteStatus,
    pub created_at: SystemTime,
    pub updated_at: SystemTime,
    /// When a link enrolled; absent while pending.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enrolled_at: Option<SystemTime>,
    /// The serial of the certificate the link holds, and when it ends.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub certificate_serial: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub certificate_expires_at: Option<SystemTime>,
    /// SHA-256 of the secret the link authenticates its heartbeats with,
    /// issued at enrollment; never the secret.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret_hash: Option<String>,
    /// The last heartbeat, and when it came.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_seen_at: Option<SystemTime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_heartbeat: Option<SiteHeartbeat>,
    /// Whether the cloud currently counts the link as up: set on a
    /// heartbeat, cleared by the sweep when none came for too long, and
    /// what `system.site_link_changed` announces.
    #[serde(default)]
    pub link_up: bool,
    /// When `link_up` last changed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link_changed_at: Option<SystemTime>,
}

impl Site {
    pub fn new(tenant_id: TenantId, name: String) -> Self {
        let now = SystemTime::now();
        Self {
            id: SiteId::new(),
            tenant_id,
            name,
            status: SiteStatus::Pending,
            created_at: now,
            updated_at: now,
            enrolled_at: None,
            certificate_serial: None,
            certificate_expires_at: None,
            secret_hash: None,
            last_seen_at: None,
            last_heartbeat: None,
            link_up: false,
            link_changed_at: None,
        }
    }

    /// Release → node ids, from the last heartbeat, as the cluster page
    /// groups a cluster's own nodes.
    pub fn versions(&self) -> BTreeMap<String, Vec<String>> {
        let mut groups: BTreeMap<String, Vec<String>> = BTreeMap::new();
        if let Some(beat) = &self.last_heartbeat {
            for node in &beat.nodes {
                groups
                    .entry(node.version.clone())
                    .or_default()
                    .push(node.node_id.clone());
            }
        }
        for nodes in groups.values_mut() {
            nodes.sort();
        }
        groups
    }
}

/// How long a token enrolls for.
pub const ENROLLMENT_TOKEN_TTL: Duration = Duration::from_secs(24 * 3600);

/// A one-time enrollment token: the cloud keeps its hash and hands the
/// token to the operator once, in the response that made it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnrollmentToken {
    pub site_id: SiteId,
    /// SHA-256 of the token, hex.
    pub token_hash: String,
    pub created_at: SystemTime,
    pub expires_at: SystemTime,
    /// When a link spent it; a spent token enrolls nobody again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub used_at: Option<SystemTime>,
}

/// The cloud's store of sites and their enrollment tokens.
#[async_trait]
pub trait SiteStore: Send + Sync {
    async fn get_site(&self, id: &SiteId) -> AnyhowResult<Option<Site>>;

    /// A tenant's sites, newest first.
    async fn list_sites(&self, tenant_id: &TenantId) -> AnyhowResult<Vec<Site>>;

    /// Every tenant's sites, newest first: what the cluster page shows a
    /// platform administrator.
    async fn list_all_sites(&self) -> AnyhowResult<Vec<Site>>;

    async fn put_site(&self, site: Site) -> AnyhowResult<()>;

    async fn delete_site(&self, id: &SiteId) -> AnyhowResult<()>;

    async fn put_enrollment_token(&self, token: EnrollmentToken) -> AnyhowResult<()>;

    /// Spend the token with this hash: the token, if it exists, has not
    /// expired and was not spent before — marked spent in the same
    /// statement, so two links presenting it at once get it once.
    async fn consume_enrollment_token(
        &self,
        token_hash: &str,
    ) -> AnyhowResult<Option<EnrollmentToken>>;

    /// A site by the hash of its secret, for a heartbeat.
    async fn get_site_by_secret_hash(&self, secret_hash: &str) -> AnyhowResult<Option<Site>>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_site_is_pending_with_its_link_down() {
        let site = Site::new(TenantId::nil(), "acme-hq".to_string());
        assert_eq!(site.status, SiteStatus::Pending);
        assert!(!site.link_up);
        assert!(site.versions().is_empty());
        let text = serde_json::to_string(&site).unwrap();
        assert!(text.contains("\"pending\""));
        assert!(
            !text.contains("secret_hash"),
            "an unset secret is not written"
        );
    }

    #[test]
    fn a_heartbeat_groups_the_nodes_by_release() {
        let mut site = Site::new(TenantId::nil(), "acme-hq".to_string());
        site.last_heartbeat = Some(SiteHeartbeat {
            link_version: "1".into(),
            link_protocol: 1,
            nodes: vec![
                SiteNode {
                    node_id: "cm-2".into(),
                    node_type: "call_manager".into(),
                    version: "0.25.268".into(),
                    healthy: true,
                },
                SiteNode {
                    node_id: "cm-1".into(),
                    node_type: "call_manager".into(),
                    version: "0.25.268".into(),
                    healthy: true,
                },
                SiteNode {
                    node_id: "conf-1".into(),
                    node_type: "conference".into(),
                    version: "0.25.267".into(),
                    healthy: false,
                },
            ],
        });
        let groups = site.versions();
        assert_eq!(groups["0.25.268"], vec!["cm-1", "cm-2"]);
        assert_eq!(groups["0.25.267"], vec!["conf-1"]);
    }
}
