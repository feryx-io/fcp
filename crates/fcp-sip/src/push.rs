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
