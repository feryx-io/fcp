//! Waking phone apps (`docs/PUSH.md`): which of a device's bindings can be
//! woken, and pushing to them. A token its provider no longer knows costs
//! the binding its push address ([`FcpRegistrar::forget_push`]).

use std::sync::Arc;

use fcp_core::storage::Registration;
use fcp_core::{Device, DynStorage};
use fcp_push::{PushKind, PushMessage, PushOutcome, PushSender};
use serde::Serialize;

use crate::registrar::FcpRegistrar;

/// A device's bindings that carry a push address: at the device's own
/// address of record, else at its owner's, the one at the address the
/// device last registered from (an app registers as its user, with
/// credentials of its own; `docs/SOFTPHONE.md` §2.1).
pub async fn device_push_bindings(
    registrar: &FcpRegistrar,
    storage: &DynStorage,
    device: &Device,
) -> Vec<Registration> {
    let realm = registrar.realm().to_string();
    let rows = |aor: String| async move {
        registrar
            .get_registration_rows_in_tenant(&device.tenant_id, &aor)
            .await
            .unwrap_or_default()
            .into_iter()
            .filter(|r| r.push.is_some())
            .collect::<Vec<_>>()
    };
    if let Some(uri) = device.sip_uri.as_deref() {
        let found = rows(uri.to_string()).await;
        if !found.is_empty() {
            return found;
        }
    }
    if let Some(name) = device.sip_username.as_deref() {
        let found = rows(format!("sip:{name}@{realm}")).await;
        if !found.is_empty() {
            return found;
        }
    }
    let Some(owner) = device.owner_user_id.as_ref() else {
        return Vec::new();
    };
    let Ok(Some(owner)) = fcp_core::CredentialStore::get_user_by_id(storage, owner).await else {
        return Vec::new();
    };
    let Some(address) = device.last_ip_address.as_deref().filter(|a| !a.is_empty()) else {
        return Vec::new();
    };
    rows(format!("sip:{}@{realm}", owner.sip_name()))
        .await
        .into_iter()
        .filter(|r| r.contact.contains(&format!("@{address}")))
        .collect()
}

