//! SIP Trunk management API endpoints.

use crate::auth::AuthIdentity;
use crate::error::ApiError;
use crate::state::ApiState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use fcp_core::permissions::Permission;
use fcp_core::storage::TrunkStore;
use fcp_core::{TenantId, Trunk, TrunkDirection, TrunkId};
use fcp_events::{PlatformEvent, SystemEvent};
use serde::{Deserialize, Serialize};
use std::time::SystemTime;
use tracing::info;

/// Helper macro to get storage or return error.
macro_rules! require_storage {
    ($state:expr) => {
        $state.storage.as_ref().ok_or_else(|| {
            ApiError::NotFound("Storage is required but not configured".to_string())
        })?
    };
}

/// Trunk response.
#[derive(Debug, Serialize)]
pub struct TrunkResponse {
    pub id: String,
    pub tenant_id: String,
    pub name: String,
    pub enabled: bool,
    pub direction: String,
    pub priority: i32,
    pub weight: i32,
    pub hosts: Vec<String>,
    pub transport_protocol: String,
    pub transport_port: i32,
    pub verify_tls: bool,
    pub has_credentials: bool,
    pub auth_username: Option<String>,
    pub auth_realm: Option<String>,
    pub register: bool,
    pub register_expiry: i32,
    pub max_concurrent_calls: i32,
    pub max_failures: i32,
    pub recovery_interval_secs: i32,
    pub health_check_enabled: bool,
    pub emergency_capable: bool,
    pub health_check_interval_secs: i32,
    pub codec_prefs: Vec<String>,
    pub caller_id_override: Option<String>,
    pub from_domain: Option<String>,
    pub strip_digits: i32,
    pub prepend_digits: Option<String>,
    pub cps_limit: i32,
    pub description: Option<String>,
    pub srtp_mode: String,
    pub dtmf_mode: String,
    pub session_timer_secs: i32,
    pub t38_enabled: bool,
    pub outbound_proxy: Option<String>,
    pub trunk_group: Option<String>,
    pub header_rules: Option<String>,
    pub health_status: String,
    pub consecutive_failures: u32,
    pub uptime_secs: Option<u64>,
    pub created_at: SystemTime,
    pub updated_at: SystemTime,
}

impl From<Trunk> for TrunkResponse {
    fn from(t: Trunk) -> Self {
        Self {
            id: t.id.to_string(),
            tenant_id: t.tenant_id.map(|t| t.to_string()).unwrap_or_default(),
            name: t.name,
            enabled: t.enabled,
            direction: t.direction.to_string(),
            priority: t.priority as i32,
            weight: t.weight as i32,
            hosts: t.hosts,
            // The JSON this API has always answered with, from the one
            // trunk model (`docs/TENANT_TRUNKS.md` §4).
            transport_protocol: t.transport.protocol.to_string().to_lowercase(),
            transport_port: t.transport.port.unwrap_or(5060) as i32,
            verify_tls: t.transport.verify_tls,
            has_credentials: t.credentials.is_some(),
            auth_username: t.credentials.as_ref().map(|c| c.username.clone()),
            auth_realm: t.credentials.as_ref().and_then(|c| c.realm.clone()),
            register: t.register,
            register_expiry: t.register_expiry.unwrap_or(3600) as i32,
            max_concurrent_calls: t.max_concurrent_calls as i32,
            max_failures: t.failover.max_failures as i32,
            recovery_interval_secs: t.failover.recovery_interval_secs as i32,
            health_check_enabled: t.failover.health_check_enabled,
            emergency_capable: t.emergency_capable,
            health_check_interval_secs: t.failover.health_check_interval_secs as i32,
            codec_prefs: t.codec.allowed_codecs,
            caller_id_override: t.caller_id_override,
            from_domain: t.from_domain,
            strip_digits: t
                .egress_number_transform
                .as_ref()
                .map(|x| x.strip_leading as i32)
                .unwrap_or(0),
            prepend_digits: t
                .egress_number_transform
                .as_ref()
                .map(|x| x.prepend.clone())
                .filter(|p| !p.is_empty()),
            cps_limit: t.cps_limit as i32,
            description: t.description,
            srtp_mode: t.srtp.mode.as_str().to_string(),
            dtmf_mode: t.dtmf.mode.as_str().to_string(),
            session_timer_secs: t.session_timer_secs as i32,
            t38_enabled: t.t38_enabled,
            outbound_proxy: t.outbound_proxy,
            trunk_group: t.trunk_group,
            header_rules: (!t.header_rules.is_empty()).then(|| t.header_rules.join(", ")),
            health_status: "unknown".to_string(),
            consecutive_failures: 0,
            uptime_secs: None,
            created_at: t.created_at,
            updated_at: t.updated_at,
        }
    }
}

