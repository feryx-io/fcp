//! Cross-node conference control (distributed conferencing, phase 3).
//!
//! A control request — lock, kick, mute, gain, announce, end — can land
//! on any conference node. Room-wide operations are applied locally and
//! then forwarded to every other node hosting the room; participant
//! operations are applied locally when the participant is here and
//! otherwise forwarded to the hosting peers until one of them has the
//! participant. Peers are reached over the HTTP API address they
//! advertise in cluster membership, on `/internal/conferences/…` routes
//! that authenticate with the shared `cluster.secret` and never forward
//! again, so a request travels exactly one hop.
//!
//! The room-wide lock also lives in the conference registry, so a node
//! that starts hosting a room later inherits it (see
//! [`ConferenceRegistry::set_locked`]).

use fcp_api_core::auth::PLATFORM_TENANT_HEADER;
use fcp_cluster::{ClusterMember, ConferenceHost, ConferenceRegistry, CONFERENCE_NODE_TYPE};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, warn};

/// Header naming the node a forwarded control request came from.
pub const NODE_HEADER: &str = "x-fcp-node-id";

/// How long one forwarded request may take, end to end.
const PEER_TIMEOUT: Duration = Duration::from_secs(5);

/// What one peer answered to a forwarded request.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct PeerOutcome {
    pub node: String,
    /// The peer answered 2xx.
    pub ok: bool,
    /// HTTP status the peer answered with; absent when it could not be
    /// reached.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<u16>,
    /// The peer's JSON body, or `{"error": …}` for a transport failure.
    pub body: serde_json::Value,
}

/// The peers a control request for a room goes to: every node the
/// registry shows hosting the room with participants, other than this
/// one, that is alive in cluster membership as a conference node. Sorted
/// by node id so fan-out order is stable.
pub fn control_peers(
    my_node: &str,
    hosts: &[ConferenceHost],
    members: &HashMap<String, ClusterMember>,
) -> Vec<ClusterMember> {
    let mut peers: Vec<ClusterMember> = hosts
        .iter()
        .filter(|h| h.node_id != my_node && h.participants > 0)
        .filter_map(|h| members.get(&h.node_id))
        .filter(|m| m.node_type == CONFERENCE_NODE_TYPE)
        .cloned()
        .collect();
    peers.sort_by(|a, b| a.node_id.cmp(&b.node_id));
    peers
}

/// Forwards control requests to the other nodes hosting a room.
pub struct ClusterControl {
    node_id: String,
    registry: ConferenceRegistry,
    members: Arc<dashmap::DashMap<String, ClusterMember>>,
    http: reqwest::Client,
    secret: String,
}

impl ClusterControl {
    pub fn new(
        node_id: String,
        registry: ConferenceRegistry,
        members: Arc<dashmap::DashMap<String, ClusterMember>>,
        secret: String,
    ) -> Self {
        let http = reqwest::Client::builder()
            .timeout(PEER_TIMEOUT)
            .build()
            .expect("reqwest client");
        Self {
            node_id,
            registry,
            members,
            http,
            secret,
        }
    }

    pub fn node_id(&self) -> &str {
        &self.node_id
    }

    /// Persist the room-wide lock flag in the registry.
    pub async fn set_locked(
        &self,
        tenant_id: &str,
        room_number: &str,
        locked: bool,
    ) -> anyhow::Result<()> {
        self.registry
            .set_locked(tenant_id, room_number, locked)
            .await
    }

    /// The other live conference nodes hosting the room.
    pub async fn hosting_peers(
        &self,
        tenant_id: &str,
        room_number: &str,
    ) -> anyhow::Result<Vec<ClusterMember>> {
        let hosts = self.registry.hosts(tenant_id, room_number).await?;
        let members: HashMap<String, ClusterMember> = self
            .members
            .iter()
            .map(|e| (e.key().clone(), e.value().clone()))
            .collect();
        Ok(control_peers(&self.node_id, &hosts, &members))
    }