/// What one push did.
#[derive(Debug, Clone, Serialize)]
pub struct PushResult {
    pub provider: String,
    /// The token, shortened.
    pub token: String,
    /// `sent`, `gone` or `failed`.
    pub outcome: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Push to each binding, and take the push address off any whose token is
/// gone.
pub async fn push_bindings(
    sender: &PushSender,
    registrar: &FcpRegistrar,
    rows: &[Registration],
    message: &PushMessage,
    device_id: Option<String>,
) -> Vec<PushResult> {
    let mut results = Vec::new();
    for row in rows {
        let Some(address) = row.push.as_ref() else {
            continue;
        };
        let outcome = sender.send(address, message).await;
        if let PushOutcome::Gone(reason) = &outcome {
            if let Err(e) = registrar.forget_push(row, device_id.clone(), reason).await {
                tracing::warn!(error = %e, "Could not take a dead token off its binding");
            }
        }
        results.push(PushResult {
            provider: address.provider.clone(),
            token: address.shown_prid(),
            outcome: outcome.label().to_string(),
            reason: match outcome {
                PushOutcome::Sent => None,
                PushOutcome::Gone(r) | PushOutcome::Failed(r) => Some(r),
            },
        });
    }
    results
}

/// An administrator's test push to a device (`POST /devices/{id}/push-test`):
/// a `test` push to each of its bindings that can be woken. The app shows
/// it and ends it at once.
pub async fn test_device(
    sender: &Arc<PushSender>,
    registrar: &FcpRegistrar,
    storage: &DynStorage,
    device: &Device,
) -> Vec<PushResult> {
    let rows = device_push_bindings(registrar, storage, device).await;
    let message = PushMessage::new(
        PushKind::Test,
        format!("test-{}", uuid::Uuid::new_v4().simple()),
        device.tenant_id.to_string(),
    );
    push_bindings(
        sender,
        registrar,
        &rows,
        &message,
        Some(device.id.to_string()),
    )
    .await
}

// ===== A phone's flow on another node (`docs/PUSH.md` §2.4) =====

/// The flow token a node relays a call down: one of the receiving node's
/// own, which only a node that read the binding can know.
pub const FLOW_RELAY_HEADER: &str = "X-FCP-Flow";

/// The phone's contact, which the relayed leg is sent to.
pub const FLOW_RELAY_CONTACT_HEADER: &str = "X-FCP-Flow-Contact";

/// The Request-URI parameter naming the flow a relay leg is for, so a 430
/// that comes back can be told apart.
pub const FLOW_RELAY_URI_PARAM: &str = "fcp-flow";

/// A call relayed to this node down a flow it holds.
#[derive(Debug, Clone)]
pub struct FlowRelay {
    /// The flow's registration's tenant: the call is theirs.
    pub tenant: fcp_core::TenantId,
    pub token: String,
    pub contact: String,
    /// The call it is a leg of, on the node that sent it.
    pub for_call: Option<fcp_core::CallId>,
}

/// The flow token in a relay leg's Request-URI, if it is one.
pub fn relay_token(target: &str) -> Option<&str> {
    target
        .split(';')
        .skip(1)
        .find_map(|p| p.strip_prefix(FLOW_RELAY_URI_PARAM)?.strip_prefix('='))
}

// ===== Ringing a sleeping app (`docs/PUSH.md` §2.2) =====

/// A woken app's new binding, for its call's fork to ring.
#[derive(Debug, Clone)]
pub struct LateContact {
    /// The URI to send the INVITE to.
    pub uri: String,
    /// Its RFC 5626 flow, when it came over one, and the node holding it.
    pub flow_token: Option<String>,
    pub flow_node: Option<String>,
}

/// One binding a call may wake.
struct WakeTarget {
    /// As it was stored when the call began.
    row: Registration,
    /// Its flow is on another node, which the call reaches first; it is
    /// pushed only if that node says the flow is gone (430).
    deferred: bool,
    pushed: bool,
    /// Back (its new binding went to the fork), or never coming (its push
    /// was not taken).
    done: bool,
}

/// One call's wait for the apps it woke.
struct CallWake {
    tenant_id: fcp_core::TenantId,
    message: PushMessage,
    targets: Vec<WakeTarget>,
    /// The call's fork, once it runs; what came back before it did waits.
    late: Option<tokio::sync::mpsc::Sender<LateContact>>,
    pending: Vec<LateContact>,
}

impl CallWake {
    /// Nothing more can come back: every target back or given up (a
    /// deferred one may yet be pushed, on a 430).
    fn settled(&self) -> bool {
        self.targets.iter().all(|t| t.done)
    }
}

/// The calls waiting for the apps they woke, by call.
#[derive(Default)]
pub struct PushWaits {
    calls: dashmap::DashMap<fcp_b2bua_core::CallId, CallWake>,
}

/// Whether `row` is the app `target` woke, come back: the same phone (its
/// instance, or its token) registered again since the binding the push
/// went to — another REGISTER (its Call-ID and CSeq), or another address.
/// (Not by time: a store may keep a binding's time to the second.)
fn woken_by(target: &Registration, row: &Registration) -> bool {
    let same_instance = target.instance_id.is_some() && row.instance_id == target.instance_id;
    let same_token = match (&target.push, &row.push) {
        (Some(a), Some(b)) => a.prid == b.prid,
        _ => false,
    };
    let registered_again =
        row.call_id != target.call_id || row.cseq != target.cseq || row.contact != target.contact;
    (same_instance || same_token) && registered_again
}

/// The URI of a stored contact, without its brackets.
pub fn stored_contact_uri(contact: &str) -> String {
    match (contact.find('<'), contact.find('>')) {
        (Some(start), Some(end)) if start < end => contact[start + 1..end].to_string(),
        _ => contact.split(';').next().unwrap_or(contact).to_string(),
    }
}

impl PushWaits {
    /// A call may wake these bindings, pushed with `message`: each `true`
    /// is deferred (its flow is on another node, tried first).
    pub fn begin(
        &self,
        call: fcp_b2bua_core::CallId,
        tenant_id: fcp_core::TenantId,
        targets: Vec<(Registration, bool)>,
        message: PushMessage,
    ) {
        let targets = targets.into_iter().map(|(row, deferred)| WakeTarget {
            row,
            deferred,
            pushed: false,
            done: false,
        });
        // A call planned twice keeps what it already waits for.
        if let Some(mut wait) = self.calls.get_mut(&call) {
            for target in targets {
                if !wait
                    .targets
                    .iter()
                    .any(|t| t.row.contact == target.row.contact)
                {
                    wait.targets.push(target);
                }
            }
            return;
        }
        self.calls.insert(
            call,
            CallWake {
                tenant_id,
                message,
                targets: targets.collect(),
                late: None,
                pending: Vec::new(),
            },
        );
    }

