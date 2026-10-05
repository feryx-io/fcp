//! FCP REST API core functionality.
//!
//! This crate provides the REST API implementation used by FCP services.
//! It includes:
//!
//! - Authentication and authorization
//! - API error handling
//! - State management
//! - v1 API endpoints
//! - Request validation
//! - Router creation and server startup

pub mod audit;
pub mod auth;
pub mod e911_validator;
pub mod error;
pub mod managed;
pub mod offboarding;
pub mod oidc;
pub mod openapi;
pub mod page;
pub mod rate_limit;
pub mod redirection;
pub mod route_permissions;
pub mod saml;
pub mod scim;
pub mod shape;
pub mod site_files;
pub mod state;
pub mod time;
pub mod trunk_health;

/// Where `/v1/trunks` reads what every node sees of a trunk
/// (`docs/TENANT_TRUNKS.md` §9). The API server does not itself join a
/// cluster; whatever wires it up hands it something that can read the
/// views the nodes publish.
#[async_trait::async_trait]
pub trait TrunkNodeReports: Send + Sync {
    async fn reports(&self) -> Vec<fcp_core::trunk_report::TrunkNodeReport>;
}

/// What a call manager would do with a call, asked of a live one
/// (`docs/TENANT_TRUNKS.md` §12, phase 6d). The API does not route calls,
/// so it does not guess: whatever wires it up knows how to reach a node.
#[async_trait::async_trait]
pub trait RoutePreviewer: Send + Sync {
    /// The node's answer as it gave it, or why no node could give one.
    async fn preview(
        &self,
        tenant: fcp_core::TenantId,
        query: &RoutePreviewQuery,
    ) -> std::result::Result<serde_json::Value, String>;
}

/// A request about a call's supervisor (`docs/ENTERPRISE_CALLING.md`
/// §3.11), carried to the call manager holding the call.
#[derive(Debug, Clone)]
pub enum SupervisionRequest {
    /// Put `supervisor` on the call: `mode` is `monitor`, `whisper` or
    /// `barge`; a whisper is heard by `heard_by` (`a` or `b`).
    Join {
        mode: &'static str,
        supervisor: String,
        heard_by: Option<String>,
    },
    /// The call's supervisor.
    Get,
    /// Take the supervisor off.
    Leave,
    /// Live call control (`docs/DEVELOPER_SURFACE.md` phase 8): `action`
    /// is `hold`, `resume`, `transfer`, `redirect`, `play`, `say` or
    /// `hangup`, with its fields in `body`.
    Control {
        action: &'static str,
        body: serde_json::Value,
    },
}

/// Where a call's supervision is asked: the node holding the call answers.
/// The API holds no calls itself; whatever wires it up knows how to reach
/// the nodes.
#[async_trait::async_trait]
pub trait CallSupervisor: Send + Sync {
    /// The node's status and body as it answered, or why no node could.
    async fn supervise(
        &self,
        tenant: fcp_core::TenantId,
        call_id: &str,
        request: SupervisionRequest,
    ) -> std::result::Result<(u16, serde_json::Value), String>;

    /// Tell a device's phone to fetch its configuration again
    /// (`docs/PROVISIONING.md` §8), through a node that can send SIP: the
    /// node's status and body (`{"sent": n}`), or why no node could.
    async fn resync_device(
        &self,
        tenant: fcp_core::TenantId,
        device_id: &str,
    ) -> std::result::Result<(u16, serde_json::Value), String> {
        let _ = (tenant, device_id);
        Err("no call manager can resync a phone".to_string())
    }
}

/// What a route preview asks about.
#[derive(
    Debug, Clone, serde::Deserialize, serde::Serialize, utoipa::ToSchema, utoipa::IntoParams,
)]
#[into_params(parameter_in = Query)]
pub struct RoutePreviewQuery {
    pub to: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>)]
    #[param(value_type = Option<String>)]
    pub source_ip: Option<std::net::IpAddr>,
}
pub mod v1;
pub mod validation;

// Re-export commonly used types
pub use error::ApiError;
pub use state::{ApiState, SecurityPolicy};

use axum::{
    http::{header, Method},
    middleware, Router,
};
use fcp_core::ApiConfig;
use tower_http::{
    cors::{Any, CorsLayer},
    trace::TraceLayer,
};

