//! HTTP API client for FCP server

use super::{error::ApiError, models::*};
use anyhow::Result;
use reqwest::Method;
use reqwest_middleware::{ClientBuilder, ClientWithMiddleware};
use reqwest_retry::{policies::ExponentialBackoff, RetryTransientMiddleware};
use serde_json;
use std::time::Duration;

/// API client for communicating with FCP server
pub struct ApiClient {
    client: ClientWithMiddleware,
    base_url: String,
    token: Option<String>,
}

impl ApiClient {
    /// Create a new API client
    pub fn new(base_url: String, token: Option<String>) -> Result<Self> {
        // Configure retry policy: 3 retries with exponential backoff from 100ms to 5s
        let retry_policy = ExponentialBackoff::builder()
            .retry_bounds(Duration::from_millis(100), Duration::from_secs(5))
            .build_with_max_retries(3);

        let reqwest_client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()?;

        let client = ClientBuilder::new(reqwest_client)
            .with(RetryTransientMiddleware::new_with_policy(retry_policy))
            .build();

        Ok(Self {
            client,
            base_url: base_url.trim_end_matches('/').to_string(),
            token,
        })
    }

    /// Get the base URL of the API
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Make an HTTP request to the API
    async fn request<T>(&self, method: Method, path: &str) -> Result<T>
    where
        T: serde::de::DeserializeOwned,
    {
        let url = format!("{}/api/v1{}", self.base_url, path);
        let mut req = self.client.request(method, &url);

        // Add authorization header if token is configured
        if let Some(token) = &self.token {
            req = req.header("Authorization", format!("Bearer {}", token));
        }

        let resp = req.send().await.map_err(|e| ApiError::Network(e.to_string()))?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(ApiError::Http(status, body).into());
        }