    pub fn has(&self, call: &fcp_b2bua_core::CallId) -> bool {
        self.calls.contains_key(call)
    }

    /// Whether the call has bindings to push now (not deferred, not yet
    /// pushed).
    pub fn has_to_push(&self, call: &fcp_b2bua_core::CallId) -> bool {
        self.calls.get(call).is_some_and(|w| {
            w.targets
                .iter()
                .any(|t| !t.deferred && !t.pushed && !t.done)
        })
    }

    /// The bindings to push now, marked pushed, by index; and what to say.
    pub fn take_to_push(
        &self,
        call: &fcp_b2bua_core::CallId,
    ) -> Option<(Vec<(usize, Registration)>, PushMessage)> {
        let mut wait = self.calls.get_mut(call)?;
        let mut out = Vec::new();
        for (i, t) in wait.targets.iter_mut().enumerate() {
            if !t.deferred && !t.pushed && !t.done {
                t.pushed = true;
                out.push((i, t.row.clone()));
            }
        }
        Some((out, wait.message.clone()))
    }

    /// The node holding a deferred binding's flow says it is gone (430):
    /// it is to be pushed now. Its index and row, when the call has it.
    pub fn undefer(&self, call: &fcp_b2bua_core::CallId, flow_token: &str) -> Option<usize> {
        let mut wait = self.calls.get_mut(call)?;
        let (i, t) = wait.targets.iter_mut().enumerate().find(|(_, t)| {
            t.deferred && !t.pushed && t.row.flow_token.as_deref() == Some(flow_token)
        })?;
        t.deferred = false;
        Some(i)
    }

    /// The node holding a deferred binding's flow reached the phone, or
    /// could not be asked: it is not pushed.
    pub fn forgo(&self, call: &fcp_b2bua_core::CallId, flow_token: &str) {
        if let Some(mut wait) = self.calls.get_mut(call) {
            for t in wait.targets.iter_mut() {
                if t.deferred && !t.pushed && t.row.flow_token.as_deref() == Some(flow_token) {
                    t.done = true;
                }
            }
            if wait.settled() {
                wait.late = None;
            }
        }
    }

    /// The call's fork is running: apps that come back from now on reach
    /// it, and those that came back already go at once.
    pub fn attach(
        &self,
        call: &fcp_b2bua_core::CallId,
        late: tokio::sync::mpsc::Sender<LateContact>,
    ) {
        if let Some(mut wait) = self.calls.get_mut(call) {
            for contact in std::mem::take(&mut wait.pending) {
                let _ = late.try_send(contact);
            }
            // Nothing more to come: the fork's channel closes once it has
            // what came.
            if !wait.settled() {
                wait.late = Some(late);
            }
        }
    }

    /// These targets will not come back (their push was not taken): the
    /// fork stops waiting for them.
    pub fn abandon(&self, call: &fcp_b2bua_core::CallId, indices: &[usize]) {
        if let Some(mut wait) = self.calls.get_mut(call) {
            for &i in indices {
                if let Some(t) = wait.targets.get_mut(i) {
                    t.done = true;
                }
            }
            if wait.settled() {
                wait.late = None;
            }
        }
    }

