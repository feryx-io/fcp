//! A supervisor on a call, asked of the call manager holding it
//! (`docs/ENTERPRISE_CALLING.md` §3.11).
//!
//! The admin API holds no calls: a call lives on the call manager that
//! placed it. The request is put to the node `[call_manager] url` names,
//! else to each call manager in the cluster until one has the call (a node
//! that does not answers 404 "Call not found"), with the tenant the admin
//! API scoped it to.

use crate::config::CallManagerConfig;
use fcp_api_core::{CallSupervisor, SupervisionRequest};
use fcp_cluster::{ClusterNodeStatus, ClusterReader};
use fcp_core::TenantId;
use std::sync::Arc;
use std::time::Duration;

const CALL_MANAGER: &str = "call-manager";

/// Longer than a supervisor's phone may ring on the node.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(40);

pub struct NodeCallSupervisor {
    http: reqwest::Client,
    config: CallManagerConfig,
    cluster: Option<Arc<ClusterReader>>,
}

impl NodeCallSupervisor {
    /// `None` when there is no call manager to ask.
    pub fn new(config: CallManagerConfig, cluster: Option<Arc<ClusterReader>>) -> Option<Self> {
        if config.url.is_none() && cluster.is_none() {
            return None;
        }
        let http = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .build()
            .ok()?;
        Some(Self {
            http,
            config,
            cluster,
        })
    }

    /// The call managers that may hold the call.
    async fn nodes(&self) -> Result<Vec<String>, String> {
        if let Some(url) = &self.config.url {
            return Ok(vec![url.trim_end_matches('/').to_string()]);
        }
        let cluster = self
            .cluster
            .as_ref()
            .ok_or_else(|| "no call manager is configured".to_string())?;
        let nodes: Vec<String> = cluster
            .list_nodes()
            .await
            .map_err(|e| format!("could not read the cluster: {e}"))?
            .into_iter()
            .filter(|n| n.node_type == CALL_MANAGER && n.status == ClusterNodeStatus::Healthy)
            .map(|n| format!("http://{}", n.http_address))
            .collect();
        if nodes.is_empty() {
            return Err("no healthy call manager in the cluster".to_string());
        }
        Ok(nodes)
    }

    async fn ask(
        &self,
        base: &str,
        tenant: TenantId,
        call_id: &str,
        request: &SupervisionRequest,
    ) -> Result<(u16, serde_json::Value), String> {
        let call = format!("{base}/calls/{call_id}");
        let tenant_query = [("tenant_id", tenant.to_string())];
        let builder = match request {
            SupervisionRequest::Join {
                mode,
                supervisor,
                heard_by,
            } => self
                .http
                .post(format!("{call}/{mode}"))
                .json(&serde_json::json!({
                    "supervisor_extension": supervisor,
                    "heard_by": heard_by,
                    "tenant_id": tenant,
                })),
            SupervisionRequest::Get => self
                .http
                .get(format!("{call}/supervisor"))
                .query(&tenant_query),
            SupervisionRequest::Leave => self
                .http
                .post(format!("{call}/supervisor/leave"))
                .query(&tenant_query),
        };
        let builder = match &self.config.auth_token {
            Some(token) => builder.bearer_auth(token),
            None => builder,
        };
        let response = builder
            .send()
            .await
            .map_err(|e| format!("the call manager at {base} did not answer: {e}"))?;
        let status = response.status().as_u16();
        let body = response
            .json::<serde_json::Value>()
            .await
            .unwrap_or_else(|_| serde_json::json!({}));
        Ok((status, body))
    }
}

/// A node's 404 for a call it does not hold (not one for a call without a
/// supervisor).
fn not_here(status: u16, body: &serde_json::Value) -> bool {
    status == 404
        && body["error"]
            .as_str()
            .is_some_and(|e| e.starts_with("Call not found"))
}

#[async_trait::async_trait]
impl CallSupervisor for NodeCallSupervisor {
    async fn supervise(
        &self,
        tenant: TenantId,
        call_id: &str,
        request: SupervisionRequest,
    ) -> Result<(u16, serde_json::Value), String> {
        let mut last_error = None;
        for base in self.nodes().await? {
            match self.ask(&base, tenant, call_id, &request).await {
                Ok((status, body)) if not_here(status, &body) => continue,
                Ok(answer) => return Ok(answer),
                Err(e) => last_error = Some(e),
            }
        }
        match last_error {
            Some(e) => Err(e),
            None => Ok((
                404,
                serde_json::json!({"error": format!("Call not found: {call_id}")}),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_node_without_the_call_is_passed_over_and_one_without_a_supervisor_is_not() {
        assert!(not_here(
            404,
            &serde_json::json!({"error": "Call not found: x"})
        ));
        assert!(!not_here(
            404,
            &serde_json::json!({"error": "the call has no supervisor"})
        ));
        assert!(!not_here(
            409,
            &serde_json::json!({"error": "Call not found"})
        ));
    }
}
