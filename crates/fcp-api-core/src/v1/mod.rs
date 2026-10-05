//! API v1 routes.

pub mod api_tokens;
pub mod app_auth;
pub mod auth_session;
pub mod billing;
pub mod call_control;
pub mod calling;
pub mod cdrs;
pub mod classes_of_service;
pub mod compliance;
pub mod conferences;
pub mod conferences_live;
pub mod config;
pub mod config_templates;
pub mod dashboard;
pub mod debug;
pub mod devices;
pub mod emergency;
pub mod emergency_networks;
pub mod encryption;
pub mod events;
pub mod extensions;
pub mod firmware;
pub mod flow_traces;
pub mod flows;
pub mod groups;
pub mod hoteling;
pub mod join_tokens;
pub mod lcr_routes;
pub mod line_bindings;
pub mod locations;
pub mod media;
pub mod metrics;
pub mod phone_numbers;
pub mod presence;
pub mod prompts;
pub mod provisioning;
pub mod queue_config;
pub mod queues;
pub mod rate_plans;
pub mod recordings;
pub mod redirection;
pub mod registrations;
pub mod routing;
pub mod schedules;
pub mod self_service;
pub mod signup;
pub mod sip_trace;
pub mod sites;
pub mod sso;
pub mod supervision;
pub mod system;
pub mod tenants;
pub mod totp;
pub mod trunks;
pub mod uploads;
pub mod usage;
pub mod user_devices;
pub mod users;
pub mod voicemail;
pub mod webhooks;

use crate::state::ApiState;
use axum::{
    extract::DefaultBodyLimit,
    middleware,
    routing::{delete, get, post, put},
    Router,
};

/// Pre-auth routes accessible without bearer token.
///
/// Login, refresh, logout, and change-expired-password must be callable
/// without an access token: refresh and logout read the session's HttpOnly
/// cookie (`docs/IDENTITY_AND_DEVICES.md` phase 1). They are registered
/// outside the bearer middleware layer.
/// Tenant signup is only mounted when explicitly enabled by configuration.
pub fn pre_auth_router(enable_signup: bool) -> Router<ApiState> {
    let router = Router::new()
        .route("/auth/discover", post(auth_session::discover))
        .route("/auth/login", post(auth_session::login))
        .route("/auth/login/totp", post(auth_session::login_totp))
        .route("/auth/totp/enroll", post(auth_session::enroll_totp))
        .route("/auth/totp/confirm", post(auth_session::confirm_totp))
        // Single sign-on (docs/IDENTITY_AND_DEVICES.md phase 3)
        .route("/auth/sso/:id/start", get(sso::start))
        .route("/auth/sso/callback", get(sso::callback))
        .route("/auth/sso/exchange", post(sso::exchange))
        .route(
            "/auth/sso/:id/backchannel-logout",
            post(sso::backchannel_logout),
        )
        // SAML sign-in (phase 4)
        .route("/auth/saml/:id", get(sso::saml_metadata))
        .route("/auth/saml/:id/acs", post(sso::saml_acs))
        .route(
            "/auth/saml/:id/slo",
            get(sso::saml_slo_redirect).post(sso::saml_slo_post),
        )
        // An app's sign-in and its session (docs/SOFTPHONE.md §2.2)
        .route("/auth/app/authorize", get(app_auth::authorize))
        .route("/auth/app/token", post(app_auth::token))
        .route("/auth/app/revoke", post(app_auth::revoke))
        .route("/auth/refresh", post(auth_session::refresh))
        .route("/auth/logout", post(auth_session::logout))
        .route(
            "/auth/change-expired-password",
            post(auth_session::change_expired_password),
        );

    if enable_signup {
        router.route("/auth/signup", post(signup::signup))
    } else {
        router
    }
}

/// WebSocket routes that bypass the bearer middleware layer.
///
/// These are registered outside the auth-protected router because browsers
/// cannot set `Authorization` headers on WebSocket connections. Each handler
/// validates auth inline via the first-message protocol.
pub fn ws_router() -> Router<ApiState> {
    Router::new().route("/events", get(events::event_stream_websocket))
}

