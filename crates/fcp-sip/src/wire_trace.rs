//! A call's SIP trace as it crossed the wire.
//!
//! Every message the node receives ([`WireTrace::received`], from the
//! handler's `handle_inbound`) and every message it sends
//! ([`WireTrace::sent`], from [`crate::transport::RealTransportDispatcher`]
//! once the send succeeded) is recorded as its bytes, with the peer's real
//! address and this node's one `ip:port`. Before, the handler recorded at
//! sixteen places of its own and rebuilt a BYE it never sent: a trace with
//! a branch no response matched, responses with no destination, a B-leg's
//! INVITE "to" a URI, and the ACKs and replies it sent missing, which drew
//! a ladder with stray columns and arrows to nowhere.
//!
//! A message is filed under the call its Call-ID belongs to (the handler's
//! Call-ID map). A B-leg's Call-ID is mapped only once its INVITE has been
//! sent, so a message of an INVITE dialog with a Call-ID not mapped yet is
//! held for up to [`HOLD`] and filed when it is; others (a REGISTER, an
//! OPTIONS ping) belong to no call and are not recorded.

use dashmap::DashMap;
use std::collections::VecDeque;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};
use tokio::sync::mpsc;

/// How long a message waits for its Call-ID to be mapped to a call.
pub const HOLD: Duration = Duration::from_secs(10);
/// The most messages held at once; the oldest go first.
const HOLD_MAX: usize = 1000;
/// The methods of an INVITE dialog: what a message waiting for its call can
/// be.
const DIALOG_METHODS: &[&str] = &[
    "INVITE", "ACK", "CANCEL", "BYE", "PRACK", "UPDATE", "INFO", "REFER", "NOTIFY",
];

/// Where the dispatcher reports what it sent.
pub trait WireTap: Send + Sync {
    fn sent(&self, transport: &str, peer: SocketAddr, payload: &[u8]);
}

pub struct WireTrace {
    tx: mpsc::Sender<fcp_core::SipTraceMessage>,
    calls: Arc<DashMap<String, String>>,
    /// This node as every trace names it: its advertised `ip:port`.
    local: String,
    node_id: String,
    held: Mutex<VecDeque<(Instant, fcp_core::SipTraceMessage)>>,
}

impl WireTrace {
    pub fn new(
        tx: mpsc::Sender<fcp_core::SipTraceMessage>,
        calls: Arc<DashMap<String, String>>,
        local: String,
        node_id: String,
    ) -> Self {
        Self {
            tx,
            calls,
            local,
            node_id,
            held: Mutex::new(VecDeque::new()),
        }
    }

    pub fn received(&self, transport: &str, peer: SocketAddr, payload: &[u8]) {
        self.record("inbound", transport, peer, payload);
    }

    fn record(&self, direction: &str, transport: &str, peer: SocketAddr, payload: &[u8]) {
        let Some(message) = Parsed::of(payload) else {
            return;
        };
        let (source_addr, dest_addr) = match direction {
            "inbound" => (peer.to_string(), self.local.clone()),
            _ => (self.local.clone(), peer.to_string()),
        };
        let trace = fcp_core::SipTraceMessage {
            id: 0,
            call_id: String::new(),
            sip_call_id: message.call_id.clone(),
            tenant_id: "_system".to_string(),
            timestamp: SystemTime::now(),
            direction: direction.to_string(),
            method_or_status: message.method_or_status,
            cseq: message.cseq,
            from_uri: message.from_uri,
            to_uri: message.to_uri,
            source_addr,
            dest_addr,
            transport: transport.to_ascii_lowercase(),
            raw_message: String::from_utf8_lossy(payload).to_string(),
            node_id: Some(self.node_id.clone()),
        };
        let mut held = self.held.lock().unwrap_or_else(|e| e.into_inner());
        self.file_held(&mut held);
        match self.calls.get(&message.call_id) {
            Some(call) => self.send(trace, call.value()),
            None if DIALOG_METHODS.contains(&message.cseq_method.as_str()) => {
                if held.len() >= HOLD_MAX {
                    held.pop_front();
                }
                held.push_back((Instant::now(), trace));
            }
            None => {}
        }
    }

    /// File every held message whose call is known now, in order, and let
    /// go of those that waited too long.
    fn file_held(&self, held: &mut VecDeque<(Instant, fcp_core::SipTraceMessage)>) {
        held.retain(|(at, trace)| {
            if let Some(call) = self.calls.get(&trace.sip_call_id) {
                self.send(trace.clone(), call.value());
                false
            } else {
                at.elapsed() < HOLD
            }
        });
    }

    fn send(&self, mut trace: fcp_core::SipTraceMessage, call_id: &str) {
        trace.call_id = call_id.to_string();
        if let Err(mpsc::error::TrySendError::Full(_)) = self.tx.try_send(trace) {
            tracing::warn!(call_id, "SIP trace channel full, dropping entry");
        }
    }
}

impl WireTap for WireTrace {
    fn sent(&self, transport: &str, peer: SocketAddr, payload: &[u8]) {
        self.record("outbound", transport, peer, payload);
    }
}

/// What a trace row needs from a message.
struct Parsed {
    method_or_status: String,
    call_id: String,
    cseq: String,
    cseq_method: String,
    from_uri: String,
    to_uri: String,
}

