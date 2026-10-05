//! The sync journal (`docs/HYBRID_CONTROL_PLANE.md` §3.2, phase 2): what
//! changed, per tenant, in the tables a site needs — written by triggers
//! (migration 079), read by the cloud's link to feed a site. An entry
//! names a table and a row and says it was put or deleted; the object
//! itself is read at feed time.

use crate::TenantId;
use anyhow::Result as AnyhowResult;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::time::SystemTime;

/// The tables a site's slice is made of, as the journal names them.
/// `rates` journals under its plan, so a link sees `rate_plans`.
pub const SYNC_TABLES: &[&str] = &[
    "tenants",
    "users",
    "user_calling_settings",
    "schedules",
    "classes_of_service",
    "devices",
    "extensions",
    "line_bindings",
    "phone_numbers",
    "routing_rules",
    "lcr_routes",
    "trunks",
    "queue_configs",
    "flows",
    "prompts",
    "config_templates",
    "conference_meetings",
    "locations",
    "emergency_networks",
    "emergency_configs",
    "rate_plans",
    "rates",
    // Single sign-on at a site (docs/IDENTITY_AND_DEVICES.md Q11): the
    // tenant's providers, their client secrets sealed with the site's own
    // key, and the identities its users have at them.
    "identity_providers",
    "user_identities",
    // Groups and who is in them (docs/USERS.md §5): the roles a site's
    // users hold, and a queue's agents.
    "groups",
    // Firmware targets, each carrying its image (docs/PROVISIONING.md §11);
    // the link fetches the images themselves.
    "firmware_targets",
    "group_members",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncOp {
    Put,
    Delete,
}

impl SyncOp {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "put" => Some(Self::Put),
            "delete" => Some(Self::Delete),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalEntry {
    pub seq: i64,
    pub tenant_id: TenantId,
    pub table: String,
    pub row_id: String,
    pub op: SyncOp,
    pub at: SystemTime,
}

/// The journal, as the cloud's link reads it.
#[async_trait]
pub trait SyncStore: Send + Sync {
    /// The newest sequence number written, over every tenant; 0 when
    /// nothing has been journaled.
    async fn journal_head(&self) -> AnyhowResult<i64>;

    /// A tenant's entries after `since`, oldest first, at most `limit`.
    async fn journal_after(
        &self,
        tenant_id: &TenantId,
        since: i64,
        limit: usize,
    ) -> AnyhowResult<Vec<JournalEntry>>;
}
