//! What every node sees of the trunks it routes by
//! (`docs/TENANT_TRUNKS.md` §9).
//!
//! Each node publishes its own view — its probes, its calls, its bindings
//! — under its own key, and anything that wants the whole picture reads
//! them all. A node that stops publishing drops out after one TTL, so the
//! answer is what the live nodes currently see and never a node that has
//! gone.

use fcp_core::trunk_report::TrunkNodeReport;
use fcp_storage_redis::RedisStorage;
use redis::AsyncCommands;
use tracing::debug;

/// One node's trunk view, published for the rest of the deployment.
pub struct TrunkRegistry {
    storage: RedisStorage,
    node_id: String,
    ttl_secs: u64,
}

impl TrunkRegistry {
    /// A registry for `node_id`; a view expires 30 s after its last
    /// refresh, which is several publishing rounds.
    pub fn new(storage: RedisStorage, node_id: String) -> Self {
        Self::with_ttl(storage, node_id, 30)
    }

    pub fn with_ttl(storage: RedisStorage, node_id: String, ttl_secs: u64) -> Self {
        Self {
            storage,
            node_id,
            ttl_secs,
        }
    }

    pub fn node_id(&self) -> &str {
        &self.node_id
    }

    fn key_for(&self, node_id: &str) -> String {
        self.storage.key(&format!("cluster:trunks:{node_id}"))
    }

    fn pattern(&self) -> String {
        self.storage.key("cluster:trunks:*")
    }

    /// Publish what this node sees, replacing what it said before.
    pub async fn publish(&self, report: &TrunkNodeReport) -> anyhow::Result<()> {
        let key = self.key_for(&self.node_id);
        let value = serde_json::to_string(report)?;
        let mut conn = self.storage.connection();
        let _: () = conn.set_ex(&key, value, self.ttl_secs).await?;
        debug!(node = %self.node_id, trunks = report.trunks.len(), "Trunk view published");
        Ok(())
    }

    /// Stop saying anything: a node on its way out leaves no stale view
    /// behind for a reader to believe for a TTL.
    pub async fn withdraw(&self) -> anyhow::Result<()> {
        let key = self.key_for(&self.node_id);
        let mut conn = self.storage.connection();
        let _: () = conn.del(&key).await?;
        Ok(())
    }

    /// What every live node sees, newest view per node, sorted by node id
    /// so two readings line up.
    pub async fn reports(&self) -> anyhow::Result<Vec<TrunkNodeReport>> {
        let mut conn = self.storage.connection();
        let mut cursor: u64 = 0;
        let mut keys: Vec<String> = Vec::new();
        loop {
            let (next, found): (u64, Vec<String>) = redis::cmd("SCAN")
                .arg(cursor)
                .arg("MATCH")
                .arg(self.pattern())
                .arg("COUNT")
                .arg(100)
                .query_async(&mut conn)
                .await?;
            keys.extend(found);
            cursor = next;
            if cursor == 0 {
                break;
            }
        }

        let mut reports = Vec::new();
        for key in keys {
            let raw: Option<String> = conn.get(&key).await?;
            let Some(raw) = raw else {
                // It expired between the scan and the read: the node is
                // gone, which is the answer either way.
                continue;
            };
            match serde_json::from_str::<TrunkNodeReport>(&raw) {
                Ok(report) => reports.push(report),
                Err(e) => debug!(key = %key, error = %e, "Unreadable trunk view, skipping"),
            }
        }
        reports.sort_by(|a, b| a.node_id.cmp(&b.node_id));
        Ok(reports)
    }
}
