//! Shared API state.

use fcp_core::config_store::ConfigStore;
use fcp_core::DynStorage;
use fcp_events::EventBus;
use fcp_flow::FlowStore;
use fcp_media::MediaManager;
use fcp_sip::handler::SipMessageHandler;
use fcp_sip::prelude::{DialogManager, SubscriptionManager};
use fcp_sip::CallManager;
use std::path::PathBuf;
use std::sync::Arc;

/// Security policy for login protection.
#[derive(Debug, Clone)]
pub struct SecurityPolicy {
    /// Maximum failed login attempts before lockout
    pub max_failed_attempts: u32,
    /// Lockout duration in seconds
    pub lockout_duration_secs: u64,
    /// Password expiration in days (None = disabled)
    pub password_expiration_days: Option<u32>,
}

impl Default for SecurityPolicy {
    fn default() -> Self {
        Self {
            max_failed_attempts: 5,
            lockout_duration_secs: 1800, // 30 minutes
            password_expiration_days: None,
        }
    }
}

/// Shared state for API handlers.
#[derive(Clone)]
pub struct ApiState {
    /// Storage for users and registrations (optional)
    pub storage: Option<Arc<DynStorage>>,

    /// Flow store for managing call flows (optional)
    pub flow_store: Option<Arc<dyn FlowStore>>,

    /// Dialog manager for inspecting active dialogs
    pub dialog_manager: Arc<DialogManager>,

    /// Subscription manager for inspecting active subscriptions
    pub subscription_manager: Arc<SubscriptionManager>,

    /// Call manager for inspecting active B2BUA calls
    pub call_manager: Arc<CallManager>,

    /// SIP message handler (for accessing rate limits, subscription index)
    pub handler: Arc<SipMessageHandler>,

    /// Media manager for recording control (optional)
    pub media_manager: Option<Arc<MediaManager>>,

    /// Event bus for real-time event streaming
    pub event_bus: Arc<dyn EventBus>,

    /// Queue manager for ACD queue operations (optional)
    pub queue_manager: Option<Arc<fcp_queue::QueueManager>>,

    /// Presence manager for presence/BLF operations (optional)
    pub presence_manager: Option<fcp_presence::PresenceManager>,

    /// SIP realm for authentication
    pub realm: String,

    /// Default tenant ID (used when no tenant context is provided)
    pub default_tenant_id: fcp_core::TenantId,

    /// Server start time for uptime tracking
    pub start_time: std::time::Instant,

    /// URL of fcp-events-server (e.g. "http://localhost:8081") for proxying
    /// cluster-wide queries (active calls, etc.)
    pub events_server_url: Option<String>,

    /// Optional bearer token for authenticating to fcp-events-server.
    pub events_server_auth_token: Option<String>,

    /// What this process calls itself when it posts an event to the events
    /// server (`X-Node-ID`); the admin's `[service] node_id`.
    pub events_node_id: Option<String>,

    /// URL of a conference server's HTTP API (e.g. "http://conf-a:8090")
    /// for live room controls. Any conference node will do: it locates
    /// or broadcasts to the nodes hosting a room through `/internal`.
    pub conference_server_url: Option<String>,

    /// Bearer token for the conference server's HTTP API, when it has
    /// `http.auth_enabled`.
    pub conference_server_auth_token: Option<String>,

    /// The secret browser join tokens are signed with — the same value the
    /// conference servers verify with (`conference.join_secret`).
    pub conference_join_secret: Option<String>,

    /// The WebRTC gateway's public base URL (e.g. "https://meet.example.com"),
    /// so a minted token comes with a link to the gateway's join page.
    pub conference_gateway_url: Option<String>,

    /// Config store for database-backed configuration management (optional)
    pub config_store: Option<Arc<dyn ConfigStore>>,

    /// Directory for voicemail greetings (used by greeting upload/download endpoints)
    pub voicemail_greetings_dir: Option<PathBuf>,