    /// Send one request to one peer's internal route.
    async fn send(
        &self,
        peer: &ClusterMember,
        tenant_id: &str,
        method: reqwest::Method,
        path: &str,
        body: Option<&serde_json::Value>,
    ) -> PeerOutcome {
        let url = format!(
            "http://{}/internal{}",
            peer.http_address,
            path.trim_start_matches("/internal")
        );
        debug!(peer = %peer.node_id, %url, "Forwarding conference control request");
        let mut req = self
            .http
            .request(method, &url)
            .bearer_auth(&self.secret)
            .header(NODE_HEADER, &self.node_id)
            .header(PLATFORM_TENANT_HEADER, tenant_id);
        if let Some(body) = body {
            req = req.json(body);
        }
        match req.send().await {
            Ok(resp) => {
                let status = resp.status();
                let body = resp
                    .json::<serde_json::Value>()
                    .await
                    .unwrap_or_else(|e| serde_json::json!({ "error": e.to_string() }));
                PeerOutcome {
                    node: peer.node_id.clone(),
                    ok: status.is_success(),
                    status: Some(status.as_u16()),
                    body,
                }
            }
            Err(e) => {
                warn!(peer = %peer.node_id, %url, error = %e, "Conference control request to peer failed");
                PeerOutcome {
                    node: peer.node_id.clone(),
                    ok: false,
                    status: None,
                    body: serde_json::json!({ "error": e.to_string() }),
                }
            }
        }
    }

    /// Room-wide operation: send to every hosting peer, concurrently.
    /// `path` is the route below `/internal`, e.g. `/conferences/8000/lock`.
    pub async fn broadcast(
        &self,
        tenant_id: &str,
        room_number: &str,
        method: reqwest::Method,
        path: &str,
        body: Option<serde_json::Value>,
    ) -> anyhow::Result<Vec<PeerOutcome>> {
        let peers = self.hosting_peers(tenant_id, room_number).await?;
        let sends = peers
            .iter()
            .map(|peer| self.send(peer, tenant_id, method.clone(), path, body.as_ref()));
        Ok(futures::future::join_all(sends).await)
    }

    /// The member record for one node, when cluster membership has it.
    pub fn member(&self, node_id: &str) -> Option<ClusterMember> {
        self.members.get(node_id).map(|m| m.clone())
    }

    /// Send one request to one named node's `/internal` route, rather
    /// than to whoever hosts a room. `None` when membership does not
    /// know the node.
    pub async fn send_to_node(
        &self,
        node_id: &str,
        tenant_id: &str,
        method: reqwest::Method,
        path: &str,
        body: Option<serde_json::Value>,
    ) -> Option<PeerOutcome> {
        let peer = self.member(node_id)?;
        Some(
            self.send(&peer, tenant_id, method, path, body.as_ref())
                .await,
        )
    }

    /// Open a streaming GET against one named node's `/internal` route —
    /// a recording's bytes, which are too big for `PeerOutcome`'s JSON
    /// and too slow for the control timeout. The response is handed back
    /// unread.
    pub async fn stream_from_node(
        &self,
        node_id: &str,
        tenant_id: &str,
        path: &str,
    ) -> anyhow::Result<reqwest::Response> {
        let peer = self
            .member(node_id)
            .ok_or_else(|| anyhow::anyhow!("node {node_id} is not in cluster membership"))?;
        let url = format!(
            "http://{}/internal{}",
            peer.http_address,
            path.trim_start_matches("/internal")
        );
        debug!(peer = %peer.node_id, %url, "Streaming a file from a peer");
        // A file transfer is not a control request: only the connect is
        // bounded, so a long download is not cut off mid-stream.
        let client = reqwest::Client::builder()
            .connect_timeout(PEER_TIMEOUT)
            .build()?;
        let resp = client
            .get(&url)
            .bearer_auth(&self.secret)
            .header(NODE_HEADER, &self.node_id)
            .header(PLATFORM_TENANT_HEADER, tenant_id)
            .send()
            .await?;
        Ok(resp)
    }