    /// The calls waiting on an app at `aor` in a tenant.
    pub fn waiting_at(
        &self,
        tenant_id: &fcp_core::TenantId,
        aor: &str,
    ) -> Vec<fcp_b2bua_core::CallId> {
        self.calls
            .iter()
            .filter(|w| {
                w.tenant_id == *tenant_id
                    && w.targets
                        .iter()
                        .any(|t| t.pushed && !t.done && t.row.aor == aor)
            })
            .map(|w| *w.key())
            .collect()
    }

    /// The bindings now at `aor` (`rows`): each that is an app this call
    /// woke goes to its fork, once.
    pub fn deliver(&self, call: &fcp_b2bua_core::CallId, rows: &[Registration]) -> usize {
        let Some(mut wait) = self.calls.get_mut(call) else {
            return 0;
        };
        let mut found = Vec::new();
        for (i, target) in wait.targets.iter().enumerate() {
            if !target.pushed || target.done {
                continue;
            }
            if let Some(row) = rows.iter().find(|row| woken_by(&target.row, row)) {
                found.push((
                    i,
                    LateContact {
                        uri: stored_contact_uri(&row.contact),
                        flow_token: row.flow_token.clone(),
                        flow_node: row.flow_node.clone(),
                    },
                ));
            }
        }
        let n = found.len();
        for (i, contact) in found {
            wait.targets[i].done = true;
            match &wait.late {
                Some(late) => {
                    let _ = late.try_send(contact);
                }
                None => wait.pending.push(contact),
            }
        }
        if wait.settled() {
            wait.late = None;
        }
        n
    }

    /// The call is over: the apps it pushed that never came back, to be
    /// told so with a cancel push.
    pub fn end(&self, call: &fcp_b2bua_core::CallId) -> Option<(Vec<Registration>, PushMessage)> {
        let (_, wait) = self.calls.remove(call)?;
        let missing = wait
            .targets
            .into_iter()
            .filter(|t| t.pushed && !t.done)
            .map(|t| t.row)
            .collect();
        let mut cancel = wait.message;
        cancel.kind = PushKind::Cancel;
        Some((missing, cancel))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, SystemTime};

    fn row(contact: &str, instance: Option<&str>, prid: &str, at: SystemTime) -> Registration {
        static CSEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);
        Registration {
            tenant_id: fcp_core::TenantId::nil(),
            aor: "sip:alice@x".into(),
            contact: contact.into(),
            expires_at: at + Duration::from_secs(600),
            call_id: "c".into(),
            cseq: CSEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            user_agent: None,
            q: None,
            registered_at: at,
            path: None,
            flow_token: Some("flow-new".into()),
            source_address: None,
            instance_id: instance.map(str::to_string),
            reg_id: None,
            push: Some(fcp_core::push::PushAddress {
                provider: "fcm".into(),
                param: "p".into(),
                prid: prid.into(),
            }),
            flow_node: None,
        }
    }

