//! Which conference node a room join goes to.
//!
//! With a cluster, the same rule the call manager applies: the node
//! already hosting the room, else the least loaded node in the caller's
//! region with video budget for the meeting's codecs
//! (`fcp_cluster::choose_conference_node`). Without one, or when it finds
//! nothing, the static `[conference] server_uri`.

use tracing::{debug, warn};

/// A static conference host: `host`, `host:port`, or a `sip:` URI whose
/// host part is used.
fn static_host(configured: &str) -> Option<String> {
    let trimmed = configured.trim();
    if trimmed.is_empty() {
        return None;
    }
    let without_scheme = trimmed
        .strip_prefix("sips:")
        .or_else(|| trimmed.strip_prefix("sip:"))
        .unwrap_or(trimmed);
    let host = without_scheme
        .rsplit_once('@')
        .map(|(_, h)| h)
        .unwrap_or(without_scheme);
    let host = host.split(';').next().unwrap_or(host);
    if host.is_empty() {
        None
    } else {
        Some(host.to_string())
    }
}

/// Cluster placement inputs.
#[cfg(feature = "storage-redis")]
pub struct ClusterPlacement {
    pub registry: fcp_cluster::ConferenceRegistry,
    pub members: std::sync::Arc<dashmap::DashMap<String, fcp_cluster::ClusterMember>>,
    pub capacity: usize,
    pub region: Option<String>,
    /// The tenant the registry keys rooms by; without it the hosting-node
    /// lookup is skipped.
    pub tenant_id: Option<String>,
}

/// Picks the conference host for a room.
pub struct ConferencePlacement {
    static_host: Option<String>,
    #[cfg(feature = "storage-redis")]
    cluster: Option<ClusterPlacement>,
}

impl ConferencePlacement {
    /// Static placement only.
    pub fn static_only(server_uri: Option<&str>) -> Self {
        Self {
            static_host: server_uri.and_then(static_host),
            #[cfg(feature = "storage-redis")]
            cluster: None,
        }
    }

    /// Cluster placement with the static host as the fallback.
    #[cfg(feature = "storage-redis")]
    pub fn with_cluster(server_uri: Option<&str>, cluster: ClusterPlacement) -> Self {
        Self {
            static_host: server_uri.and_then(static_host),
            cluster: Some(cluster),
        }
    }

    /// Whether any placement is possible at all.
    pub fn is_configured(&self) -> bool {
        #[cfg(feature = "storage-redis")]
        if self.cluster.is_some() {
            return true;
        }
        self.static_host.is_some()
    }

    /// The `host[:port]` a join of `room_number` should INVITE, given the
    /// video codecs the browser negotiated (empty for an audio-only join).
    pub async fn host_for(&self, room_number: &str, video_codecs: &[String]) -> Option<String> {
        #[cfg(feature = "storage-redis")]
        if let Some(cluster) = &self.cluster {
            let hosts = match &cluster.tenant_id {
                Some(tenant) => match cluster.registry.hosts(tenant, room_number).await {
                    Ok(hosts) => hosts,
                    Err(e) => {
                        warn!(room = %room_number, error = %e, "conference registry lookup failed; placing by load");
                        Vec::new()
                    }
                },
                None => Vec::new(),
            };
            let members: Vec<fcp_cluster::ClusterMember> =
                cluster.members.iter().map(|m| m.value().clone()).collect();
            let need = (!video_codecs.is_empty())
                .then(|| fcp_cluster::VideoNeed::of(video_codecs.to_vec()));
            if let Some(pick) = fcp_cluster::choose_conference_node(
                &hosts,
                &members,
                cluster.capacity,
                cluster.region.as_deref(),
                need.as_ref(),
            ) {
                debug!(room = %room_number, node = %pick.node_id, host = %pick.sip_address, "room placed by cluster");
                return Some(pick.sip_address.to_string());
            }
            debug!(room = %room_number, "no conference node in the cluster; trying the static host");
        }
        let _ = video_codecs;
        if self.static_host.is_none() {
            warn!(room = %room_number, "no conference host: set [conference] server_uri or enable the cluster");
        }
        self.static_host.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn static_host_accepts_hosts_and_uris() {
        assert_eq!(static_host("10.0.0.7").as_deref(), Some("10.0.0.7"));
        assert_eq!(
            static_host("conf.example.com:5080").as_deref(),
            Some("conf.example.com:5080")
        );
        assert_eq!(
            static_host("sip:conf@10.0.0.7:5060").as_deref(),
            Some("10.0.0.7:5060")
        );
        assert_eq!(
            static_host("sips:10.0.0.7;transport=tls").as_deref(),
            Some("10.0.0.7")
        );
        assert_eq!(static_host("  "), None);
        assert_eq!(static_host("sip:@"), None);
    }

    #[tokio::test]
    async fn static_placement_returns_the_configured_host() {
        let p = ConferencePlacement::static_only(Some("sip:conf@10.0.0.7"));
        assert!(p.is_configured());
        assert_eq!(p.host_for("8001", &[]).await.as_deref(), Some("10.0.0.7"));
        let none = ConferencePlacement::static_only(None);
        assert!(!none.is_configured());
        assert!(none.host_for("8001", &["H264".into()]).await.is_none());
    }
}