impl TrunkResponse {
    /// Enrich with health data from the health map.
    fn with_health(mut self, health_map: &Option<crate::trunk_health::TrunkHealthMap>) -> Self {
        if !self.enabled {
            self.health_status = "disabled".to_string();
            self.uptime_secs = None;
            return self;
        }
        if let Some(map) = health_map {
            if let Some(entry) = map.get(&self.id) {
                self.health_status = entry.status.to_string();
                self.consecutive_failures = entry.consecutive_failures;
                self.uptime_secs = entry.uptime_secs();
            }
        }
        self
    }
}

/// Create trunk request.
#[derive(Debug, Deserialize)]
pub struct CreateTrunkRequest {
    pub name: String,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub direction: Option<String>,
    #[serde(default)]
    pub priority: Option<i32>,
    #[serde(default)]
    pub weight: Option<i32>,
    #[serde(default)]
    pub hosts: Option<Vec<String>>,
    #[serde(default)]
    pub transport_protocol: Option<String>,
    #[serde(default)]
    pub transport_port: Option<i32>,
    #[serde(default)]
    pub verify_tls: Option<bool>,
    pub auth_username: Option<String>,
    pub auth_password: Option<String>,
    pub auth_realm: Option<String>,
    #[serde(default)]
    pub register: Option<bool>,
    #[serde(default)]
    pub register_expiry: Option<i32>,
    #[serde(default)]
    pub max_concurrent_calls: Option<i32>,
    #[serde(default)]
    pub max_failures: Option<i32>,
    #[serde(default)]
    pub recovery_interval_secs: Option<i32>,
    #[serde(default)]
    pub health_check_enabled: Option<bool>,
    #[serde(default)]
    pub emergency_capable: Option<bool>,
    #[serde(default)]
    pub health_check_interval_secs: Option<i32>,
    #[serde(default)]
    pub codec_prefs: Option<Vec<String>>,
    pub caller_id_override: Option<String>,
    pub from_domain: Option<String>,
    #[serde(default)]
    pub strip_digits: Option<i32>,
    pub prepend_digits: Option<String>,
    #[serde(default)]
    pub cps_limit: Option<i32>,
    pub description: Option<String>,
    pub srtp_mode: Option<String>,
    pub dtmf_mode: Option<String>,
    #[serde(default)]
    pub session_timer_secs: Option<i32>,
    #[serde(default)]
    pub t38_enabled: Option<bool>,
    pub outbound_proxy: Option<String>,
    pub trunk_group: Option<String>,
    pub header_rules: Option<String>,
}

/// Update trunk request.
#[derive(Debug, Deserialize)]
pub struct UpdateTrunkRequest {
    pub name: Option<String>,
    pub enabled: Option<bool>,
    pub direction: Option<String>,
    pub priority: Option<i32>,
    pub weight: Option<i32>,
    pub hosts: Option<Vec<String>>,
    pub transport_protocol: Option<String>,
    pub transport_port: Option<i32>,
    pub verify_tls: Option<bool>,
    pub auth_username: Option<String>,
    pub auth_password: Option<String>,
    pub auth_realm: Option<String>,
    pub register: Option<bool>,
    pub register_expiry: Option<i32>,
    pub max_concurrent_calls: Option<i32>,
    pub max_failures: Option<i32>,
    pub recovery_interval_secs: Option<i32>,
    pub health_check_enabled: Option<bool>,
    #[serde(default)]
    pub emergency_capable: Option<bool>,
    pub health_check_interval_secs: Option<i32>,
    pub codec_prefs: Option<Vec<String>>,
    pub caller_id_override: Option<String>,
    pub from_domain: Option<String>,
    pub strip_digits: Option<i32>,
    pub prepend_digits: Option<String>,
    pub cps_limit: Option<i32>,
    pub description: Option<String>,
    pub srtp_mode: Option<String>,
    pub dtmf_mode: Option<String>,
    pub session_timer_secs: Option<i32>,
    pub t38_enabled: Option<bool>,
    pub outbound_proxy: Option<String>,
    pub trunk_group: Option<String>,
    pub header_rules: Option<String>,
}