    /// Directory for voicemail recordings (used by message audio playback)
    pub voicemail_recordings_dir: Option<PathBuf>,

    /// Directory for profile picture uploads
    pub profile_pictures_dir: Option<PathBuf>,

    /// Directory for audio prompt files (used by prompt upload/download endpoints)
    pub prompts_dir: Option<PathBuf>,

    /// Security policy for login protection (lockout, password expiration)
    pub security_policy: SecurityPolicy,

    /// TTS service for synthesizing prompt audio (optional)
    pub tts_service: Option<Arc<fcp_media::tts::TtsService>>,

    /// Auth config for WebSocket first-message authentication (optional).
    /// When set, WebSocket endpoints require a valid token in the first message.
    pub auth_config: Option<crate::auth::AuthConfig>,

    /// The keys sign-in sessions' access tokens are signed and verified
    /// with (`[api.session]`, else derived from `auth_token`). When `None`,
    /// sign-in (login, me, refresh) fails.
    pub session_keys: Option<fcp_core::session::SessionKeys>,

    /// How long a sign-in session lasts, and its access tokens.
    pub session_ttl: std::time::Duration,
    pub session_access_ttl: std::time::Duration,

    /// Reject provisioning requests that did not arrive over HTTPS
    /// (detected via the `X-Forwarded-Proto` header set by a reverse proxy).
    pub provisioning_require_https: bool,

    /// What provisioned phones are told (`[api.provisioning]`).
    pub provisioning: fcp_core::ApiProvisioningConfig,

    /// Where vendors' redirection services answer (`[api.redirection]`).
    pub redirection: fcp_core::ApiRedirectionConfig,

    /// Trunk health monitoring map (populated by background health probe task).
    pub trunk_health: Option<crate::trunk_health::TrunkHealthMap>,
    /// Where the view each node has of its trunks is read from
    /// (`docs/TENANT_TRUNKS.md` §9). Absent on a deployment with no
    /// cluster, where this server's own probes are the only view there is.
    pub trunk_reports: Option<Arc<dyn crate::TrunkNodeReports>>,
    /// Where a route preview is asked (`docs/TENANT_TRUNKS.md` §12, phase
    /// 6d). Absent when no call manager can be reached: the preview then
    /// answers 503.
    pub route_previewer: Option<Arc<dyn crate::RoutePreviewer>>,
    /// Where a call's supervision is asked (§3.11). Absent, the supervision
    /// routes answer 503.
    pub call_supervisor: Option<Arc<dyn crate::CallSupervisor>>,

    /// Deployment mode for permission evaluation (Enterprise, ServiceProvider, Carrier).
    pub deployment_mode: fcp_core::permissions::DeploymentMode,

    /// API tokens' request limits (`crate::rate_limit`); the router sets
    /// it from `[api.rate_limit]` over [`Self::rate_counter`].
    pub rate_limiter: Arc<crate::rate_limit::RateLimiter>,
    /// Where token requests are counted: the admin's Redis, when it has
    /// one; this process otherwise.
    pub rate_counter: Arc<dyn crate::rate_limit::RateCounter>,

    /// E911 address validator (completeness checks by default; a
    /// provider-backed validator can be injected).
    pub e911_validator: Arc<dyn fcp_core::e911::E911AddressValidator>,

    /// Directory holding call recordings, pruned by the retention policy.
    pub call_recordings_dir: Option<PathBuf>,

