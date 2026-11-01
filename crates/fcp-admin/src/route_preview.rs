//! Asking a live call manager what it would do with a call
//! (`docs/TENANT_TRUNKS.md` §12, phase 6d).
//!
//! The admin API routes no calls, so it does not work the answer out
//! itself: it asks a call manager's `GET /routes/preview`, which runs the
//! node's own policy, rules, least-cost routes and trunks. The node is the
//! one `[call_manager] url` names, else a healthy, non-draining call
//! manager from cluster membership.

use crate::config::CallManagerConfig;
use fcp_api_core::{RoutePreviewQuery, RoutePreviewer};
use fcp_cluster::{ClusterNodeStatus, ClusterReader};
use fcp_core::TenantId;
use std::sync::Arc;
use std::time::Duration;

/// The cluster's name for a call manager node.
const CALL_MANAGER: &str = "call-manager";

pub struct CallManagerPreviewer {
    http: reqwest::Client,
    config: CallManagerConfig,
    cluster: Option<Arc<ClusterReader>>,
}

impl CallManagerPreviewer {
    /// `None` when there is no call manager to ask: no URL and no cluster.
    pub fn new(config: CallManagerConfig, cluster: Option<Arc<ClusterReader>>) -> Option<Self> {
        if config.url.is_none() && cluster.is_none() {
            return None;
        }
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .ok()?;
        Some(Self {
            http,
            config,
            cluster,
        })
    }

    /// The base URL of the call manager to ask.
    async fn node_url(&self) -> Result<String, String> {
        if let Some(url) = &self.config.url {
            return Ok(url.trim_end_matches('/').to_string());
        }
        let cluster = self
            .cluster
            .as_ref()
            .ok_or_else(|| "no call manager is configured".to_string())?;
        let nodes = cluster
            .list_nodes()
            .await
            .map_err(|e| format!("could not read the cluster: {e}"))?;
        nodes
            .into_iter()
            .filter(|n| n.node_type == CALL_MANAGER && !n.draining)
            .find(|n| n.status == ClusterNodeStatus::Healthy)
            .map(|n| format!("http://{}", n.http_address))
            .ok_or_else(|| "no healthy call manager in the cluster".to_string())
    }
}

#[async_trait::async_trait]
impl RoutePreviewer for CallManagerPreviewer {
    async fn preview(
        &self,
        tenant: TenantId,
        query: &RoutePreviewQuery,
    ) -> Result<serde_json::Value, String> {
        let base = self.node_url().await?;
        let mut params: Vec<(&str, String)> =
            vec![("tenant", tenant.to_string()), ("to", query.to.clone())];
        if let Some(from) = &query.from {
            params.push(("from", from.clone()));
        }
        if let Some(ip) = query.source_ip {
            params.push(("source_ip", ip.to_string()));
        }
        if let Some(location) = query.location_id {
            params.push(("location", location.to_string()));
        }
        let mut request = self
            .http
            .get(format!("{base}/routes/preview"))
            .query(&params);
        if let Some(token) = &self.config.auth_token {
            request = request.bearer_auth(token);
        }
        let response = request
            .send()
            .await
            .map_err(|e| format!("the call manager at {base} did not answer: {e}"))?;
        let status = response.status();
        let body: serde_json::Value = response
            .json()
            .await
            .map_err(|e| format!("the call manager's answer did not read: {e}"))?;
        if !status.is_success() {
            let why = body["error"].as_str().unwrap_or("no reason given");
            return Err(format!("the call manager answered {status}: {why}"));
        }
        Ok(body)
    }
}
