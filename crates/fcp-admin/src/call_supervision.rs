//! A supervisor on a call (`docs/ENTERPRISE_CALLING.md` §3.11), live call
//! control (`docs/DEVELOPER_SURFACE.md` phase 8), a call's details and its
//! recording, asked of the call manager holding the call.
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
        self.nodes_where(|status| status == ClusterNodeStatus::Healthy)
            .await
    }

    /// The call managers whose status `keep` takes, or the one configured.
    async fn nodes_where(
        &self,
        keep: impl Fn(ClusterNodeStatus) -> bool,
    ) -> Result<Vec<String>, String> {
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
            .filter(|n| n.node_type == CALL_MANAGER && keep(n.status))
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
            SupervisionRequest::Show => self.http.get(&call).query(&tenant_query),
            SupervisionRequest::Leave => self
                .http
                .post(format!("{call}/supervisor/leave"))
                .query(&tenant_query),
            SupervisionRequest::Control { action, body } => {
                let mut body = body.clone();
                if let Some(fields) = body.as_object_mut() {
                    fields.insert("tenant_id".to_string(), serde_json::json!(tenant));
                }
                self.http.post(format!("{call}/{action}")).json(&body)
            }
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

    /// Every call manager is asked: each sends down the flows it holds (a
    /// phone on TCP or TLS behind NAT is reachable only through the node
    /// it registered at), and the first that answers also sends to the
    /// contacts with no flow. What they sent is added up.
    async fn resync_device(
        &self,
        tenant: TenantId,
        device_id: &str,
    ) -> Result<(u16, serde_json::Value), String> {
        let mut sent = 0u64;
        let mut answered = false;
        let mut not_found = false;
        let mut last_error = None;
        // A degraded node still holds its phones' flows: it is asked too.
        for base in self
            .nodes_where(|status| status != ClusterNodeStatus::Offline)
            .await?
        {
            let builder = self
                .http
                .post(format!("{base}/devices/{device_id}/resync"))
                .json(&serde_json::json!({ "tenant_id": tenant, "flowless": !answered }));
            let builder = match &self.config.auth_token {
                Some(token) => builder.bearer_auth(token),
                None => builder,
            };
            match builder.send().await {
                Ok(response) if response.status().is_success() => {
                    let body = response
                        .json::<serde_json::Value>()
                        .await
                        .unwrap_or_else(|_| serde_json::json!({}));
                    sent += body["sent"].as_u64().unwrap_or(0);
                    answered = true;
                }
                Ok(response) if response.status().as_u16() == 404 => not_found = true,
                Ok(response) => {
                    last_error = Some(format!(
                        "the call manager at {base} answered {}",
                        response.status()
                    ))
                }
                Err(e) => {
                    last_error = Some(format!("the call manager at {base} did not answer: {e}"))
                }
            }
        }
        if answered {
            return Ok((
                200,
                serde_json::json!({ "device_id": device_id, "sent": sent }),
            ));
        }
        if not_found {
            return Ok((
                404,
                serde_json::json!({ "error": format!("Device not found: {device_id}") }),
            ));
        }
        Err(last_error.unwrap_or_else(|| "no call manager to ask".to_string()))
    }

    /// Every call manager is asked, a degraded one too (it still holds its
    /// parked calls); the lists are put together by slot.
    async fn parked_calls(&self, tenant: TenantId) -> Result<Vec<serde_json::Value>, String> {
        let mut calls = Vec::new();
        let mut answered = false;
        let mut last_error = None;
        for base in self
            .nodes_where(|status| status != ClusterNodeStatus::Offline)
            .await?
        {
            let builder = self
                .http
                .get(format!("{base}/parking/calls"))
                .query(&[("tenant_id", tenant.to_string())]);
            let builder = match &self.config.auth_token {
                Some(token) => builder.bearer_auth(token),
                None => builder,
            };
            match builder.send().await {
                Ok(response) if response.status().is_success() => {
                    answered = true;
                    let body = response
                        .json::<serde_json::Value>()
                        .await
                        .unwrap_or_else(|_| serde_json::json!({}));
                    if let Some(list) = body["calls"].as_array() {
                        calls.extend(list.iter().cloned());
                    }
                }
                Ok(response) => {
                    last_error = Some(format!(
                        "the call manager at {base} answered {}",
                        response.status()
                    ))
                }
                Err(e) => {
                    last_error = Some(format!("the call manager at {base} did not answer: {e}"))
                }
            }
        }
        if !answered {
            return Err(last_error.unwrap_or_else(|| "no call manager to ask".to_string()));
        }
        calls.sort_by(|a, b| {
            let key = |v: &serde_json::Value| {
                let slot = v["slot"].as_str().unwrap_or_default().to_string();
                (slot.len(), slot)
            };
            key(a).cmp(&key(b))
        });
        Ok(calls)
    }

    /// Each call manager in turn until one holds the slot (a node without
    /// it answers 404).
    async fn parked_call(
        &self,
        tenant: TenantId,
        slot: &str,
        target: Option<&str>,
    ) -> Result<(u16, serde_json::Value), String> {
        let mut last_error = None;
        for base in self
            .nodes_where(|status| status != ClusterNodeStatus::Offline)
            .await?
        {
            let url = format!("{base}/parking/calls/{slot}");
            let builder = match target {
                Some(target) => self
                    .http
                    .post(format!("{url}/retrieve"))
                    .json(&serde_json::json!({ "target": target, "tenant_id": tenant })),
                None => self
                    .http
                    .delete(&url)
                    .query(&[("tenant_id", tenant.to_string())]),
            };
            let builder = match &self.config.auth_token {
                Some(token) => builder.bearer_auth(token),
                None => builder,
            };
            match builder.send().await {
                Ok(response) if response.status().as_u16() == 404 => continue,
                Ok(response) => {
                    let status = response.status().as_u16();
                    let body = response
                        .json::<serde_json::Value>()
                        .await
                        .unwrap_or_else(|_| serde_json::json!({}));
                    return Ok((status, body));
                }
                Err(e) => {
                    last_error = Some(format!("the call manager at {base} did not answer: {e}"))
                }
            }
        }
        match last_error {
            Some(e) => Err(e),
            None => Ok((
                404,
                serde_json::json!({"error": format!("No call is parked in {slot}")}),
            )),
        }
    }

    /// The first call manager that answers places it: any node can ring a
    /// tenant's phone. One that cannot speak (503) passes it on.
    async fn notify_call(
        &self,
        tenant: TenantId,
        request: serde_json::Value,
    ) -> Result<(u16, serde_json::Value), String> {
        let mut last_error = None;
        for base in self.nodes().await? {
            let builder = self
                .http
                .post(format!("{base}/emergency/notify-call"))
                .json(&request);
            let builder = match &self.config.auth_token {
                Some(token) => builder.bearer_auth(token),
                None => builder,
            };
            match builder.send().await {
                Ok(response) => {
                    let status = response.status().as_u16();
                    let body = response
                        .json::<serde_json::Value>()
                        .await
                        .unwrap_or_else(|_| serde_json::json!({}));
                    if status == 503 {
                        last_error = Some(format!(
                            "the call manager at {base}: {}",
                            body["error"].as_str().unwrap_or("unavailable")
                        ));
                        continue;
                    }
                    return Ok((status, body));
                }
                Err(e) => {
                    last_error = Some(format!("the call manager at {base} did not answer: {e}"))
                }
            }
        }
        let _ = tenant;
        Err(last_error.unwrap_or_else(|| "no call manager to ask".to_string()))
    }

    /// The first call manager with the publisher's keys sends it: a push
    /// goes through Apple or Google, so any node will do. One without
    /// `[push]` (503) passes it on.
    async fn push_test_device(
        &self,
        tenant: TenantId,
        device_id: &str,
    ) -> Result<(u16, serde_json::Value), String> {
        let mut last_error = None;
        for base in self
            .nodes_where(|status| status != ClusterNodeStatus::Offline)
            .await?
        {
            let builder = self
                .http
                .post(format!("{base}/devices/{device_id}/push-test"))
                .json(&serde_json::json!({ "tenant_id": tenant }));
            let builder = match &self.config.auth_token {
                Some(token) => builder.bearer_auth(token),
                None => builder,
            };
            match builder.send().await {
                Ok(response) => {
                    let status = response.status().as_u16();
                    let body = response
                        .json::<serde_json::Value>()
                        .await
                        .unwrap_or_else(|_| serde_json::json!({}));
                    if status == 503 {
                        last_error = Some(format!(
                            "the call manager at {base}: {}",
                            body["error"].as_str().unwrap_or("unavailable")
                        ));
                        continue;
                    }
                    return Ok((status, body));
                }
                Err(e) => {
                    last_error = Some(format!("the call manager at {base} did not answer: {e}"))
                }
            }
        }
        Err(last_error.unwrap_or_else(|| "no call manager to ask".to_string()))
    }
}