    #[test]
    fn a_woken_app_reaches_its_call_once_by_instance_or_token() {
        let waits = PushWaits::default();
        let call = fcp_b2bua_core::CallId::new();
        let before = SystemTime::now() - Duration::from_secs(60);
        let asleep = row("<sip:app@10.0.0.7:1>", Some("<urn:a>"), "tok", before);
        waits.begin(
            call,
            fcp_core::TenantId::nil(),
            vec![(asleep.clone(), false)],
            PushMessage::new(PushKind::Call, "c1", "t"),
        );
        let (to_push, _) = waits.take_to_push(&call).unwrap();
        assert_eq!(to_push.len(), 1);
        assert_eq!(
            waits.waiting_at(&fcp_core::TenantId::nil(), "sip:alice@x"),
            vec![call]
        );

        // The old binding itself, and another phone's, are not it.
        assert_eq!(waits.deliver(&call, std::slice::from_ref(&asleep)), 0);
        let other = row(
            "<sip:x@10.0.0.9:2>",
            Some("<urn:b>"),
            "other",
            SystemTime::now(),
        );
        assert_eq!(waits.deliver(&call, &[other]), 0);

        // Back before the fork runs: kept, then handed over on attach.
        let back = row(
            "<sip:app@198.51.100.4:9>",
            Some("<urn:a>"),
            "tok2",
            SystemTime::now(),
        );
        assert_eq!(waits.deliver(&call, std::slice::from_ref(&back)), 1);
        let (tx, mut rx) = tokio::sync::mpsc::channel(4);
        waits.attach(&call, tx);
        let late = rx.try_recv().unwrap();
        assert_eq!(late.uri, "sip:app@198.51.100.4:9");
        assert_eq!(late.flow_token.as_deref(), Some("flow-new"));
        // Once.
        assert_eq!(waits.deliver(&call, &[back]), 0);
        assert!(waits
            .waiting_at(&fcp_core::TenantId::nil(), "sip:alice@x")
            .is_empty());
        let (missing, cancel) = waits.end(&call).unwrap();
        assert!(missing.is_empty());
        assert_eq!(cancel.kind, PushKind::Cancel);
    }

    #[test]
    fn without_an_instance_the_token_finds_it_and_the_rest_are_cancelled() {
        let waits = PushWaits::default();
        let call = fcp_b2bua_core::CallId::new();
        let before = SystemTime::now() - Duration::from_secs(60);
        let a = row("<sip:a@10.0.0.7:1>", None, "tok-a", before);
        let b = row("<sip:b@10.0.0.8:1>", None, "tok-b", before);
        waits.begin(
            call,
            fcp_core::TenantId::nil(),
            vec![(a, false), (b, false)],
            PushMessage::new(PushKind::Call, "c1", "t"),
        );
        waits.take_to_push(&call).unwrap();
        let back = row("<sip:a@198.51.100.4:9>", None, "tok-a", SystemTime::now());
        assert_eq!(waits.deliver(&call, &[back]), 1);
        let (missing, _) = waits.end(&call).unwrap();
        assert_eq!(missing.len(), 1);
        assert_eq!(missing[0].push.as_ref().unwrap().prid, "tok-b");
        assert!(!waits.has(&call));
    }

    #[test]
    fn a_deferred_app_is_pushed_only_when_its_node_says_its_flow_is_gone() {
        let waits = PushWaits::default();
        let call = fcp_b2bua_core::CallId::new();
        let before = SystemTime::now() - Duration::from_secs(60);
        let mut elsewhere = row("<sip:app@10.0.0.7:1>", Some("<urn:a>"), "tok", before);
        elsewhere.flow_token = Some("flow-on-b".into());
        waits.begin(
            call,
            fcp_core::TenantId::nil(),
            vec![(elsewhere.clone(), true)],
            PushMessage::new(PushKind::Call, "c1", "t"),
        );
        // Nothing to push at first, and no cancel for what was never pushed.
        assert!(!waits.has_to_push(&call));
        assert!(waits.take_to_push(&call).unwrap().0.is_empty());
        // Its node answers 430: it is pushed.
        assert_eq!(waits.undefer(&call, "flow-on-b"), Some(0));
        assert!(waits.has_to_push(&call));
        assert_eq!(waits.take_to_push(&call).unwrap().0.len(), 1);
        assert_eq!(waits.undefer(&call, "flow-on-b"), None);
        let (missing, _) = waits.end(&call).unwrap();
        assert_eq!(missing.len(), 1, "pushed and never back: cancelled");

        let quiet = fcp_b2bua_core::CallId::new();
        waits.begin(
            quiet,
            fcp_core::TenantId::nil(),
            vec![(elsewhere, true)],
            PushMessage::new(PushKind::Call, "c2", "t"),
        );
        let (missing, _) = waits.end(&quiet).unwrap();
        assert!(missing.is_empty(), "never pushed: nothing to cancel");
    }
}
