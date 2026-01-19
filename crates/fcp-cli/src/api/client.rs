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

    // ==================== Tenant Endpoints ====================

    /// List all tenants
    pub async fn list_tenants(&self) -> Result<Vec<TenantResponse>> {
        self.request(Method::GET, "/tenants").await
    }

    /// Get a specific tenant
    pub async fn get_tenant(&self, id: &str) -> Result<TenantResponse> {
        self.request(Method::GET, &format!("/tenants/{}", id)).await
    }

    /// Create a new tenant
    pub async fn create_tenant(&self, req: &CreateTenantRequest) -> Result<TenantResponse> {
        self.request_with_body(Method::POST, "/tenants", req).await
    }

    /// Update a tenant
    pub async fn update_tenant(&self, id: &str, req: &UpdateTenantRequest) -> Result<TenantResponse> {
        self.request_with_body(Method::PUT, &format!("/tenants/{}", id), req).await
    }

    /// Delete a tenant
    pub async fn delete_tenant(&self, id: &str) -> Result<()> {
        self.request_no_content(Method::DELETE, &format!("/tenants/{}", id)).await
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

    /// Set digest password (Cisco-style separate SIP credentials)
    pub async fn set_digest_password(&self, username: &str, digest_password: &str) -> Result<()> {
        let req = serde_json::json!({
            "digest_password": digest_password
        });
        self.request_with_body_no_content(Method::PUT, &format!("/users/{}/digest-password", username), &req).await
    }

    /// Remove digest password (revert to using main password for SIP)
    pub async fn remove_digest_password(&self, username: &str) -> Result<()> {
        self.request_no_content(Method::DELETE, &format!("/users/{}/digest-password", username)).await
    }

    /// Delete a user
    pub async fn delete_user(&self, username: &str) -> Result<()> {
        self.request_no_content(Method::DELETE, &format!("/users/{}", username)).await
    }

    // ==================== Registration Endpoints ====================

    /// List registrations for an AOR
    pub async fn list_registrations(&self, aor: &str) -> Result<Vec<RegistrationResponse>> {
        let encoded_aor = urlencoding::encode(aor);
        self.request(Method::GET, &format!("/registrations/{}", encoded_aor)).await
    }

    /// Delete all registrations for an AOR
    pub async fn delete_registrations(&self, aor: &str) -> Result<()> {
        let encoded_aor = urlencoding::encode(aor);
        self.request_no_content(Method::DELETE, &format!("/registrations/{}", encoded_aor)).await
    }

    /// Delete a specific registration
    pub async fn delete_registration(&self, aor: &str, contact: &str) -> Result<()> {
        let encoded_aor = urlencoding::encode(aor);
        let encoded_contact = urlencoding::encode(contact);
        self.request_no_content(Method::DELETE, &format!("/registrations/{}/{}", encoded_aor, encoded_contact)).await
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

    // ==================== Phone Number Endpoints ====================

    /// List phone numbers, optionally filtered by tenant
    pub async fn list_phone_numbers(&self, tenant_id: Option<&str>) -> Result<Vec<PhoneNumberResponse>> {
        let path = match tenant_id {
            Some(id) => format!("/phone-numbers?tenant_id={}", id),
            None => "/phone-numbers".to_string(),
        };
        self.request(Method::GET, &path).await
    }

    /// Get a specific phone number
    pub async fn get_phone_number(&self, id: &str) -> Result<PhoneNumberResponse> {
        self.request(Method::GET, &format!("/phone-numbers/{}", id)).await
    }

    /// Create a new phone number
    pub async fn create_phone_number<B>(&self, req: &B) -> Result<PhoneNumberResponse>
    where
        B: serde::Serialize,
    {
        self.request_with_body(Method::POST, "/phone-numbers", req).await
    }

    /// Update a phone number
    pub async fn update_phone_number<B>(&self, id: &str, req: &B) -> Result<PhoneNumberResponse>
    where
        B: serde::Serialize,
    {
        self.request_with_body(Method::PATCH, &format!("/phone-numbers/{}", id), req).await
    }

    /// Delete a phone number
    pub async fn delete_phone_number(&self, id: &str) -> Result<()> {
        self.request_no_content(Method::DELETE, &format!("/phone-numbers/{}", id)).await
    }

    /// Activate a phone number
    pub async fn activate_phone_number(&self, id: &str) -> Result<PhoneNumberResponse> {
        self.request(Method::POST, &format!("/phone-numbers/{}/activate", id)).await
    }

    // ==================== Device Endpoints ====================

    /// List devices, optionally filtered by tenant
    pub async fn list_devices(&self, tenant_id: Option<&str>) -> Result<Vec<DeviceResponse>> {
        let path = match tenant_id {
            Some(id) => format!("/devices?tenant_id={}", id),
            None => "/devices".to_string(),
        };
        self.request(Method::GET, &path).await
    }

    /// List devices for a specific user
    pub async fn list_devices_by_user(&self, user_id: &str) -> Result<Vec<DeviceResponse>> {
        self.request(Method::GET, &format!("/devices?user_id={}", user_id)).await
    }

    /// Get a specific device
    pub async fn get_device(&self, id: &str) -> Result<DeviceResponse> {
        self.request(Method::GET, &format!("/devices/{}", id)).await
    }

    /// Create a new device
    pub async fn create_device<B>(&self, req: &B) -> Result<DeviceResponse>
    where
        B: serde::Serialize,
    {
        self.request_with_body(Method::POST, "/devices", req).await
    }

    /// Update a device
    pub async fn update_device<B>(&self, id: &str, req: &B) -> Result<DeviceResponse>
    where
        B: serde::Serialize,
    {
        self.request_with_body(Method::PATCH, &format!("/devices/{}", id), req).await
    }

    /// Delete a device
    pub async fn delete_device(&self, id: &str) -> Result<()> {
        self.request_no_content(Method::DELETE, &format!("/devices/{}", id)).await
    }

    // ==================== Line Binding Endpoints ====================

    /// List line bindings, optionally filtered by tenant
    pub async fn list_line_bindings(&self, tenant_id: Option<&str>) -> Result<Vec<LineBindingResponse>> {
        let path = match tenant_id {
            Some(id) => format!("/line-bindings?tenant_id={}", id),
            None => "/line-bindings".to_string(),
        };
        self.request(Method::GET, &path).await
    }

    /// Get a specific line binding
    pub async fn get_line_binding(&self, id: &str) -> Result<LineBindingResponse> {
        self.request(Method::GET, &format!("/line-bindings/{}", id)).await
    }

    /// Create a new line binding
    pub async fn create_line_binding<B>(&self, req: &B) -> Result<LineBindingResponse>
    where
        B: serde::Serialize,
    {
        self.request_with_body(Method::POST, "/line-bindings", req).await
    }

    /// Update a line binding
    pub async fn update_line_binding<B>(&self, id: &str, req: &B) -> Result<LineBindingResponse>
    where
        B: serde::Serialize,
    {
        self.request_with_body(Method::PATCH, &format!("/line-bindings/{}", id), req).await
    }

    /// Delete a line binding
    pub async fn delete_line_binding(&self, id: &str) -> Result<()> {
        self.request_no_content(Method::DELETE, &format!("/line-bindings/{}", id)).await
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