/// Create the API router with all routes and middleware.
///
/// # Panics
///
/// Panics if `auth_enabled` is true but `auth_token` is not configured.
/// This is intentional — fail closed rather than silently running without auth.
pub fn create_router(state: ApiState, api_config: &ApiConfig) -> Router {
    create_router_with(state, api_config, Router::new())
}

/// [`create_router`] with extra routes merged *inside* the `/api/v1` nest.
///
/// Binaries that add their own v1 endpoints (fcp-admin's cluster routes)
/// use this so they share the auth, CORS, and pre-auth wiring instead of
/// re-implementing it. Routes merged outside the nest would be shadowed by
/// axum's prefix matching.
pub fn create_router_with(
    mut state: ApiState,
    api_config: &ApiConfig,
    extra_v1: Router<ApiState>,
) -> Router {
    // CORS configuration
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::PATCH,
            Method::DELETE,
        ])
        .allow_headers([
            header::CONTENT_TYPE,
            header::AUTHORIZATION,
            header::HeaderName::from_static("x-fcp-provisioning-token"),
            header::HeaderName::from_static(auth::PLATFORM_TENANT_HEADER),
            header::HeaderName::from_static(shape::REQUEST_ID_HEADER),
        ])
        .expose_headers([
            header::RETRY_AFTER,
            header::HeaderName::from_static(shape::REQUEST_ID_HEADER),
            header::HeaderName::from_static(rate_limit::LIMIT_HEADER),
            header::HeaderName::from_static(rate_limit::REMAINING_HEADER),
            header::HeaderName::from_static(rate_limit::RESET_HEADER),
        ]);

    // API tokens' request limits, counted where the state says.
    state.rate_limiter = std::sync::Arc::new(rate_limit::RateLimiter::new(
        api_config.rate_limit.clone(),
        std::sync::Arc::clone(&state.rate_counter),
    ));

    // Admin audit log: inside the auth layer (so it sees the caller's
    // identity and the matched route), on both the protected and the
    // pre-auth routers.
    let audit_layer = middleware::from_fn_with_state(state.clone(), audit::audit_requests);

    // Build the auth-protected API router
    let mut router = Router::new()
        .route(
            "/metrics",
            axum::routing::get(v1::metrics::prometheus_metrics)
                .route_layer(middleware::from_fn(auth::require_admin)),
        )
        .nest(
            "/api/v1",
            v1::router()
                .merge(extra_v1)
                // Inside the audit layer, so a refusal is audited too.
                .layer(middleware::from_fn_with_state(
                    state.clone(),
                    route_permissions::enforce,
                ))
                // Outside the permission check, so a token refused a route
                // still counts against its limit.
                .layer(middleware::from_fn_with_state(
                    state.clone(),
                    rate_limit::enforce,
                ))
                .layer(audit_layer.clone())
                .layer(middleware::from_fn_with_state(
                    state.clone(),
                    managed::refuse_synced_writes,
                )),
        );

    // SCIM 2.0 (`docs/SCIM.md`): the tenant's provisioner, by an API token
    // holding `scim_provision`; behind the bearer layer like the API.
    router = router.nest(
        "/scim/v2",
        scim::router()
            .layer(middleware::from_fn_with_state(
                state.clone(),
                rate_limit::enforce,
            ))
            .layer(audit_layer.clone()),
    );

    // Sign-in sessions (`docs/IDENTITY_AND_DEVICES.md` phase 1): signed
    // with `[api.session] signing_key`, else a key derived from the static
    // token as before. Independent of the bearer middleware.
    state = state.with_session_config(&api_config.session, api_config.auth_token.as_deref());
    // Single sign-on (phase 3): providers send browsers back to
    // `[api.sso] public_url`.
    state = state.with_sso_config(&api_config.sso);
    state = state
        .with_provisioning_require_https(api_config.provisioning_require_https)
        .with_provisioning_config(api_config.provisioning.clone())
        .with_redirection_config(api_config.redirection.clone())
        .with_firmware_config(api_config.firmware.clone());

    // Conditionally apply bearer token authentication middleware
    if api_config.auth_enabled {
        let auth_token = api_config.auth_token.as_ref().unwrap_or_else(|| {
            panic!(
                "FATAL: auth_enabled=true but no auth_token configured. \
                 Set api.auth_token in config or disable auth with api.auth_enabled=false."
            )
        });

        tracing::info!("API authentication enabled");
        let mut auth_config =
            auth::AuthConfig::new(auth_token.clone()).with_deployment_mode(state.deployment_mode);
        // Database-backed `fcp_…` tokens (created via /api/v1/api-tokens)
        if let Some(store) = state
            .storage
            .as_ref()
            .and_then(|s| s.api_token_store().cloned())
        {
            auth_config = auth_config.with_api_token_store(store);
            tracing::info!("API token authentication enabled");
        }
        // Share the session keys and sessions so the bearer middleware also
        // accepts a signed-in user's access token, while its session lives.
        if let Some(ref keys) = state.session_keys {
            auth_config = auth_config.with_session_keys(keys.clone());
        }
        if let Some(sessions) = state
            .storage
            .as_ref()
            .and_then(|s| s.login_session_store().cloned())
        {
            auth_config = auth_config.with_login_sessions(sessions);
        }
        // Share credential store so JWT auth can verify user is still active
        if let Some(ref storage) = state.storage {
            auth_config = auth_config.with_credential_store(storage.credential_store_arc());
        }
        // Store auth config in ApiState for WebSocket first-message auth
        state.auth_config = Some(auth_config.clone());
        router = router.layer(middleware::from_fn(move |req, next| {
            let config = auth_config.clone();
            auth::validate_bearer_token(config, req, next)
        }));
    } else {
        // With authentication off every caller is the platform: the
        // handlers that check a permission need an identity to check, and
        // without one they answered 500 where the rest of the API was open.
        router = router.layer(middleware::from_fn(
            |mut req: axum::extract::Request, next: middleware::Next| async move {
                req.extensions_mut().insert(auth::AuthIdentity::StaticToken);
                next.run(req).await
            },
        ));
    }

    // Pre-auth, WebSocket, and metrics routes merged AFTER the auth layer
    // so they bypass bearer middleware:
    // - pre_auth: login/logout/change-expired-password (user has no token yet)
    // - optional signup: only exposed when explicitly enabled and JWT issuance works
    // - ws: browsers can't set Authorization headers on WebSocket connections
    let enable_signup = api_config.allow_public_signup && state.session_keys.is_some();
    if api_config.allow_public_signup && !enable_signup {
        tracing::warn!(
            "Public signup requested but auth_token/JWT secret is unavailable; signup route disabled"
        );
    }
    router = router.merge(Router::new().nest(
        "/api/v1",
        v1::pre_auth_router(enable_signup).layer(audit_layer),
    ));
    router = router.merge(Router::new().nest("/api/v1", v1::ws_router()));
    // A tenant's phones fetch their files by the key in the path
    // (`docs/PROVISIONING.md`): no bearer token, which a phone cannot send.
    router = router.merge(
        Router::new()
            .route(
                "/provision/:key/:file",
                axum::routing::get(v1::provisioning::serve_file),
            )
            // Its firmware, where its tenant targets an image (§11).
            .route(
                "/provision/:key/firmware/:image/:file",
                axum::routing::get(v1::provisioning::serve_firmware),
            ),
    );
    // `/api/v1/health` says `ok` and a version and nothing else; it is
    // what a load balancer or the API gateway probes, and a probe carries
    // no token (`docs/DEPLOYMENT_GAPS.md` §1.4).
    router = router.merge(Router::new().nest(
        "/api/v1",
        Router::new().route("/health", axum::routing::get(v1::system::health)),
    ));

    let request_id = header::HeaderName::from_static(shape::REQUEST_ID_HEADER);
    router
        .layer(middleware::from_fn(shape::json_errors))
        .layer(cors)
        .layer(TraceLayer::new_for_http())
        // Outermost: every response, errors and CORS refusals included,
        // carries the request's id.
        .layer(tower_http::request_id::PropagateRequestIdLayer::new(
            request_id.clone(),
        ))
        .layer(tower_http::request_id::SetRequestIdLayer::new(
            request_id,
            tower_http::request_id::MakeRequestUuid,
        ))
        .with_state(state)
}

/// Start the API server.
pub async fn serve(
    bind_addr: std::net::SocketAddr,
    state: ApiState,
    api_config: &ApiConfig,
) -> anyhow::Result<()> {
    let app = create_router(state, api_config);

    let listener = tokio::net::TcpListener::bind(bind_addr).await?;
    tracing::info!("API server listening on {}", bind_addr);

    axum::serve(listener, app).await?;

    Ok(())
}
