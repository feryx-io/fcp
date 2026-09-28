//! Shared call appearances on this node (`docs/ENTERPRISE_CALLING.md` §3.9).
//!
//! A shared line's appearances live in an [`AppearanceTable`] — this
//! node's memory, or the cluster's Redis so that every node's phones see
//! every node's calls. Each node keeps its own phones' subscriptions:
//!
//! - `call-info` watchers, sent the line's lamps (`Call-Info` on a NOTIFY
//!   inside the subscription) whenever an appearance changes, whichever
//!   node changed it (`line.appearance_changed` carries the news);
//! - `line-seize` subscriptions, each claiming an index for a phone about
//!   to call from it.
//!
//! Calls move their appearances through the call manager's appearance
//! hook: a call to the line claims the lowest free index alerting, a call
//! from it the index the phone seized (or the lowest free one), and the
//! hook follows it through active, held and idle. A phone retrieving a
//! held appearance takes the line's leg of that call; when the call is on
//! another node the INVITE is sent there with the appearance's handoff
//! token (`X-FCP-Node-Handoff`), which only a node that read the table
//! can know.

use crate::registrar::{FcpRegistrar, InviteAuth};
use crate::uac::SipUserAgentClient;
use async_trait::async_trait;
use bytes::Bytes;
use dashmap::DashMap;
use fcp_b2bua_core::{CallAppearance, CallId, CallManager, LegSide};
use fcp_core::sca::{self as model, Appearance, AppearanceIndex, AppearanceState};
use fcp_core::storage::{LineBindingStore, PhoneNumberStore};
use fcp_core::{DynStorage, LineBinding, LineBindingId, TenantId};
use sip_core::{Headers, Method, Request, RequestLine, SipUri};
use smol_str::SmolStr;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

/// What a retrieve forwarded to the node holding the call carries: the
/// appearance's handoff token.
pub const NODE_HANDOFF_HEADER: &str = "X-FCP-Node-Handoff";

/// How long a call's appearance stands in the table without its node
/// refreshing it: a node that died mid-call frees its keys this long after.
const CALL_LEASE_SECS: u64 = 30 * 60;

/// How often this node refreshes its calls' appearances and forgets its
/// lapsed subscriptions.
const REFRESH_EVERY: Duration = Duration::from_secs(5 * 60);

/// A `call-info` subscription's length when the phone asks for none, and
/// the most it is granted.
const CALL_INFO_EXPIRES: u32 = 3600;

/// A `line-seize` subscription's length when the phone asks for none.
const SEIZE_EXPIRES: u32 = 15;

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Where a line's appearances are kept.
#[async_trait]
pub trait AppearanceTable: Send + Sync {
    /// The line's live appearances (lapsed ones left out).
    async fn list(
        &self,
        tenant: &TenantId,
        line: &LineBindingId,
    ) -> anyhow::Result<Vec<Appearance>>;

    /// Put `entry` at `index` — or at the lowest index up to `max` that
    /// will take it when `index` is `None` — if the index is free, lapsed,
    /// already `entry.owner`'s, or `replacing`'s (a seize becoming the
    /// call). Returns the index taken.
    async fn claim(
        &self,
        tenant: &TenantId,
        line: &LineBindingId,
        index: Option<u8>,
        max: u8,
        entry: Appearance,
        replacing: Option<&str>,
    ) -> anyhow::Result<Option<u8>>;

    /// Free `index` if `owner` holds it.
    async fn release(
        &self,
        tenant: &TenantId,
        line: &LineBindingId,
        index: u8,
        owner: &str,
    ) -> anyhow::Result<()>;
}

/// Whether an index holding `current` may be given to `owner`.
pub fn may_take(
    current: Option<&Appearance>,
    owner: &str,
    replacing: Option<&str>,
    now: u64,
) -> bool {
    match current {
        None => true,
        Some(a) => a.lapsed(now) || a.owner == owner || Some(a.owner.as_str()) == replacing,
    }
}

/// This node's appearances, in memory: a node without a cluster.
#[derive(Default)]
pub struct LocalAppearances {
    lines: DashMap<(TenantId, LineBindingId), BTreeMap<u8, Appearance>>,
}

#[async_trait]
impl AppearanceTable for LocalAppearances {
    async fn list(
        &self,
        tenant: &TenantId,
        line: &LineBindingId,
    ) -> anyhow::Result<Vec<Appearance>> {
        let now = now_secs();
        Ok(self
            .lines
            .get(&(*tenant, *line))
            .map(|l| l.values().filter(|a| !a.lapsed(now)).cloned().collect())
            .unwrap_or_default())
    }

    async fn claim(
        &self,
        tenant: &TenantId,
        line: &LineBindingId,
        index: Option<u8>,
        max: u8,
        mut entry: Appearance,
        replacing: Option<&str>,
    ) -> anyhow::Result<Option<u8>> {
        let now = now_secs();
        let mut appearances = self.lines.entry((*tenant, *line)).or_default();
        let candidates: Vec<u8> = match index {
            Some(i) => vec![i],
            None => (1..=max).collect(),
        };
        for i in candidates {
            if may_take(appearances.get(&i), &entry.owner, replacing, now) {
                entry.index = i;
                appearances.insert(i, entry);
                return Ok(Some(i));
            }
        }
        Ok(None)
    }