        resp.json().await.map_err(|e| ApiError::Deserialization(e.to_string()).into())
    }

    /// Make an HTTP request with a JSON body
    async fn request_with_body<B, T>(&self, method: Method, path: &str, body: &B) -> Result<T>
    where
        B: serde::Serialize,
        T: serde::de::DeserializeOwned,
    {
        let url = format!("{}/api/v1{}", self.base_url, path);
        let json_body = serde_json::to_string(body)
            .map_err(|e| anyhow::anyhow!("Failed to serialize request body: {}", e))?;

        let mut req = self.client
            .request(method, &url)
            .header("Content-Type", "application/json")
            .body(json_body);

        if let Some(token) = &self.token {
            req = req.header("Authorization", format!("Bearer {}", token));
        }

        let resp = req.send().await.map_err(|e| ApiError::Network(e.to_string()))?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(ApiError::Http(status, body).into());
        }

        resp.json().await.map_err(|e| ApiError::Deserialization(e.to_string()).into())
    }

    /// Make an HTTP request without expecting a response body
    async fn request_no_content(&self, method: Method, path: &str) -> Result<()> {
        let url = format!("{}/api/v1{}", self.base_url, path);
        let mut req = self.client.request(method, &url);

        if let Some(token) = &self.token {
            req = req.header("Authorization", format!("Bearer {}", token));
        }

        let resp = req.send().await.map_err(|e| ApiError::Network(e.to_string()))?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(ApiError::Http(status, body).into());
        }

        Ok(())
    }

    /// Make an HTTP request with a JSON body but no response content expected
    async fn request_with_body_no_content<B>(&self, method: Method, path: &str, body: &B) -> Result<()>
    where
        B: serde::Serialize,
    {
        let url = format!("{}/api/v1{}", self.base_url, path);
        let json_body = serde_json::to_string(body)
            .map_err(|e| anyhow::anyhow!("Failed to serialize request body: {}", e))?;

        let mut req = self.client
            .request(method, &url)
            .header("Content-Type", "application/json")
            .body(json_body);

        if let Some(token) = &self.token {
            req = req.header("Authorization", format!("Bearer {}", token));
        }

        let resp = req.send().await.map_err(|e| ApiError::Network(e.to_string()))?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(ApiError::Http(status, body).into());
        }

        Ok(())
    }

    // ==================== System Endpoints ====================

    /// Get server health
    pub async fn get_health(&self) -> Result<HealthResponse> {
        self.request(Method::GET, "/health").await
    }

    /// Get server status
    pub async fn get_status(&self) -> Result<StatusResponse> {
        self.request(Method::GET, "/status").await
    }

    // ==================== User Endpoints ====================

    /// List all users
    pub async fn list_users(&self) -> Result<Vec<UserResponse>> {
        self.request(Method::GET, "/users").await
    }

    /// Get a specific user
    pub async fn get_user(&self, username: &str) -> Result<UserResponse> {
        self.request(Method::GET, &format!("/users/{}", username)).await
    }

    /// Create a new user
    pub async fn create_user(&self, req: &CreateUserRequest) -> Result<UserResponse> {
        self.request_with_body(Method::POST, "/users", req).await
    }

    /// Update a user
    pub async fn update_user(&self, username: &str, req: &UpdateUserRequest) -> Result<UserResponse> {
        self.request_with_body(Method::PUT, &format!("/users/{}", username), req).await
    }

    /// Update user password
    pub async fn update_password(&self, username: &str, req: &UpdatePasswordRequest) -> Result<()> {
        self.request_with_body_no_content(Method::PUT, &format!("/users/{}/password", username), req).await
    }

    /// Delete a user
    pub async fn delete_user(&self, username: &str) -> Result<()> {
        self.request_no_content(Method::DELETE, &format!("/users/{}", username)).await
    }

    // ==================== Registration Endpoints ====================

    /// List registrations for an AOR
    pub async fn list_registrations(&self, aor: &str) -> Result<Vec<RegistrationResponse>> {
        self.request(Method::GET, &format!("/registrations/{}", aor)).await
    }

    /// Delete all registrations for an AOR
    pub async fn delete_registrations(&self, aor: &str) -> Result<()> {
        self.request_no_content(Method::DELETE, &format!("/registrations/{}", aor)).await
    }

    /// Delete a specific registration
    pub async fn delete_registration(&self, aor: &str, contact: &str) -> Result<()> {
        self.request_no_content(Method::DELETE, &format!("/registrations/{}/{}", aor, contact)).await
    }

    // ==================== Call Endpoints ====================

    /// List all active calls
    pub async fn list_calls(&self) -> Result<Vec<CallResponse>> {
        self.request(Method::GET, "/calls").await
    }

    /// Get a specific call
    pub async fn get_call(&self, call_id: &str) -> Result<CallResponse> {
        self.request(Method::GET, &format!("/calls/{}", call_id)).await
    }

    /// Hang up a call
    pub async fn hangup_call(&self, call_id: &str) -> Result<HangupResponse> {
        self.request(Method::DELETE, &format!("/calls/{}", call_id)).await
    }

    /// Start recording a call
    pub async fn start_recording(&self, call_id: &str, path: &str) -> Result<RecordingResponse> {
        let req = StartRecordingRequest { path: path.to_string() };
        self.request_with_body(Method::POST, &format!("/calls/{}/recording/start", call_id), &req).await
    }

    /// Stop recording a call
    pub async fn stop_recording(&self, call_id: &str) -> Result<RecordingResponse> {
        self.request(Method::POST, &format!("/calls/{}/recording/stop", call_id)).await
    }

    // ==================== Debug Endpoints ====================

    /// Get debug summary
    pub async fn get_debug_summary(&self) -> Result<DebugSummaryResponse> {
        self.request(Method::GET, "/debug/summary").await
    }

    /// List active dialogs
    pub async fn list_dialogs(&self) -> Result<Vec<DialogResponse>> {
        self.request(Method::GET, "/debug/dialogs").await
    }

    /// List active subscriptions
    pub async fn list_subscriptions(&self) -> Result<Vec<SubscriptionResponse>> {
        self.request(Method::GET, "/debug/subscriptions").await
    }

    /// List rate limits
    pub async fn list_rate_limits(&self) -> Result<Vec<RateLimitResponse>> {
        self.request(Method::GET, "/debug/rate-limits").await
    }

    /// List active calls (debug view)
    pub async fn list_debug_calls(&self) -> Result<Vec<DebugCallResponse>> {
        self.request(Method::GET, "/debug/calls").await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_client_creation() {
        let client = ApiClient::new("http://localhost:8080".to_string(), None);
        assert!(client.is_ok());
    }

    #[test]
    fn test_base_url_trailing_slash() {
        let client = ApiClient::new("http://localhost:8080/".to_string(), None).unwrap();
        assert_eq!(client.base_url, "http://localhost:8080");
    }
}
