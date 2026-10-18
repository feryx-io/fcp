//! BFCP floor control for SIP room systems (design §15.6, 7e).
//!
//! A room system that shares a screen the way RFC 8856 has it offers an
//! `m=application … UDP/BFCP` (or `TCP/BFCP`) section beside its
//! `a=content:slides` one and asks for the floor with a `FloorRequest`
//! before it sends a frame. This node is its floor control server: the
//! SDP answer names the conference, the user and the floor
//! ([`BfcpAssignment`]), one socket ([`ConferenceSection::bfcp_bind`])
//! takes every endpoint's messages — UDP datagrams sorted by conference
//! and user id, TCP connections bound to a user by their first message —
//! and each request is put to the room's content floor, the same floor a
//! browser or a BFCP-less phone takes by sending. A BFCP endpoint never
//! takes it by sending: its content is refused until a request is
//! granted, and a host's stop, an idle timeout or a peer node's older
//! share come back to it as `Revoked`.
//!
//! The protocol is `forge_bfcp`'s; this module is the glue between its
//! [`FloorServer`] — one per room and transport — and the room.
//!
//! [`ConferenceSection::bfcp_bind`]: crate::config::ConferenceSection::bfcp_bind

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use fcp_media::BfcpTransport;
use forge_bfcp::{
    Event, FloorServer, Header, Message, Notification, TcpFramer, Transport, UdpInbound, UdpPeer,
};
use forge_conference::video::ContentRefusal;
use forge_conference::ConferenceRoom;

use crate::rooms::RoomKey;
use tokio::net::{TcpListener, UdpSocket};
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

/// The one floor a conference has: its shared screen.
pub const FLOOR_ID: u16 = 1;
/// How often UDP transactions are retransmitted or given up on.
const TICK: Duration = Duration::from_millis(100);

/// What the SDP answer tells a BFCP endpoint (RFC 8856): the ids its
/// messages carry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BfcpAssignment {
    pub transport: BfcpTransport,
    pub conference_id: u32,
    pub user_id: u16,
    pub floor_id: u16,
}

fn transport_of(t: BfcpTransport) -> Transport {
    match t {
        BfcpTransport::Udp => Transport::Unreliable,
        BfcpTransport::Tcp => Transport::Reliable,
    }
}

/// How a message reaches one endpoint.
enum Link {
    Udp {
        /// Where its datagrams come from, once one has.
        peer: Option<SocketAddr>,
        txn: UdpPeer,
    },
    Tcp {
        /// The connection's writer, once the endpoint has connected.
        tx: Option<mpsc::UnboundedSender<Vec<u8>>>,
    },
}

struct User {
    /// The room's participant, once the caller is admitted; the ids are
    /// handed out with the answer, before that.
    participant_id: Option<String>,
    /// When the ids were handed out: a reservation never bound to a
    /// participant (the call failed after its answer) is forgotten.
    reserved_at: Instant,
    link: Link,
    /// The meeting's policy for this participant, set when the content
    /// leg attaches; until then no request is granted.
    may_share: bool,
    /// A host stopped their share: no request is granted until their
    /// content section is renegotiated.
    host_stopped: bool,
    /// The request that holds the floor, if one does.
    granted: Option<u16>,
}

struct RoomFloor {
    /// The room, once a participant of it is bound.
    room: Option<Arc<ConferenceRoom>>,
    conference_id: u32,
    udp: Option<FloorServer>,
    tcp: Option<FloorServer>,
    users: HashMap<u16, User>,
}

impl RoomFloor {
    fn server(&mut self, transport: Transport) -> &mut FloorServer {
        let slot = match transport {
            Transport::Unreliable => &mut self.udp,
            Transport::Reliable => &mut self.tcp,
        };
        slot.get_or_insert_with(|| FloorServer::new(self.conference_id, FLOOR_ID, transport))
    }

    fn user_of(&self, participant_id: &str) -> Option<u16> {
        self.users
            .iter()
            .find(|(_, u)| u.participant_id.as_deref() == Some(participant_id))
            .map(|(id, _)| *id)
    }

    fn video(&self) -> Option<Arc<forge_conference::video::VideoRoom>> {
        self.room.as_ref().and_then(|r| r.video())
    }
}

/// How long a reservation waits to be bound to a participant.
const RESERVATION_TTL: Duration = Duration::from_secs(120);

/// Bytes for one endpoint, sent once the lock is dropped.
enum Send {
    Udp(SocketAddr, Vec<u8>),
    Tcp(mpsc::UnboundedSender<Vec<u8>>, Vec<u8>),
}

#[derive(Default)]
struct Inner {
    rooms: HashMap<RoomKey, RoomFloor>,
    /// Conference id → room number, for sorting inbound messages.
    conferences: HashMap<u32, RoomKey>,
    next_conference: u32,
    next_user: u16,
}

/// Every room's floor control on this node.
pub struct Floors {
    inner: Mutex<Inner>,
    udp: OnceLock<Arc<UdpSocket>>,
}

impl Default for Floors {
    fn default() -> Self {
        Floors::new()
    }
}