/// List trunks.
pub async fn list_trunks(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
) -> Result<impl IntoResponse, ApiError> {
    crate::auth::check_permission(&identity, Permission::TrunkRead, state.deployment_mode)?;
    let storage = require_storage!(state);
    let tenant_id = tenant.0;

    let trunks = storage
        .list_trunks(&tenant_id)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to list trunks: {}", e)))?;

    let health_map = &state.trunk_health;
    let response: Vec<TrunkResponse> = trunks
        .into_iter()
        .map(|t| TrunkResponse::from(t).with_health(health_map))
        .collect();
    Ok(Json(response))
}

/// Get a single trunk.
pub async fn get_trunk(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    crate::auth::check_permission(&identity, Permission::TrunkRead, state.deployment_mode)?;
    let storage = require_storage!(state);
    let trunk_id = TrunkId::from_string(id);

    let trunk = storage
        .get_trunk(&trunk_id)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to get trunk: {}", e)))?
        .ok_or_else(|| ApiError::NotFound("Trunk not found".to_string()))?;
    if trunk.tenant_id != Some(tenant.0) {
        return Err(ApiError::NotFound("Trunk not found".to_string()));
    }

    Ok(Json(
        TrunkResponse::from(trunk).with_health(&state.trunk_health),
    ))
}

/// The fields `/v1/trunks` accepts, applied to the one trunk model
/// (`docs/TENANT_TRUNKS.md` §4). The API's JSON is flat — `transport_port`,
/// `srtp_mode`, `strip_digits` — while the model groups them; this is where
/// the two meet, for both create and update.
#[allow(clippy::too_many_arguments)]
struct TrunkFields {
    enabled: Option<bool>,
    direction: Option<String>,
    priority: Option<i32>,
    weight: Option<i32>,
    hosts: Option<Vec<String>>,
    transport_protocol: Option<String>,
    transport_port: Option<i32>,
    verify_tls: Option<bool>,
    auth_username: Option<String>,
    auth_password: Option<String>,
    auth_realm: Option<String>,
    register: Option<bool>,
    register_expiry: Option<i32>,
    max_concurrent_calls: Option<i32>,
    max_failures: Option<i32>,
    recovery_interval_secs: Option<i32>,
    health_check_enabled: Option<bool>,
    emergency_capable: Option<bool>,
    health_check_interval_secs: Option<i32>,
    codec_prefs: Option<Vec<String>>,
    caller_id_override: Option<String>,
    from_domain: Option<String>,
    strip_digits: Option<i32>,
    prepend_digits: Option<String>,
    cps_limit: Option<i32>,
    description: Option<String>,
    srtp_mode: Option<String>,
    dtmf_mode: Option<String>,
    session_timer_secs: Option<i32>,
    t38_enabled: Option<bool>,
    outbound_proxy: Option<String>,
    trunk_group: Option<String>,
    header_rules: Option<String>,
}