    /// Tell every peer hosting the room who is speaking here (§15.4,
    /// 5c). Fire and forget: a claim is refreshed on a timer, so one
    /// that does not land is superseded rather than retried, and a peer
    /// that cannot be reached must not hold up the room.
    pub async fn announce_speaker(
        &self,
        tenant_id: &str,
        room_number: &str,
        claim: serde_json::Value,
    ) {
        self.announce(tenant_id, room_number, "speaker", claim)
            .await
    }

    /// Tell every peer hosting the room who is sharing a screen here and
    /// since when (§15.6, 7d), the same way.
    pub async fn announce_content(
        &self,
        tenant_id: &str,
        room_number: &str,
        claim: serde_json::Value,
    ) {
        self.announce(tenant_id, room_number, "content", claim)
            .await
    }

    async fn announce(
        &self,
        tenant_id: &str,
        room_number: &str,
        what: &str,
        claim: serde_json::Value,
    ) {
        let peers = match self.hosting_peers(tenant_id, room_number).await {
            Ok(peers) => peers,
            Err(e) => {
                debug!(room = %room_number, what, error = %e, "Could not read the registry; claim not announced");
                return;
            }
        };
        let path = format!("/conferences/{room_number}/{what}");
        let sends = peers
            .iter()
            .map(|peer| self.send(peer, tenant_id, reqwest::Method::POST, &path, Some(&claim)));
        for outcome in futures::future::join_all(sends).await {
            if !outcome.ok {
                debug!(peer = %outcome.node, room = %room_number, what, "Claim not applied");
            }
        }
    }

    /// Participant operation: try the hosting peers one at a time until
    /// one answers 2xx. Returns every outcome tried, the success last.
    pub async fn locate(
        &self,
        tenant_id: &str,
        room_number: &str,
        method: reqwest::Method,
        path: &str,
        body: Option<serde_json::Value>,
    ) -> anyhow::Result<Vec<PeerOutcome>> {
        let peers = self.hosting_peers(tenant_id, room_number).await?;
        let mut outcomes = Vec::with_capacity(peers.len());
        for peer in &peers {
            let outcome = self
                .send(peer, tenant_id, method.clone(), path, body.as_ref())
                .await;
            let done = outcome.ok;
            outcomes.push(outcome);
            if done {
                break;
            }
        }
        Ok(outcomes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn member(id: &str, node_type: &str) -> ClusterMember {
        ClusterMember {
            node_id: id.to_string(),
            node_type: node_type.to_string(),
            http_address: "10.0.0.1:8090".parse().unwrap(),
            sip_address: "10.0.0.1:5060".parse().unwrap(),
            sip_tls_address: None,
            capabilities: Vec::new(),
            active_calls: 0,
            started_at: 0,
            version: String::new(),
            public_url: None,
            capacity: None,
            draining: false,
            region: None,
            video_budget: Vec::new(),
            registering_trunks: Vec::new(),
            view: None,
        }
    }

    fn host(id: &str, participants: usize) -> ConferenceHost {
        ConferenceHost {
            node_id: id.to_string(),
            participants,
        }
    }

    #[test]
    fn control_peers_are_live_hosting_conference_nodes_other_than_self() {
        let hosts = vec![
            host("conf-d", 4),
            host("conf-a", 2),
            host("conf-b", 0),
            host("conf-c", 1),
            host("conf-e", 3),
            host("conf-f", 1),
        ];
        let members: HashMap<String, ClusterMember> = [
            member("conf-a", CONFERENCE_NODE_TYPE),
            member("conf-b", CONFERENCE_NODE_TYPE),
            member("conf-c", "call_manager"),
            member("conf-d", CONFERENCE_NODE_TYPE),
            member("conf-e", CONFERENCE_NODE_TYPE),
        ]
        .into_iter()
        .map(|m| (m.node_id.clone(), m))
        .collect();

        let peers: Vec<String> = control_peers("conf-a", &hosts, &members)
            .into_iter()
            .map(|m| m.node_id)
            .collect();
        // self, empty hosts, non-conference members, dead members excluded; sorted.
        assert_eq!(peers, vec!["conf-d".to_string(), "conf-e".to_string()]);
        assert!(control_peers("conf-a", &[], &members).is_empty());
    }
}
