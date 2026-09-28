//! Class of service on the call path (`docs/ENTERPRISE_CALLING.md` §3.7):
//! the class a user calls under, and whether it lets them reach what they
//! dialled.

use fcp_core::class_of_service::{classify, effective_class, Classified, Dialing};
use fcp_core::storage::PhoneNumberStore;
use fcp_core::{
    ClassOfService, ClassOfServiceStore, DestinationClass, DynStorage, TenantId, TenantStore,
    UserId,
};
use tracing::warn;

/// The class a user calls under: their own, else the tenant's default,
/// else `None` — no restriction. A store that fails leaves the user
/// unrestricted, and says so: a class is a policy, not a lock, and a
/// database blip must not stop every call.
pub async fn class_for(
    storage: &DynStorage,
    tenant: &TenantId,
    user: &UserId,
) -> Option<ClassOfService> {
    let classes = match storage.list_classes_of_service(tenant).await {
        Ok(classes) => classes,
        Err(e) => {
            warn!(tenant_id = %tenant, error = %e, "Failed to read the tenant's classes of service; the call is unrestricted");
            return None;
        }
    };
    if classes.is_empty() {
        return None;
    }
    let settings = crate::ring_plan::settings_for(storage, tenant, user).await;
    effective_class(&classes, settings.class_of_service_id.as_ref())
}

/// The user part of a URI (`sip:`, `sips:`, `tel:`, or bare).
pub fn uri_user(uri: &str) -> &str {
    let uri = uri.trim().trim_start_matches('<');
    let rest = uri
        .strip_prefix("sip:")
        .or_else(|| uri.strip_prefix("sips:"))
        .or_else(|| uri.strip_prefix("tel:"))
        .unwrap_or(uri);
    let user = rest.split('@').next().unwrap_or(rest);
    user.split(';').next().unwrap_or(user)
}

/// The host part of a URI with its port, if it has one (`sip:1001@h:5060`
/// → `h:5060`).
pub fn uri_host(uri: &str) -> &str {
    let uri = uri.trim().trim_start_matches('<');
    let rest = uri
        .strip_prefix("sip:")
        .or_else(|| uri.strip_prefix("sips:"))
        .unwrap_or(uri);
    let host = rest.rsplit_once('@').map(|(_, h)| h).unwrap_or(rest);
    host.split([';', '>', '?']).next().unwrap_or(host)
}

/// What `dialed` (a URI's user part) is to the tenant: its country and
/// overrides decide, and a number of the tenant's own is internal.
pub async fn classify_for_tenant(
    storage: &DynStorage,
    tenant: &TenantId,
    dialed: &str,
    max_extension_digits: usize,
) -> Classified {
    let calling = match storage.get_tenant(tenant).await {
        Ok(Some(t)) => t.calling,
        Ok(None) => Default::default(),
        Err(e) => {
            warn!(tenant_id = %tenant, error = %e, "Failed to read the tenant's calling defaults");
            Default::default()
        }
    };
    let mut classified = classify(
        dialed,
        &Dialing {
            country: calling.country.as_deref(),
            max_extension_digits,
            overrides: &calling.destination_overrides,
        },
    );
    if let Some(e164) = &classified.e164 {
        if let Ok(Some(number)) = storage.get_phone_number_by_e164(e164).await {
            if number.tenant_id == *tenant {
                classified.class = DestinationClass::Internal;
            }
        }
    }
    classified
}

/// A call the class does not allow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    pub class_name: String,
    pub kind: RefusalKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefusalKind {
    /// The kind of destination is not allowed.
    Destination(DestinationClass),
    /// The user is on as many calls as the class allows.
    CallLimit(u32),
}

impl Refusal {
    /// The `Reason` header value (RFC 3326).
    pub fn reason_header(&self) -> String {
        let text = match self.kind {
            RefusalKind::Destination(d) => format!(
                "class of service: {} calls not allowed",
                d.as_str().replace('_', "-")
            ),
            RefusalKind::CallLimit(max) => {
                format!("class of service: at most {max} calls at once")
            }
        };
        format!("SIP;cause=403;text=\"{text}\"")
    }

    /// What the record and `call.failed` say.
    pub fn detail(&self) -> String {
        match self.kind {
            RefusalKind::Destination(d) => format!(
                "class_of_service: {d} calls not allowed by `{}`",
                self.class_name
            ),
            RefusalKind::CallLimit(max) => format!(
                "class_of_service: at most {max} calls at once under `{}`",
                self.class_name
            ),
        }
    }
}

/// Whether `user` may dial `dialed`: `Err` when their class refuses it.
pub async fn check_dial(
    storage: &DynStorage,
    tenant: &TenantId,
    user: &UserId,
    dialed: &str,
    max_extension_digits: usize,
) -> Result<Option<ClassOfService>, Refusal> {
    let Some(class) = class_for(storage, tenant, user).await else {
        return Ok(None);
    };
    let classified = classify_for_tenant(storage, tenant, dialed, max_extension_digits).await;
    if class.allows(classified.class) {
        Ok(Some(class))
    } else {
        Err(Refusal {
            class_name: class.name.clone(),
            kind: RefusalKind::Destination(classified.class),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_user_part_of_what_was_dialled() {
        assert_eq!(
            uri_user("sip:+15185550100@pbx.local;user=phone"),
            "+15185550100"
        );
        assert_eq!(uri_user("<sips:1001@pbx.local>"), "1001");
        assert_eq!(uri_user("tel:+442079460000"), "+442079460000");
        assert_eq!(uri_user("0800123456"), "0800123456");
    }

    #[test]
    fn a_refusal_names_the_kind_of_call() {
        let refusal = Refusal {
            class_name: "local".into(),
            kind: RefusalKind::Destination(DestinationClass::TollFree),
        };
        assert_eq!(
            refusal.reason_header(),
            "SIP;cause=403;text=\"class of service: toll-free calls not allowed\""
        );
        assert!(refusal.detail().starts_with("class_of_service: toll_free"));
    }
}
