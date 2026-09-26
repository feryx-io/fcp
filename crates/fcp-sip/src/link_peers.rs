//! The sites bound at this cloud (`docs/HYBRID_CONTROL_PLANE.md` §3.6).
//!
//! A site's call manager registers its link trunk's account here over
//! TLS, from wherever it is, and opens no port of its own: the cloud's
//! calls go down the connection the site opened (RFC 5626, the flow the
//! REGISTER left), and the site's own calls are known by where they come
//! from. This registry keeps, per link peer trunk, where the site is
//! bound from and the flow to reach it on; a binding lasts as long as the
//! registration says and no longer than its flow.

use dashmap::DashMap;
use fcp_core::{SiteId, Trunk};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};
use tracing::{debug, info};

use crate::flow::FlowRegistry;

/// A site's registration at this node.
#[derive(Debug, Clone)]
pub struct LinkPeerBinding {
    /// The cloud's end of the site's link.
    pub trunk: Arc<Trunk>,
    pub site_id: SiteId,
    /// Where the REGISTER came from: the site as this node sees it, which
    /// is where its calls come from and where the flow leads.
    pub peer: SocketAddr,
    /// The flow the REGISTER left, when it came over TLS.
    pub flow_token: Option<String>,
    pub bound_at: SystemTime,
    expires_at: Instant,
}

impl LinkPeerBinding {
    /// Whether the binding still stands: not expired, and its flow, when
    /// it has one, still open.
    pub fn is_live(&self, flows: &FlowRegistry) -> bool {
        if Instant::now() >= self.expires_at {
            return false;
        }
        match &self.flow_token {
            Some(token) => flows.is_valid(token),
            None => true,
        }
    }

    /// How long the registration has left.
    pub fn remaining(&self) -> Duration {
        self.expires_at.saturating_duration_since(Instant::now())
    }
}

/// The link peers bound at this node, by trunk key ([`Trunk::key`]).
#[derive(Default)]
pub struct LinkPeers {
    bound: DashMap<String, LinkPeerBinding>,
}

impl LinkPeers {
    pub fn new() -> Self {
        Self::default()
    }

    /// A site registered: bound from `peer`, reachable down `flow_token`,
    /// for `ttl`. A refresh replaces the binding, flow and all.
    pub fn bind(
        &self,
        trunk: Arc<Trunk>,
        peer: SocketAddr,
        flow_token: Option<String>,
        ttl: Duration,
    ) {
        let Some(site_id) = trunk.site_id else {
            return;
        };
        let key = trunk.key();
        let fresh = !self.bound.contains_key(&key);
        self.bound.insert(
            key,
            LinkPeerBinding {
                trunk: Arc::clone(&trunk),
                site_id,
                peer,
                flow_token,
                bound_at: SystemTime::now(),
                expires_at: Instant::now() + ttl,
            },
        );
        if fresh {
            info!(site_id = %site_id, trunk = %trunk.name, peer = %peer, "A site is bound: its link registered");
        } else {
            debug!(site_id = %site_id, peer = %peer, "A site's link refreshed its binding");
        }
    }

    /// A site deregistered, or its registration lapsed.
    pub fn unbind(&self, trunk_key: &str) {
        if let Some((_, gone)) = self.bound.remove(trunk_key) {
            info!(site_id = %gone.site_id, trunk = %gone.trunk.name, "A site is unbound");
        }
    }

    /// The site bound from exactly `peer`: a site's calls come over the
    /// connection its REGISTER did, and nothing looser is believed — two
    /// peers may share an address, a carrier and a site behind one NAT,
    /// every node on a loopback.
    pub fn from_peer(&self, peer: SocketAddr, flows: &FlowRegistry) -> Option<Arc<Trunk>> {
        self.bound
            .iter()
            .find(|b| b.peer == peer && b.is_live(flows))
            .map(|b| Arc::clone(&b.trunk))
    }

    /// The flow the site bound from exactly `peer` is reached on, if it
    /// registered over one: what a call that came up it is answered and
    /// re-invited down.
    pub fn flow_token_for_peer(&self, peer: SocketAddr, flows: &FlowRegistry) -> Option<String> {
        self.bound
            .iter()
            .find(|b| b.peer == peer && b.is_live(flows))
            .and_then(|b| b.flow_token.clone())
    }

