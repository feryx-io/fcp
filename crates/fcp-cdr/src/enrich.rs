//! The enricher (`docs/CDR_PIPELINE.md` §4.3, step 2): what the node that
//! owned the call could not know, filled in by the processor.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use fcp_core::storage::{CallDetailRecord, PhoneNumberStore};
use fcp_core::{CallDirection, TenantId};
use tracing::warn;

/// Fills fields of a record before it is rated and stored.
#[async_trait]
pub trait Enricher: Send + Sync {
    async fn enrich(&self, record: &mut CallDetailRecord);
}

/// Stores the call's direction, from the tenant's own numbers: a call to
/// one of them is inbound; a call from one, or one that left on a trunk,
/// is outbound; anything else stayed inside the tenant. A record that
/// already has a direction keeps it.
pub struct DirectionEnricher {
    numbers: Arc<dyn PhoneNumberStore>,
    ttl: Duration,
    cache: Mutex<HashMap<TenantId, CachedNumbers>>,
}

/// A tenant's numbers and when they were read.
type CachedNumbers = (Instant, Arc<HashSet<String>>);

impl DirectionEnricher {
    /// Read tenants' numbers from `numbers`, each tenant's kept for `ttl`.
    pub fn new(numbers: Arc<dyn PhoneNumberStore>, ttl: Duration) -> Self {
        Self {
            numbers,
            ttl,
            cache: Mutex::new(HashMap::new()),
        }
    }

    /// The tenant's numbers as digits, or `None` when they could not be
    /// read (the record is then stored without a direction).
    async fn tenant_numbers(&self, tenant_id: &TenantId) -> Option<Arc<HashSet<String>>> {
        {
            let cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
            if let Some((read_at, numbers)) = cache.get(tenant_id) {
                if read_at.elapsed() < self.ttl {
                    return Some(Arc::clone(numbers));
                }
            }
        }
        match self.numbers.list_phone_numbers(tenant_id).await {
            Ok(list) => {
                let set: Arc<HashSet<String>> =
                    Arc::new(list.iter().filter_map(|n| number_digits(&n.e164)).collect());
                self.cache
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(*tenant_id, (Instant::now(), Arc::clone(&set)));
                Some(set)
            }
            Err(e) => {
                warn!(tenant_id = %tenant_id, error = %e, "Could not read the tenant's numbers; the record keeps no direction");
                None
            }
        }
    }
}

#[async_trait]
impl Enricher for DirectionEnricher {
    async fn enrich(&self, record: &mut CallDetailRecord) {
        if record.direction.is_some() {
            return;
        }
        let Some(numbers) = self.tenant_numbers(&record.tenant_id).await else {
            return;
        };
        record.direction = Some(direction(record, &numbers));
    }
}

/// The direction of `record` for a tenant owning `numbers` (digits only).
pub fn direction(record: &CallDetailRecord, numbers: &HashSet<String>) -> CallDirection {
    let owned = |uri: &str| uri_number(uri).is_some_and(|n| numbers.contains(&n));
    if owned(&record.to_uri) {
        CallDirection::Inbound
    } else if owned(&record.from_uri) || record.trunk_name.is_some() {
        CallDirection::Outbound
    } else {
        CallDirection::Internal
    }
}

/// The number in a URI's user part, as digits, in the shapes the platform
/// stores: `+15550100`, `tel:+1-555-0100;x=y`, `sip:+15550100@host`,
/// `"Name" <sip:+15550100@host>`. `None` for a name (`sip:alice@host`).
pub fn uri_number(uri: &str) -> Option<String> {
    let inner = match (uri.find('<'), uri.rfind('>')) {
        (Some(open), Some(close)) if open < close => &uri[open + 1..close],
        _ => uri,
    };
    let without_scheme = inner
        .strip_prefix("sips:")
        .or_else(|| inner.strip_prefix("sip:"))
        .or_else(|| inner.strip_prefix("tel:"))
        .unwrap_or(inner);
    let user = without_scheme.split(['@', ';']).next().unwrap_or_default();
    number_digits(user)
}

/// Digits of a number written with `+`, spaces, dashes, dots or
/// parentheses; `None` when anything else is in it.
fn number_digits(text: &str) -> Option<String> {
    let significant: String = text
        .chars()
        .filter(|c| !matches!(c, '+' | '-' | '.' | '(' | ')' | ' '))
        .collect();
    (!significant.is_empty() && significant.chars().all(|c| c.is_ascii_digit()))
        .then_some(significant)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::processor::tests::record;

    fn numbers() -> HashSet<String> {
        ["15185550100".to_string()].into_iter().collect()
    }

    #[test]
    fn numbers_come_out_of_every_uri_shape_the_platform_stores() {
        for uri in [
            "+15185550100",
            "tel:+1-518-555-0100",
            "tel:+15185550100;phone-context=x",
            "sip:+15185550100@carrier.example",
            "sips:+15185550100;user=phone",
            "\"Front desk\" <sip:+15185550100@carrier.example>",
        ] {
            assert_eq!(uri_number(uri).as_deref(), Some("15185550100"), "{uri}");
        }
        assert_eq!(uri_number("sip:alice@example.com"), None);
        assert_eq!(uri_number("queue:support"), None);
    }

    #[test]
    fn a_call_to_a_tenant_number_is_inbound_from_one_or_over_a_trunk_outbound_else_internal() {
        let mut inbound = record("sip:+15185550100@pbx");
        inbound.from_uri = "sip:+12125550199@carrier".into();
        assert_eq!(direction(&inbound, &numbers()), CallDirection::Inbound);

        let mut from_did = record("sip:+12125550199@carrier");
        from_did.from_uri = "sip:+15185550100@pbx".into();
        assert_eq!(direction(&from_did, &numbers()), CallDirection::Outbound);

        let mut over_trunk = record("sip:+12125550199@carrier");
        over_trunk.trunk_name = Some("carrier-a".into());
        assert_eq!(direction(&over_trunk, &numbers()), CallDirection::Outbound);

        let internal = record("sip:bob@pbx");
        assert_eq!(direction(&internal, &numbers()), CallDirection::Internal);
    }
}