impl Floors {
    pub fn new() -> Floors {
        Floors {
            inner: Mutex::new(Inner {
                next_conference: 1,
                next_user: 1,
                ..Default::default()
            }),
            udp: OnceLock::new(),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    // ---- the conference's side --------------------------------------------

    /// A caller whose offer carries a BFCP section is handed its ids for
    /// the answer. The room may not exist yet: the caller is bound to it
    /// with [`bind`](Self::bind) once admitted, and a reservation that
    /// never is — the call failed after its answer — is forgotten.
    pub fn reserve(&self, room_number: &RoomKey, transport: BfcpTransport) -> BfcpAssignment {
        let mut inner = self.lock();
        let conference_id = match inner.rooms.get(room_number) {
            Some(rf) => rf.conference_id,
            None => {
                let id = loop {
                    let id = inner.next_conference;
                    inner.next_conference = inner.next_conference.wrapping_add(1).max(1);
                    if !inner.conferences.contains_key(&id) {
                        break id;
                    }
                };
                inner.conferences.insert(id, room_number.clone());
                inner.rooms.insert(
                    room_number.clone(),
                    RoomFloor {
                        room: None,
                        conference_id: id,
                        udp: None,
                        tcp: None,
                        users: HashMap::new(),
                    },
                );
                id
            }
        };
        let user_id = loop {
            let id = inner.next_user;
            inner.next_user = inner.next_user.wrapping_add(1).max(1);
            let taken = inner.rooms.values().any(|rf| rf.users.contains_key(&id));
            if !taken {
                break id;
            }
        };
        let rf = inner.rooms.get_mut(room_number).expect("just inserted");
        rf.server(transport_of(transport))
            .add_user(user_id, None, None);
        rf.users.insert(
            user_id,
            User {
                participant_id: None,
                reserved_at: Instant::now(),
                link: match transport {
                    BfcpTransport::Udp => Link::Udp {
                        peer: None,
                        txn: UdpPeer::new(),
                    },
                    BfcpTransport::Tcp => Link::Tcp { tx: None },
                },
                may_share: false,
                host_stopped: false,
                granted: None,
            },
        );
        debug!(room = %room_number, conference_id, user_id, transport = transport.proto(), "BFCP ids reserved");
        BfcpAssignment {
            transport,
            conference_id,
            user_id,
            floor_id: FLOOR_ID,
        }
    }

    /// The caller is in the room: their requests can be put to its floor.
    pub fn bind(
        &self,
        room_number: &RoomKey,
        user_id: u16,
        room: &Arc<ConferenceRoom>,
        participant_id: &str,
    ) {
        let mut inner = self.lock();
        let Some(rf) = inner.rooms.get_mut(room_number) else {
            return;
        };
        rf.room = Some(Arc::clone(room));
        if let Some(u) = rf.users.get_mut(&user_id) {
            u.participant_id = Some(participant_id.to_string());
            info!(room = %room_number, participant = %participant_id, user_id, "BFCP endpoint admitted");
        }
    }

    /// The meeting's policy for the participant, known once their content
    /// leg attaches. A BFCP endpoint never takes the floor by sending:
    /// its implicit sending is switched off here, whatever the policy.
    pub fn set_may_share(&self, room_number: &RoomKey, participant_id: &str, may_share: bool) {
        let mut inner = self.lock();
        let Some(rf) = inner.rooms.get_mut(room_number) else {
            return;
        };
        let Some(id) = rf.user_of(participant_id) else {
            return;
        };
        if let Some(u) = rf.users.get_mut(&id) {
            u.may_share = may_share;
            u.host_stopped = false;
        }
        if let Some(video) = rf.video() {
            video.set_participant_content_enabled(participant_id, false);
        }
    }

    /// The caller left (or never made it in): their ids, and the floor
    /// they held, go with them.
    pub fn leave(&self, room_number: &RoomKey, user_id: u16) {
        let sends = {
            let mut inner = self.lock();
            let Some(rf) = inner.rooms.get_mut(room_number) else {
                return;
            };
            let id = user_id;
            let user = rf.users.remove(&id);
            let mut sends = Vec::new();
            if let Some(user) = user {
                let t = match user.link {
                    Link::Udp { .. } => Transport::Unreliable,
                    Link::Tcp { .. } => Transport::Reliable,
                };
                let (notifications, _) = rf.server(t).remove_user(id);
                deliver_all(rf, notifications, Instant::now(), &mut sends);
            }
            if rf.users.is_empty() {
                let cid = rf.conference_id;
                inner.rooms.remove(room_number);
                inner.conferences.remove(&cid);
            }
            sends
        };
        self.send_all(sends);
    }

    /// The room's floor left the participant for `reason` (the pump's
    /// notice): a BFCP endpoint hears `Revoked` unless it released the
    /// floor itself, and a host's stop keeps it from asking again until
    /// its content section is renegotiated.
    pub fn floor_released(&self, room_number: &RoomKey, participant_id: &str, reason: &str) {
        let sends = {
            let mut inner = self.lock();
            let Some(rf) = inner.rooms.get_mut(room_number) else {
                return;
            };
            let Some(id) = rf.user_of(participant_id) else {
                return;
            };
            let Some(user) = rf.users.get_mut(&id) else {
                return;
            };
            if reason == "host" {
                user.host_stopped = true;
            }
            let Some(request_id) = user.granted.take() else {
                return;
            };
            let t = match user.link {
                Link::Udp { .. } => Transport::Unreliable,
                Link::Tcp { .. } => Transport::Reliable,
            };
            let notifications = rf.server(t).revoke(request_id, Some(reason));
            if let Some(video) = rf.video() {
                video.set_participant_content_enabled(participant_id, false);
            }
            let mut sends = Vec::new();
            deliver_all(rf, notifications, Instant::now(), &mut sends);
            sends
        };
        self.send_all(sends);
    }

    /// The participant's BFCP state for the API.
    pub fn participant_json(
        &self,
        room_number: &RoomKey,
        user_id: u16,
    ) -> Option<serde_json::Value> {
        let inner = self.lock();
        let rf = inner.rooms.get(room_number)?;
        let id = user_id;
        let user = rf.users.get(&id)?;
        let (transport, connected) = match &user.link {
            Link::Udp { peer, .. } => ("udp", peer.is_some()),
            Link::Tcp { tx } => ("tcp", tx.is_some()),
        };
        let server = match &user.link {
            Link::Udp { .. } => rf.udp.as_ref(),
            Link::Tcp { .. } => rf.tcp.as_ref(),
        };
        let pending = server.is_some_and(|s| {
            s.requests()
                .any(|r| r.user_id == id && r.status == forge_bfcp::RequestStatus::Pending)
        });
        Some(serde_json::json!({
            "transport": transport,
            "conference_id": rf.conference_id,
            "user_id": id,
            "floor_id": FLOOR_ID,
            "connected": connected,
            "floor": if user.granted.is_some() { "granted" } else if pending { "pending" } else { "none" },
            "host_stopped": user.host_stopped,
        }))
    }

    // ---- the wire's side --------------------------------------------------

    /// A datagram from an endpoint.
    pub fn datagram(&self, from: SocketAddr, bytes: &[u8]) {
        let now = Instant::now();
        let sends = {
            let mut inner = self.lock();
            let Ok(header) = Header::parse(bytes) else {
                debug!(%from, "BFCP datagram without a readable header");
                return;
            };
            let Some(room_number) = inner.conferences.get(&header.conference_id).cloned() else {
                debug!(%from, conference_id = header.conference_id, "BFCP datagram for no conference here");
                return;
            };
            let Some(rf) = inner.rooms.get_mut(&room_number) else {
                return;
            };
            let mut sends = Vec::new();
            let sorted = match rf.users.get_mut(&header.user_id) {
                Some(User {
                    link: Link::Udp { peer, txn },
                    ..
                }) => {
                    *peer = Some(from);
                    txn.inbound(bytes, now)
                }
                Some(_) => {
                    debug!(%from, user_id = header.user_id, "BFCP datagram from a TCP endpoint's user");
                    return;
                }
                // An unknown user: the server answers with the error.
                None => UdpInbound::Request,
            };
            match sorted {
                UdpInbound::Replay(bytes) => sends.push(Send::Udp(from, bytes)),
                UdpInbound::Acked { .. } => {
                    if let Some(User {
                        link: Link::Udp { txn, .. },
                        ..
                    }) = rf.users.get_mut(&header.user_id)
                    {
                        for bytes in txn.tick(now).send {
                            sends.push(Send::Udp(from, bytes));
                        }
                    }
                }
                UdpInbound::Stray => {}
                UdpInbound::Request | UdpInbound::Unreadable => {
                    process(
                        rf,
                        Transport::Unreliable,
                        bytes,
                        Some(from),
                        now,
                        &mut sends,
                    );
                }
            }
            sends
        };
        self.send_all(sends);
    }

    /// A message from a TCP connection; `tx` is the connection's writer,
    /// bound to the user the message names.
    fn tcp_message(&self, tx: &mpsc::UnboundedSender<Vec<u8>>, message: &Message) {
        let now = Instant::now();
        let Ok(bytes) = message.to_bytes() else {
            return;
        };
        let sends = {
            let mut inner = self.lock();
            let Some(room_number) = inner.conferences.get(&message.conference_id).cloned() else {
                // Answered by a server that does not know the conference.
                let reply = Message::error(
                    message.echo(),
                    forge_bfcp::ErrorCode::ConferenceDoesNotExist,
                    None,
                );
                if let Ok(b) = reply.to_bytes() {
                    let _ = tx.send(b);
                }
                return;
            };
            let Some(rf) = inner.rooms.get_mut(&room_number) else {
                return;
            };
            if let Some(User {
                link: Link::Tcp { tx: slot },
                ..
            }) = rf.users.get_mut(&message.user_id)
            {
                *slot = Some(tx.clone());
            }
            let mut sends = Vec::new();
            process(rf, Transport::Reliable, &bytes, None, now, &mut sends);
            sends
        };
        self.send_all(sends);
    }

    /// Retransmit what is due on every UDP endpoint, and notice the ones
    /// that have stopped answering.
    pub fn tick(&self) {
        let now = Instant::now();
        let sends = {
            let mut inner = self.lock();
            let mut sends = Vec::new();
            for (room_number, rf) in inner.rooms.iter_mut() {
                let stale: Vec<u16> = rf
                    .users
                    .iter()
                    .filter(|(_, u)| {
                        u.participant_id.is_none()
                            && now.saturating_duration_since(u.reserved_at) > RESERVATION_TTL
                    })
                    .map(|(id, _)| *id)
                    .collect();
                for id in stale {
                    debug!(room = %room_number, user_id = id, "BFCP reservation never bound; forgotten");
                    rf.users.remove(&id);
                    for t in [Transport::Unreliable, Transport::Reliable] {
                        rf.server(t).remove_user(id);
                    }
                }
                for (id, user) in rf.users.iter_mut() {
                    if let Link::Udp { peer, txn } = &mut user.link {
                        let tick = txn.tick(now);
                        if let Some(peer) = peer {
                            for bytes in tick.send {
                                sends.push(Send::Udp(*peer, bytes));
                            }
                        }
                        if tick.broken {
                            warn!(room = %room_number, user_id = id, participant = ?user.participant_id, "BFCP endpoint stopped acknowledging");
                        }
                    }
                }
            }
            sends
        };
        self.send_all(sends);
    }

    fn send_all(&self, sends: Vec<Send>) {
        for s in sends {
            match s {
                Send::Udp(to, bytes) => {
                    if let Some(socket) = self.udp.get() {
                        if let Err(e) = socket.try_send_to(&bytes, to) {
                            debug!(%to, error = %e, "BFCP send failed");
                        }
                    }
                }
                Send::Tcp(tx, bytes) => {
                    let _ = tx.send(bytes);
                }
            }
        }
    }

    // ---- the sockets --------------------------------------------------------

    /// Serve endpoints over UDP on `socket`, forever.
    pub async fn serve_udp(self: Arc<Self>, socket: Arc<UdpSocket>) {
        let _ = self.udp.set(Arc::clone(&socket));
        let mut buf = vec![0u8; 2048];
        loop {
            match socket.recv_from(&mut buf).await {
                Ok((n, from)) => self.datagram(from, &buf[..n]),
                Err(e) => {
                    warn!(error = %e, "BFCP UDP socket failed");
                    tokio::time::sleep(Duration::from_millis(200)).await;
                }
            }
        }
    }

    /// Serve endpoints over TCP on `listener`, forever: each connection
    /// is read into frames and written from a channel, and belongs to
    /// whichever user its messages name.
    pub async fn serve_tcp(self: Arc<Self>, listener: TcpListener) {
        loop {
            let (stream, from) = match listener.accept().await {
                Ok(c) => c,
                Err(e) => {
                    warn!(error = %e, "BFCP TCP accept failed");
                    tokio::time::sleep(Duration::from_millis(200)).await;
                    continue;
                }
            };
            debug!(%from, "BFCP TCP connection");
            let floors = Arc::clone(&self);
            tokio::spawn(async move {
                use tokio::io::{AsyncReadExt, AsyncWriteExt};
                let (mut rd, mut wr) = stream.into_split();
                let (tx, mut rx) = mpsc::unbounded_channel::<Vec<u8>>();
                let writer = tokio::spawn(async move {
                    while let Some(bytes) = rx.recv().await {
                        if wr.write_all(&bytes).await.is_err() {
                            break;
                        }
                    }
                });
                let mut framer = TcpFramer::new();
                let mut buf = vec![0u8; 2048];
                'conn: loop {
                    let n = match rd.read(&mut buf).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => n,
                    };
                    framer.push(&buf[..n]);
                    loop {
                        match framer.next_message() {
                            Ok(Some(m)) => floors.tcp_message(&tx, &m),
                            Ok(None) => break,
                            Err(e) => {
                                // Answered when the header was readable; the
                                // stream goes on only if the frame was cut out.
                                if let Some(echo) = e.echo {
                                    let reply = Message::error(echo, e.error_code(), None);
                                    if let Ok(b) = reply.to_bytes() {
                                        let _ = tx.send(b);
                                    }
                                }
                                if !matches!(
                                    e.kind,
                                    forge_bfcp::ParseErrorKind::UnknownPrimitive(_)
                                ) {
                                    break 'conn;
                                }
                            }
                        }
                    }
                }
                drop(tx);
                let _ = writer.await;
                debug!(%from, "BFCP TCP connection closed");
            });
        }
    }