    async fn release(
        &self,
        tenant: &TenantId,
        line: &LineBindingId,
        index: u8,
        owner: &str,
    ) -> anyhow::Result<()> {
        if let Some(mut appearances) = self.lines.get_mut(&(*tenant, *line)) {
            if appearances.get(&index).is_some_and(|a| a.owner == owner) {
                appearances.remove(&index);
            }
        }
        self.lines.remove_if(&(*tenant, *line), |_, a| a.is_empty());
        Ok(())
    }
}

/// A shared line, as a request names it.
#[derive(Debug, Clone)]
pub struct SharedLine {
    pub binding: LineBinding,
    /// Its number, E.164.
    pub number: String,
}

/// What an INVITE from a shared line's phone is.
#[derive(Debug)]
pub enum Outbound {
    /// Not from a shared line: routed as any call.
    NotShared,
    /// A call from the line, at this appearance.
    Claimed(CallAppearance),
    /// A phone picking up a held appearance of a call on this node.
    RetrieveHere(CallId),
    /// … of a call on another node: send the INVITE there.
    RetrieveThere { node_uri: String, handoff: String },
    /// Refused with this status.
    Refuse(u16, &'static str),
}

/// What a call to a shared line is given.
#[derive(Debug)]
pub enum Inbound {
    Claimed(CallAppearance),
    /// Every appearance is in use.
    Busy,
    /// The table could not be read or written: the call rings without an
    /// appearance.
    Unkept,
}

/// One side of a subscription this node answers: enough to send NOTIFYs
/// inside it.
struct Subscription {
    tenant: TenantId,
    line: LineBindingId,
    number: String,
    /// The subscriber's Call-ID, their tag and ours.
    call_id: String,
    their_tag: String,
    our_tag: String,
    /// The subscriber's `From`, without its tag: our NOTIFYs' `To`.
    subscriber: String,
    /// Where NOTIFYs go.
    contact: SipUri,
    cseq: AtomicU32,
    expires_at: std::sync::Mutex<Instant>,
    /// For a `line-seize` subscription: the index it holds.
    seized: std::sync::Mutex<Option<u8>>,
}

impl Subscription {
    fn lapsed(&self) -> bool {
        *self.expires_at.lock().expect("expiry") <= Instant::now()
    }

    fn remaining(&self) -> u64 {
        self.expires_at
            .lock()
            .expect("expiry")
            .saturating_duration_since(Instant::now())
            .as_secs()
    }
}

/// A call's appearance moving, told by the call manager's hook.
struct Change {
    call_id: CallId,
    appearance: CallAppearance,
    state: Option<AppearanceState>,
}

/// Shared call appearances on this node.
pub struct Sca {
    table: Arc<dyn AppearanceTable>,
    storage: Arc<DynStorage>,
    uac: Arc<SipUserAgentClient>,
    registrar: Option<Arc<FcpRegistrar>>,
    call_manager: std::sync::OnceLock<Weak<CallManager>>,
    node_id: String,
    /// This node's SIP address (`host:port`): the URI lamps are about, and
    /// where another node sends a retrieve of a call held here.
    host: String,
    watchers: DashMap<String, Arc<Subscription>>,
    seizes: DashMap<String, Arc<Subscription>>,
    /// Handoff token → the call on this node it lets another node retrieve.
    handoffs: DashMap<String, CallId>,
    /// The appearances of this node's calls, as last written.
    live: DashMap<CallId, (CallAppearance, AppearanceState)>,
    changes: std::sync::OnceLock<mpsc::UnboundedSender<Change>>,
    me: Weak<Sca>,
}

impl Sca {
    pub fn new(
        table: Arc<dyn AppearanceTable>,
        storage: Arc<DynStorage>,
        uac: Arc<SipUserAgentClient>,
        registrar: Option<Arc<FcpRegistrar>>,
        node_id: String,
        host: String,
    ) -> Arc<Self> {
        Arc::new_cyclic(|me| Self {
            table,
            storage,
            uac,
            registrar,
            call_manager: std::sync::OnceLock::new(),
            node_id,
            host,
            watchers: DashMap::new(),
            seizes: DashMap::new(),
            handoffs: DashMap::new(),
            live: DashMap::new(),
            changes: std::sync::OnceLock::new(),
            me: me.clone(),
        })
    }