impl Parsed {
    fn of(payload: &[u8]) -> Option<Self> {
        let text = std::str::from_utf8(payload).ok()?;
        let head = text.split("\r\n\r\n").next()?;
        let mut lines = head.split("\r\n");
        let first = lines.next()?.trim();
        let method_or_status = if let Some(status) = first.strip_prefix("SIP/2.0 ") {
            status.split_whitespace().next()?.to_string()
        } else if first.ends_with(" SIP/2.0") {
            first.split_whitespace().next()?.to_string()
        } else {
            // A keepalive, or not SIP.
            return None;
        };
        let (mut call_id, mut cseq, mut from, mut to) = (None, None, None, None);
        for line in lines {
            let Some((name, value)) = line.split_once(':') else {
                continue;
            };
            let value = value.trim();
            match name.trim().to_ascii_lowercase().as_str() {
                "call-id" | "i" => call_id = Some(value),
                "cseq" => cseq = Some(value),
                "from" | "f" => from = Some(value),
                "to" | "t" => to = Some(value),
                _ => {}
            }
        }
        let cseq = cseq.unwrap_or("").to_string();
        Some(Self {
            method_or_status,
            call_id: call_id?.to_string(),
            cseq_method: cseq.split_whitespace().nth(1).unwrap_or("").to_string(),
            cseq,
            from_uri: uri_of(from.unwrap_or("")),
            to_uri: uri_of(to.unwrap_or("")),
        })
    }
}

/// The URI of a From or To value: inside `<…>`, else before its parameters.
fn uri_of(value: &str) -> String {
    match (value.find('<'), value.find('>')) {
        (Some(open), Some(close)) if open < close => value[open + 1..close].to_string(),
        _ => value.split(';').next().unwrap_or("").trim().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn invite(call_id: &str) -> String {
        format!(
            "INVITE sip:bob@192.0.2.9:61185 SIP/2.0\r\n\
             Via: SIP/2.0/UDP 203.0.113.7:5060;branch=z9hG4bK1;rport\r\n\
             From: \"Alice\" <sip:alice@example.com>;tag=a\r\n\
             To: <sip:bob@192.0.2.9:61185>\r\n\
             Call-ID: {call_id}\r\n\
             CSeq: 1 INVITE\r\n\
             Content-Length: 0\r\n\r\n"
        )
    }

    fn trace() -> (
        WireTrace,
        Arc<DashMap<String, String>>,
        mpsc::Receiver<fcp_core::SipTraceMessage>,
    ) {
        let (tx, rx) = mpsc::channel(16);
        let calls = Arc::new(DashMap::new());
        let trace = WireTrace::new(
            tx,
            Arc::clone(&calls),
            "203.0.113.7:5060".into(),
            "cm-1".into(),
        );
        (trace, calls, rx)
    }

    #[test]
    fn a_message_is_recorded_as_sent_with_both_addresses() {
        let (trace, calls, mut rx) = trace();
        calls.insert("c1".into(), "call-1".into());
        let peer: SocketAddr = "192.0.2.9:61185".parse().unwrap();
        trace.sent("UDP", peer, invite("c1").as_bytes());
        let row = rx.try_recv().unwrap();
        assert_eq!(row.call_id, "call-1");
        assert_eq!(row.direction, "outbound");
        assert_eq!(row.method_or_status, "INVITE");
        assert_eq!(row.source_addr, "203.0.113.7:5060");
        assert_eq!(row.dest_addr, "192.0.2.9:61185");
        assert_eq!(row.transport, "udp");
        assert_eq!(row.from_uri, "sip:alice@example.com");
        assert_eq!(row.cseq, "1 INVITE");
        assert_eq!(row.raw_message, invite("c1"));

        let ok =
            "SIP/2.0 200 OK\r\ni: c1\r\nCSeq: 1 INVITE\r\nt: <sip:bob@192.0.2.9>;tag=b\r\n\r\n";
        trace.received("UDP", peer, ok.as_bytes());
        let row = rx.try_recv().unwrap();
        assert_eq!(row.method_or_status, "200");
        assert_eq!(row.source_addr, "192.0.2.9:61185");
        assert_eq!(row.dest_addr, "203.0.113.7:5060");
        assert_eq!(row.to_uri, "sip:bob@192.0.2.9");
    }

    #[test]
    fn a_b_leg_s_invite_waits_for_its_call_and_keeps_its_place() {
        let (trace, calls, mut rx) = trace();
        let peer: SocketAddr = "192.0.2.9:61185".parse().unwrap();
        trace.sent("UDP", peer, invite("b-leg").as_bytes());
        assert!(
            rx.try_recv().is_err(),
            "held: the Call-ID is not mapped yet"
        );
        calls.insert("b-leg".into(), "call-1".into());
        let trying = "SIP/2.0 100 Trying\r\nCall-ID: b-leg\r\nCSeq: 1 INVITE\r\n\r\n";
        trace.received("UDP", peer, trying.as_bytes());
        assert_eq!(rx.try_recv().unwrap().method_or_status, "INVITE");
        assert_eq!(rx.try_recv().unwrap().method_or_status, "100");
    }

    #[test]
    fn what_belongs_to_no_call_is_not_recorded() {
        let (trace, _calls, mut rx) = trace();
        let peer: SocketAddr = "192.0.2.9:5060".parse().unwrap();
        let register =
            "REGISTER sip:example.com SIP/2.0\r\nCall-ID: r1\r\nCSeq: 7 REGISTER\r\n\r\n";
        trace.received("UDP", peer, register.as_bytes());
        trace.received("UDP", peer, b"\r\n\r\n");
        assert!(trace.held.lock().unwrap().is_empty());
        assert!(rx.try_recv().is_err());
    }
}
