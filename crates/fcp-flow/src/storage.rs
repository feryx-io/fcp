//! Flow storage abstraction.
//!
//! Provides traits and types for storing and querying flow definitions.

use crate::schema::FlowDefinition;
use crate::FlowError;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Result type for storage operations
pub type Result<T> = std::result::Result<T, FlowError>;

/// Flow storage trait.
///
/// Provides CRUD operations and querying for flow definitions.
#[async_trait]
pub trait FlowStore: Send + Sync {
    /// Get a flow by ID.
    async fn get_flow(&self, id: &str) -> Result<Option<FlowDefinition>>;

    /// Get a flow by ID and version.
    async fn get_flow_version(&self, id: &str, version: u32) -> Result<Option<FlowDefinition>>;

    /// List all flows for a tenant.
    async fn list_flows(&self, tenant_id: Option<&str>) -> Result<Vec<FlowDefinition>>;

    /// Query flows with filtering and pagination.
    async fn query_flows(&self, query: &FlowQuery) -> Result<FlowQueryResult>;

    /// Insert a new flow (creates version 1).
    async fn insert_flow(&self, flow: FlowDefinition) -> Result<()>;

    /// Update an existing flow (increments version).
    async fn update_flow(&self, flow: FlowDefinition) -> Result<()>;

    /// Delete a flow (soft delete - marks as disabled).
    async fn delete_flow(&self, id: &str) -> Result<()>;

    /// Write a flow exactly as given — its version, its enabled flag, its
    /// timestamps — replacing what is there. A site applying the cloud's
    /// copy uses this (`docs/HYBRID_CONTROL_PLANE.md` §3.2); an
    /// administrator's edits go through [`insert_flow`] and
    /// [`update_flow`], which number the versions.
    ///
    /// [`insert_flow`]: FlowStore::insert_flow
    /// [`update_flow`]: FlowStore::update_flow
    async fn put_flow(&self, flow: FlowDefinition) -> Result<()> {
        if self.get_flow(&flow.id).await?.is_some() {
            self.update_flow(flow).await
        } else {
            self.insert_flow(flow).await
        }
    }

    /// Get all versions of a flow.
    async fn list_flow_versions(&self, id: &str) -> Result<Vec<FlowDefinition>>;

    /// Enable a flow.
    async fn enable_flow(&self, id: &str) -> Result<()>;

    /// Disable a flow.
    async fn disable_flow(&self, id: &str) -> Result<()>;

    /// Hold a flow the node built for itself — one call's inline flow, a
    /// mailbox's built-in voicemail — where [`get_flow`] finds it. A store
    /// with no place apart writes it as [`insert_flow`] does; [`BuiltinFlows`]
    /// keeps it in memory, off the tenant's list (`docs/DEEP_TESTING.md`
    /// D8: every `*98` call left a flow row behind).
    ///
    /// [`get_flow`]: FlowStore::get_flow
    /// [`insert_flow`]: FlowStore::insert_flow
    async fn hold_flow(&self, flow: FlowDefinition) -> Result<()> {
        self.insert_flow(flow).await
    }

    /// Let go of a flow [`hold_flow`] held.
    ///
    /// [`hold_flow`]: FlowStore::hold_flow
    async fn release_flow(&self, _id: &str) -> Result<()> {
        Ok(())
    }
}

/// A store's flows, and in front of them the flows a node builds for
/// itself: held in memory, found by id, never listed and never written.
pub struct BuiltinFlows {
    inner: std::sync::Arc<dyn FlowStore>,
    held: std::sync::RwLock<std::collections::HashMap<String, FlowDefinition>>,
}

impl BuiltinFlows {
    pub fn new(inner: std::sync::Arc<dyn FlowStore>) -> Self {
        Self {
            inner,
            held: std::sync::RwLock::new(std::collections::HashMap::new()),
        }
    }