    /// Follow the call manager's calls: their appearances written as they
    /// move, in order, refreshed on a tick with lapsed subscriptions
    /// forgotten.
    pub fn install(self: &Arc<Self>, call_manager: &Arc<CallManager>) {
        let _ = self.call_manager.set(Arc::downgrade(call_manager));
        let (tx, mut rx) = mpsc::unbounded_channel::<Change>();
        let _ = self.changes.set(tx.clone());
        call_manager.set_on_appearance_changed(Arc::new(move |call_id, appearance, state| {
            let _ = tx.send(Change {
                call_id,
                appearance,
                state,
            });
        }));
        let me = Arc::downgrade(self);
        tokio::spawn(async move {
            while let Some(change) = rx.recv().await {
                let Some(me) = me.upgrade() else { return };
                me.call_changed(change).await;
            }
        });
        let me = Arc::downgrade(self);
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(REFRESH_EVERY);
            tick.tick().await;
            loop {
                tick.tick().await;
                let Some(me) = me.upgrade() else { return };
                me.refresh().await;
            }
        });
    }

    fn call_manager(&self) -> Option<Arc<CallManager>> {
        self.call_manager.get().and_then(Weak::upgrade)
    }

    /// Whether a SUBSCRIBE is for one of the shared-line packages.
    pub fn handles_event(request: &Request) -> bool {
        event_of(request)
            .is_some_and(|e| e == model::CALL_INFO_EVENT || e == model::LINE_SEIZE_EVENT)
    }

    /// The shared line a SIP user part names: its number, or the number
    /// without its `+`.
    pub async fn shared_line(&self, user: &str) -> Option<SharedLine> {
        let user = user.trim();
        let mut candidates = vec![user.to_string()];
        if !user.starts_with('+') && !user.is_empty() && user.chars().all(|c| c.is_ascii_digit()) {
            candidates.push(format!("+{user}"));
        }
        for e164 in candidates {
            let Ok(Some(number)) = self.storage.get_phone_number_by_e164(&e164).await else {
                continue;
            };
            match self
                .storage
                .get_line_binding_by_phone_number(&number.id)
                .await
            {
                Ok(Some(binding)) if binding.shared => {
                    return Some(SharedLine {
                        binding,
                        number: number.e164,
                    })
                }
                Ok(_) => return None,
                Err(e) => {
                    warn!(number = %e164, error = %e, "A number's line could not be read");
                    return None;
                }
            }
        }
        None
    }

    /// A shared line's lamps: its appearances as the table holds them.
    pub async fn appearances(&self, tenant: &TenantId, line: &LineBindingId) -> Vec<Appearance> {
        match self.table.list(tenant, line).await {
            Ok(list) => list,
            Err(e) => {
                warn!(line = %line, error = %e, "A shared line's appearances could not be read");
                Vec::new()
            }
        }
    }

    // ------------------------------------------------------------------
    // Subscriptions
    // ------------------------------------------------------------------

    /// Answer a `call-info` or `line-seize` SUBSCRIBE.
    pub async fn handle_subscribe(
        &self,
        request: &Request,
        handle: &sip_transaction::ServerTransactionHandle,
    ) -> anyhow::Result<()> {
        let event = event_of(request).unwrap_or_default();
        let user = request_user(request).unwrap_or_default();
        let Some(line) = self.shared_line(&user).await else {
            return respond(handle, request, 404, "Not a Shared Line").await;
        };

        // Who is asking: credentials that may be the line (a device bound
        // to it, its owner), wearing the line's address.
        if let Some(registrar) = &self.registrar {
            match registrar.authenticate_invite(request).await {
                InviteAuth::Admitted { .. } | InviteAuth::NotRequired => {}
                InviteAuth::WrongUser { authenticated, .. } => {
                    info!(line = %line.number, authenticated = %authenticated, "A shared-line SUBSCRIBE from credentials that are not the line's");
                    return respond(handle, request, 403, "Forbidden").await;
                }
                InviteAuth::Challenge { stale } => {
                    let response = match registrar.invite_challenge(request, stale) {
                        Ok(r) => r,
                        Err(_) => sip_uas::UserAgentServer::create_response(
                            request,
                            500,
                            "Server Internal Error",
                        ),
                    };
                    handle.send_final(response).await;
                    return Ok(());
                }
            }
        }
        let is_line = crate::registrar::from_user(request)
            .is_some_and(|from| from == line.number || format!("+{from}") == line.number);
        if !is_line {
            return respond(handle, request, 403, "Subscriber Is Not the Line").await;
        }

        if event == model::LINE_SEIZE_EVENT {
            self.subscribe_seize(request, handle, &line).await
        } else {
            self.subscribe_call_info(request, handle, &line).await
        }
    }

    async fn subscribe_call_info(
        &self,
        request: &Request,
        handle: &sip_transaction::ServerTransactionHandle,
        line: &SharedLine,
    ) -> anyhow::Result<()> {
        let expires = expires_of(request)
            .unwrap_or(CALL_INFO_EXPIRES)
            .min(CALL_INFO_EXPIRES);
        let call_id = header(request, "Call-ID").unwrap_or_default();
        let existing = self.watchers.get(&call_id).map(|w| Arc::clone(&w));
        let sub = match existing {
            Some(sub) => sub,
            None if expires == 0 => {
                return respond_ok(handle, request, None, 0).await;
            }
            None => match self.new_subscription(request, line) {
                Some(sub) => sub,
                None => return respond(handle, request, 400, "Bad Request").await,
            },
        };
        *sub.expires_at.lock().expect("expiry") =
            Instant::now() + Duration::from_secs(u64::from(expires));
        respond_ok(handle, request, Some(&sub.our_tag), expires).await?;
        if expires == 0 {
            self.watchers.remove(&call_id);
            let appearances = self.appearances(&sub.tenant, &sub.line).await;
            self.send_notify(
                &sub,
                model::CALL_INFO_EVENT,
                "terminated;reason=timeout",
                Some(model::call_info_value(&self.host, &appearances)),
            );
            return Ok(());
        }
        info!(line = %line.number, subscriber = %sub.subscriber, expires, "A phone watches a shared line");
        self.watchers.insert(call_id, Arc::clone(&sub));
        let appearances = self.appearances(&sub.tenant, &sub.line).await;
        self.send_notify(
            &sub,
            model::CALL_INFO_EVENT,
            &format!("active;expires={expires}"),
            Some(model::call_info_value(&self.host, &appearances)),
        );
        Ok(())
    }

    async fn subscribe_seize(
        &self,
        request: &Request,
        handle: &sip_transaction::ServerTransactionHandle,
        line: &SharedLine,
    ) -> anyhow::Result<()> {
        let expires = expires_of(request)
            .unwrap_or(SEIZE_EXPIRES)
            .min(model::MAX_SEIZE_SECS as u32);
        let call_id = header(request, "Call-ID").unwrap_or_default();
        let owner = Appearance::seize_owner(&call_id);
        let existing = self.seizes.get(&call_id).map(|s| Arc::clone(&s));

        if expires == 0 {
            let Some(sub) = existing else {
                return respond_ok(handle, request, None, 0).await;
            };
            self.seizes.remove(&call_id);
            respond_ok(handle, request, Some(&sub.our_tag), 0).await?;
            let index = *sub.seized.lock().expect("seized");
            if let Some(index) = index {
                // Still seized, not yet the call: the key goes back to idle.
                if let Err(e) = self
                    .table
                    .release(&line.binding.tenant_id, &line.binding.id, index, &owner)
                    .await
                {
                    warn!(line = %line.number, error = %e, "A seized appearance could not be freed");
                }
                self.changed(line, index, None, None);
                self.send_notify(
                    &sub,
                    model::LINE_SEIZE_EVENT,
                    "terminated;reason=noresource",
                    Some(model::seized_value(&self.host, index)),
                );
            }
            return Ok(());
        }

        let wanted = match header(request, "Call-Info").and_then(|v| model::requested_index(&v)) {
            Some(AppearanceIndex::Index(i)) if i >= 1 && i <= line.binding.max_appearances => {
                Some(i)
            }
            Some(AppearanceIndex::Index(_)) => {
                return respond(handle, request, 400, "No Such Appearance").await;
            }
            Some(AppearanceIndex::Any) | None => existing
                .as_ref()
                .and_then(|s| *s.seized.lock().expect("seized")),
        };
        let entry = Appearance {
            index: 0,
            state: AppearanceState::Seized,
            owner: owner.clone(),
            node: self.node_id.clone(),
            node_uri: None,
            handoff: None,
            contact: contact_of(request),
            remote: None,
            expires_at: now_secs() + u64::from(expires),
        };
        let claimed = self
            .table
            .claim(
                &line.binding.tenant_id,
                &line.binding.id,
                wanted,
                line.binding.max_appearances,
                entry,
                None,
            )
            .await;
        let index = match claimed {
            Ok(Some(index)) => index,
            Ok(None) => {
                info!(line = %line.number, wanted = ?wanted, "No appearance to seize");
                return respond(handle, request, 480, "Appearance In Use").await;
            }
            Err(e) => {
                warn!(line = %line.number, error = %e, "An appearance could not be seized");
                return respond(handle, request, 503, "Service Unavailable").await;
            }
        };
        let sub = match existing {
            Some(sub) => sub,
            None => match self.new_subscription(request, line) {
                Some(sub) => sub,
                None => return respond(handle, request, 400, "Bad Request").await,
            },
        };
        *sub.expires_at.lock().expect("expiry") =
            Instant::now() + Duration::from_secs(u64::from(expires));
        *sub.seized.lock().expect("seized") = Some(index);
        self.seizes.insert(call_id, Arc::clone(&sub));
        respond_ok(handle, request, Some(&sub.our_tag), expires).await?;
        info!(line = %line.number, index, "An appearance seized");
        self.send_notify(
            &sub,
            model::LINE_SEIZE_EVENT,
            &format!("active;expires={expires}"),
            Some(model::seized_value(&self.host, index)),
        );
        self.changed(line, index, Some(AppearanceState::Seized), None);
        Ok(())
    }

    fn new_subscription(&self, request: &Request, line: &SharedLine) -> Option<Arc<Subscription>> {
        let contact = contact_of(request).and_then(|c| SipUri::parse(&c).ok())?;
        let from = header(request, "From")?;
        let their_tag = tag_of(&from).unwrap_or_default();
        Some(Arc::new(Subscription {
            tenant: line.binding.tenant_id,
            line: line.binding.id,
            number: line.number.clone(),
            call_id: header(request, "Call-ID")?,
            their_tag,
            our_tag: uuid::Uuid::new_v4().simple().to_string(),
            subscriber: without_tag(&from),
            contact,
            cseq: AtomicU32::new(0),
            expires_at: std::sync::Mutex::new(Instant::now()),
            seized: std::sync::Mutex::new(None),
        }))
    }

    /// Send a NOTIFY inside `sub`, from a task of its own.
    fn send_notify(
        &self,
        sub: &Arc<Subscription>,
        event: &str,
        state: &str,
        call_info: Option<String>,
    ) {
        let cseq = sub.cseq.fetch_add(1, Ordering::SeqCst) + 1;
        let request =
            match notify_request(sub, &self.host, cseq, event, state, call_info.as_deref()) {
                Ok(r) => r,
                Err(e) => {
                    warn!(error = %e, "A shared-line NOTIFY could not be built");
                    return;
                }
            };
        let uac = Arc::clone(&self.uac);
        let target = sub.contact.clone();
        let number = sub.number.clone();
        tokio::spawn(async move {
            match uac
                .inner()
                .send_request(request, sip_uac::integrated::RequestTarget::Uri(target))
                .await
            {
                Ok(r) if r.start_line().is_success() => {}
                Ok(r) => {
                    debug!(line = %number, status = r.start_line().code(), "A shared-line NOTIFY was refused")
                }
                Err(e) => {
                    debug!(line = %number, error = %e, "A shared-line NOTIFY was not delivered")
                }
            }
        });
    }

    /// Send every watcher of the line on this node its lamps.
    pub async fn notify_line(&self, tenant: &TenantId, line: &LineBindingId) {
        let watching: Vec<Arc<Subscription>> = self
            .watchers
            .iter()
            .filter(|w| w.tenant == *tenant && w.line == *line && !w.lapsed())
            .map(|w| Arc::clone(&w))
            .collect();
        if watching.is_empty() {
            return;
        }
        let value = model::call_info_value(&self.host, &self.appearances(tenant, line).await);
        for sub in watching {
            let state = format!("active;expires={}", sub.remaining());
            self.send_notify(&sub, model::CALL_INFO_EVENT, &state, Some(value.clone()));
        }
    }

    /// An appearance of `line` changed on this node: its watchers here are
    /// told now, every other node's through the event.
    fn changed(
        &self,
        line: &SharedLine,
        index: u8,
        state: Option<AppearanceState>,
        call_id: Option<CallId>,
    ) {
        self.announce(
            line.binding.tenant_id,
            line.binding.id,
            line.number.clone(),
            index,
            state,
            call_id,
        );
    }

    fn announce(
        &self,
        tenant: TenantId,
        line: LineBindingId,
        number: String,
        index: u8,
        state: Option<AppearanceState>,
        call_id: Option<CallId>,
    ) {
        if let Some(cm) = self.call_manager() {
            cm.publish(fcp_events::PlatformEvent::Presence(
                fcp_events::PresenceEvent::AppearanceChanged {
                    line_binding_id: line.to_string(),
                    number,
                    index,
                    state: state.map(|s| s.as_str()).unwrap_or("idle").to_string(),
                    call_id: call_id.map(|c| c.to_string()),
                    node_id: Some(self.node_id.clone()),
                    tenant_id: tenant,
                    timestamp: SystemTime::now(),
                },
            ));
        }
        let me_tenant = tenant;
        // The watchers here, after the table write this follows.
        let watchers_here = self
            .watchers
            .iter()
            .any(|w| w.tenant == me_tenant && w.line == line);
        if watchers_here {
            if let Some(me) = self.self_arc() {
                tokio::spawn(async move { me.notify_line(&me_tenant, &line).await });
            }
        }
    }

    /// An `line.appearance_changed` event: another node's change reaches
    /// this node's watchers. This node's own were told when it made it.
    pub async fn on_event(&self, event: &fcp_events::PresenceEvent) {
        if let fcp_events::PresenceEvent::AppearanceChanged {
            line_binding_id,
            node_id,
            tenant_id,
            ..
        } = event
        {
            if node_id.as_deref() == Some(self.node_id.as_str()) {
                return;
            }
            if let Ok(line) = line_binding_id.parse::<LineBindingId>() {
                self.notify_line(tenant_id, &line).await;
            }
        }
    }

    fn self_arc(&self) -> Option<Arc<Self>> {
        self.me.upgrade()
    }

    // ------------------------------------------------------------------
    // Calls
    // ------------------------------------------------------------------

    /// A call to a shared line: the lowest free appearance, alerting.
    pub async fn inbound(&self, line: &SharedLine, call_id: CallId) -> Inbound {
        let handoff = uuid::Uuid::new_v4().simple().to_string();
        let entry = self.call_entry(call_id, AppearanceState::Alerting, &handoff, None, None);
        match self
            .table
            .claim(
                &line.binding.tenant_id,
                &line.binding.id,
                None,
                line.binding.max_appearances,
                entry,
                None,
            )
            .await
        {
            Ok(Some(index)) => {
                self.handoffs.insert(handoff.clone(), call_id);
                Inbound::Claimed(CallAppearance {
                    tenant_id: line.binding.tenant_id,
                    line_binding_id: line.binding.id,
                    number: line.number.clone(),
                    index,
                    side: LegSide::B,
                    handoff,
                })
            }
            Ok(None) => Inbound::Busy,
            Err(e) => {
                // The table is away: the call rings as an unshared line's
                // would rather than being refused.
                warn!(line = %line.number, error = %e, "No appearance could be claimed; the call rings without one");
                Inbound::Unkept
            }
        }
    }

    /// What an INVITE from a phone is to the shared lines.
    pub async fn outbound(&self, request: &Request, call_id: CallId) -> Outbound {
        let Some(from) = crate::registrar::from_user(request) else {
            return Outbound::NotShared;
        };
        let Some(line) = self.shared_line(&from).await else {
            return Outbound::NotShared;
        };
        let wanted = header(request, "Call-Info").and_then(|v| model::requested_index(&v));
        let tenant = line.binding.tenant_id;
        let now = now_secs();
        let appearances = self.appearances(&tenant, &line.binding.id).await;
        let contact = contact_of(request).map(|c| contact_key(&c));
        let mut replacing = None;
        let index = match wanted {
            Some(AppearanceIndex::Index(i)) => {
                match appearances.iter().find(|a| a.index == i && !a.lapsed(now)) {
                    Some(a) if a.state == AppearanceState::Held => {
                        let Some(held_call) =
                            a.call_id().and_then(|c| c.parse::<fcp_core::CallId>().ok())
                        else {
                            return Outbound::Refuse(403, "Appearance In Use");
                        };
                        let held_call = CallId::from_uuid(*held_call.as_uuid());
                        if a.node == self.node_id {
                            return Outbound::RetrieveHere(held_call);
                        }
                        return match (a.node_uri.clone(), a.handoff.clone()) {
                            (Some(node_uri), Some(handoff)) => {
                                Outbound::RetrieveThere { node_uri, handoff }
                            }
                            _ => Outbound::Refuse(480, "Appearance Unreachable"),
                        };
                    }
                    Some(a) if a.state == AppearanceState::Seized => {
                        if a.contact.as_deref().map(contact_key) != contact {
                            return Outbound::Refuse(403, "Appearance Seized by Another Phone");
                        }
                        replacing = Some(a.owner.clone());
                        Some(i)
                    }
                    Some(_) => return Outbound::Refuse(403, "Appearance In Use"),
                    None => Some(i),
                }
            }
            // No index named: this phone's own seize if it holds one, else
            // the lowest free.
            _ => match appearances.iter().find(|a| {
                a.state == AppearanceState::Seized
                    && a.contact.as_deref().map(contact_key) == contact
            }) {
                Some(a) => {
                    replacing = Some(a.owner.clone());
                    Some(a.index)
                }
                None => None,
            },
        };
        let handoff = uuid::Uuid::new_v4().simple().to_string();
        let entry = self.call_entry(
            call_id,
            AppearanceState::Progressing,
            &handoff,
            contact_of(request),
            None,
        );
        match self
            .table
            .claim(
                &tenant,
                &line.binding.id,
                index,
                line.binding.max_appearances,
                entry,
                replacing.as_deref(),
            )
            .await
        {
            Ok(Some(index)) => {
                if let Some(owner) = &replacing {
                    // The seize is the call now.
                    if let Some(sip_call_id) = owner.strip_prefix("seize:") {
                        if let Some(sub) = self.seizes.get(sip_call_id) {
                            *sub.seized.lock().expect("seized") = None;
                        }
                    }
                }
                self.handoffs.insert(handoff.clone(), call_id);
                Outbound::Claimed(CallAppearance {
                    tenant_id: tenant,
                    line_binding_id: line.binding.id,
                    number: line.number,
                    index,
                    side: LegSide::A,
                    handoff,
                })
            }
            Ok(None) => Outbound::Refuse(486, "All Appearances In Use"),
            Err(e) => {
                warn!(line = %line.number, error = %e, "No appearance could be claimed; the call is placed without one");
                Outbound::NotShared
            }
        }
    }

    /// The call on this node a handoff token lets another node retrieve.
    pub fn handoff_call(&self, token: &str) -> Option<CallId> {
        self.handoffs.get(token.trim()).map(|c| *c)
    }

    fn call_entry(
        &self,
        call_id: CallId,
        state: AppearanceState,
        handoff: &str,
        contact: Option<String>,
        remote: Option<String>,
    ) -> Appearance {
        Appearance {
            index: 0,
            state,
            owner: Appearance::call_owner(&call_id.to_string()),
            node: self.node_id.clone(),
            node_uri: Some(format!("sip:{}", self.host)),
            handoff: Some(handoff.to_string()),
            contact,
            remote,
            expires_at: now_secs() + CALL_LEASE_SECS,
        }
    }

    async fn call_changed(&self, change: Change) {
        let Change {
            call_id,
            appearance,
            state,
        } = change;
        let owner = Appearance::call_owner(&call_id.to_string());
        match state {
            None => {
                self.live.remove(&call_id);
                self.handoffs.remove(&appearance.handoff);
                if let Err(e) = self
                    .table
                    .release(
                        &appearance.tenant_id,
                        &appearance.line_binding_id,
                        appearance.index,
                        &owner,
                    )
                    .await
                {
                    warn!(call_id = %call_id, error = %e, "A call's appearance could not be freed");
                }
            }
            Some(state) => {
                self.live.insert(call_id, (appearance.clone(), state));
                self.handoffs.insert(appearance.handoff.clone(), call_id);
                self.write(&call_id, &appearance, state).await;
            }
        }
        self.announce(
            appearance.tenant_id,
            appearance.line_binding_id,
            appearance.number.clone(),
            appearance.index,
            state,
            Some(call_id),
        );
    }

    async fn write(&self, call_id: &CallId, appearance: &CallAppearance, state: AppearanceState) {
        let (contact, remote) = self
            .call_manager()
            .and_then(|cm| cm.get_call(call_id))
            .map(|call| {
                let a = Some(call.a_leg.remote_uri.clone());
                let b = call.b_leg.as_ref().map(|b| b.remote_uri.clone());
                match appearance.side {
                    LegSide::A => (a, b),
                    LegSide::B => (b, a),
                }
            })
            .unwrap_or_default();
        let entry = self.call_entry(*call_id, state, &appearance.handoff, contact, remote);
        match self
            .table
            .claim(
                &appearance.tenant_id,
                &appearance.line_binding_id,
                Some(appearance.index),
                appearance.index,
                entry,
                None,
            )
            .await
        {
            Ok(Some(_)) => {}
            Ok(None) => {
                warn!(call_id = %call_id, index = appearance.index, "A call's appearance was taken by another")
            }
            Err(e) => {
                warn!(call_id = %call_id, error = %e, "A call's appearance could not be written")
            }
        }
    }

    /// Keep this node's calls' appearances from lapsing, and forget the
    /// subscriptions that lapsed.
    async fn refresh(&self) {
        let live: Vec<(CallId, CallAppearance, AppearanceState)> = self
            .live
            .iter()
            .map(|e| (*e.key(), e.value().0.clone(), e.value().1))
            .collect();
        for (call_id, appearance, state) in live {
            self.write(&call_id, &appearance, state).await;
        }
        self.watchers.retain(|_, w| !w.lapsed());
        self.seizes.retain(|_, s| !s.lapsed());
    }
}