/// Create the v1 API router.
///
/// Routes are split into two groups:
/// - **User routes**: accessible by any authenticated user (including EndUser)
/// - **Admin routes**: require admin-level role (not EndUser); protected by
///   the [`crate::auth::require_admin`] middleware
pub fn router() -> Router<ApiState> {
    // Routes accessible by any authenticated user (EndUser and above)
    let user_routes = Router::new()
        // Auth session endpoints that require a valid JWT
        .route("/auth/me", get(auth_session::me))
        .route("/auth/app/approve", post(app_auth::approve))
        .route("/auth/app/sessions", post(app_auth::create_session))
        // Self-service (end-user portal)
        .route(
            "/me/profile",
            get(self_service::get_profile).put(self_service::update_profile),
        )
        .route(
            "/me/profile/avatar",
            get(self_service::get_avatar)
                .post(self_service::upload_avatar)
                .delete(self_service::delete_avatar),
        )
        .route("/me/password", put(self_service::change_password))
        // A second factor (docs/IDENTITY_AND_DEVICES.md phase 2b)
        .route("/me/totp", get(totp::get_totp))
        .route("/me/totp/enroll", post(totp::enroll_totp))
        .route("/me/totp/confirm", post(totp::confirm_totp))
        .route("/me/totp/disable", post(totp::disable_totp))
        .route("/me/totp/recovery-codes", post(totp::renew_recovery_codes))
        .route(
            "/me/voicemail",
            get(self_service::get_voicemail).put(self_service::update_voicemail),
        )
        .route(
            "/me/voicemail/stats",
            get(self_service::get_voicemail_stats),
        )
        .route(
            "/me/voicemail/messages",
            get(self_service::list_voicemail_messages),
        )
        .route(
            "/me/voicemail/messages/:id",
            put(self_service::update_voicemail_message)
                .delete(self_service::delete_voicemail_message),
        )
        .route(
            "/me/voicemail/messages/:id/audio",
            get(self_service::get_message_audio),
        )
        .route(
            "/me/voicemail/greetings",
            get(self_service::list_voicemail_greetings)
                .post(self_service::upload_voicemail_greeting),
        )
        .route(
            "/me/voicemail/greetings/synthesize",
            post(self_service::synthesize_voicemail_greeting),
        )
        .route(
            "/me/voicemail/greetings/:greeting_type",
            delete(self_service::delete_voicemail_greeting),
        )
        .route(
            "/me/voicemail/greetings/:greeting_type/active",
            put(self_service::set_voicemail_greeting_active),
        )
        .route(
            "/me/voicemail/greetings/:greeting_type/audio",
            get(self_service::download_voicemail_greeting_audio),
        )
        .route(
            "/me/location",
            get(self_service::get_my_location)
                .put(self_service::declare_my_location)
                .delete(self_service::clear_my_location),
        )
        .route(
            "/me/calling",
            get(self_service::get_my_calling).put(self_service::update_my_calling),
        )
        // Your devices: phones and app installs (docs/SOFTPHONE.md §2.1)
        .route("/me/devices", get(user_devices::list_mine))
        .route("/me/devices/:id", delete(user_devices::delete_mine))
        .route("/me/executives", get(self_service::get_my_executives))
        .route(
            "/me/executives/:user_id",
            axum::routing::put(self_service::update_my_executive),
        )
        .route(
            "/me/hoteling",
            get(self_service::get_my_hoteling).delete(self_service::logout_my_hoteling),
        )
        .route("/me/conferences", get(self_service::list_my_conferences))
        .route(
            "/me/conferences/:id",
            get(self_service::get_my_conference).patch(self_service::update_my_conference),
        )
        .route(
            "/me/conferences/:id/dial-out",
            post(self_service::dial_out_my_conference),
        )
        .route(
            "/me/conferences/:id/join-tokens",
            post(self_service::mint_my_join_token),
        )
        // Personal API tokens (docs/DEVELOPER_SURFACE.md §3, phase 1).
        .route(
            "/me/api-tokens",
            get(api_tokens::list_my_api_tokens).post(api_tokens::create_my_api_token),
        )
        .route(
            "/me/api-tokens/:id",
            delete(api_tokens::revoke_my_api_token),
        )
        .route(
            "/me/api-tokens/:id/rotate",
            post(api_tokens::rotate_my_api_token),
        );

    // Admin-only routes — EndUser JWTs are rejected with 403 by require_admin.
    let admin_routes = Router::new()
        // Dashboard
        .route("/dashboard/summary", get(dashboard::summary))
        .route("/status", get(system::status_scoped))
        // Encryption at rest (platform admin only, enforced in the handlers)
        .route("/system/encryption", get(encryption::status))
        .route("/system/encryption/rotate", post(encryption::rotate))
        .route("/system/encryption/reencrypt", post(encryption::reencrypt))
        .route("/system/encryption/seal", post(encryption::seal))
        // Emergency calling (E911)
        .route(
            "/emergency/config",
            get(emergency::get_config).put(emergency::update_config),
        )
        .route("/emergency/readiness", get(emergency::readiness))
        .route(
            "/emergency/networks",
            get(emergency_networks::list_networks).post(emergency_networks::create_network),
        )
        .route(
            "/emergency/networks/:id",
            get(emergency_networks::get_network)
                .patch(emergency_networks::update_network)
                .delete(emergency_networks::delete_network),
        )
        .route("/emergency/calls", get(emergency::list_calls))
        .route("/emergency/notify-call", post(emergency::notify_call))
        .route(
            "/emergency/calls/:call_id/notifications",
            get(emergency::list_call_notifications),
        )
        .route(
            "/emergency/validate-address",
            post(emergency::validate_address),
        )
        .route(
            "/phone-numbers/:id/e911/validate",
            post(emergency::validate_phone_number),
        )
        .route(
            "/locations/:id/e911/validate",
            post(emergency::validate_location),
        )
        .route(
            "/devices/:id/e911/validate",
            post(emergency::validate_device),
        )
        // Usage & Billing
        .route("/usage", get(usage::get_usage))
        .route("/usage/by-number", get(usage::get_usage_by_number))
        .route("/billing/summary", get(usage::billing_summary))
        .route(
            "/billing/balance",
            get(billing::get_balance).put(billing::update_balance),
        )
        .route(
            "/billing/balance/transactions",
            get(billing::list_transactions).post(billing::create_transaction),
        )
        .route(
            "/rate-plans",
            get(rate_plans::list_rate_plans).post(rate_plans::create_rate_plan),
        )
        .route(
            "/rate-plans/:id",
            get(rate_plans::get_rate_plan)
                .patch(rate_plans::update_rate_plan)
                .delete(rate_plans::delete_rate_plan),
        )
        .route(
            "/rate-plans/:id/rates",
            get(rate_plans::list_rates).post(rate_plans::upsert_rate),
        )
        .route(
            "/rate-plans/:id/rates/:rate_id",
            axum::routing::delete(rate_plans::delete_rate),
        )
        // NOTE: /events WebSocket is registered separately via ws_router()
        // Debug endpoints
        .route("/debug/summary", get(debug::debug_summary))
        .route("/debug/dialogs", get(debug::list_dialogs))
        .route("/debug/subscriptions", get(debug::list_subscriptions))
        .route("/debug/calls", get(debug::list_calls))
        .route("/debug/rate-limits", get(debug::list_rate_limits))
        // Tenant management
        .route(
            "/tenants",
            get(tenants::list_tenants).post(tenants::create_tenant),
        )
        .route(
            "/tenants/:id",
            get(tenants::get_tenant)
                .put(tenants::update_tenant)
                .delete(tenants::delete_tenant),
        )
        .route("/tenants/:id/deletion", get(tenants::get_tenant_deletion))
        // Sites: a tenant's own stacks on its own network
        // (docs/HYBRID_CONTROL_PLANE.md).
        .route("/sites", get(sites::list_sites).post(sites::create_site))
        .route(
            "/sites/:id",
            get(sites::get_site).delete(sites::delete_site),
        )
        .route(
            "/sites/:id/enrollment-token",
            post(sites::new_enrollment_token),
        )
        .route("/sites/:id/disable", post(sites::disable_site))
        .route("/sites/:id/enable", post(sites::enable_site))
        // User management
        .route("/users", get(users::list_users).post(users::create_user))
        .route("/users/by-id/:id", get(users::get_user_by_id))
        .route("/users/:username/totp", delete(totp::reset_totp))
        .route("/users/:username/identities", get(sso::list_identities))
        .route("/users/:username/groups", get(groups::list_user_groups))
        // A tenant's groups (docs/USERS.md §5).
        .route(
            "/groups",
            get(groups::list_groups).post(groups::create_group),
        )
        .route(
            "/groups/:id",
            get(groups::get_group)
                .put(groups::update_group)
                .delete(groups::delete_group),
        )
        .route(
            "/groups/:id/members",
            get(groups::list_members).post(groups::add_member),
        )
        .route(
            "/groups/:id/members/:username",
            delete(groups::remove_member),
        )
        .route(
            "/users/:username/identities/:provider_id",
            delete(sso::unlink_identity),
        )
        // A tenant's identity providers (phase 3)
        .route("/identity-providers", get(sso::list).post(sso::create))
        .route(
            "/identity-providers/:id",
            get(sso::get).put(sso::update).delete(sso::delete),
        )
        .route("/identity-providers/:id/check", post(sso::check_provider))
        .route("/identity-providers/:id/sp-key", post(sso::rotate_sp_key))
        .route(
            "/identity-providers/:id/refresh-metadata",
            post(sso::refresh_metadata),
        )
        .route(
            "/users/:username",
            get(users::get_user)
                .put(users::update_user)
                .delete(users::delete_user),
        )
        .route("/users/:username/password", put(users::update_password))
        .route("/users/:username/unlock", post(users::unlock_user))
        .route("/users/:username/restore", post(users::restore_user))
        .route("/users/:username/purge", post(users::purge_user))
        .route(
            "/users/:username/digest-password",
            put(users::update_digest_password).delete(users::delete_digest_password),
        )
        .route(
            "/users/:username/calling",
            get(calling::get_user_calling).put(calling::update_user_calling),
        )
        .route("/users/:username/devices", get(user_devices::list_users))
        // Registrations
        .route("/registrations", get(registrations::list_registrations))
        .route(
            "/registrations/:aor",
            get(registrations::get_registrations).delete(registrations::delete_registrations),
        )
        .route(
            "/registrations/:aor/:contact",
            delete(registrations::delete_registration),
        )
        // Call management
        .route("/calls", get(media::list_calls).post(media::originate_call))
        .route(
            "/calls/:call_id",
            get(media::get_call).delete(media::hangup_call),
        )
        .route(
            "/calls/:call_id/recording/start",
            post(media::start_recording),
        )
        .route(
            "/calls/:call_id/recording/stop",
            post(media::stop_recording),
        )
        // Live call control (`docs/DEVELOPER_SURFACE.md` phase 8)
        .route("/calls/:call_id/hold", post(call_control::hold))
        .route("/calls/:call_id/resume", post(call_control::resume))
        .route("/calls/:call_id/transfer", post(call_control::transfer))
        .route("/calls/:call_id/redirect", post(call_control::redirect))
        .route("/calls/:call_id/play", post(call_control::play))
        .route("/calls/:call_id/say", post(call_control::say))
        .route("/calls/:call_id/hangup", post(call_control::hangup))
        // A supervisor on a call (`docs/ENTERPRISE_CALLING.md` §3.11)
        .route("/calls/:call_id/monitor", post(supervision::monitor))
        .route("/calls/:call_id/whisper", post(supervision::whisper))
        .route("/calls/:call_id/barge", post(supervision::barge))
        .route(
            "/calls/:call_id/supervisor",
            get(supervision::get).delete(supervision::leave),
        )
        // SIP trace
        .route(
            "/calls/:call_id/sip-messages",
            get(sip_trace::get_sip_messages),
        )
        // Compliance: retention/consent policy, retention runs, audit log
        .route(
            "/compliance/policy",
            get(compliance::get_policy).put(compliance::update_policy),
        )
        .route(
            "/compliance/retention/run",
            post(compliance::run_retention_now),
        )
        .route(
            "/compliance/retention/runs",
            get(compliance::list_retention_runs),
        )
        .route("/audit-log", get(compliance::list_audit_log))
        .route("/compliance/deletions", get(compliance::list_deletions))
        .route(
            "/compliance/holds",
            get(compliance::list_holds).post(compliance::place_hold),
        )
        .route("/compliance/holds/:id", get(compliance::get_hold))
        .route(
            "/compliance/holds/:id/release",
            post(compliance::release_hold),
        )
        // Subject requests: export (and, next, erasure) of one person's data
        .route(
            "/compliance/subject-requests",
            get(compliance::list_subject_requests).post(compliance::file_subject_request),
        )
        .route(
            "/compliance/subject-requests/:id",
            get(compliance::get_subject_request),
        )
        .route(
            "/compliance/subject-requests/:id/bundle",
            get(compliance::download_subject_bundle),
        )
        .route(
            "/compliance/subject-requests/:id/confirm",
            post(compliance::confirm_subject_request),
        )
        // Conference recordings: the registry, and the file behind each row
        .route("/recordings", get(recordings::list_recordings))
        .route(
            "/recordings/:id",
            get(recordings::get_recording).delete(recordings::delete_recording),
        )
        .route(
            "/recordings/:id/download",
            get(recordings::download_recording),
        )
        // CDR
        .route("/cdrs", get(cdrs::list_cdrs))
        .route("/cdrs/export", get(cdrs::export_cdrs))
        .route("/cdrs/:call_id", get(cdrs::get_cdr))
        // Flow execution traces
        .route("/flow-traces", get(flow_traces::list_flow_traces))
        .route(
            "/flow-traces/:call_id",
            get(flow_traces::get_call_flow_traces),
        )
        // Flows
        .route("/flows", get(flows::list_flows).post(flows::create_flow))
        .route(
            "/flows/:id",
            get(flows::get_flow)
                .put(flows::update_flow)
                .delete(flows::delete_flow),
        )
        .route("/flows/:id/versions", get(flows::list_flow_versions))
        .route("/flows/:id/versions/:version", get(flows::get_flow_version))
        // Phone numbers
        .route(
            "/phone-numbers",
            get(phone_numbers::list_phone_numbers).post(phone_numbers::create_phone_number),
        )
        // IMPORTANT: /phone-numbers/available must come BEFORE /phone-numbers/:id
        // to prevent "available" from being captured as an :id parameter.
        .route(
            "/phone-numbers/available",
            get(phone_numbers::search_available_numbers),
        )
        .route(
            "/phone-numbers/:id",
            get(phone_numbers::get_phone_number)
                .patch(phone_numbers::update_phone_number)
                .delete(phone_numbers::delete_phone_number),
        )
        .route(
            "/phone-numbers/:id/activate",
            post(phone_numbers::activate_phone_number),
        )
        .route(
            "/phone-numbers/:id/purchase",
            post(phone_numbers::purchase_phone_number),
        )
        .route(
            "/phone-numbers/:id/release",
            post(phone_numbers::release_phone_number),
        )
        // Line bindings
        .route(
            "/line-bindings",
            get(line_bindings::list_line_bindings).post(line_bindings::create_line_binding),
        )
        .route(
            "/line-bindings/:id",
            get(line_bindings::get_line_binding)
                .patch(line_bindings::update_line_binding)
                .delete(line_bindings::delete_line_binding),
        )
        // Extensions
        .route(
            "/extensions",
            get(extensions::list_extensions).post(extensions::create_extension),
        )
        .route(
            "/extensions/:number",
            get(extensions::get_extension)
                .patch(extensions::update_extension)
                .delete(extensions::delete_extension),
        )
        // Devices
        .route(
            "/devices",
            get(devices::list_devices).post(devices::create_device),
        )
        .route(
            "/devices/:id",
            get(devices::get_device)
                .patch(devices::update_device)
                .delete(devices::delete_device),
        )
        // Provisioning that phones can use (docs/PROVISIONING.md)
        .route("/devices/:id/fetches", get(provisioning::device_fetches))
        .route(
            "/devices/:id/render",
            axum::routing::post(devices::render_device),
        )
        .route(
            "/devices/:id/resync",
            axum::routing::post(devices::resync_device),
        )
        .route(
            "/devices/import",
            axum::routing::post(devices::import_devices),
        )
        .route(
            "/provisioning/key",
            get(provisioning::get_key).post(provisioning::rotate_key),
        )
        .route("/provisioning/pending", get(devices::list_pending))
        // Firmware (docs/PROVISIONING.md §11): images streamed in, so the
        // body limit is the handler's ([api.firmware] max_image_mb).
        .route(
            "/firmware",
            get(firmware::list_images)
                .post(firmware::upload_image)
                .layer(DefaultBodyLimit::disable()),
        )
        .route(
            "/firmware/:id",
            axum::routing::delete(firmware::delete_image),
        )
        .route(
            "/firmware/targets",
            get(firmware::list_targets).post(firmware::create_target),
        )
        .route(
            "/firmware/targets/:id",
            axum::routing::put(firmware::update_target).delete(firmware::delete_target),
        )
        // Zero-touch (docs/PROVISIONING.md §10)
        .route("/provisioning/redirection", get(redirection::list_accounts))
        .route(
            "/provisioning/redirection/:service",
            axum::routing::put(redirection::put_account).delete(redirection::delete_account),
        )
        .route(
            "/provisioning/redirection/:service/check",
            axum::routing::post(redirection::check_account),
        )
        .route(
            "/provisioning/redirection/:service/sync",
            axum::routing::post(redirection::sync_account),
        )
        .route(
            "/devices/:id/redirection",
            get(redirection::device_redirection).post(redirection::register_device),
        )
        .route(
            "/provisioning/pending/:mac",
            axum::routing::delete(devices::dismiss_pending),
        )
        .route(
            "/provisioning/pending/:mac/claim",
            axum::routing::post(devices::claim_pending),
        )
        // Hoteling sessions (docs/ENTERPRISE_CALLING.md §3.8)
        .route("/hoteling/sessions", get(hoteling::list_sessions))
        .route(
            "/hoteling/sessions/:id",
            axum::routing::delete(hoteling::end_session_by_id),
        )
        // Config Templates (device provisioning)
        .route(
            "/config-templates",
            get(config_templates::list_config_templates)
                .post(config_templates::create_config_template),
        )
        .route(
            "/config-templates/library",
            get(config_templates::list_template_library),
        )
        .route(
            "/config-templates/preview",
            axum::routing::post(config_templates::preview_template),
        )
        .route(
            "/config-templates/:id/resync",
            axum::routing::post(config_templates::resync_template),
        )
        .route(
            "/config-templates/library/:vendor",
            axum::routing::post(config_templates::instantiate_library_template),
        )
        .route(
            "/config-templates/:id",
            get(config_templates::get_config_template)
                .patch(config_templates::update_config_template)
                .delete(config_templates::delete_config_template),
        )
        // Locations
        .route(
            "/locations",
            get(locations::list_locations).post(locations::create_location),
        )
        .route(
            "/locations/:id",
            get(locations::get_location)
                .patch(locations::update_location)
                .delete(locations::delete_location),
        )
        // Queues / ACD
        .route("/queues", get(queues::list_queues))
        .route("/queues/stats", get(queues::get_all_queue_stats))
        .route("/queues/:name", get(queues::get_queue))
        .route("/queues/:name/stats", get(queues::get_queue_stats))
        .route(
            "/queues/:name/agents",
            get(queues::list_queue_agents).post(queues::agent_login),
        )
        .route(
            "/queues/:name/agents/:id",
            delete(queues::agent_logout).patch(queues::update_agent_state),
        )
        .route("/queues/:name/agents/:id/pause", post(queues::pause_agent))
        .route(
            "/queues/:name/agents/:id/resume",
            post(queues::resume_agent),
        )
        // Presence
        .route("/presence", get(presence::list_presences))
        .route("/presence/stats", get(presence::get_stats))
        .route(
            "/presence/:aor",
            get(presence::get_presence)
                .put(presence::update_presence)
                .delete(presence::delete_presence),
        )
        .route("/presence/:aor/watchers", get(presence::list_watchers))
        .route(
            "/presence/:aor/watchers/:subscription_id",
            delete(presence::delete_subscription),
        )
        .route(
            "/presence/:aor/policy",
            get(presence::get_policy)
                .put(presence::set_policy)
                .delete(presence::delete_policy),
        )
        .route(
            "/presence/:aor/pres-rules",
            get(presence::get_pres_rules).put(presence::set_pres_rules),
        )
        // Configuration
        .route("/config", get(config::list_configs))
        .route("/config/audit", get(config::get_audit_log))
        .route("/config/:service", get(config::list_service_config))
        .route(
            "/config/:service/:section",
            get(config::get_config)
                .put(config::update_config)
                .delete(config::delete_config),
        )
        .route(
            "/config/:service/:section/validate",
            post(config::validate_config),
        )
        // Audio Prompts
        .route(
            "/prompts",
            get(prompts::list_prompts).post(prompts::create_prompt),
        )
        .route(
            "/prompts/:id",
            get(prompts::get_prompt)
                .put(prompts::update_prompt)
                .delete(prompts::delete_prompt),
        )
        .route(
            "/prompts/:id/audio",
            get(prompts::download_audio)
                .post(prompts::upload_audio)
                .layer(DefaultBodyLimit::max(50 * 1024 * 1024)), // 50 MB for audio uploads
        )
        .route("/prompts/:id/synthesize", post(prompts::synthesize_prompt))
        // Voicemail (admin management)
        .route(
            "/voicemail/mailboxes",
            get(voicemail::list_mailboxes).post(voicemail::create_mailbox),
        )
        .route(
            "/voicemail/mailboxes/:id",
            get(voicemail::get_mailbox)
                .put(voicemail::update_mailbox)
                .delete(voicemail::delete_mailbox),
        )
        .route(
            "/voicemail/mailboxes/:id/enable",
            post(voicemail::enable_mailbox),
        )
        .route(
            "/voicemail/mailboxes/:id/disable",
            post(voicemail::disable_mailbox),
        )
        .route(
            "/voicemail/mailboxes/:id/stats",
            get(voicemail::get_mailbox_stats),
        )
        .route(
            "/voicemail/mailboxes/:mailbox_id/messages",
            get(voicemail::list_messages),
        )
        .route(
            "/voicemail/messages/:id",
            get(voicemail::get_message)
                .put(voicemail::update_message)
                .delete(voicemail::delete_message),
        )
        // Voicemail Greetings (admin)
        .route(
            "/voicemail/mailboxes/:id/greetings",
            get(voicemail::list_greetings).post(voicemail::upload_greeting),
        )
        .route(
            "/voicemail/mailboxes/:id/greetings/:greeting_type",
            delete(voicemail::delete_greeting),
        )
        .route(
            "/voicemail/mailboxes/:id/greetings/:greeting_type/active",
            put(voicemail::set_greeting_active),
        )
        .route(
            "/voicemail/mailboxes/:id/greetings/:greeting_type/audio",
            get(voicemail::download_greeting_audio),
        )
        // API Tokens
        .route(
            "/api-tokens",
            get(api_tokens::list_api_tokens).post(api_tokens::create_api_token),
        )
        .route(
            "/api-tokens/permissions",
            get(api_tokens::grantable_permissions),
        )
        .route("/api-tokens/:id", delete(api_tokens::delete_api_token))
        .route("/api-tokens/:id/rotate", post(api_tokens::rotate_api_token))
        // Least-cost routes (`docs/TENANT_TRUNKS.md` §12, phase 6c)
        .route(
            "/lcr-routes",
            get(lcr_routes::list_lcr_routes).post(lcr_routes::create_lcr_route),
        )
        .route(
            "/lcr-routes/:id",
            get(lcr_routes::get_lcr_route)
                .patch(lcr_routes::update_lcr_route)
                .delete(lcr_routes::delete_lcr_route),
        )
        // Routing Rules
        .route("/routing/preview", get(routing::preview_routing))
        .route(
            "/classes-of-service",
            get(classes_of_service::list_classes).post(classes_of_service::create_class),
        )
        .route(
            "/classes-of-service/classify",
            get(classes_of_service::classify),
        )
        .route(
            "/classes-of-service/:id",
            get(classes_of_service::get_class)
                .put(classes_of_service::update_class)
                .delete(classes_of_service::delete_class),
        )
        .route(
            "/webhooks",
            get(webhooks::list_webhooks).post(webhooks::create_webhook),
        )
        .route("/webhooks/event-types", get(webhooks::list_event_types))
        .route(
            "/webhooks/:id",
            get(webhooks::get_webhook)
                .patch(webhooks::update_webhook)
                .delete(webhooks::delete_webhook),
        )
        .route(
            "/webhooks/:id/rotate-secret",
            post(webhooks::rotate_webhook_secret),
        )
        .route("/webhooks/:id/test", post(webhooks::test_webhook))
        .route("/webhooks/:id/deliveries", get(webhooks::list_deliveries))
        .route(
            "/webhooks/:id/deliveries/:delivery_id/replay",
            post(webhooks::replay_delivery),
        )
        .route(
            "/schedules",
            get(schedules::list_schedules).post(schedules::create_schedule),
        )
        .route(
            "/schedules/:id",
            get(schedules::get_schedule)
                .put(schedules::update_schedule)
                .delete(schedules::delete_schedule),
        )
        .route(
            "/routing/rules",
            get(routing::list_routing_rules).post(routing::create_routing_rule),
        )
        .route(
            "/routing/rules/:id",
            get(routing::get_routing_rule)
                .patch(routing::update_routing_rule)
                .delete(routing::delete_routing_rule),
        )
        // Queue Configs (persistent CRUD)
        .route(
            "/queue-configs",
            get(queue_config::list_queue_configs).post(queue_config::create_queue_config),
        )
        .route(
            "/queue-configs/:id",
            get(queue_config::get_queue_config)
                .patch(queue_config::update_queue_config)
                .delete(queue_config::delete_queue_config),
        )
        // SIP Trunks
        .route(
            "/trunks",
            get(trunks::list_trunks).post(trunks::create_trunk),
        )
        .route(
            "/trunks/:id",
            get(trunks::get_trunk)
                .patch(trunks::update_trunk)
                .delete(trunks::delete_trunk),
        )
        .route("/trunks/:id/test", post(trunks::test_trunk))
        .route("/trunks/:id/clone", post(trunks::clone_trunk))
        .route("/trunks/:id/stats", get(trunks::get_trunk_stats))
        // Conference / Meetings
        .route(
            "/conferences/meetings",
            get(conferences::list_meetings).post(conferences::create_meeting),
        )
        .route(
            "/conferences/meetings/:id",
            get(conferences::get_meeting)
                .patch(conferences::update_meeting)
                .delete(conferences::delete_meeting),
        )
        .route(
            "/conferences/meetings/:id/enable",
            post(conferences::enable_meeting),
        )
        .route(
            "/conferences/meetings/:id/disable",
            post(conferences::disable_meeting),
        )
        .route(
            "/conferences/meetings/:id/join-tokens",
            post(conferences::mint_join_token),
        )
        // Live conference controls, proxied to the conference server
        .route(
            "/conferences/live/:room/video",
            get(conferences_live::video_status),
        )
        .route(
            "/conferences/live/:room/video/layout",
            put(conferences_live::set_video_layout),
        )
        .route(
            "/conferences/live/:room/video/pin/:pid",
            post(conferences_live::pin),
        )
        .route(
            "/conferences/live/:room/video/pin",
            delete(conferences_live::unpin),
        )
        .route(
            "/conferences/live/:room/video/spotlight/:pid",
            post(conferences_live::spotlight),
        )
        .route(
            "/conferences/live/:room/video/spotlight",
            delete(conferences_live::unspotlight),
        )
        .route(
            "/conferences/live/:room/participants/:pid/video/:state",
            post(conferences_live::participant_video),
        )
        .route(
            "/conferences/live/:room/video/content/stop",
            post(conferences_live::stop_content),
        )
        .route(
            "/conferences/live/:room/video/content-layout",
            put(conferences_live::set_content_layout),
        )
        .route(
            "/conferences/live/:room/dial-out",
            post(conferences_live::dial_out),
        )
        .layer(middleware::from_fn(crate::auth::require_admin));

    user_routes.merge(admin_routes)
}