    /// Run the UDP timers, forever.
    pub async fn serve_ticks(self: Arc<Self>) {
        let mut ticker = tokio::time::interval(TICK);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            self.tick();
        }
    }
}

/// Run one message through the room's floor server and the room's floor,
/// queueing what goes back.
fn process(
    rf: &mut RoomFloor,
    transport: Transport,
    bytes: &[u8],
    from: Option<SocketAddr>,
    now: Instant,
    sends: &mut Vec<Send>,
) {
    let handled = rf.server(transport).handle(bytes);
    if let Some(reply) = handled.reply {
        let user_id = reply.user_id;
        match rf.users.get_mut(&user_id) {
            Some(user) => deliver(user, reply, true, now, sends),
            // An error to a user the room does not know: straight back.
            None => {
                if let (Some(from), Ok(b)) = (from, reply.to_bytes()) {
                    sends.push(Send::Udp(from, b));
                }
            }
        }
    }
    deliver_all(rf, handled.notifications, now, sends);
    for event in handled.events {
        match event {
            Event::Requested {
                user_id,
                request_id,
                ..
            } => decide(rf, transport, user_id, request_id, now, sends),
            Event::Released { user_id, .. } => release(rf, user_id),
            Event::Goodbye { user_id, .. } => release(rf, user_id),
            Event::Hello { .. } | Event::Acked { .. } => {}
        }
    }
}

