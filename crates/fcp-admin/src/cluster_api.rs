//! Cluster monitoring REST API endpoints.
//!
//! Provides read-only access to cluster state by reading Redis heartbeat keys.
//! When Redis is not configured, endpoints return `{ available: false }` with
//! setup instructions.

use axum::{extract::Extension, routing::get, Json, Router};
use fcp_cluster::ClusterReader;
use serde::Serialize;
use std::sync::Arc;

/// Shared state for cluster API handlers, injected via Extension.
#[derive(Clone)]
pub struct ClusterApiState {
    /// None when Redis is not configured.
    pub reader: Option<Arc<ClusterReader>>,
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
/// The returned router has routes at `/cluster/nodes` and `/cluster/stats`,
/// intended to be nested under `/api/v1`.
pub fn cluster_router<S>(state: ClusterApiState) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    Router::new()
        .route("/cluster/nodes", get(list_nodes))
        .route("/cluster/stats", get(get_stats))
        .layer(Extension(state))
}

async fn list_nodes(Extension(state): Extension<ClusterApiState>) -> Json<ClusterNodesResponse> {
    let Some(reader) = &state.reader else {
        return Json(ClusterNodesResponse {
            available: false,
            message: Some(
                "Cluster monitoring is not configured. Add [cluster] redis_url = \"redis://localhost:6379\" to your admin config file.".to_string(),
            ),
            nodes: vec![],
        });
    };

    match reader.list_nodes().await {
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