/// The request's `Event` package, parameters dropped, lower case.
fn event_of(request: &Request) -> Option<String> {
    header(request, "Event").map(|e| {
        e.split(';')
            .next()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase()
    })
}

fn header(request: &Request, name: &str) -> Option<String> {
    sip_parse::header(request.headers(), name).map(|v| v.to_string())
}

fn expires_of(request: &Request) -> Option<u32> {
    header(request, "Expires").and_then(|v| v.trim().parse().ok())
}

/// The user part of the Request-URI.
fn request_user(request: &Request) -> Option<String> {
    let uri = request.uri().to_string();
    let rest = uri
        .strip_prefix("sip:")
        .or_else(|| uri.strip_prefix("sips:"))
        .unwrap_or(&uri);
    let (user, _) = rest.split_once('@')?;
    Some(user.split(';').next().unwrap_or(user).to_string())
}

/// The request's `Contact` URI, brackets and parameters outside them
/// dropped.
fn contact_of(request: &Request) -> Option<String> {
    let contact = header(request, "Contact")?;
    let uri = match contact.split_once('<') {
        Some((_, rest)) => rest.split('>').next().unwrap_or(rest),
        None => contact.split(';').next().unwrap_or(&contact),
    };
    Some(uri.trim().to_string())
}

/// Two contacts are one phone: the same user, host and port.
fn contact_key(contact: &str) -> String {
    let uri = contact
        .trim()
        .trim_start_matches('<')
        .split('>')
        .next()
        .unwrap_or(contact);
    let rest = uri
        .strip_prefix("sip:")
        .or_else(|| uri.strip_prefix("sips:"))
        .unwrap_or(uri);
    rest.split(';').next().unwrap_or(rest).to_ascii_lowercase()
}