fn apply_trunk_fields(trunk: &mut Trunk, fields: TrunkFields) -> Result<(), ApiError> {
    use fcp_core::trunk::{SrtpMode, TrunkCredentials, TrunkDtmfMode};

    if let Some(enabled) = fields.enabled {
        trunk.enabled = enabled;
    }
    if let Some(dir) = &fields.direction {
        trunk.direction = dir
            .parse::<TrunkDirection>()
            .map_err(ApiError::BadRequest)?;
    }
    if let Some(priority) = fields.priority {
        trunk.priority = priority.max(0) as u32;
    }
    if let Some(weight) = fields.weight {
        trunk.weight = weight.max(0) as u32;
    }
    if let Some(hosts) = fields.hosts {
        trunk.hosts = hosts;
    }
    if let Some(proto) = &fields.transport_protocol {
        trunk.transport.protocol = match proto.to_ascii_lowercase().as_str() {
            "tcp" => fcp_core::transport::TransportType::Tcp,
            "tls" => fcp_core::transport::TransportType::Tls,
            "ws" => fcp_core::transport::TransportType::Ws,
            "wss" => fcp_core::transport::TransportType::Wss,
            "udp" => fcp_core::transport::TransportType::Udp,
            other => {
                return Err(ApiError::BadRequest(format!(
                    "transport_protocol '{other}' must be udp, tcp, tls, ws or wss"
                )))
            }
        };
    }
    if let Some(port) = fields.transport_port {
        trunk.transport.port = u16::try_from(port).ok().filter(|p| *p > 0);
    }
    if let Some(verify) = fields.verify_tls {
        trunk.transport.verify_tls = verify;
    }
    // Credentials are one thing; a username with no password is a trunk
    // that would fail its first challenge.
    if fields.auth_username.is_some()
        || fields.auth_password.is_some()
        || fields.auth_realm.is_some()
    {
        let mut credentials = trunk.credentials.clone().unwrap_or(TrunkCredentials {
            username: String::new(),
            password: String::new(),
            realm: None,
        });
        if let Some(username) = fields.auth_username {
            credentials.username = username;
        }
        if let Some(password) = fields.auth_password {
            credentials.password = password;
        }
        if let Some(realm) = fields.auth_realm {
            credentials.realm = Some(realm);
        }
        trunk.credentials = (!credentials.username.is_empty()).then_some(credentials);
    }
    if let Some(register) = fields.register {
        trunk.register = register;
    }
    if let Some(expiry) = fields.register_expiry {
        trunk.register_expiry = u32::try_from(expiry).ok().filter(|e| *e > 0);
    }
    if let Some(max) = fields.max_concurrent_calls {
        trunk.max_concurrent_calls = max.max(0) as u32;
    }
    if let Some(max) = fields.max_failures {
        trunk.failover.max_failures = max.max(1) as u32;
    }
    if let Some(secs) = fields.recovery_interval_secs {
        trunk.failover.recovery_interval_secs = secs.max(1) as u64;
    }
    if let Some(enabled) = fields.health_check_enabled {
        trunk.failover.health_check_enabled = enabled;
    }
    if let Some(capable) = fields.emergency_capable {
        trunk.emergency_capable = capable;
    }
    if let Some(secs) = fields.health_check_interval_secs {
        trunk.failover.health_check_interval_secs = secs.max(1) as u64;
    }
    if let Some(codecs) = fields.codec_prefs {
        trunk.codec.allowed_codecs = codecs;
    }
    if fields.caller_id_override.is_some() {
        trunk.caller_id_override = fields.caller_id_override;
    }
    if fields.from_domain.is_some() {
        trunk.from_domain = fields.from_domain;
    }
    if fields.strip_digits.is_some() || fields.prepend_digits.is_some() {
        let mut transform = trunk.egress_number_transform.clone().unwrap_or_default();
        if let Some(n) = fields.strip_digits {
            transform.strip_leading = n.max(0) as usize;
        }
        if let Some(prepend) = fields.prepend_digits {
            transform.prepend = prepend;
        }
        trunk.egress_number_transform = (!transform.is_noop()).then_some(transform);
    }
    if let Some(cps) = fields.cps_limit {
        trunk.cps_limit = cps.max(0) as u32;
    }
    if fields.description.is_some() {
        trunk.description = fields.description;
    }
    if let Some(mode) = &fields.srtp_mode {
        trunk.srtp.mode = SrtpMode::parse(mode);
    }
    if let Some(mode) = &fields.dtmf_mode {
        trunk.dtmf.mode = TrunkDtmfMode::parse(mode);
    }
    if let Some(secs) = fields.session_timer_secs {
        trunk.session_timer_secs = secs.max(0) as u32;
    }
    if let Some(t38) = fields.t38_enabled {
        trunk.t38_enabled = t38;
    }
    if fields.outbound_proxy.is_some() {
        trunk.outbound_proxy = fields.outbound_proxy;
    }
    if fields.trunk_group.is_some() {
        trunk.trunk_group = fields.trunk_group;
    }
    if let Some(rules) = fields.header_rules {
        trunk.header_rules = rules
            .split(',')
            .map(str::trim)
            .filter(|r| !r.is_empty())
            .map(str::to_string)
            .collect();
    }
    Ok(())
}

impl From<CreateTrunkRequest> for TrunkFields {
    fn from(r: CreateTrunkRequest) -> Self {
        Self {
            enabled: r.enabled,
            direction: r.direction,
            priority: r.priority,
            weight: r.weight,
            hosts: r.hosts,
            transport_protocol: r.transport_protocol,
            transport_port: r.transport_port,
            verify_tls: r.verify_tls,
            auth_username: r.auth_username,
            auth_password: r.auth_password,
            auth_realm: r.auth_realm,
            register: r.register,
            register_expiry: r.register_expiry,
            max_concurrent_calls: r.max_concurrent_calls,
            max_failures: r.max_failures,
            recovery_interval_secs: r.recovery_interval_secs,
            health_check_enabled: r.health_check_enabled,
            emergency_capable: r.emergency_capable,
            health_check_interval_secs: r.health_check_interval_secs,
            codec_prefs: r.codec_prefs,
            caller_id_override: r.caller_id_override,
            from_domain: r.from_domain,
            strip_digits: r.strip_digits,
            prepend_digits: r.prepend_digits,
            cps_limit: r.cps_limit,
            description: r.description,
            srtp_mode: r.srtp_mode,
            dtmf_mode: r.dtmf_mode,
            session_timer_secs: r.session_timer_secs,
            t38_enabled: r.t38_enabled,
            outbound_proxy: r.outbound_proxy,
            trunk_group: r.trunk_group,
            header_rules: r.header_rules,
        }
    }
}