    /// The cloud this deployment is a site of (`docs/HYBRID_CONTROL_PLANE.md`
    /// §4), when it is one: the objects the link keeps in sync are read-only
    /// here, and a write to one is refused naming this address.
    pub managed_by: Option<String>,
    /// Who fetches a file a site holds, through its link
    /// (`docs/HYBRID_CONTROL_PLANE.md` §3.3); none where no site links here.
    pub site_files: Option<Arc<dyn crate::site_files::SiteFiles>>,
    /// Sends a tenant's test deliveries and replays
    /// (docs/DEVELOPER_SURFACE.md §3, phase 5); its configuration says
    /// whether a webhook may name a private address. Absent, webhooks are
    /// managed but not tested or replayed here.
    pub webhook_dispatcher: Option<Arc<fcp_webhooks::WebhookDispatcher>>,
    /// Sign-in through tenants' identity providers (`[api.sso]`,
    /// `docs/IDENTITY_AND_DEVICES.md` phase 3). Absent, providers can be
    /// configured but nobody signs in through one.
    pub sso: Option<Arc<SsoRuntime>>,
}

/// What single sign-on needs at run time.
pub struct SsoRuntime {
    /// Where browsers reach this admin, without a trailing slash.
    pub public_url: String,
    pub client: crate::oidc::OidcClient,
}

impl SsoRuntime {
    /// The address providers send browsers back to.
    pub fn redirect_uri(&self) -> String {
        format!("{}/api/v1/auth/sso/callback", self.public_url)
    }
}

