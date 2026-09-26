//! Cluster monitoring REST API endpoints.
//!
//! Provides read-only access to cluster state by reading Redis heartbeat keys.
//! When Redis is not configured, endpoints return `{ available: false }` with
//! setup instructions.

use axum::{
    extract::{Extension, Query},
    routing::get,
    Json, Router,
};
use fcp_cluster::ClusterReader;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Shared state for cluster API handlers, injected via Extension.
#[derive(Clone)]
pub struct ClusterApiState {
    /// None when Redis is not configured.
    pub reader: Option<Arc<ClusterReader>>,
    /// Every tenant's sites (`docs/HYBRID_CONTROL_PLANE.md`), for the
    /// cluster page's Sites section; None without a database.
    pub storage: Option<Arc<fcp_core::DynStorage>>,
}

/// Response for GET /api/v1/cluster/sites: every tenant's sites, as the
/// tenant API shows one.
#[derive(Serialize)]
pub struct ClusterSitesResponse {
    pub available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    pub sites: Vec<fcp_api_core::v1::sites::SiteResponse>,
}

async fn list_sites(Extension(state): Extension<ClusterApiState>) -> Json<ClusterSitesResponse> {
    let Some(storage) = &state.storage else {
        return Json(ClusterSitesResponse {
            available: false,
            message: Some("No database: sites are not kept here.".to_string()),
            sites: vec![],
        });
    };
    let head = fcp_core::SyncStore::journal_head(storage.as_ref())
        .await
        .unwrap_or_default();
    match fcp_core::SiteStore::list_all_sites(storage.as_ref()).await {
        Ok(sites) => Json(ClusterSitesResponse {
            available: true,
            message: None,
            sites: sites
                .into_iter()
                .map(|s| fcp_api_core::v1::sites::SiteResponse::from(s).behind(head))
                .collect(),
        }),
        Err(e) => {
            tracing::error!(error = %e, "Failed to list sites");
            Json(ClusterSitesResponse {
                available: true,
                message: Some(format!("Failed to read sites: {}", e)),
                sites: vec![],
            })
        }
    }
}

/// Query for GET /api/v1/cluster/nodes.
#[derive(Debug, Default, Deserialize)]
pub struct ClusterNodesQuery {
    /// Only nodes advertising this region.
    pub region: Option<String>,
}

/// Response for GET /api/v1/cluster/nodes
#[derive(Serialize)]
pub struct ClusterNodesResponse {
    pub available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    pub nodes: Vec<fcp_cluster::ClusterNodeInfo>,
}

/// Response for GET /api/v1/cluster/stats
#[derive(Serialize)]
pub struct ClusterStatsResponse {
    pub available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stats: Option<fcp_cluster::ClusterStats>,
}

/// Build the cluster API router.
///
/// Uses `Extension<ClusterApiState>` for state injection so it can be
/// merged into any `Router<S>` regardless of the outer state type.
/// The returned router has routes at `/cluster/nodes` (optionally
/// `?region=…`) and `/cluster/stats`, intended to be nested under `/api/v1`.
pub fn cluster_router<S>(state: ClusterApiState) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    Router::new()
        .route("/cluster/nodes", get(list_nodes))
        .route("/cluster/stats", get(get_stats))
        .route("/cluster/sites", get(list_sites))
        .layer(Extension(state))
}

async fn list_nodes(
    Extension(state): Extension<ClusterApiState>,
    Query(query): Query<ClusterNodesQuery>,
) -> Json<ClusterNodesResponse> {
    let Some(reader) = &state.reader else {
        return Json(ClusterNodesResponse {
            available: false,
            message: Some(
                "Cluster monitoring is not configured. Add [cluster] redis_url = \"redis://localhost:6379\" to your admin config file.".to_string(),
            ),
            nodes: vec![],
        });
    };

    let listed = match query.region.as_deref().map(str::trim) {
        Some(region) if !region.is_empty() => reader.list_nodes_in_region(region).await,
        _ => reader.list_nodes().await,
    };
    match listed {
        Ok(nodes) => Json(ClusterNodesResponse {
            available: true,
            message: None,
            nodes,
        }),
        Err(e) => {
            tracing::error!(error = %e, "Failed to list cluster nodes");
            Json(ClusterNodesResponse {
                available: true,
                message: Some(format!("Failed to read cluster state: {}", e)),
                nodes: vec![],
            })
        }
    }
}

async fn get_stats(Extension(state): Extension<ClusterApiState>) -> Json<ClusterStatsResponse> {
    let Some(reader) = &state.reader else {
        return Json(ClusterStatsResponse {
            available: false,
            message: Some(
                "Cluster monitoring is not configured. Add [cluster] redis_url = \"redis://localhost:6379\" to your admin config file.".to_string(),
            ),
            stats: None,
        });
    };

    match reader.get_stats().await {
        Ok(stats) => Json(ClusterStatsResponse {
            available: true,
            message: None,
            stats: Some(stats),
        }),
        Err(e) => {
            tracing::error!(error = %e, "Failed to get cluster stats");
            Json(ClusterStatsResponse {
                available: true,
                message: Some(format!("Failed to read cluster state: {}", e)),
                stats: None,
            })
        }
    }
}