/// A `FloorRequest` put to the room's floor: granted or denied at once.
fn decide(
    rf: &mut RoomFloor,
    transport: Transport,
    user_id: u16,
    request_id: u16,
    now: Instant,
    sends: &mut Vec<Send>,
) {
    let Some(user) = rf.users.get(&user_id) else {
        return;
    };
    let pid = user.participant_id.clone().unwrap_or_default();
    let refused: Option<String> = if user.participant_id.is_none() {
        Some("not in the conference yet".into())
    } else if user.host_stopped {
        Some("stopped by the host".into())
    } else if !user.may_share {
        Some("sharing is not allowed for you in this meeting".into())
    } else {
        match rf.video() {
            None => Some("this meeting has no video".into()),
            Some(video) => {
                video.set_participant_content_enabled(&pid, true);
                match video.request_content(&pid) {
                    Ok(()) => None,
                    Err(why) => {
                        video.set_participant_content_enabled(&pid, false);
                        Some(refusal_text(&why).to_string())
                    }
                }
            }
        }
    };
    let notifications = match refused {
        None => {
            if let Some(u) = rf.users.get_mut(&user_id) {
                u.granted = Some(request_id);
            }
            info!(conference_id = rf.conference_id, participant = %pid, request_id, "BFCP floor granted");
            rf.server(transport).grant(request_id)
        }
        Some(why) => {
            debug!(conference_id = rf.conference_id, participant = %pid, request_id, why = %why, "BFCP floor denied");
            rf.server(transport).deny(request_id, Some(&why))
        }
    };
    deliver_all(rf, notifications, now, sends);
}

