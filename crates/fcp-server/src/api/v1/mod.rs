//! API v1 routes.

pub mod cdrs;
pub mod debug;
pub mod devices;
pub mod events;
pub mod extensions;
pub mod line_bindings;
pub mod media;
pub mod metrics;
pub mod phone_numbers;
pub mod registrations;
pub mod system;
pub mod tenants;
pub mod users;

use crate::api::state::ApiState;
use axum::{
    routing::{delete, get, post, put},
    Router,
};

/// Create the v1 API router.
pub fn router() -> Router<ApiState> {
    Router::new()
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
        .route(
            "/users/:username/digest-password",
            put(users::update_digest_password).delete(users::delete_digest_password),
        )
        // Registration viewing endpoints
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
        // CDR (Call Detail Record) endpoints
        .route("/cdrs", get(cdrs::list_cdrs))
        .route("/cdrs/:call_id", get(cdrs::get_cdr))
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
}