impl From<UpdateTrunkRequest> for TrunkFields {
    fn from(r: UpdateTrunkRequest) -> Self {
        Self {
            enabled: r.enabled,
            direction: r.direction,
            priority: r.priority,
            weight: r.weight,
            hosts: r.hosts,
            transport_protocol: r.transport_protocol,
            transport_port: r.transport_port,
            verify_tls: r.verify_tls,
            auth_username: r.auth_username,
            auth_password: r.auth_password,
            auth_realm: r.auth_realm,
            register: r.register,
            register_expiry: r.register_expiry,
            max_concurrent_calls: r.max_concurrent_calls,
            max_failures: r.max_failures,
            recovery_interval_secs: r.recovery_interval_secs,
            health_check_enabled: r.health_check_enabled,
            emergency_capable: r.emergency_capable,
            health_check_interval_secs: r.health_check_interval_secs,
            codec_prefs: r.codec_prefs,
            caller_id_override: r.caller_id_override,
            from_domain: r.from_domain,
            strip_digits: r.strip_digits,
            prepend_digits: r.prepend_digits,
            cps_limit: r.cps_limit,
            description: r.description,
            srtp_mode: r.srtp_mode,
            dtmf_mode: r.dtmf_mode,
            session_timer_secs: r.session_timer_secs,
            t38_enabled: r.t38_enabled,
            outbound_proxy: r.outbound_proxy,
            trunk_group: r.trunk_group,
            header_rules: r.header_rules,
        }
    }
}

/// Tell every node that a trunk changed, so the ones routing calls re-read
/// the tenants' trunks now rather than at the next backstop tick
/// (`docs/TENANT_TRUNKS.md` §4).
pub(crate) async fn announce_trunk_change(state: &ApiState, entity_id: &str, tenant_id: TenantId) {
    let _ = state
        .event_bus
        .publish(PlatformEvent::System(SystemEvent::ConfigChanged {
            entity_type: "trunk".to_string(),
            entity_id: entity_id.to_string(),
            tenant_id,
            timestamp: SystemTime::now(),
        }))
        .await;
}

/// Create a new trunk.
pub async fn create_trunk(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    Json(req): Json<CreateTrunkRequest>,
) -> Result<impl IntoResponse, ApiError> {
    crate::auth::check_permission(&identity, Permission::TrunkCreate, state.deployment_mode)?;
    let storage = require_storage!(state);
    let tenant_id = tenant.0;

    if req.name.trim().is_empty() {
        return Err(ApiError::BadRequest("Trunk name is required".to_string()));
    }

    let name = req.name.clone();
    let mut trunk = Trunk::new(name, tenant_id);
    apply_trunk_fields(&mut trunk, TrunkFields::from(req))?;

    info!(trunk_name = %trunk.name, trunk_id = %trunk.id, "Creating SIP trunk");

    storage
        .put_trunk(trunk.clone())
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to create trunk: {}", e)))?;

    announce_trunk_change(&state, &trunk.id.to_string(), tenant_id).await;

    Ok((StatusCode::CREATED, Json(TrunkResponse::from(trunk))))
}

/// Update an existing trunk.
pub async fn update_trunk(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    Path(id): Path<String>,
    Json(req): Json<UpdateTrunkRequest>,
) -> Result<impl IntoResponse, ApiError> {
    crate::auth::check_permission(&identity, Permission::TrunkUpdate, state.deployment_mode)?;
    let storage = require_storage!(state);
    let trunk_id = TrunkId::from_string(id);

    let mut trunk = storage
        .get_trunk(&trunk_id)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to get trunk: {}", e)))?
        .ok_or_else(|| ApiError::NotFound("Trunk not found".to_string()))?;
    if trunk.tenant_id != Some(tenant.0) {
        return Err(ApiError::NotFound("Trunk not found".to_string()));
    }

    if let Some(name) = req.name.clone() {
        trunk.name = name;
    }
    apply_trunk_fields(&mut trunk, TrunkFields::from(req))?;

    trunk.updated_at = SystemTime::now();

    info!(trunk_name = %trunk.name, trunk_id = %trunk.id, "Updating SIP trunk");

    storage
        .put_trunk(trunk.clone())
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to update trunk: {}", e)))?;

    announce_trunk_change(&state, &trunk.id.to_string(), tenant.0).await;

    Ok(Json(TrunkResponse::from(trunk)))
}