    /// Where `site` is bound, if it is.
    pub fn for_site(&self, site: SiteId, flows: &FlowRegistry) -> Option<LinkPeerBinding> {
        self.bound
            .iter()
            .find(|b| b.site_id == site && b.is_live(flows))
            .map(|b| b.value().clone())
    }

    /// The binding of the trunk with `key`, if it stands.
    pub fn for_trunk(&self, key: &str, flows: &FlowRegistry) -> Option<LinkPeerBinding> {
        self.bound
            .get(key)
            .filter(|b| b.is_live(flows))
            .map(|b| b.value().clone())
    }

    /// Every binding that stands.
    pub fn live(&self, flows: &FlowRegistry) -> Vec<LinkPeerBinding> {
        self.bound
            .iter()
            .filter(|b| b.is_live(flows))
            .map(|b| b.value().clone())
            .collect()
    }

    /// Drop the bindings that expired or whose flow closed.
    pub fn sweep(&self, flows: &FlowRegistry) -> usize {
        let mut gone = 0;
        self.bound.retain(|_, b| {
            let live = b.is_live(flows);
            if !live {
                info!(site_id = %b.site_id, trunk = %b.trunk.name, "A site is unbound: its registration lapsed or its connection closed");
                gone += 1;
            }
            live
        });
        gone
    }
}

/// The sites' link accounts, as a credential store answers for them: the
/// cloud SBC's registrar challenges a site's REGISTER with these, from
/// the link peer trunks it knows (`docs/HYBRID_CONTROL_PLANE.md` §3.6).
/// Each account is its trunk's tenant's; nothing is ever written.
pub struct SiteAccounts {
    trunks: Arc<dyn crate::trunk_ingress::InboundTrunks>,
    /// The realm the registrar challenges with, which the HA1 is made in.
    realm: String,
}

impl SiteAccounts {
    pub fn new(trunks: Arc<dyn crate::trunk_ingress::InboundTrunks>, realm: String) -> Self {
        Self { trunks, realm }
    }

    fn account(&self, username: &str) -> Option<fcp_core::User> {
        let trunk = self.trunks.link_peer_by_account(username)?;
        let credentials = trunk.credentials.as_ref()?;
        let tenant_id = trunk.tenant_id?;
        let now = SystemTime::now();
        Some(fcp_core::User {
            id: fcp_core::UserId::new(),
            username: credentials.username.clone(),
            password: String::new(),
            digest_password: None,
            sip_ha1_hash: Some(fcp_core::storage::password::compute_ha1(
                &credentials.username,
                &self.realm,
                &credentials.password,
            )),
            sip_provisioning_password: None,
            display_name: Some(trunk.name.clone()),
            tenant_id,
            role: fcp_core::UserRole::EndUser,
            department_id: None,
            enabled: trunk.enabled,
            created_at: now,
            updated_at: now,
            media_preferences: Default::default(),
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
            account_status: fcp_core::AccountStatus::Active,
            last_login: None,
            emergency_contact: Default::default(),
            primary_phone_number_id: None,
            voicemail_box_id: None,
            profile_picture: None,
            failed_login_attempts: 0,
            account_locked_until: None,
        })
    }
}

#[async_trait::async_trait]
impl fcp_core::storage::CredentialStore for SiteAccounts {
    async fn get_user(&self, username: &str) -> anyhow::Result<Option<fcp_core::User>> {
        Ok(self.account(username))
    }

    /// The account, whichever tenant it is asked for in: a site's id is
    /// unique across tenants, a registrar that knows no tenants asks in
    /// its default's name, and the password decides. The account's own
    /// `tenant_id` is the site's.
    async fn get_user_in_tenant(
        &self,
        username: &str,
        _tenant_id: &fcp_core::TenantId,
    ) -> anyhow::Result<Option<fcp_core::User>> {
        Ok(self.account(username))
    }

    async fn list_users_by_username(&self, username: &str) -> anyhow::Result<Vec<fcp_core::User>> {
        Ok(self.account(username).into_iter().collect())
    }

    async fn put_user(&self, _user: fcp_core::User) -> anyhow::Result<()> {
        anyhow::bail!("a site's link account is made at enrollment, not here")
    }

    async fn delete_user(&self, _username: &str) -> anyhow::Result<()> {
        anyhow::bail!("a site's link account is made at enrollment, not here")
    }

