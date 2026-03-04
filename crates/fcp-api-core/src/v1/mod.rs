//! API v1 routes.

pub mod api_tokens;
pub mod auth_session;
pub mod cdrs;
pub mod config;
pub mod dashboard;
pub mod debug;
pub mod devices;
pub mod events;
pub mod extensions;
pub mod flows;
pub mod line_bindings;
pub mod media;
pub mod metrics;
pub mod phone_numbers;
pub mod presence;
pub mod queues;
pub mod registrations;
pub mod self_service;
pub mod sip_trace;
pub mod system;
pub mod tenants;
pub mod users;
pub mod voicemail;

use crate::state::ApiState;
use axum::{
    routing::{delete, get, patch, post, put},
    Router,
};

/// Create the v1 API router.
pub fn router() -> Router<ApiState> {
    Router::new()
        // Auth session endpoints (for admin UI)
        .route("/auth/login", post(auth_session::login))
        .route("/auth/logout", post(auth_session::logout))
        .route("/auth/me", get(auth_session::me))
        .route("/auth/refresh", post(auth_session::refresh))
        .route("/auth/change-expired-password", post(auth_session::change_expired_password))
        // Dashboard endpoints (for admin UI)
        .route("/dashboard/summary", get(dashboard::summary))
        // System endpoints
        .route("/health", get(system::health))
        .route("/status", get(system::status))
        // Event streaming (WebSocket)
        .route("/events", get(events::event_stream_websocket))
        // Debug endpoints (for live system inspection)
        .route("/debug/summary", get(debug::debug_summary))
        .route("/debug/dialogs", get(debug::list_dialogs))
        .route("/debug/subscriptions", get(debug::list_subscriptions))
        .route("/debug/calls", get(debug::list_calls))
        .route("/debug/rate-limits", get(debug::list_rate_limits))
        // Tenant management endpoints
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
        // User management endpoints
        .route("/users", get(users::list_users).post(users::create_user))
        .route(
            "/users/:username",
            get(users::get_user)
                .put(users::update_user)
                .delete(users::delete_user),
        )
        .route("/users/:username/password", put(users::update_password))
        .route("/users/:username/unlock", post(users::unlock_user))
        .route(
            "/users/:username/digest-password",
            put(users::update_digest_password).delete(users::delete_digest_password),
        )
        // Registration viewing endpoints
        .route("/registrations", get(registrations::list_registrations))
        .route(
            "/registrations/:aor",
            get(registrations::get_registrations).delete(registrations::delete_registrations),
        )
        .route(
            "/registrations/:aor/:contact",
            delete(registrations::delete_registration),
        )
        // Call management endpoints
        .route("/calls", get(media::list_calls).post(media::originate_call))
        .route(
            "/calls/:call_id",
            get(media::get_call).delete(media::hangup_call),
        )
        // Recording endpoints
        .route(
            "/calls/:call_id/recording/start",
            post(media::start_recording),
        )
        .route(
            "/calls/:call_id/recording/stop",
            post(media::stop_recording),
        )
        // SIP trace endpoints (for ladder diagram debugging)
        .route(
            "/calls/:call_id/sip-messages",
            get(sip_trace::get_sip_messages),
        )
        // CDR (Call Detail Record) endpoints
        .route("/cdrs", get(cdrs::list_cdrs))
        .route("/cdrs/:call_id", get(cdrs::get_cdr))
        // Flow engine endpoints
        .route("/flows", get(flows::list_flows).post(flows::create_flow))
        .route(
            "/flows/:id",
            get(flows::get_flow)
                .put(flows::update_flow)
                .delete(flows::delete_flow),
        )
        .route("/flows/:id/versions", get(flows::list_flow_versions))
        // Phone number management
        .route(
            "/phone-numbers",
            get(phone_numbers::list_phone_numbers).post(phone_numbers::create_phone_number),
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
        // Configuration management
        .route("/config", get(config::list_configs))
        .route("/config/audit", get(config::get_audit_log))
        .route(
            "/config/:service",
            get(config::list_service_config),
        )
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
        // Voicemail
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
        // Voicemail Greetings
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
            "/api-tokens/:id",
            delete(api_tokens::delete_api_token),
        )
        // Self-service (end-user portal)
        .route("/me/profile", get(self_service::get_profile).put(self_service::update_profile))
        .route(
            "/me/profile/avatar",
            get(self_service::get_avatar)
                .post(self_service::upload_avatar)
                .delete(self_service::delete_avatar),
        )
        .route("/me/password", put(self_service::change_password))
        .route("/me/voicemail", get(self_service::get_voicemail).put(self_service::update_voicemail))
        .route("/me/voicemail/stats", get(self_service::get_voicemail_stats))
        .route("/me/voicemail/messages", get(self_service::list_voicemail_messages))
        .route(
            "/me/voicemail/messages/:id",
            put(self_service::update_voicemail_message).delete(self_service::delete_voicemail_message),
        )
        .route("/me/voicemail/messages/:id/audio", get(self_service::get_message_audio))
        .route(
            "/me/voicemail/greetings",
            get(self_service::list_voicemail_greetings).post(self_service::upload_voicemail_greeting),
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
        .route("/me/forwarding", get(self_service::get_forwarding))
        .route("/me/forwarding/:id", patch(self_service::update_forwarding))
}