fn refusal_text(why: &ContentRefusal) -> &'static str {
    match why {
        ContentRefusal::Held(_) => "someone else is sharing",
        ContentRefusal::Disabled => "sharing is off in this meeting",
        ContentRefusal::ParticipantDisabled => "stopped by the host",
        ContentRefusal::NoSource => "no content section to share on",
        ContentRefusal::Budget => "no capacity for a share on this node",
    }
}

/// The endpoint gave the floor up, or left.
fn release(rf: &mut RoomFloor, user_id: u16) {
    let Some(user) = rf.users.get_mut(&user_id) else {
        return;
    };
    user.granted = None;
    let Some(pid) = user.participant_id.clone() else {
        return;
    };
    if let Some(video) = rf.video() {
        video.release_content(&pid);
        video.set_participant_content_enabled(&pid, false);
    }
}

fn deliver_all(
    rf: &mut RoomFloor,
    notifications: Vec<Notification>,
    now: Instant,
    sends: &mut Vec<Send>,
) {
    for n in notifications {
        if let Some(user) = rf.users.get_mut(&n.user_id) {
            deliver(user, n.message, false, now, sends);
        }
    }
}

/// Queue a message for one endpoint: a response is cached for replay
/// over UDP, a notification waits its turn.
fn deliver(user: &mut User, message: Message, response: bool, now: Instant, sends: &mut Vec<Send>) {
    match &mut user.link {
        Link::Udp { peer, txn } => {
            let Some(peer) = *peer else {
                debug!(participant = ?user.participant_id, "BFCP message with nowhere to send it yet");
                return;
            };
            if response {
                if let Ok(bytes) = message.to_bytes() {
                    txn.cache_response(message.transaction_id, bytes.clone(), now);
                    sends.push(Send::Udp(peer, bytes));
                }
            } else if let Some(bytes) = txn.notify(&message, now) {
                sends.push(Send::Udp(peer, bytes));
            }
        }
        Link::Tcp { tx } => {
            if let (Some(tx), Ok(bytes)) = (tx.as_ref(), message.to_bytes()) {
                sends.push(Send::Tcp(tx.clone(), bytes));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_bfcp::server::reported_status;
    use forge_bfcp::{
        Attribute, ErrorCode, Message, Primitive, RequestStatus, VERSION_RELIABLE,
        VERSION_UNRELIABLE,
    };
    use forge_conference::video::{ContentRefusal, StreamKind, VideoBackend, VideoRoomSettings};
    use forge_conference::AudioFormat;
    use forge_core::VideoCodec;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// A room with video and three callers who each have a content
    /// section to share on.
    fn room() -> Arc<ConferenceRoom> {
        let room = Arc::new(
            ConferenceRoom::new(
                "5000",
                AudioFormat::pcm_mono(),
                160,
                forge_mixer::MixerOptions::default(),
            )
            .unwrap(),
        );
        for (pid, host) in [("alice", true), ("bob", false), ("carol", false)] {
            room.add_participant(pid, host).unwrap();
        }
        let video = room.enable_video(
            VideoRoomSettings {
                content: true,
                codecs: vec![VideoCodec::VP8],
                ..VideoRoomSettings::default()
            },
            &VideoBackend::raw(),
        );
        for pid in ["alice", "bob", "carol"] {
            video
                .add_source_kind(pid, StreamKind::Content, VideoCodec::VP8, "")
                .unwrap();
        }
        room
    }

    async fn udp_recv(sock: &UdpSocket) -> Message {
        let mut buf = [0u8; 1500];
        let (n, _) = tokio::time::timeout(Duration::from_secs(3), sock.recv_from(&mut buf))
            .await
            .expect("timed out waiting for a BFCP message")
            .unwrap();
        Message::parse(&buf[..n]).expect("a BFCP message")
    }

    async fn udp_ask(sock: &UdpSocket, to: SocketAddr, m: &Message) -> Message {
        sock.send_to(&m.to_bytes().unwrap(), to).await.unwrap();
        udp_recv(sock).await
    }

    /// Acknowledge a server-initiated message over UDP.
    async fn udp_ack(sock: &UdpSocket, to: SocketAddr, m: &Message) {
        let ack = match m.primitive {
            Primitive::FloorRequestStatus => Primitive::FloorRequestStatusAck,
            Primitive::FloorStatus => Primitive::FloorStatusAck,
            other => panic!("nothing to ack in {other}"),
        };
        let a = Message::response(m.echo(), ack);
        assert!(a.responder);
        sock.send_to(&a.to_bytes().unwrap(), to).await.unwrap();
    }

    fn status_info(m: &Message) -> Option<String> {
        m.floor_request_information().first().and_then(|(_, info)| {
            info.iter().find_map(|a| match a {
                Attribute::OverallRequestStatus { attributes, .. } => {
                    attributes.iter().find_map(|b| match b {
                        Attribute::StatusInfo(s) => Some(s.clone()),
                        _ => None,
                    })
                }
                _ => None,
            })
        })
    }

    /// The scripted client of §15.6 over UDP: hello, a request granted
    /// through the room's floor, a second one denied while it is held, a
    /// release, a retransmission answered from the cache, a host's stop
    /// coming back as Revoked, the policy, a query, and a leave.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_room_system_takes_and_loses_the_floor_over_udp() {
        let rk = RoomKey::new(fcp_core::TenantId::new(), "5000");
        let room = room();
        let video = room.video().unwrap();
        let floors = Arc::new(Floors::new());
        let server = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
        let to = server.local_addr().unwrap();
        let serving = tokio::spawn(Arc::clone(&floors).serve_udp(Arc::clone(&server)));
        let ticking = tokio::spawn(Arc::clone(&floors).serve_ticks());

        // The answer's ids: one conference for the room, a user each.
        let a = floors.reserve(&rk, BfcpTransport::Udp);
        let b = floors.reserve(&rk, BfcpTransport::Udp);
        assert_eq!(a.conference_id, b.conference_id);
        assert_ne!(a.user_id, b.user_id);
        assert_eq!(a.floor_id, FLOOR_ID);
        floors.bind(&rk, a.user_id, &room, "alice");
        floors.bind(&rk, b.user_id, &room, "bob");
        floors.set_may_share(&rk, "alice", true);
        // Bob's policy comes later: until then he is refused.
        let cid = a.conference_id;
        // A BFCP endpoint never takes the floor by sending.
        assert_eq!(
            video.request_content("alice"),
            Err(ContentRefusal::ParticipantDisabled)
        );

        let alice = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let bob = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let msg =
            |p: Primitive, txn: u16, user: u16| Message::new(VERSION_UNRELIABLE, p, cid, txn, user);

        // Hello, with the response flag and Alice's transaction.
        let ack = udp_ask(&alice, to, &msg(Primitive::Hello, 1, a.user_id)).await;
        assert_eq!(ack.primitive, Primitive::HelloAck);
        assert!(ack.responder);
        assert_eq!(ack.transaction_id, 1);
        // Someone the room does not know.
        let err = udp_ask(&alice, to, &msg(Primitive::Hello, 1, 999)).await;
        assert_eq!(err.error_code(), Some(ErrorCode::UserDoesNotExist));

        // A request: Pending in Alice's transaction, then Granted in the
        // server's, which she acknowledges.
        let pending = udp_ask(
            &alice,
            to,
            &msg(Primitive::FloorRequest, 2, a.user_id).with(Attribute::FloorId(FLOOR_ID)),
        )
        .await;
        let (rid, st) = reported_status(&pending).unwrap();
        assert_eq!(st, RequestStatus::Pending);
        let granted = udp_recv(&alice).await;
        assert_eq!(
            reported_status(&granted),
            Some((rid, RequestStatus::Granted))
        );
        assert!(!granted.responder);
        assert_ne!(granted.transaction_id, 0);
        udp_ack(&alice, to, &granted).await;
        assert_eq!(video.content_holder().as_deref(), Some("alice"));
        let json = floors.participant_json(&rk, a.user_id).unwrap();
        assert_eq!(json["floor"], "granted");
        assert_eq!(json["transport"], "udp");
        assert_eq!(json["connected"], true);

        // Bob asks while she holds it: denied, and told why.
        let pending = udp_ask(
            &bob,
            to,
            &msg(Primitive::FloorRequest, 1, b.user_id).with(Attribute::FloorId(FLOOR_ID)),
        )
        .await;
        assert_eq!(reported_status(&pending).unwrap().1, RequestStatus::Pending);
        let denied = udp_recv(&bob).await;
        assert_eq!(reported_status(&denied).unwrap().1, RequestStatus::Denied);
        assert_eq!(
            status_info(&denied).as_deref(),
            Some("sharing is not allowed for you in this meeting")
        );
        udp_ack(&bob, to, &denied).await;
        floors.set_may_share(&rk, "bob", true);
        let pending = udp_ask(
            &bob,
            to,
            &msg(Primitive::FloorRequest, 2, b.user_id).with(Attribute::FloorId(FLOOR_ID)),
        )
        .await;
        assert_eq!(reported_status(&pending).unwrap().1, RequestStatus::Pending);
        let denied = udp_recv(&bob).await;
        assert_eq!(
            status_info(&denied).as_deref(),
            Some("someone else is sharing")
        );
        udp_ack(&bob, to, &denied).await;

        // Alice releases: Released, the floor free, and her sending off
        // again. The same release again is answered from the cache.
        let release =
            msg(Primitive::FloorRelease, 3, a.user_id).with(Attribute::FloorRequestId(rid));
        let released = udp_ask(&alice, to, &release).await;
        assert_eq!(
            reported_status(&released),
            Some((rid, RequestStatus::Released))
        );
        assert!(video.content_holder().is_none());
        assert_eq!(
            video.request_content("alice"),
            Err(ContentRefusal::ParticipantDisabled)
        );
        let again = udp_ask(&alice, to, &release).await;
        assert_eq!(again, released);

        // Granted again, then stopped by the host: Revoked, with the
        // reason, and no request granted until renegotiated.
        let pending = udp_ask(
            &alice,
            to,
            &msg(Primitive::FloorRequest, 4, a.user_id).with(Attribute::FloorId(FLOOR_ID)),
        )
        .await;
        let (rid2, _) = reported_status(&pending).unwrap();
        let granted = udp_recv(&alice).await;
        assert_eq!(
            reported_status(&granted),
            Some((rid2, RequestStatus::Granted))
        );
        udp_ack(&alice, to, &granted).await;
        assert_eq!(video.stop_content().as_deref(), Some("alice"));
        floors.floor_released(&rk, "alice", "host");
        let revoked = udp_recv(&alice).await;
        assert_eq!(
            reported_status(&revoked),
            Some((rid2, RequestStatus::Revoked))
        );
        assert_eq!(status_info(&revoked).as_deref(), Some("host"));
        udp_ack(&alice, to, &revoked).await;
        assert_eq!(
            floors.participant_json(&rk, a.user_id).unwrap()["host_stopped"],
            true
        );
        let pending = udp_ask(
            &alice,
            to,
            &msg(Primitive::FloorRequest, 5, a.user_id).with(Attribute::FloorId(FLOOR_ID)),
        )
        .await;
        assert_eq!(reported_status(&pending).unwrap().1, RequestStatus::Pending);
        let denied = udp_recv(&alice).await;
        assert_eq!(status_info(&denied).as_deref(), Some("stopped by the host"));
        udp_ack(&alice, to, &denied).await;
        // Her content section renegotiated: allowed again.
        floors.set_may_share(&rk, "alice", true);
        let pending = udp_ask(
            &alice,
            to,
            &msg(Primitive::FloorRequest, 6, a.user_id).with(Attribute::FloorId(FLOOR_ID)),
        )
        .await;
        let (rid3, _) = reported_status(&pending).unwrap();
        let granted = udp_recv(&alice).await;
        assert_eq!(
            reported_status(&granted),
            Some((rid3, RequestStatus::Granted))
        );
        udp_ack(&alice, to, &granted).await;

        // Bob asks about the floor and is kept told: Alice's leaving
        // brings a FloorStatus with nobody on it.
        let status = udp_ask(
            &bob,
            to,
            &msg(Primitive::FloorQuery, 3, b.user_id).with(Attribute::FloorId(FLOOR_ID)),
        )
        .await;
        assert_eq!(status.primitive, Primitive::FloorStatus);
        assert_eq!(status.floor_request_information()[0].0, rid3);
        floors.leave(&rk, a.user_id);
        let status = udp_recv(&bob).await;
        assert_eq!(status.primitive, Primitive::FloorStatus);
        assert!(status.floor_request_information().is_empty());
        udp_ack(&bob, to, &status).await;
        assert!(floors.participant_json(&rk, a.user_id).is_none());
        // A message from her ids now: no such user.
        let err = udp_ask(&alice, to, &msg(Primitive::Hello, 7, a.user_id)).await;
        assert_eq!(err.error_code(), Some(ErrorCode::UserDoesNotExist));

        serving.abort();
        ticking.abort();
    }

    /// The same over TCP: version 1, notifications with transaction id 0
    /// and nothing to acknowledge, on a connection bound by its first
    /// message.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_room_system_takes_the_floor_over_tcp() {
        let rk = RoomKey::new(fcp_core::TenantId::new(), "5000");
        let room = room();
        let video = room.video().unwrap();
        let floors = Arc::new(Floors::new());
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let to = listener.local_addr().unwrap();
        let serving = tokio::spawn(Arc::clone(&floors).serve_tcp(listener));
        let c = floors.reserve(&rk, BfcpTransport::Tcp);
        floors.bind(&rk, c.user_id, &room, "carol");
        floors.set_may_share(&rk, "carol", true);
        let json = floors.participant_json(&rk, c.user_id).unwrap();
        assert_eq!(
            (json["transport"].as_str(), json["connected"].as_bool()),
            (Some("tcp"), Some(false))
        );

        let mut stream = tokio::net::TcpStream::connect(to).await.unwrap();
        let mut framer = TcpFramer::new();
        let mut buf = [0u8; 1500];
        let msg = |p: Primitive, txn: u16| {
            Message::new(VERSION_RELIABLE, p, c.conference_id, txn, c.user_id)
        };
        async fn next(
            stream: &mut tokio::net::TcpStream,
            framer: &mut TcpFramer,
            buf: &mut [u8],
        ) -> Message {
            loop {
                if let Some(m) = framer.next_message().unwrap() {
                    return m;
                }
                let n = tokio::time::timeout(Duration::from_secs(3), stream.read(buf))
                    .await
                    .expect("timed out waiting for a BFCP frame")
                    .unwrap();
                assert!(n > 0, "connection closed");
                framer.push(&buf[..n]);
            }
        }
        stream
            .write_all(&msg(Primitive::Hello, 1).to_bytes().unwrap())
            .await
            .unwrap();
        let ack = next(&mut stream, &mut framer, &mut buf).await;
        assert_eq!(ack.primitive, Primitive::HelloAck);
        assert!(!ack.responder);
        assert_eq!(
            floors.participant_json(&rk, c.user_id).unwrap()["connected"],
            true
        );

        stream
            .write_all(
                &msg(Primitive::FloorRequest, 2)
                    .with(Attribute::FloorId(FLOOR_ID))
                    .to_bytes()
                    .unwrap(),
            )
            .await
            .unwrap();
        let pending = next(&mut stream, &mut framer, &mut buf).await;
        let (rid, st) = reported_status(&pending).unwrap();
        assert_eq!(st, RequestStatus::Pending);
        assert_eq!(pending.transaction_id, 2);
        let granted = next(&mut stream, &mut framer, &mut buf).await;
        assert_eq!(
            reported_status(&granted),
            Some((rid, RequestStatus::Granted))
        );
        assert_eq!(granted.transaction_id, 0, "server-initiated over TCP");
        assert_eq!(video.content_holder().as_deref(), Some("carol"));

        stream
            .write_all(
                &msg(Primitive::FloorRelease, 3)
                    .with(Attribute::FloorRequestId(rid))
                    .to_bytes()
                    .unwrap(),
            )
            .await
            .unwrap();
        let released = next(&mut stream, &mut framer, &mut buf).await;
        assert_eq!(
            reported_status(&released),
            Some((rid, RequestStatus::Released))
        );
        assert!(video.content_holder().is_none());

        // Another conference's id on this node: refused.
        stream
            .write_all(
                &Message::new(
                    VERSION_RELIABLE,
                    Primitive::Hello,
                    c.conference_id + 7,
                    4,
                    c.user_id,
                )
                .to_bytes()
                .unwrap(),
            )
            .await
            .unwrap();
        let err = next(&mut stream, &mut framer, &mut buf).await;
        assert_eq!(err.error_code(), Some(ErrorCode::ConferenceDoesNotExist));
        serving.abort();
    }

    /// A reservation the call never bound is forgotten by the timers.
    #[test]
    fn an_unbound_reservation_is_forgotten() {
        let rk = RoomKey::new(fcp_core::TenantId::new(), "5000");
        let floors = Floors::new();
        let a = floors.reserve(&rk, BfcpTransport::Udp);
        assert!(floors.participant_json(&rk, a.user_id).is_some());
        {
            let mut inner = floors.lock();
            let rf = inner.rooms.get_mut(&rk).unwrap();
            rf.users.get_mut(&a.user_id).unwrap().reserved_at =
                Instant::now() - RESERVATION_TTL - Duration::from_secs(1);
        }
        floors.tick();
        assert!(floors.participant_json(&rk, a.user_id).is_none());
        // The room's floor itself stays until the last user leaves.
        let b = floors.reserve(&rk, BfcpTransport::Tcp);
        assert_eq!(
            a.conference_id, b.conference_id,
            "the room keeps its conference id"
        );
        floors.leave(&rk, b.user_id);
        assert!(!floors.lock().rooms.contains_key(&rk));
    }
}