impl ApiState {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        storage: Option<Arc<DynStorage>>,
        flow_store: Option<Arc<dyn FlowStore>>,
        dialog_manager: Arc<DialogManager>,
        subscription_manager: Arc<SubscriptionManager>,
        call_manager: Arc<CallManager>,
        handler: Arc<SipMessageHandler>,
        media_manager: Option<Arc<MediaManager>>,
        event_bus: Arc<dyn EventBus>,
        queue_manager: Option<Arc<fcp_queue::QueueManager>>,
        presence_manager: Option<fcp_presence::PresenceManager>,
        realm: String,
        default_tenant_id: fcp_core::TenantId,
    ) -> Self {
        Self {
            storage,
            flow_store,
            dialog_manager,
            subscription_manager,
            call_manager,
            handler,
            media_manager,
            event_bus,
            queue_manager,
            presence_manager,
            realm,
            default_tenant_id,
            start_time: std::time::Instant::now(),
            events_server_url: None,
            events_server_auth_token: None,
            events_node_id: None,
            conference_server_url: None,
            conference_server_auth_token: None,
            conference_join_secret: None,
            conference_gateway_url: None,
            config_store: None,
            voicemail_greetings_dir: None,
            voicemail_recordings_dir: None,
            profile_pictures_dir: None,
            prompts_dir: None,
            security_policy: SecurityPolicy::default(),
            tts_service: None,
            auth_config: None,
            session_keys: None,
            session_ttl: fcp_core::session::DEFAULT_SESSION_TTL,
            session_access_ttl: fcp_core::session::DEFAULT_ACCESS_TTL,
            provisioning_require_https: true,
            provisioning: fcp_core::ApiProvisioningConfig::default(),
            redirection: fcp_core::ApiRedirectionConfig::default(),
            trunk_health: None,
            trunk_reports: None,
            route_previewer: None,
            call_supervisor: None,
            deployment_mode: fcp_core::permissions::DeploymentMode::default(),
            rate_limiter: Arc::new(crate::rate_limit::RateLimiter::new(
                fcp_core::ApiRateLimitConfig::default(),
                Arc::new(crate::rate_limit::LocalCounter::default()),
            )),
            rate_counter: Arc::new(crate::rate_limit::LocalCounter::default()),
            e911_validator: Arc::new(fcp_core::e911::BasicAddressValidator),
            call_recordings_dir: None,
            managed_by: None,
            site_files: None,
            webhook_dispatcher: None,
            sso: None,
        }
    }

    /// Set the events server URL for proxying cluster-wide queries.
    pub fn with_events_server_url(mut self, url: Option<String>) -> Self {
        self.events_server_url = url;
        self
    }

    /// Set the events server bearer token for cluster-wide queries.
    pub fn with_events_server_auth_token(mut self, token: Option<String>) -> Self {
        self.events_server_auth_token = token;
        self
    }

    /// Set the node id this process posts events under.
    pub fn with_events_node_id(mut self, node_id: impl Into<String>) -> Self {
        self.events_node_id = Some(node_id.into());
        self
    }

    /// Set the conference server URL for live room controls.
    pub fn with_conference_server_url(mut self, url: Option<String>) -> Self {
        self.conference_server_url = url.map(|u| u.trim_end_matches('/').to_string());
        self
    }

    /// Set the conference server bearer token for live room controls.
    pub fn with_conference_server_auth_token(mut self, token: Option<String>) -> Self {
        self.conference_server_auth_token = token;
        self
    }

    /// Set the secret browser join tokens are signed with.
    pub fn with_conference_join_secret(mut self, secret: Option<String>) -> Self {
        self.conference_join_secret = secret.filter(|s| !s.is_empty());
        self
    }

    /// Set the WebRTC gateway's public base URL for join links.
    pub fn with_conference_gateway_url(mut self, url: Option<String>) -> Self {
        self.conference_gateway_url = url
            .map(|u| u.trim_end_matches('/').to_string())
            .filter(|u| !u.is_empty());
        self
    }

    /// Set the config store for database-backed configuration management.
    pub fn with_config_store(mut self, store: Arc<dyn ConfigStore>) -> Self {
        self.config_store = Some(store);
        self
    }

    /// Set the voicemail greetings directory.
    pub fn with_voicemail_greetings_dir(mut self, dir: PathBuf) -> Self {
        self.voicemail_greetings_dir = Some(dir);
        self
    }

    /// Set the voicemail recordings directory.
    pub fn with_voicemail_recordings_dir(mut self, dir: PathBuf) -> Self {
        self.voicemail_recordings_dir = Some(dir);
        self
    }

    /// Set the profile pictures directory.
    pub fn with_profile_pictures_dir(mut self, dir: PathBuf) -> Self {
        self.profile_pictures_dir = Some(dir);
        self
    }

    /// Set the prompts directory for audio prompt files.
    pub fn with_prompts_dir(mut self, dir: PathBuf) -> Self {
        self.prompts_dir = Some(dir);
        self
    }

    /// Set the security policy for login protection.
    pub fn with_security_policy(mut self, policy: SecurityPolicy) -> Self {
        self.security_policy = policy;
        self
    }

    /// Set the TTS service for prompt audio synthesis.
    pub fn with_tts_service(mut self, tts: Arc<fcp_media::tts::TtsService>) -> Self {
        self.tts_service = Some(tts);
        self
    }

    /// Set the auth config for WebSocket first-message authentication.
    pub fn with_auth_config(mut self, config: crate::auth::AuthConfig) -> Self {
        self.auth_config = Some(config);
        self
    }

    /// Where vendors' redirection services answer.
    pub fn with_redirection_config(mut self, config: fcp_core::ApiRedirectionConfig) -> Self {
        self.redirection = config;
        self
    }

    /// What provisioned phones are told.
    pub fn with_provisioning_config(mut self, config: fcp_core::ApiProvisioningConfig) -> Self {
        self.provisioning = config;
        self
    }

    /// Set whether the provisioning endpoint requires HTTPS.
    pub fn with_provisioning_require_https(mut self, require: bool) -> Self {
        self.provisioning_require_https = require;
        self
    }

    /// Set the call recordings directory pruned by retention runs.
    pub fn with_call_recordings_dir(mut self, dir: PathBuf) -> Self {
        self.call_recordings_dir = Some(dir);
        self
    }

    /// Replace the E911 address validator.
    pub fn with_e911_validator(
        mut self,
        validator: Arc<dyn fcp_core::e911::E911AddressValidator>,
    ) -> Self {
        self.e911_validator = validator;
        self
    }

    /// Count API token requests here — the admin's Redis, so every admin
    /// node counts together.
    pub fn with_rate_counter(mut self, counter: Arc<dyn crate::rate_limit::RateCounter>) -> Self {
        self.rate_counter = counter;
        self
    }

    /// Set the deployment mode for permission evaluation.
    pub fn with_deployment_mode(mut self, mode: fcp_core::permissions::DeploymentMode) -> Self {
        self.deployment_mode = mode;
        self
    }

    /// The cloud this deployment is a site of, if it is one: synced objects
    /// become read-only (see [`crate::managed`]).
    pub fn with_site_files(mut self, files: Option<Arc<dyn crate::site_files::SiteFiles>>) -> Self {
        self.site_files = files;
        self
    }

    pub fn with_webhook_dispatcher(
        mut self,
        dispatcher: Option<Arc<fcp_webhooks::WebhookDispatcher>>,
    ) -> Self {
        self.webhook_dispatcher = dispatcher;
        self
    }

    pub fn with_managed_by(mut self, cloud: Option<String>) -> Self {
        self.managed_by = cloud
            .map(|c| c.trim().trim_end_matches('/').to_string())
            .filter(|c| !c.is_empty());
        self
    }

    /// Where to read what each node sees of its trunks.
    pub fn with_trunk_reports(mut self, reports: Arc<dyn crate::TrunkNodeReports>) -> Self {
        self.trunk_reports = Some(reports);
        self
    }

    /// Where a call's supervision is asked.
    pub fn with_call_supervisor(mut self, supervisor: Arc<dyn crate::CallSupervisor>) -> Self {
        self.call_supervisor = Some(supervisor);
        self
    }

    /// Where to ask a call manager for a route preview.
    pub fn with_route_previewer(mut self, previewer: Arc<dyn crate::RoutePreviewer>) -> Self {
        self.route_previewer = Some(previewer);
        self
    }

    pub fn with_trunk_health(mut self, health: crate::trunk_health::TrunkHealthMap) -> Self {
        self.trunk_health = Some(health);
        self
    }

    /// Sign in through identity providers as `[api.sso]` says.
    pub fn with_sso_config(mut self, sso: &fcp_core::ApiSsoConfig) -> Self {
        self.sso = sso.public_url().map(|public_url| {
            Arc::new(SsoRuntime {
                public_url,
                client: crate::oidc::OidcClient::new(sso.allow_private_issuers),
            })
        });
        self
    }

    /// Sign sessions as `[api.session]` says.
    pub fn with_session_config(
        mut self,
        session: &fcp_core::ApiSessionConfig,
        auth_token: Option<&str>,
    ) -> Self {
        self.session_keys = session.keys(auth_token);
        self.session_ttl = std::time::Duration::from_secs(session.ttl_secs);
        self.session_access_ttl = std::time::Duration::from_secs(session.access_ttl_secs);
        self
    }

    /// Publish an event to the local event bus and forward to the events server.
    ///
    /// This ensures events generated by the API (e.g. mailbox lifecycle) reach
    /// both local WebSocket subscribers and remote services via fcp-events-server.
    pub async fn publish_event(&self, event: fcp_events::PlatformEvent) {
        // Publish to local bus (for admin UI WebSocket)
        let _ = self.event_bus.publish(event.clone()).await;

        // Forward to events server ingest endpoint (for remote services)
        if let Some(ref base_url) = self.events_server_url {
            let ingest_url = format!("{}/ingest", base_url);
            let event_clone = event;
            let auth_token = self.events_server_auth_token.clone();
            let node_id = self.events_node_id.clone();
            tokio::spawn(async move {
                let client = reqwest::Client::new();
                let mut request = client
                    .post(&ingest_url)
                    .json(&event_clone)
                    .timeout(std::time::Duration::from_secs(5));
                if let Some(node_id) = node_id {
                    request = request.header("X-Node-ID", node_id);
                }
                if let Some(token) = auth_token.as_deref() {
                    request = request.bearer_auth(token);
                }

                if let Err(e) = request.send().await {
                    tracing::warn!(%e, "Failed to forward event to events server");
                }
            });
        }
    }
}
