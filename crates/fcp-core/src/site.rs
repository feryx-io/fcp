//! A site: a tenant's own stack on the tenant's own network, enrolled to
//! this cloud and kept by `fcp-link` (`docs/HYBRID_CONTROL_PLANE.md` §2,
//! phase 1). The cloud is the system of record for the site's
//! configuration; the site is the system of record for its calls. This
//! module holds what the cloud knows of a site: its record, the one-time
//! token it enrolls with, and what its link last reported.

use crate::{SiteId, SiteRequestId, TenantId, TrunkId};
use anyhow::Result as AnyhowResult;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::time::{Duration, SystemTime};

/// Where a site is in its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
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
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SiteNode {
    pub node_id: String,
    pub node_type: String,
    pub version: String,
    /// Whether the node answered its health check when the link looked.
    pub healthy: bool,
}

/// Where the site's copy of the tenant's configuration stands
/// (`docs/HYBRID_CONTROL_PLANE.md` §3.2, phase 3): the journal cursor the
/// link has applied through, when it last applied anything, and what went
/// wrong if it is stuck.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SiteSync {
    /// The journal sequence applied through; 0 before the first snapshot.
    pub cursor: i64,
    /// When the link last applied a snapshot or a change, epoch seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub applied_at: Option<u64>,
    /// The last failure to fetch or apply, until the next success.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
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
    /// Where the site's configuration stands; absent from a link that
    /// keeps no site database.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sync: Option<SiteSync>,
    /// Who is registered at the site, at how many contacts, since when
    /// (`docs/HYBRID_CONTROL_PLANE.md` §3.3); absent from a link with no
    /// site Redis. A summary, not the rows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registrations: Option<SiteRegistrations>,
    /// Every service at the site, as its beacon says it
    /// (`docs/SERVICES.md`); empty from an older link or one with no site
    /// Redis.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub services: Vec<crate::service_health::ServiceRecord>,
    /// The calls parked at the site now, from its Redis
    /// (`docs/PARKING.md` phase 4), for the cloud's board; empty from an
    /// older link.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parked: Vec<crate::storage::ClusterParkedCall>,
    /// The site's own backups (`docs/SCHEDULED_BACKUPS.md` B5), from its
    /// database; empty from an older link or a site with none.
    #[serde(default)]
    pub backups: Vec<crate::backup::BackupStatus>,
}

/// The site's registrations as its link last saw them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SiteRegistrations {
    /// Every registered address of record, up to [`SITE_REGISTRATIONS_CAP`].
    #[serde(default)]
    pub users: Vec<SiteRegistration>,
    /// Addresses of record registered, counted before the cap.
    pub total_users: u32,
    /// Contacts registered, counted before the cap.
    pub total_contacts: u32,
}

/// How many addresses of record a heartbeat lists, at most.
pub const SITE_REGISTRATIONS_CAP: usize = 1000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SiteRegistration {
    pub aor: String,
    pub contacts: u32,
    /// The earliest registration among the contacts, seconds since the
    /// epoch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<u64>,
}

/// What the cloud asks a site to do with a file it holds
/// (`docs/HYBRID_CONTROL_PLANE.md` §3.3): a site opens no port, so the
/// cloud queues the request and the link collects it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SiteRequestKind {
    /// Send the file's bytes up: a download, a playback, an export.
    FetchFile { class: String, path: String },
    /// Remove the file: retention, an erasure, a deletion in the cloud.
    DeleteFile { class: String, path: String },
}

/// How a site's request ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SiteRequestOutcome {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SiteRequest {
    pub id: SiteRequestId,
    pub site_id: SiteId,
    pub tenant_id: TenantId,
    pub kind: SiteRequestKind,
    pub created_at: SystemTime,
    /// When the link collected it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub taken_at: Option<SystemTime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub done_at: Option<SystemTime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<SiteRequestOutcome>,
}

/// A batch of records a site ships up its link
/// (`docs/HYBRID_CONTROL_PLANE.md` §3.3): rows as the site has them, and
/// the deletions its log recorded since the last batch.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ShippedRecords {
    #[serde(default)]
    pub puts: Vec<ShippedRecord>,
    #[serde(default)]
    pub deletions: Vec<ShippedDeletion>,
}

/// One row, named by its table: `call_recordings`, `conference_recordings`,
/// `voicemail_mailboxes` or `voicemail_messages`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShippedRecord {
    pub table: String,
    pub id: String,
    pub object: serde_json::Value,
}

/// One deletion, as the site's deletion log has it: the class
/// (`call_recording`, `conference_recording`, `voicemail`) and the row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShippedDeletion {
    pub class: String,
    pub item: String,
}

/// What the cloud made of a batch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ShippedAnswer {
    /// Rows written or deleted.
    pub applied: usize,
    /// Rows the cloud already had, or would not take (another tenant's,
    /// a table it does not ship).
    pub skipped: usize,
    /// Records the processor refused, with the reason, by position.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rejected: Vec<(usize, String)>,
}

/// A site's events, as one of its nodes published them, sent up its link.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShippedEvents {
    pub node_id: String,
    pub events: Vec<serde_json::Value>,
}

impl SiteRequest {
    pub fn new(site_id: SiteId, tenant_id: TenantId, kind: SiteRequestKind) -> Self {
        Self {
            id: SiteRequestId::new(),
            site_id,
            tenant_id,
            kind,
            created_at: SystemTime::now(),
            taken_at: None,
            done_at: None,
            outcome: None,
        }
    }

    pub fn is_done(&self) -> bool {
        self.done_at.is_some()
    }
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
    /// The site's link trunk, made at enrollment when the cloud names a
    /// SIP address for sites (`docs/HYBRID_CONTROL_PLANE.md` §3.6).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trunk_id: Option<TrunkId>,
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
            trunk_id: None,
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

    /// Write a request for a site, new or with its outcome
    /// (`docs/HYBRID_CONTROL_PLANE.md` §3.3).
    async fn put_site_request(&self, request: SiteRequest) -> AnyhowResult<()> {
        let _ = request;
        anyhow::bail!("this store keeps no site requests")
    }

    async fn get_site_request(&self, id: &SiteRequestId) -> AnyhowResult<Option<SiteRequest>> {
        let _ = id;
        Ok(None)
    }

    /// The requests a site has not yet answered, oldest first.
    async fn pending_site_requests(
        &self,
        site_id: &SiteId,
        limit: usize,
    ) -> AnyhowResult<Vec<SiteRequest>> {
        let _ = (site_id, limit);
        Ok(Vec::new())
    }
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
            sync: None,
            registrations: None,
            services: Vec::new(),
            parked: Vec::new(),
            backups: Vec::new(),
        });
        let groups = site.versions();
        assert_eq!(groups["0.25.268"], vec!["cm-1", "cm-2"]);
        assert_eq!(groups["0.25.267"], vec!["conf-1"]);
    }
}
