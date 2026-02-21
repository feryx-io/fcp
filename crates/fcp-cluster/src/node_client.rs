//! HTTP client for inter-node communication.
//!
//! Used for cross-node operations like call parking/pickup, hangup, and
//! supervisor injection. All requests are authenticated with a shared
//! cluster secret via Bearer token.
//!
//! Propagates W3C Trace Context (`traceparent`) and `x-fcp-node-id` headers
//! for distributed tracing correlation across cluster nodes.

use crate::ClusterMember;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use tracing::{debug, Span};

/// HTTP client for communicating with other cluster nodes.
#[derive(Clone)]
pub struct NodeClient {
    http: reqwest::Client,
    cluster_secret: String,
    node_id: String,
}

/// Result of an unpark operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnparkResult {
    pub call_id: String,
    pub orbit: u16,
    pub node_id: String,
}

/// Request to unpark a call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnparkRequest {
    pub orbit: u16,
}

/// Request to hang up a call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HangupRequest {
    pub call_id: String,
    pub reason: Option<String>,
}

/// Request for supervisor operations (monitor/whisper/barge).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SupervisorRequest {
    pub call_id: String,
    pub mode: String,
    pub supervisor_extension: String,
}

/// Build a W3C `traceparent` header value from the current tracing span.
///
/// Extracts the trace ID and span ID from the current `tracing::Span` and
/// formats them as `00-{trace_id}-{span_id}-01`. Returns `None` if no span
/// is active or if IDs are not available.
fn traceparent_header() -> Option<String> {
    let span = Span::current();
    let id = span.id()?;
    // Format: 00-<trace_id>-<span_id>-01
    // We use the span ID as both trace and span for correlation.
    // In a full OpenTelemetry setup, these would come from the OTel layer.
    let span_id = format!("{:016x}", id.into_u64());
    // Use span ID padded to 32 hex chars as trace ID
    let trace_id = format!("{:0>32}", span_id);
    Some(format!("00-{}-{}-01", trace_id, span_id))
}

impl NodeClient {
    /// Create a new node client.
    pub fn new(cluster_secret: String, node_id: String) -> Self {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .expect("Failed to create HTTP client");

        Self {
            http,
            cluster_secret,
            node_id,
        }
    }

    /// Build a URL for an internal endpoint on a given node.
    fn internal_url(&self, node: &ClusterMember, path: &str) -> String {
        format!("http://{}/internal{}", node.http_address, path)
    }

    /// Attach tracing and node ID headers to an outgoing request.
    fn inject_headers(
        &self,
        builder: reqwest::RequestBuilder,
    ) -> reqwest::RequestBuilder {
        let builder = builder.header("x-fcp-node-id", &self.node_id);
        if let Some(tp) = traceparent_header() {
            builder.header("traceparent", tp)
        } else {
            builder
        }
    }

    /// Unpark a call from a remote node.
    pub async fn unpark(
        &self,
        node: &ClusterMember,
        orbit: u16,
    ) -> Result<UnparkResult> {
        let url = self.internal_url(node, "/unpark");

        debug!(url = %url, orbit = orbit, "Sending unpark request to remote node");

        let req = self
            .http
            .post(&url)
            .bearer_auth(&self.cluster_secret)
            .json(&UnparkRequest { orbit });

        let resp = self.inject_headers(req).send().await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Unpark request failed ({}): {}", status, body);
        }

        let result: UnparkResult = resp.json().await?;
        Ok(result)
    }

    /// Hang up a call on a remote node.
    pub async fn hangup(
        &self,
        node: &ClusterMember,
        call_id: &str,
        reason: Option<String>,
    ) -> Result<()> {
        let url = self.internal_url(node, "/hangup");

        debug!(url = %url, call_id = %call_id, "Sending hangup request to remote node");

        let req = self
            .http
            .post(&url)
            .bearer_auth(&self.cluster_secret)
            .json(&HangupRequest {
                call_id: call_id.to_string(),
                reason,
            });

        let resp = self.inject_headers(req).send().await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Hangup request failed ({}): {}", status, body);
        }

        Ok(())
    }

    /// Inject a supervisor (monitor/whisper/barge) into a call on a remote node.
    pub async fn supervisor(
        &self,
        node: &ClusterMember,
        request: SupervisorRequest,
    ) -> Result<()> {
        let url = self.internal_url(node, "/supervisor");

        debug!(
            url = %url,
            call_id = %request.call_id,
            mode = %request.mode,
            "Sending supervisor request to remote node"
        );

        let req = self
            .http
            .post(&url)
            .bearer_auth(&self.cluster_secret)
            .json(&request);

        let resp = self.inject_headers(req).send().await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Supervisor request failed ({}): {}", status, body);
        }

        Ok(())
    }
}