/// Delete a trunk.
pub async fn delete_trunk(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    crate::auth::check_permission(&identity, Permission::TrunkDelete, state.deployment_mode)?;
    let storage = require_storage!(state);
    let trunk_id = TrunkId::from_string(id);

    // Verify it exists
    let trunk = storage
        .get_trunk(&trunk_id)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to get trunk: {}", e)))?
        .ok_or_else(|| ApiError::NotFound("Trunk not found".to_string()))?;
    if trunk.tenant_id != Some(tenant.0) {
        return Err(ApiError::NotFound("Trunk not found".to_string()));
    }

    info!(trunk_name = %trunk.name, trunk_id = %trunk.id, "Deleting SIP trunk");

    storage
        .delete_trunk(&trunk_id)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to delete trunk: {}", e)))?;

    announce_trunk_change(&state, &trunk_id.to_string(), tenant.0).await;

    Ok(StatusCode::NO_CONTENT)
}

/// Test trunk connectivity by sending an OPTIONS probe.
pub async fn test_trunk(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    crate::auth::check_permission(&identity, Permission::TrunkUpdate, state.deployment_mode)?;
    let storage = require_storage!(state);
    let trunk_id = TrunkId::from_string(id);

    let trunk = storage
        .get_trunk(&trunk_id)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to get trunk: {}", e)))?
        .ok_or_else(|| ApiError::NotFound("Trunk not found".to_string()))?;
    if trunk.tenant_id != Some(tenant.0) {
        return Err(ApiError::NotFound("Trunk not found".to_string()));
    }

    if trunk.hosts.is_empty() {
        return Err(ApiError::BadRequest(
            "Trunk has no hosts configured".to_string(),
        ));
    }

    info!(trunk_name = %trunk.name, "Sending OPTIONS test probe");
    let success = crate::trunk_health::probe_trunk_hosts_public(&trunk).await;

    Ok(Json(serde_json::json!({
        "trunk_id": trunk.id.to_string(),
        "trunk_name": trunk.name,
        "success": success,
        "message": if success { "Trunk is reachable" } else { "Trunk did not respond" },
    })))
}

/// Clone a trunk (create a copy with a new name).
pub async fn clone_trunk(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    crate::auth::check_permission(&identity, Permission::TrunkCreate, state.deployment_mode)?;
    let storage = require_storage!(state);
    let trunk_id = TrunkId::from_string(id);

    let source = storage
        .get_trunk(&trunk_id)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to get trunk: {}", e)))?
        .ok_or_else(|| ApiError::NotFound("Trunk not found".to_string()))?;
    if source.tenant_id != Some(tenant.0) {
        return Err(ApiError::NotFound("Trunk not found".to_string()));
    }

    let mut clone = source.clone();
    clone.id = fcp_core::TrunkId::new();
    clone.name = format!("{} (copy)", clone.name);
    clone.tenant_id = Some(tenant.0);
    clone.enabled = false; // Disable clone by default
    let now = SystemTime::now();
    clone.created_at = now;
    clone.updated_at = now;

    info!(trunk_name = %clone.name, source = %source.name, "Cloning SIP trunk");

    storage
        .put_trunk(clone.clone())
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to create trunk clone: {}", e)))?;

    announce_trunk_change(&state, &clone.id.to_string(), tenant.0).await;

    Ok((StatusCode::CREATED, Json(TrunkResponse::from(clone))))
}

/// Get trunk stats (ASR, ACD, PDD, hourly volume) from CDRs.
pub async fn get_trunk_stats(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    crate::auth::check_permission(&identity, Permission::TrunkRead, state.deployment_mode)?;
    let storage = require_storage!(state);
    let trunk_id = TrunkId::from_string(id);

    let trunk = storage
        .get_trunk(&trunk_id)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to get trunk: {}", e)))?
        .ok_or_else(|| ApiError::NotFound("Trunk not found".to_string()))?;
    if trunk.tenant_id != Some(tenant.0) {
        return Err(ApiError::NotFound("Trunk not found".to_string()));
    }

    let stats = storage
        .get_trunk_stats(&trunk.name)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to get trunk stats: {}", e)))?;

    Ok(Json(stats))
}
