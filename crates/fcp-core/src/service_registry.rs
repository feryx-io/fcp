//! The admin's record of every service it has seen (`docs/SERVICES.md`
//! §2, phase S2): a beacon lasts fifteen seconds, so a service that stops
//! would simply vanish; the record keeps it, says since when it is down,
//! and lets an operator forget one that was decommissioned.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::time::SystemTime;

/// One service as last seen. `site_id` is none for the platform's own.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ServiceInstance {
    pub id: String,
    pub site_id: Option<String>,
    pub kind: String,
    pub host: String,
    pub version: String,
    pub first_seen: SystemTime,
    pub last_seen: SystemTime,
    /// Its beacon was there at the last look.
    pub up: bool,
    /// When it went down; none while up.
    pub down_since: Option<SystemTime>,
    /// Its last beacon said `stopping`: shut down on purpose.
    pub stopped: bool,
    /// Its last beacon as written (the `ServiceRecord`), for the page.
    pub last: serde_json::Value,
}

/// Where the records are kept.
#[async_trait]
pub trait ServiceInstanceStore: Send + Sync {
    /// Every record, the platform's and every site's.
    async fn list_service_instances(&self) -> anyhow::Result<Vec<ServiceInstance>>;

    /// Write a record, replacing the one with its site and id.
    async fn put_service_instance(&self, instance: &ServiceInstance) -> anyhow::Result<()>;

    /// Forget one; whether it was there.
    async fn delete_service_instance(
        &self,
        site_id: Option<&str>,
        id: &str,
    ) -> anyhow::Result<bool>;

    /// Forget every record down since before `before`; how many.
    async fn forget_service_instances_down_before(&self, before: SystemTime)
        -> anyhow::Result<u64>;
}
