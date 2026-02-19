//! RFC 4826 Resource List Subscriptions (RLS) types.
//!
//! A resource list allows a single SUBSCRIBE to monitor multiple presentities.
//! The server sends multipart NOTIFY with RLMI + individual PIDF bodies.

use fcp_core::TenantId;
use serde::{Deserialize, Serialize};

use crate::types::PresenceState;

/// A resource list definition — a named group of presentities.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceList {
    /// The list URI (e.g., `sip:my-team@example.com`).
    pub uri: String,
    /// Optional display name for the list.
    pub name: Option<String>,
    /// Entries (presentity URIs) in the list.
    pub entries: Vec<ResourceListEntry>,
    /// Tenant this list belongs to.
    pub tenant_id: TenantId,
}

/// A single entry in a resource list.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceListEntry {
    /// Presentity AOR (e.g., `alice@example.com`).
    pub uri: String,
    /// Optional display name for this entry.
    pub display_name: Option<String>,
}

/// An RLS subscription — tracks a watcher subscribed to a resource list.
#[derive(Debug, Clone)]
pub struct RlsSubscription {
    /// Resource list URI being watched.
    pub list_uri: String,
    /// The watcher's AOR.
    pub watcher_aor: String,
    /// SIP Contact URI for sending NOTIFYs.
    pub contact_uri: String,
    /// Unix epoch seconds when this subscription expires.
    pub expires_at: u64,
    /// Tenant context.
    pub tenant_id: TenantId,
    /// Virtual subscriptions — one per list entry.
    pub virtual_subs: Vec<VirtualSubscription>,
    /// Subscription ID (from SIP dialog).
    pub subscription_id: String,
    /// RLMI version counter for this subscription.
    pub version: u32,
    /// SIP Call-ID from SUBSCRIBE.
    pub sip_call_id: Option<String>,
    /// From-tag from SUBSCRIBE.
    pub sip_from_tag: Option<String>,
    /// To-tag from our 200 OK.
    pub sip_to_tag: Option<String>,
}

/// A virtual subscription tracking one entry within an RLS subscription.
#[derive(Debug, Clone)]
pub struct VirtualSubscription {
    /// The presentity AOR this virtual sub monitors.
    pub presentity_aor: String,
    /// Last known presence state for this presentity.
    pub last_state: Option<PresenceState>,
    /// Last CSeq used in NOTIFY for this virtual sub.
    pub last_notify_cseq: u32,
}