    /// Let go of every held flow whose id `matches` (a call's, when it ends).
    pub fn release_where(&self, matches: impl Fn(&str) -> bool) {
        if let Ok(mut held) = self.held.write() {
            held.retain(|id, _| !matches(id));
        }
    }

    /// How many flows are held.
    pub fn held(&self) -> usize {
        self.held.read().map(|h| h.len()).unwrap_or(0)
    }

    fn held_flow(&self, id: &str) -> Option<FlowDefinition> {
        self.held.read().ok()?.get(id).cloned()
    }
}

#[async_trait]
impl FlowStore for BuiltinFlows {
    async fn get_flow(&self, id: &str) -> Result<Option<FlowDefinition>> {
        match self.held_flow(id) {
            Some(flow) => Ok(Some(flow)),
            None => self.inner.get_flow(id).await,
        }
    }

    async fn get_flow_version(&self, id: &str, version: u32) -> Result<Option<FlowDefinition>> {
        match self.held_flow(id) {
            Some(flow) => Ok((flow.version == version).then_some(flow)),
            None => self.inner.get_flow_version(id, version).await,
        }
    }

    async fn list_flows(&self, tenant_id: Option<&str>) -> Result<Vec<FlowDefinition>> {
        self.inner.list_flows(tenant_id).await
    }

    async fn query_flows(&self, query: &FlowQuery) -> Result<FlowQueryResult> {
        self.inner.query_flows(query).await
    }

    async fn insert_flow(&self, flow: FlowDefinition) -> Result<()> {
        self.inner.insert_flow(flow).await
    }

    async fn update_flow(&self, flow: FlowDefinition) -> Result<()> {
        self.inner.update_flow(flow).await
    }

    async fn delete_flow(&self, id: &str) -> Result<()> {
        self.inner.delete_flow(id).await
    }

    async fn put_flow(&self, flow: FlowDefinition) -> Result<()> {
        self.inner.put_flow(flow).await
    }

    async fn list_flow_versions(&self, id: &str) -> Result<Vec<FlowDefinition>> {
        match self.held_flow(id) {
            Some(flow) => Ok(vec![flow]),
            None => self.inner.list_flow_versions(id).await,
        }
    }

    async fn enable_flow(&self, id: &str) -> Result<()> {
        self.inner.enable_flow(id).await
    }

    async fn disable_flow(&self, id: &str) -> Result<()> {
        self.inner.disable_flow(id).await
    }

    async fn hold_flow(&self, flow: FlowDefinition) -> Result<()> {
        if let Ok(mut held) = self.held.write() {
            held.insert(flow.id.clone(), flow);
        }
        Ok(())
    }

    async fn release_flow(&self, id: &str) -> Result<()> {
        if let Ok(mut held) = self.held.write() {
            held.remove(id);
        }
        Ok(())
    }
}

/// Query parameters for filtering flows.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FlowQuery {
    /// Filter by tenant ID
    pub tenant_id: Option<String>,

    /// Filter by enabled status
    pub enabled: Option<bool>,

    /// Search by name (partial match)
    pub name_contains: Option<String>,

    /// Search by description (partial match)
    pub description_contains: Option<String>,

    /// Filter flows updated after this timestamp
    pub updated_after: Option<chrono::DateTime<chrono::Utc>>,

    /// Filter flows updated before this timestamp
    pub updated_before: Option<chrono::DateTime<chrono::Utc>>,

    /// Limit number of results
    pub limit: usize,

    /// Offset for pagination
    pub offset: usize,

    /// Sort by field (name, updated_at, created_at)
    pub sort_by: Option<SortField>,

    /// Sort order
    pub sort_order: SortOrder,
}

/// Fields that can be used for sorting
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SortField {
    Name,
    CreatedAt,
    UpdatedAt,
    Version,
}

/// Sort order
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum SortOrder {
    Asc,
    #[default]
    Desc,
}