/// A header value's `tag` parameter.
fn tag_of(value: &str) -> Option<String> {
    let after = match value.rfind('>') {
        Some(i) => &value[i + 1..],
        None => value,
    };
    after.split(';').find_map(|p| {
        let (name, v) = p.split_once('=')?;
        name.trim()
            .eq_ignore_ascii_case("tag")
            .then(|| v.trim().to_string())
    })
}

/// A `From`/`To` value without its `tag` parameter.
fn without_tag(value: &str) -> String {
    let (head, params) = match value.rfind('>') {
        Some(i) => (&value[..=i], &value[i + 1..]),
        None => match value.find(';') {
            Some(i) => (&value[..i], &value[i..]),
            None => (value, ""),
        },
    };
    let kept: Vec<&str> = params
        .split(';')
        .filter(|p| {
            let p = p.trim();
            !p.is_empty() && !p.to_ascii_lowercase().starts_with("tag=")
        })
        .collect();
    let mut out = head.trim().to_string();
    for p in kept {
        out.push(';');
        out.push_str(p.trim());
    }
    out
}

async fn respond(
    handle: &sip_transaction::ServerTransactionHandle,
    request: &Request,
    code: u16,
    reason: &str,
) -> anyhow::Result<()> {
    let response = sip_uas::UserAgentServer::create_response(request, code, reason);
    handle.send_final(response).await;
    Ok(())
}

