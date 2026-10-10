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

// ===== Ringing a sleeping app (`docs/PUSH.md` §2.2) =====

/// A woken app's new binding, for its call's fork to ring.
#[derive(Debug, Clone)]
pub struct LateContact {
    /// The URI to send the INVITE to.
    pub uri: String,
    /// Its RFC 5626 flow, when it came over one.
    pub flow_token: Option<String>,
}

/// One call's wait for the apps it woke.
struct CallWake {
    tenant_id: fcp_core::TenantId,
    message: PushMessage,
    /// The bindings pushed, as they were stored.
    targets: Vec<Registration>,
    /// Whether each target's app has come back.
    delivered: Vec<bool>,
    /// The call's fork, once it runs; what came back before it did waits.
    late: Option<tokio::sync::mpsc::Sender<LateContact>>,
    pending: Vec<LateContact>,
}

/// The calls waiting for apps they woke, by call.
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
    /// A call will wait for these apps, pushed with `message`.
    pub fn begin(
        &self,
        call: fcp_b2bua_core::CallId,
        tenant_id: fcp_core::TenantId,
        targets: Vec<Registration>,
        message: PushMessage,
    ) {
        // A call planned twice keeps what it already waits for.
        if let Some(mut wait) = self.calls.get_mut(&call) {
            for target in targets {
                if !wait.targets.iter().any(|t| t.contact == target.contact) {
                    wait.targets.push(target);
                    wait.delivered.push(false);
                }
            }
            return;
        }
        let delivered = vec![false; targets.len()];
        self.calls.insert(
            call,
            CallWake {
                tenant_id,
                message,
                targets,
                delivered,
                late: None,
                pending: Vec::new(),
            },
        );
    }

    pub fn has(&self, call: &fcp_b2bua_core::CallId) -> bool {
        self.calls.contains_key(call)
    }

    /// The bindings a call is to push, and what to say.
    pub fn targets(
        &self,
        call: &fcp_b2bua_core::CallId,
    ) -> Option<(Vec<Registration>, PushMessage)> {
        self.calls
            .get(call)
            .map(|w| (w.targets.clone(), w.message.clone()))
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
            // Every app already back or given up: the fork's channel closes
            // once it has what came.
            if !wait.delivered.iter().all(|d| *d) {
                wait.late = Some(late);
            }
        }
    }

    /// These targets will not come back (their push was not taken): the
    /// fork stops waiting for them, and is not told of them again.
    pub fn abandon(&self, call: &fcp_b2bua_core::CallId, indices: &[usize]) {
        if let Some(mut wait) = self.calls.get_mut(call) {
            for &i in indices {
                if let Some(d) = wait.delivered.get_mut(i) {
                    *d = true;
                }
            }
            if wait.delivered.iter().all(|d| *d) {
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
                        .zip(&w.delivered)
                        .any(|(t, done)| !done && t.aor == aor)
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
            if wait.delivered[i] {
                continue;
            }
            if let Some(row) = rows.iter().find(|row| woken_by(target, row)) {
                found.push((
                    i,
                    LateContact {
                        uri: stored_contact_uri(&row.contact),
                        flow_token: row.flow_token.clone(),
                    },
                ));
            }
        }
        let n = found.len();
        for (i, contact) in found {
            wait.delivered[i] = true;
            match &wait.late {
                Some(late) => {
                    let _ = late.try_send(contact);
                }
                None => wait.pending.push(contact),
            }
        }
        // Every app back: the fork's channel closes once it has them.
        if wait.delivered.iter().all(|d| *d) {
            wait.late = None;
        }
        n
    }

    /// The call is over: the apps it woke that never came back, to be
    /// told so with a cancel push.
    pub fn end(&self, call: &fcp_b2bua_core::CallId) -> Option<(Vec<Registration>, PushMessage)> {
        let (_, wait) = self.calls.remove(call)?;
        let missing = wait
            .targets
            .into_iter()
            .zip(wait.delivered)
            .filter(|(_, done)| !done)
            .map(|(t, _)| t)
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
            vec![asleep.clone()],
            PushMessage::new(PushKind::Call, "c1", "t"),
        );
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
            vec![a, b],
            PushMessage::new(PushKind::Call, "c1", "t"),
        );
        let back = row("<sip:a@198.51.100.4:9>", None, "tok-a", SystemTime::now());
        assert_eq!(waits.deliver(&call, &[back]), 1);
        let (missing, _) = waits.end(&call).unwrap();
        assert_eq!(missing.len(), 1);
        assert_eq!(missing[0].push.as_ref().unwrap().prid, "tok-b");
        assert!(!waits.has(&call));
    }
}