impl NodeCallSupervisor {
    /// A site's push (`docs/PUSH.md` §2.4), sent by the first call manager
    /// with the publisher's keys; one without (503) passes it on.
    pub async fn send_push(
        &self,
        push: &fcp_core::push::RelayedPush,
    ) -> Result<(u16, serde_json::Value), String> {
        let mut last_error = None;
        for base in self
            .nodes_where(|status| status != ClusterNodeStatus::Offline)
            .await?
        {
            let builder = self.http.post(format!("{base}/push")).json(push);
            let builder = match &self.config.auth_token {
                Some(token) => builder.bearer_auth(token),
                None => builder,
            };
            match builder.send().await {
                Ok(response) => {
                    let status = response.status().as_u16();
                    let body = response
                        .json::<serde_json::Value>()
                        .await
                        .unwrap_or_else(|_| serde_json::json!({}));
                    if status == 503 {
                        last_error = Some(format!(
                            "the call manager at {base}: {}",
                            body["error"].as_str().unwrap_or("unavailable")
                        ));
                        continue;
                    }
                    return Ok((status, body));
                }
                Err(e) => {
                    last_error = Some(format!("the call manager at {base} did not answer: {e}"))
                }
            }
        }
        Err(last_error.unwrap_or_else(|| "no call manager to ask".to_string()))
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