    async fn delete_user_in_tenant(
        &self,
        _username: &str,
        _tenant_id: &fcp_core::TenantId,
    ) -> anyhow::Result<()> {
        anyhow::bail!("a site's link account is made at enrollment, not here")
    }

    async fn list_users(&self) -> anyhow::Result<Vec<fcp_core::User>> {
        Ok(Vec::new())
    }
}

/// The user part of a request's To URI: the account a REGISTER binds.
pub fn to_user(request: &sip_core::Request) -> Option<String> {
    let to = sip_parse::header(request.headers(), "To")?;
    let to = to.as_str();
    let uri = to
        .split_once('<')
        .and_then(|(_, rest)| rest.split_once('>').map(|(u, _)| u))
        .unwrap_or(to);
    let uri = uri.trim_start_matches("sip:").trim_start_matches("sips:");
    let user = uri.split_once('@').map(|(u, _)| u.trim())?;
    (!user.is_empty()).then(|| user.to_string())
}

/// How long a REGISTER asks for: the Contact's `expires` parameter, else
/// the Expires header, else an hour.
pub fn requested_expiry(request: &sip_core::Request) -> u32 {
    if let Some(contact) = sip_parse::header(request.headers(), "Contact") {
        let contact = contact.as_str();
        for param in contact.split(';').skip(1) {
            if let Some((name, value)) = param.split_once('=') {
                if name.trim().eq_ignore_ascii_case("expires") {
                    if let Ok(v) = value.trim().trim_matches('"').parse::<u32>() {
                        return v;
                    }
                }
            }
        }
    }
    sip_parse::header(request.headers(), "Expires")
        .and_then(|h| h.as_str().trim().parse::<u32>().ok())
        .unwrap_or(3600)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peer_trunk(site: SiteId) -> Arc<Trunk> {
        let mut t = Trunk::new("link".into(), fcp_core::TenantId::new());
        t.site_id = Some(site);
        t.link = Some(fcp_core::LinkEnd::Cloud);
        Arc::new(t)
    }

    #[test]
    fn a_site_is_found_by_its_exact_peer_and_nothing_looser() {
        let flows = FlowRegistry::new();
        let peers = LinkPeers::new();
        let site = SiteId::new();
        let trunk = peer_trunk(site);
        let from: SocketAddr = "203.0.113.9:44000".parse().unwrap();
        peers.bind(Arc::clone(&trunk), from, None, Duration::from_secs(60));
        assert!(peers.from_peer(from, &flows).is_some());
        assert!(peers
            .from_peer("203.0.113.9:1".parse().unwrap(), &flows)
            .is_none());
        assert!(peers
            .from_peer("203.0.113.10:44000".parse().unwrap(), &flows)
            .is_none());
        assert_eq!(peers.for_site(site, &flows).unwrap().peer, from);
    }

    #[test]
    fn an_expired_binding_no_longer_stands() {
        let flows = FlowRegistry::new();
        let peers = LinkPeers::new();
        let site = SiteId::new();
        let from: SocketAddr = "203.0.113.9:44000".parse().unwrap();
        peers.bind(peer_trunk(site), from, None, Duration::from_secs(0));
        assert!(peers.for_site(site, &flows).is_none());
        assert_eq!(peers.sweep(&flows), 1);
    }

    #[test]
    fn a_binding_dies_with_its_flow() {
        let flows = FlowRegistry::new();
        let peers = LinkPeers::new();
        let site = SiteId::new();
        let (tx, rx) = tokio::sync::mpsc::channel(1);
        let from: SocketAddr = "203.0.113.9:44000".parse().unwrap();
        let token = flows.register(tx, from);
        peers.bind(peer_trunk(site), from, Some(token), Duration::from_secs(60));
        assert!(peers.for_site(site, &flows).is_some());
        drop(rx);
        assert!(peers.for_site(site, &flows).is_none());
    }

    #[test]
    fn a_trunk_without_a_site_is_not_bound() {
        let flows = FlowRegistry::new();
        let peers = LinkPeers::new();
        let trunk = Arc::new(Trunk::new("carrier".into(), fcp_core::TenantId::new()));
        peers.bind(
            trunk,
            "203.0.113.9:5060".parse().unwrap(),
            None,
            Duration::from_secs(60),
        );
        assert!(peers.live(&flows).is_empty());
    }
}