/// Result of a flow query with pagination info.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowQueryResult {
    /// Matching flows
    pub flows: Vec<FlowDefinition>,

    /// Total number of matching flows (before pagination)
    pub total: usize,

    /// Limit used for this query
    pub limit: usize,

    /// Offset used for this query
    pub offset: usize,
}

impl FlowQueryResult {
    /// Create a new query result.
    pub fn new(flows: Vec<FlowDefinition>, total: usize, limit: usize, offset: usize) -> Self {
        Self {
            flows,
            total,
            limit,
            offset,
        }
    }

    /// Check if there are more results available.
    pub fn has_more(&self) -> bool {
        self.offset + self.limit < self.total
    }

    /// Get the next offset for pagination.
    pub fn next_offset(&self) -> Option<usize> {
        if self.has_more() {
            Some(self.offset + self.limit)
        } else {
            None
        }
    }
}

/// In-memory flow storage implementation for testing and development.
///
/// This is a simple implementation that stores flows in memory.
/// For production use, implement FlowStore for your database backend
/// (PostgresStorage, etc.)
pub struct InMemoryFlowStore {
    flows: Arc<RwLock<HashMap<String, Vec<FlowDefinition>>>>,
}

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

impl InMemoryFlowStore {
    /// Create a new in-memory flow store.
    pub fn new() -> Self {
        Self {
            flows: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

impl Default for InMemoryFlowStore {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl FlowStore for InMemoryFlowStore {
    async fn get_flow(&self, id: &str) -> Result<Option<FlowDefinition>> {
        let flows = self.flows.read().await;
        if let Some(versions) = flows.get(id) {
            // Return latest version
            Ok(versions.last().cloned())
        } else {
            Ok(None)
        }
    }

    async fn get_flow_version(&self, id: &str, version: u32) -> Result<Option<FlowDefinition>> {
        let flows = self.flows.read().await;
        if let Some(versions) = flows.get(id) {
            Ok(versions.iter().find(|f| f.version == version).cloned())
        } else {
            Ok(None)
        }
    }

    async fn list_flows(&self, tenant_id: Option<&str>) -> Result<Vec<FlowDefinition>> {
        let flows = self.flows.read().await;
        let mut result = Vec::new();

        for versions in flows.values() {
            if let Some(latest) = versions.last() {
                if let Some(tid) = tenant_id {
                    if latest.tenant_id.as_deref() == Some(tid) {
                        result.push(latest.clone());
                    }
                } else {
                    result.push(latest.clone());
                }
            }
        }

        Ok(result)
    }

    async fn query_flows(&self, query: &FlowQuery) -> Result<FlowQueryResult> {
        let flows = self.flows.read().await;
        let mut matching = Vec::new();

        // Collect latest versions that match query
        for versions in flows.values() {
            if let Some(flow) = versions.last() {
                let mut matches = true;

                // Filter by tenant_id
                if let Some(ref tid) = query.tenant_id {
                    if flow.tenant_id.as_deref() != Some(tid) {
                        matches = false;
                    }
                }

                // Filter by enabled
                if let Some(enabled) = query.enabled {
                    if flow.enabled != enabled {
                        matches = false;
                    }
                }

                // Filter by name contains
                if let Some(ref name) = query.name_contains {
                    if !flow.name.to_lowercase().contains(&name.to_lowercase()) {
                        matches = false;
                    }
                }

                // Filter by description contains
                if let Some(ref desc) = query.description_contains {
                    if let Some(ref flow_desc) = flow.description {
                        if !flow_desc.to_lowercase().contains(&desc.to_lowercase()) {
                            matches = false;
                        }
                    } else {
                        matches = false;
                    }
                }

                // Filter by updated_after
                if let Some(ref after) = query.updated_after {
                    if let Some(ref updated) = flow.updated_at {
                        if updated < after {
                            matches = false;
                        }
                    }
                }

                // Filter by updated_before
                if let Some(ref before) = query.updated_before {
                    if let Some(ref updated) = flow.updated_at {
                        if updated > before {
                            matches = false;
                        }
                    }
                }

                if matches {
                    matching.push(flow.clone());
                }
            }
        }

        // Sort results
        match query.sort_by {
            Some(SortField::Name) => {
                matching.sort_by(|a, b| match query.sort_order {
                    SortOrder::Asc => a.name.cmp(&b.name),
                    SortOrder::Desc => b.name.cmp(&a.name),
                });
            }
            Some(SortField::CreatedAt) => {
                matching.sort_by(|a, b| match query.sort_order {
                    SortOrder::Asc => a.created_at.cmp(&b.created_at),
                    SortOrder::Desc => b.created_at.cmp(&a.created_at),
                });
            }
            Some(SortField::UpdatedAt) => {
                matching.sort_by(|a, b| match query.sort_order {
                    SortOrder::Asc => a.updated_at.cmp(&b.updated_at),
                    SortOrder::Desc => b.updated_at.cmp(&a.updated_at),
                });
            }
            Some(SortField::Version) => {
                matching.sort_by(|a, b| match query.sort_order {
                    SortOrder::Asc => a.version.cmp(&b.version),
                    SortOrder::Desc => b.version.cmp(&a.version),
                });
            }
            None => {}
        }

        let total = matching.len();
        let offset = query.offset;
        let limit = query.limit;

        // Apply pagination
        let paginated: Vec<_> = matching.into_iter().skip(offset).take(limit).collect();

        Ok(FlowQueryResult::new(paginated, total, limit, offset))
    }

    async fn insert_flow(&self, mut flow: FlowDefinition) -> Result<()> {
        let mut flows = self.flows.write().await;

        // Set version to 1 for new flows
        flow.version = 1;
        flow.created_at = Some(chrono::Utc::now());
        flow.updated_at = Some(chrono::Utc::now());

        flows.insert(flow.id.clone(), vec![flow]);
        Ok(())
    }

    async fn update_flow(&self, mut flow: FlowDefinition) -> Result<()> {
        let mut flows = self.flows.write().await;

        if let Some(versions) = flows.get_mut(&flow.id) {
            // Increment version
            let latest_version = versions.last().map(|f| f.version).unwrap_or(0);
            flow.version = latest_version + 1;
            flow.updated_at = Some(chrono::Utc::now());

            versions.push(flow);
            Ok(())
        } else {
            Err(FlowError::NotFound(flow.id))
        }
    }

    async fn delete_flow(&self, id: &str) -> Result<()> {
        let mut flows = self.flows.write().await;
        flows.remove(id);
        Ok(())
    }

    async fn list_flow_versions(&self, id: &str) -> Result<Vec<FlowDefinition>> {
        let flows = self.flows.read().await;
        Ok(flows.get(id).cloned().unwrap_or_default())
    }

    async fn enable_flow(&self, id: &str) -> Result<()> {
        let mut flows = self.flows.write().await;
        if let Some(versions) = flows.get_mut(id) {
            if let Some(latest) = versions.last_mut() {
                latest.enabled = true;
                latest.updated_at = Some(chrono::Utc::now());
            }
            Ok(())
        } else {
            Err(FlowError::NotFound(id.to_string()))
        }
    }

    async fn disable_flow(&self, id: &str) -> Result<()> {
        let mut flows = self.flows.write().await;
        if let Some(versions) = flows.get_mut(id) {
            if let Some(latest) = versions.last_mut() {
                latest.enabled = false;
                latest.updated_at = Some(chrono::Utc::now());
            }
            Ok(())
        } else {
            Err(FlowError::NotFound(id.to_string()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn test_query_result_pagination() {
        let result = FlowQueryResult::new(vec![], 100, 10, 0);
        assert!(result.has_more());
        assert_eq!(result.next_offset(), Some(10));

        let result = FlowQueryResult::new(vec![], 100, 10, 90);
        assert!(!result.has_more());
        assert_eq!(result.next_offset(), None);
    }

    /// A held flow is found by id and nowhere else: not listed, not
    /// written, gone when let go.
    #[tokio::test]
    async fn a_held_flow_is_found_but_never_listed_or_written() {
        use crate::schema::{FlowNode, NodeType};
        let inner = std::sync::Arc::new(InMemoryFlowStore::new());
        let store = BuiltinFlows::new(inner.clone());
        let mut nodes = HashMap::new();
        nodes.insert(
            "end".to_string(),
            FlowNode {
                id: "end".to_string(),
                name: None,
                node_type: NodeType::Hangup { reason: None },
            },
        );
        let flow = |id: &str| FlowDefinition {
            id: id.to_string(),
            name: id.to_string(),
            description: None,
            version: 1,
            initial_node: "end".to_string(),
            nodes: nodes.clone(),
            variables: HashMap::new(),
            tenant_id: Some("tenant-a".to_string()),
            enabled: true,
            created_at: None,
            updated_at: None,
        };
        store.hold_flow(flow("vm-access-call-1")).await.unwrap();
        store.insert_flow(flow("ivr")).await.unwrap();

        assert!(store.get_flow("vm-access-call-1").await.unwrap().is_some());
        assert!(inner.get_flow("vm-access-call-1").await.unwrap().is_none());
        let listed: Vec<_> = store
            .list_flows(Some("tenant-a"))
            .await
            .unwrap()
            .into_iter()
            .map(|f| f.id)
            .collect();
        assert_eq!(listed, vec!["ivr".to_string()]);

        store.release_where(|id| id.ends_with("call-1"));
        assert!(store.get_flow("vm-access-call-1").await.unwrap().is_none());
        assert_eq!(store.held(), 0);
        assert!(store.get_flow("ivr").await.unwrap().is_some());
    }

    #[tokio::test]
    async fn test_in_memory_store() {
        use crate::schema::{FlowNode, NodeType};

        let store = InMemoryFlowStore::new();

        // Create a test flow
        let mut nodes = HashMap::new();
        nodes.insert(
            "start".to_string(),
            FlowNode {
                id: "start".to_string(),
                name: Some("Start".to_string()),
                node_type: NodeType::Answer {
                    next: "end".to_string(),
                    on_error: None,
                },
            },
        );
        nodes.insert(
            "end".to_string(),
            FlowNode {
                id: "end".to_string(),
                name: Some("End".to_string()),
                node_type: NodeType::Hangup { reason: None },
            },
        );

        let flow = FlowDefinition {
            id: "test-flow".to_string(),
            name: "Test Flow".to_string(),
            description: Some("A test flow".to_string()),
            version: 1,
            initial_node: "start".to_string(),
            nodes,
            variables: HashMap::new(),
            tenant_id: None,
            enabled: true,
            created_at: None,
            updated_at: None,
        };

        // Insert flow
        store.insert_flow(flow.clone()).await.unwrap();

        // Get flow
        let retrieved = store.get_flow("test-flow").await.unwrap().unwrap();
        assert_eq!(retrieved.name, "Test Flow");
        assert_eq!(retrieved.version, 1);

        // Update flow
        let mut updated = retrieved.clone();
        updated.name = "Updated Flow".to_string();
        store.update_flow(updated).await.unwrap();

        // Get updated flow
        let retrieved = store.get_flow("test-flow").await.unwrap().unwrap();
        assert_eq!(retrieved.name, "Updated Flow");
        assert_eq!(retrieved.version, 2);

        // List versions
        let versions = store.list_flow_versions("test-flow").await.unwrap();
        assert_eq!(versions.len(), 2);
        assert_eq!(versions[0].version, 1);
        assert_eq!(versions[1].version, 2);
    }
}