/// A 200 to a SUBSCRIBE: our tag on the `To`, the granted `Expires`.
async fn respond_ok(
    handle: &sip_transaction::ServerTransactionHandle,
    request: &Request,
    our_tag: Option<&str>,
    expires: u32,
) -> anyhow::Result<()> {
    let mut response = sip_uas::UserAgentServer::create_response(request, 200, "OK");
    if let (Some(tag), Some(to)) = (our_tag, header(request, "To")) {
        if tag_of(&to).is_none() {
            let _ = response
                .headers_mut()
                .set_or_push("To", format!("{to};tag={tag}"));
        }
    }
    let _ = response
        .headers_mut()
        .push(SmolStr::new("Expires"), SmolStr::new(expires.to_string()));
    handle.send_final(response).await;
    Ok(())
}

/// A NOTIFY inside `sub`.
fn notify_request(
    sub: &Subscription,
    host: &str,
    cseq: u32,
    event: &str,
    state: &str,
    call_info: Option<&str>,
) -> anyhow::Result<Request> {
    let mut headers = Headers::new();
    let branch = format!("z9hG4bK{}", uuid::Uuid::new_v4().simple());
    let mut push = |name: &str, value: String| {
        headers
            .push(SmolStr::new(name), SmolStr::new(value))
            .map_err(|e| anyhow::anyhow!("{name}: {e}"))
    };
    push("Via", format!("SIP/2.0/UDP placeholder;branch={branch}"))?;
    push("Max-Forwards", "70".into())?;
    push(
        "From",
        format!("<sip:{}@{host}>;tag={}", sub.number, sub.our_tag),
    )?;
    let to = if sub.their_tag.is_empty() {
        sub.subscriber.clone()
    } else {
        format!("{};tag={}", sub.subscriber, sub.their_tag)
    };
    push("To", to)?;
    push("Call-ID", sub.call_id.clone())?;
    push("CSeq", format!("{cseq} NOTIFY"))?;
    push("Contact", format!("<sip:{host}>"))?;
    push("Event", event.to_string())?;
    push("Subscription-State", state.to_string())?;
    if let Some(value) = call_info {
        push("Call-Info", value.to_string())?;
    }
    push("Content-Length", "0".into())?;
    Request::new(
        RequestLine::new(Method::Notify, sub.contact.clone()),
        headers,
        Bytes::new(),
    )
    .map_err(|e| anyhow::anyhow!("NOTIFY: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(owner: &str, expires_at: u64) -> Appearance {
        Appearance {
            index: 0,
            state: AppearanceState::Active,
            owner: owner.into(),
            node: "n".into(),
            node_uri: None,
            handoff: None,
            contact: None,
            remote: None,
            expires_at,
        }
    }

    #[tokio::test]
    async fn the_lowest_free_index_is_claimed_and_its_owner_keeps_it() {
        let table = LocalAppearances::default();
        let (t, l) = (TenantId::nil(), LineBindingId::new());
        let far = now_secs() + 60;
        assert_eq!(
            table
                .claim(&t, &l, None, 2, entry("call:a", far), None)
                .await
                .unwrap(),
            Some(1)
        );
        assert_eq!(
            table
                .claim(&t, &l, None, 2, entry("call:b", far), None)
                .await
                .unwrap(),
            Some(2)
        );
        assert_eq!(
            table
                .claim(&t, &l, None, 2, entry("call:c", far), None)
                .await
                .unwrap(),
            None,
            "a line with every appearance in use takes no more"
        );
        assert_eq!(
            table
                .claim(&t, &l, Some(1), 2, entry("call:a", far), None)
                .await
                .unwrap(),
            Some(1),
            "the owner moves its own appearance on"
        );
        assert_eq!(
            table
                .claim(&t, &l, Some(1), 2, entry("call:c", far), None)
                .await
                .unwrap(),
            None
        );
        table.release(&t, &l, 1, "call:c").await.unwrap();
        assert_eq!(
            table.list(&t, &l).await.unwrap().len(),
            2,
            "only the owner frees an index"
        );
        table.release(&t, &l, 1, "call:a").await.unwrap();
        assert_eq!(
            table
                .claim(&t, &l, None, 2, entry("call:c", far), None)
                .await
                .unwrap(),
            Some(1)
        );
    }

    #[tokio::test]
    async fn a_seize_becomes_the_call_and_a_lapsed_one_frees_its_key() {
        let table = LocalAppearances::default();
        let (t, l) = (TenantId::nil(), LineBindingId::new());
        let far = now_secs() + 60;
        table
            .claim(&t, &l, Some(3), 4, entry("seize:x", far), None)
            .await
            .unwrap();
        assert_eq!(
            table
                .claim(&t, &l, Some(3), 4, entry("call:y", far), None)
                .await
                .unwrap(),
            None
        );
        assert_eq!(
            table
                .claim(&t, &l, Some(3), 4, entry("call:y", far), Some("seize:x"))
                .await
                .unwrap(),
            Some(3)
        );
        table
            .claim(&t, &l, Some(2), 4, entry("seize:z", now_secs()), None)
            .await
            .unwrap();
        assert_eq!(
            table
                .claim(&t, &l, Some(2), 4, entry("call:w", far), None)
                .await
                .unwrap(),
            Some(2),
            "a seize whose time has passed holds nothing"
        );
    }

    #[test]
    fn subscription_headers_are_read_as_phones_write_them() {
        assert_eq!(tag_of("<sip:+1555@h>;tag=abc").as_deref(), Some("abc"));
        assert_eq!(tag_of("\"A;tag=x\" <sip:+1555@h;tag=no>"), None);
        assert_eq!(
            without_tag("<sip:+1555@h>;tag=abc;x=y"),
            "<sip:+1555@h>;x=y"
        );
        assert_eq!(without_tag("sip:+1555@h;tag=abc"), "sip:+1555@h");
        assert_eq!(
            contact_key("<sip:+1555@127.0.0.1:5070;transport=udp>"),
            "+1555@127.0.0.1:5070"
        );
    }
}
