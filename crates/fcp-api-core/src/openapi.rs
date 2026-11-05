//! The admin API's OpenAPI contract (docs/DEVELOPER_SURFACE.md §3, phase 3).
//!
//! Every route under `/api/v1` carries a `#[utoipa::path]` beside its
//! handler; [`ApiDoc`] gathers them. `fcp-admin` adds its cluster routes
//! and serves the whole at `/api/v1/openapi.json`, and its
//! `tests/openapi.rs` keeps `docs/openapi.json` equal to it. The tests here
//! hold the contract to the router: every route bound is in the spec, and
//! nothing else is.

use utoipa::openapi::security::{HttpAuthScheme, HttpBuilder, SecurityScheme};
use utoipa::{Modify, OpenApi};

/// The admin API's contract.
#[derive(OpenApi)]
#[openapi(
    info(
        title = "FCP admin API",
        // The API's version, not the release's: the release changes with
        // every commit, the contract only when it changes.
        version = "v1",
        description = "The Ferrous Communications Platform admin API. Every request carries `Authorization: Bearer <token>`: the static token, a session's access token from `/auth/login` (refreshed by `/auth/refresh` with its HttpOnly cookie), or an API token (`docs/API_TOKENS.md`). Errors are `{error, code, details}`, times are RFC 3339, and lists are `{data, next_cursor, total}` paged by `limit` and `cursor`.",
        license(name = "Proprietary")
    ),
    servers((url = "/", description = "This server")),
    modifiers(&BearerAuth, &EventWebhooks),
    security(("bearer" = [])),
    paths(
        crate::v1::auth_session::discover,
        crate::v1::auth_session::login_totp,
        crate::v1::auth_session::enroll_totp,
        crate::v1::auth_session::confirm_totp,
        crate::v1::totp::get_totp,
        crate::v1::totp::enroll_totp,
        crate::v1::totp::confirm_totp,
        crate::v1::totp::disable_totp,
        crate::v1::totp::renew_recovery_codes,
        crate::v1::totp::reset_totp,
        crate::v1::app_auth::authorize,
        crate::v1::app_auth::approve,
        crate::v1::app_auth::create_session,
        crate::v1::app_auth::token,
        crate::v1::app_auth::revoke,
        crate::v1::softphone::get_config,
        crate::v1::softphone::rotate_password,
        crate::v1::softphone::set_line_label,
        crate::v1::user_devices::list_mine,
        crate::v1::user_devices::delete_mine,
        crate::v1::user_devices::list_users,
        crate::v1::sso::start,
        crate::v1::sso::callback,
        crate::v1::sso::exchange,
        crate::v1::sso::backchannel_logout,
        crate::v1::sso::saml_acs,
        crate::v1::sso::saml_metadata,
        crate::v1::sso::saml_slo_redirect,
        crate::v1::sso::saml_slo_post,
        crate::v1::sso::rotate_sp_key,
        crate::v1::sso::refresh_metadata,
        crate::v1::sso::list,
        crate::v1::sso::get,
        crate::v1::sso::create,
        crate::v1::sso::update,
        crate::v1::sso::delete,
        crate::v1::sso::check_provider,
        crate::v1::sso::list_identities,
        crate::v1::sso::unlink_identity,        crate::v1::auth_session::login,
        crate::v1::auth_session::logout,
        crate::v1::auth_session::change_expired_password,
        crate::v1::signup::signup,
        crate::v1::events::event_stream_websocket,
        crate::v1::auth_session::me,
        crate::v1::auth_session::refresh,
        crate::v1::self_service::get_profile,
        crate::v1::self_service::update_profile,
        crate::v1::self_service::get_avatar,
        crate::v1::self_service::upload_avatar,
        crate::v1::self_service::delete_avatar,
        crate::v1::self_service::change_password,
        crate::v1::self_service::get_voicemail,
        crate::v1::self_service::update_voicemail,
        crate::v1::self_service::get_voicemail_stats,
        crate::v1::self_service::list_voicemail_messages,
        crate::v1::self_service::update_voicemail_message,
        crate::v1::self_service::delete_voicemail_message,
        crate::v1::self_service::get_message_audio,
        crate::v1::self_service::list_voicemail_greetings,
        crate::v1::self_service::upload_voicemail_greeting,
        crate::v1::self_service::synthesize_voicemail_greeting,
        crate::v1::self_service::delete_voicemail_greeting,
        crate::v1::self_service::set_voicemail_greeting_active,
        crate::v1::self_service::download_voicemail_greeting_audio,
        crate::v1::self_service::get_my_location,
        crate::v1::self_service::declare_my_location,
        crate::v1::self_service::clear_my_location,
        crate::v1::me_data::list_my_calls,
        crate::v1::me_data::start_my_recording,
        crate::v1::me_data::stop_my_recording,
        crate::v1::call_quality::report_call_quality,
        crate::v1::device_captures::upload_capture,
        crate::v1::device_captures::list_my_captures,
        crate::v1::me_data::list_directory,
        crate::v1::me_contacts::list_contacts,
        crate::v1::me_contacts::create_contact,
        crate::v1::me_contacts::get_contact,
        crate::v1::me_contacts::update_contact,
        crate::v1::me_contacts::delete_contact,
        crate::v1::me_contacts::export_contacts,
        crate::v1::me_contacts::import_contacts,
        crate::v1::me_presence::get_my_presence,
        crate::v1::me_presence::update_my_presence,
        crate::v1::me_presence::list_statuses,
        crate::v1::me_presence::create_status,
        crate::v1::me_presence::update_status,
        crate::v1::me_presence::delete_status,
        crate::v1::me_presence::order_statuses,
        crate::v1::me_presence::get_user_presence,
        crate::v1::me_events::my_events,
        crate::v1::self_service::get_my_calling,
        crate::v1::self_service::update_my_calling,
        crate::v1::self_service::get_my_executives,
        crate::v1::self_service::update_my_executive,
        crate::v1::self_service::get_my_hoteling,
        crate::v1::self_service::logout_my_hoteling,
        crate::v1::self_service::list_my_conferences,
        crate::v1::self_service::get_my_conference,
        crate::v1::self_service::update_my_conference,
        crate::v1::self_service::dial_out_my_conference,
        crate::v1::self_service::mint_my_join_token,
        crate::v1::api_tokens::list_my_api_tokens,
        crate::v1::api_tokens::create_my_api_token,
        crate::v1::api_tokens::revoke_my_api_token,
        crate::v1::api_tokens::rotate_my_api_token,
        crate::v1::dashboard::summary,
        crate::v1::system::status_scoped,
        crate::v1::encryption::status,
        crate::v1::encryption::rotate,
        crate::v1::encryption::reencrypt,
        crate::v1::encryption::rewrap,
        crate::v1::encryption::seal,
        crate::v1::emergency::get_config,
        crate::v1::emergency::update_config,
        crate::v1::emergency::readiness,
        crate::v1::emergency_networks::list_networks,
        crate::v1::emergency_networks::create_network,
        crate::v1::emergency_networks::get_network,
        crate::v1::emergency_networks::update_network,
        crate::v1::emergency_networks::delete_network,
        crate::v1::emergency::list_calls,
        crate::v1::emergency::notify_call,
        crate::v1::emergency::list_call_notifications,
        crate::v1::emergency::validate_address,
        crate::v1::emergency::validate_phone_number,
        crate::v1::emergency::validate_location,
        crate::v1::emergency::validate_device,
        crate::v1::usage::get_usage,
        crate::v1::usage::get_usage_by_number,
        crate::v1::usage::billing_summary,
        crate::v1::billing::get_balance,
        crate::v1::billing::update_balance,
        crate::v1::billing::list_transactions,
        crate::v1::billing::create_transaction,
        crate::v1::rate_plans::list_rate_plans,
        crate::v1::rate_plans::create_rate_plan,
        crate::v1::rate_plans::get_rate_plan,
        crate::v1::rate_plans::update_rate_plan,
        crate::v1::rate_plans::delete_rate_plan,
        crate::v1::rate_plans::list_rates,
        crate::v1::rate_plans::upsert_rate,
        crate::v1::rate_plans::delete_rate,
        crate::v1::debug::debug_summary,
        crate::v1::debug::list_dialogs,
        crate::v1::debug::list_subscriptions,
        crate::v1::debug::list_calls,
        crate::v1::debug::list_rate_limits,
        crate::v1::tenants::list_tenants,
        crate::v1::tenants::create_tenant,
        crate::v1::tenants::get_tenant,
        crate::v1::tenants::update_tenant,
        crate::v1::tenants::delete_tenant,
        crate::v1::tenants::get_tenant_deletion,
        crate::v1::sites::list_sites,
        crate::v1::sites::create_site,
        crate::v1::sites::get_site,
        crate::v1::sites::delete_site,
        crate::v1::sites::new_enrollment_token,
        crate::v1::sites::disable_site,
        crate::v1::sites::enable_site,
        crate::v1::users::list_users,
        crate::v1::users::create_user,
        crate::v1::users::get_user,
        crate::v1::users::get_user_by_id,
        crate::v1::users::update_user,
        crate::v1::users::delete_user,
        crate::v1::users::update_password,
        crate::v1::users::unlock_user,
        crate::v1::users::restore_user,
        crate::v1::users::purge_user,
        crate::v1::users::update_digest_password,
        crate::v1::users::delete_digest_password,
        crate::v1::calling::get_user_calling,
        crate::v1::calling::update_user_calling,
        crate::v1::registrations::list_registrations,
        crate::v1::registrations::get_registrations,
        crate::v1::registrations::delete_registrations,
        crate::v1::registrations::delete_registration,
        crate::v1::media::list_calls,
        crate::v1::media::get_call,
        crate::v1::media::hangup_call,
        crate::v1::media::start_recording,
        crate::v1::media::stop_recording,
        crate::v1::call_control::hold,
        crate::v1::call_control::resume,
        crate::v1::call_control::transfer,
        crate::v1::call_control::redirect,
        crate::v1::call_control::play,
        crate::v1::call_control::say,
        crate::v1::call_control::hangup,
        crate::v1::supervision::monitor,
        crate::v1::supervision::whisper,
        crate::v1::supervision::barge,
        crate::v1::supervision::get,
        crate::v1::supervision::leave,
        crate::v1::sip_trace::get_sip_messages,
        crate::v1::compliance::get_policy,
        crate::v1::compliance::update_policy,
        crate::v1::compliance::run_retention_now,
        crate::v1::compliance::list_retention_runs,
        crate::v1::compliance::list_audit_log,
        crate::v1::compliance::export_audit_log,
        crate::v1::compliance::list_deletions,
        crate::v1::compliance::export_deletions,
        crate::v1::compliance::list_holds,
        crate::v1::compliance::place_hold,
        crate::v1::compliance::get_hold,
        crate::v1::compliance::release_hold,
        crate::v1::compliance::list_subject_requests,
        crate::v1::compliance::file_subject_request,
        crate::v1::compliance::get_subject_request,
        crate::v1::compliance::download_subject_bundle,
        crate::v1::compliance::confirm_subject_request,
        crate::v1::recordings::list_recordings,
        crate::v1::recordings::get_recording,
        crate::v1::recordings::delete_recording,
        crate::v1::recordings::download_recording,
        crate::v1::cdrs::list_cdrs,
        crate::v1::cdrs::export_cdrs,
        crate::v1::cdrs::get_cdr,
        crate::v1::call_quality::get_call_quality,
        crate::v1::device_captures::list_captures,
        crate::v1::device_captures::list_allows,
        crate::v1::device_captures::create_allow,
        crate::v1::device_captures::delete_allow,
        crate::v1::device_captures::delete_capture,
        crate::v1::device_captures::download_capture,
        crate::v1::flow_traces::list_flow_traces,
        crate::v1::flow_traces::get_call_flow_traces,
        crate::v1::flows::list_flows,
        crate::v1::flows::create_flow,
        crate::v1::flows::get_flow,
        crate::v1::flows::update_flow,
        crate::v1::flows::delete_flow,
        crate::v1::flows::list_flow_versions,
        crate::v1::flows::get_flow_version,
        crate::v1::flows::list_untenanted_flows,
        crate::v1::flows::assign_flow,
        crate::v1::phone_numbers::list_phone_numbers,
        crate::v1::phone_numbers::create_phone_number,
        crate::v1::phone_numbers::search_available_numbers,
        crate::v1::phone_numbers::get_phone_number,
        crate::v1::phone_numbers::update_phone_number,
        crate::v1::phone_numbers::delete_phone_number,
        crate::v1::phone_numbers::activate_phone_number,
        crate::v1::phone_numbers::purchase_phone_number,
        crate::v1::phone_numbers::release_phone_number,
        crate::v1::line_bindings::list_line_bindings,
        crate::v1::line_bindings::create_line_binding,
        crate::v1::line_bindings::get_line_binding,
        crate::v1::line_bindings::update_line_binding,
        crate::v1::line_bindings::delete_line_binding,
        crate::v1::extensions::list_extensions,
        crate::v1::extensions::create_extension,
        crate::v1::extensions::get_extension,
        crate::v1::extensions::update_extension,
        crate::v1::extensions::delete_extension,
        crate::v1::devices::list_devices,
        crate::v1::devices::create_device,
        crate::v1::devices::get_device,
        crate::v1::devices::update_device,
        crate::v1::devices::delete_device,
        crate::v1::hoteling::list_sessions,
        crate::v1::hoteling::end_session_by_id,
        crate::v1::config_templates::list_config_templates,
        crate::v1::config_templates::create_config_template,
        crate::v1::config_templates::list_template_library,
        crate::v1::config_templates::instantiate_library_template,
        crate::v1::config_templates::get_config_template,
        crate::v1::config_templates::update_config_template,
        crate::v1::config_templates::delete_config_template,
        crate::v1::locations::list_locations,
        crate::v1::locations::create_location,
        crate::v1::locations::get_location,
        crate::v1::locations::update_location,
        crate::v1::locations::delete_location,
        crate::v1::queues::list_queues,
        crate::v1::queues::get_all_queue_stats,
        crate::v1::queues::get_queue,
        crate::v1::queues::get_queue_stats,
        crate::v1::queues::list_queue_agents,
        crate::v1::queues::agent_login,
        crate::v1::queues::agent_logout,
        crate::v1::queues::update_agent_state,
        crate::v1::queues::pause_agent,
        crate::v1::queues::resume_agent,
        crate::v1::presence::list_presences,
        crate::v1::presence::get_stats,
        crate::v1::presence::get_presence,
        crate::v1::presence::update_presence,
        crate::v1::presence::delete_presence,
        crate::v1::presence::list_watchers,
        crate::v1::presence::delete_subscription,
        crate::v1::presence::get_policy,
        crate::v1::presence::set_policy,
        crate::v1::presence::delete_policy,
        crate::v1::presence::get_pres_rules,
        crate::v1::presence::set_pres_rules,
        crate::v1::config::list_configs,
        crate::v1::config::get_audit_log,
        crate::v1::config::list_service_config,
        crate::v1::config::get_config,
        crate::v1::config::update_config,
        crate::v1::config::delete_config,
        crate::v1::config::validate_config,
        crate::v1::prompts::list_prompts,
        crate::v1::prompts::create_prompt,
        crate::v1::prompts::get_prompt,
        crate::v1::prompts::update_prompt,
        crate::v1::prompts::delete_prompt,
        crate::v1::prompts::download_audio,
        crate::v1::prompts::upload_audio,
        crate::v1::prompts::synthesize_prompt,
        crate::v1::prompts::override_prompt,
        crate::v1::voicemail::list_mailboxes,
        crate::v1::voicemail::create_mailbox,
        crate::v1::voicemail::get_mailbox,
        crate::v1::voicemail::update_mailbox,
        crate::v1::voicemail::delete_mailbox,
        crate::v1::voicemail::enable_mailbox,
        crate::v1::voicemail::disable_mailbox,
        crate::v1::voicemail::get_mailbox_stats,
        crate::v1::voicemail::list_messages,
        crate::v1::voicemail::get_message,
        crate::v1::voicemail::update_message,
        crate::v1::voicemail::delete_message,
        crate::v1::voicemail::list_greetings,
        crate::v1::voicemail::upload_greeting,
        crate::v1::voicemail::delete_greeting,
        crate::v1::voicemail::set_greeting_active,
        crate::v1::voicemail::download_greeting_audio,
        crate::v1::api_tokens::list_api_tokens,
        crate::v1::api_tokens::create_api_token,
        crate::v1::api_tokens::grantable_permissions,
        crate::v1::api_tokens::delete_api_token,
        crate::v1::api_tokens::rotate_api_token,
        crate::v1::webhooks::list_webhooks,
        crate::v1::webhooks::create_webhook,
        crate::v1::webhooks::list_event_types,
        crate::v1::webhooks::get_webhook,
        crate::v1::webhooks::update_webhook,
        crate::v1::webhooks::delete_webhook,
        crate::v1::webhooks::rotate_webhook_secret,
        crate::v1::webhooks::test_webhook,
        crate::v1::webhooks::list_deliveries,
        crate::v1::webhooks::replay_delivery,
        crate::v1::lcr_routes::list_lcr_routes,
        crate::v1::lcr_routes::create_lcr_route,
        crate::v1::lcr_routes::get_lcr_route,
        crate::v1::lcr_routes::update_lcr_route,
        crate::v1::lcr_routes::delete_lcr_route,
        crate::v1::routing::preview_routing,
        crate::v1::classes_of_service::list_classes,
        crate::v1::classes_of_service::create_class,
        crate::v1::classes_of_service::classify,
        crate::v1::classes_of_service::get_class,
        crate::v1::classes_of_service::update_class,
        crate::v1::classes_of_service::delete_class,
        crate::v1::provisioning::get_key,
        crate::v1::provisioning::rotate_key,
        crate::v1::provisioning::device_fetches,
        crate::v1::devices::render_device,
        crate::v1::firmware::list_images,
        crate::v1::firmware::upload_image,
        crate::v1::firmware::delete_image,
        crate::v1::firmware::list_targets,
        crate::v1::firmware::create_target,
        crate::v1::firmware::update_target,
        crate::v1::firmware::delete_target,
        crate::v1::branding::get_brand,
        crate::v1::branding::put_brand,
        crate::v1::branding::delete_brand,
        crate::v1::branding::list_assets,
        crate::v1::branding::upload_asset,
        crate::v1::branding::delete_asset,
        crate::v1::branding::deployment_profile,
        crate::v1::redirection::list_accounts,
        crate::v1::redirection::put_account,
        crate::v1::redirection::delete_account,
        crate::v1::redirection::check_account,
        crate::v1::redirection::sync_account,
        crate::v1::redirection::device_redirection,
        crate::v1::redirection::register_device,
        crate::v1::devices::resync_device,
        crate::v1::devices::push_test_device,
        crate::v1::config_templates::resync_template,
        crate::v1::devices::import_devices,
        crate::v1::devices::list_pending,
        crate::v1::devices::claim_pending,
        crate::v1::devices::dismiss_pending,
        crate::v1::config_templates::preview_template,
        crate::v1::groups::list_groups,
        crate::v1::groups::create_group,
        crate::v1::groups::get_group,
        crate::v1::groups::update_group,
        crate::v1::groups::delete_group,
        crate::v1::groups::list_members,
        crate::v1::groups::add_member,
        crate::v1::groups::remove_member,
        crate::v1::groups::list_user_groups,
        crate::v1::schedules::list_schedules,
        crate::v1::schedules::create_schedule,
        crate::v1::schedules::get_schedule,
        crate::v1::schedules::update_schedule,
        crate::v1::schedules::delete_schedule,
        crate::v1::parking::list_lots,
        crate::v1::parking::create_lot,
        crate::v1::parking::get_lot,
        crate::v1::parking::update_lot,
        crate::v1::parking::delete_lot,
        crate::v1::parking::list_calls,
        crate::v1::parking::retrieve_call,
        crate::v1::parking::hang_up_call,
        crate::v1::routing::list_routing_rules,
        crate::v1::routing::create_routing_rule,
        crate::v1::routing::get_routing_rule,
        crate::v1::routing::update_routing_rule,
        crate::v1::routing::delete_routing_rule,
        crate::v1::queue_config::list_queue_configs,
        crate::v1::queue_config::create_queue_config,
        crate::v1::queue_config::get_queue_config,
        crate::v1::queue_config::update_queue_config,
        crate::v1::queue_config::delete_queue_config,
        crate::v1::trunks::list_trunks,
        crate::v1::trunks::create_trunk,
        crate::v1::trunks::get_trunk,
        crate::v1::trunks::update_trunk,
        crate::v1::trunks::delete_trunk,
        crate::v1::trunks::test_trunk,
        crate::v1::trunks::clone_trunk,
        crate::v1::trunks::get_trunk_stats,
        crate::v1::conferences::list_meetings,
        crate::v1::conferences::create_meeting,
        crate::v1::conferences::get_meeting,
        crate::v1::conferences::update_meeting,
        crate::v1::conferences::delete_meeting,
        crate::v1::conferences::enable_meeting,
        crate::v1::conferences::disable_meeting,
        crate::v1::conferences::mint_join_token,
        crate::v1::conferences_live::video_status,
        crate::v1::conferences_live::set_video_layout,
        crate::v1::conferences_live::pin,
        crate::v1::conferences_live::unpin,
        crate::v1::conferences_live::spotlight,
        crate::v1::conferences_live::unspotlight,
        crate::v1::conferences_live::participant_video,
        crate::v1::conferences_live::stop_content,
        crate::v1::conferences_live::set_content_layout,
        crate::v1::conferences_live::dial_out,
    )
)]
pub struct ApiDoc;

/// The bearer scheme every route uses.
struct BearerAuth;

impl Modify for BearerAuth {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        let components = openapi.components.get_or_insert_with(Default::default);
        components.add_security_scheme(
            "bearer",
            SecurityScheme::Http(
                HttpBuilder::new()
                    .scheme(HttpAuthScheme::Bearer)
                    .description(Some(
                        "The static token, a session JWT, or an `fcp_…` API token",
                    ))
                    .build(),
            ),
        );
    }
}

/// The spec's `webhooks` section: what a tenant's webhook receives
/// (docs/DEVELOPER_SURFACE.md §3, phase 5) — one POST whose body is a
/// `WebhookEvent`, its `event_type` one of the platform's event types.
struct EventWebhooks;

impl Modify for EventWebhooks {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        use utoipa::openapi::path::{HttpMethod, OperationBuilder, PathItem};
        use utoipa::openapi::request_body::RequestBodyBuilder;
        use utoipa::openapi::schema::{ObjectBuilder, Schema, Type};
        use utoipa::openapi::{ContentBuilder, PathsBuilder, RefOr, ResponseBuilder};
        use utoipa::PartialSchema;

        let mut names: Vec<&str> = fcp_events::PlatformEvent::EVENT_TYPES.to_vec();
        names.push(crate::v1::webhooks::TEST_EVENT_TYPE);
        let mut event = crate::v1::webhooks::WebhookEvent::schema();
        if let RefOr::T(Schema::Object(object)) = &mut event {
            object.properties.insert(
                "event_type".to_string(),
                ObjectBuilder::new()
                    .schema_type(Type::String)
                    .enum_values(Some(names))
                    .description(Some(
                        "A name from `/api/v1/webhooks/event-types`, or `webhook.test`",
                    ))
                    .into(),
            );
        }
        let components = openapi.components.get_or_insert_with(Default::default);
        components.schemas.insert("WebhookEvent".to_string(), event);

        let operation = OperationBuilder::new()
            .operation_id(Some("webhook_event"))
            .summary(Some("A platform event, delivered to a tenant's webhook"))
            .description(Some(
                "Verify `X-FCP-Signature` (`t=<unix>,v1=<hex HMAC-SHA256 of \"<t>.<body>\">`) with the webhook's secret before trusting the body, and refuse a stale `t`. During a rotation two `v1` values are sent. Answer 2xx; anything else is retried with backoff.",
            ))
            .parameter(
                utoipa::openapi::path::ParameterBuilder::new()
                    .name("X-FCP-Signature")
                    .parameter_in(utoipa::openapi::path::ParameterIn::Header)
                    .required(utoipa::openapi::Required::True)
                    .schema(Some(ObjectBuilder::new().schema_type(Type::String))),
            )
            .parameter(
                utoipa::openapi::path::ParameterBuilder::new()
                    .name("X-FCP-Event")
                    .parameter_in(utoipa::openapi::path::ParameterIn::Header)
                    .required(utoipa::openapi::Required::True)
                    .schema(Some(ObjectBuilder::new().schema_type(Type::String))),
            )
            .request_body(Some(
                RequestBodyBuilder::new()
                    .content(
                        "application/json",
                        ContentBuilder::new()
                            .schema(Some(utoipa::openapi::Ref::from_schema_name(
                                "WebhookEvent",
                            )))
                            .build(),
                    )
                    .required(Some(utoipa::openapi::Required::True))
                    .build(),
            ))
            .response(
                "2XX",
                ResponseBuilder::new().description("Received").build(),
            )
            .build();
        openapi.webhooks = Some(
            PathsBuilder::new()
                .path("platform_event", PathItem::new(HttpMethod::Post, operation))
                .build(),
        );
    }
}

/// The contract as JSON, pretty-printed: what `docs/openapi.json` holds.
pub fn json(doc: &utoipa::openapi::OpenApi) -> String {
    let mut out = doc.to_pretty_json().expect("the spec serialises");
    out.push('\n');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    /// `(METHOD, /api/v1/path)` for every operation in the spec, with
    /// `{param}` written as the router writes it, `:param`.
    fn operations(doc: &utoipa::openapi::OpenApi) -> BTreeSet<(String, String)> {
        let spec = serde_json::to_value(doc).unwrap();
        let mut out = BTreeSet::new();
        for (path, item) in spec["paths"].as_object().unwrap() {
            for method in ["get", "post", "put", "patch", "delete"] {
                if item.get(method).is_some() {
                    let router_path = regex_like_braces(path);
                    out.insert((method.to_uppercase(), router_path));
                }
            }
        }
        out
    }

    fn regex_like_braces(path: &str) -> String {
        let mut out = String::new();
        let mut in_param = false;
        for c in path.chars() {
            match c {
                '{' => {
                    in_param = true;
                    out.push(':');
                }
                '}' => in_param = false,
                c => {
                    let _ = in_param;
                    out.push(c)
                }
            }
        }
        out
    }

    #[test]
    fn the_spec_holds_every_route_and_nothing_else() {
        let doc = ApiDoc::openapi();
        let documented = operations(&doc);
        let mut bound = BTreeSet::new();
        for (method, path, _) in crate::route_permissions::ROUTES {
            if path.starts_with("/cluster/") || path.starts_with("/system/services") {
                continue; // fcp-admin's, documented there
            }
            bound.insert((method.to_string(), format!("/api/v1{path}")));
        }
        // The routes outside the bearer layer.
        for (method, path) in [
            ("POST", "/auth/discover"),
            ("POST", "/auth/login"),
            ("POST", "/auth/login/totp"),
            ("POST", "/auth/totp/enroll"),
            ("POST", "/auth/totp/confirm"),
            ("GET", "/auth/sso/:id/start"),
            ("GET", "/auth/sso/callback"),
            ("POST", "/auth/sso/exchange"),
            ("POST", "/auth/sso/:id/backchannel-logout"),
            ("GET", "/auth/saml/:id"),
            ("POST", "/auth/saml/:id/acs"),
            ("GET", "/auth/saml/:id/slo"),
            ("POST", "/auth/saml/:id/slo"),
            ("GET", "/auth/app/authorize"),
            ("POST", "/auth/app/token"),
            ("POST", "/auth/app/revoke"),
            ("POST", "/auth/refresh"),
            ("POST", "/auth/logout"),
            ("POST", "/auth/change-expired-password"),
            ("POST", "/auth/signup"),
            ("GET", "/events"),
            ("GET", "/me/events"),
        ] {
            bound.insert((method.to_string(), format!("/api/v1{path}")));
        }
        let missing: Vec<_> = bound.difference(&documented).collect();
        assert!(missing.is_empty(), "routes the spec lacks: {missing:#?}");
        let extra: Vec<_> = documented.difference(&bound).collect();
        assert!(extra.is_empty(), "operations no route serves: {extra:#?}");
    }

    #[test]
    fn every_operation_is_named_and_can_fail_with_the_error_shape() {
        let spec = serde_json::to_value(ApiDoc::openapi()).unwrap();
        let mut ids = BTreeSet::new();
        for (path, item) in spec["paths"].as_object().unwrap() {
            for (method, op) in item.as_object().unwrap() {
                let id = op["operationId"].as_str().unwrap_or_default().to_string();
                assert!(!id.is_empty(), "{method} {path} has no operationId");
                assert!(ids.insert(id.clone()), "{id} twice");
                assert!(
                    op["responses"]["default"].is_object(),
                    "{method} {path} documents no error"
                );
            }
        }
    }

    /// Two schemas of one name overwrite each other in the spec, silently:
    /// every `ToSchema` type the API uses has a name of its own, or an
    /// `#[schema(as = module::Name)]` to give it one.
    #[test]
    fn no_two_schemas_share_a_name() {
        let root = concat!(env!("CARGO_MANIFEST_DIR"), "/..");
        let mut names = std::collections::BTreeMap::<String, Vec<String>>::new();
        for dir in [
            "fcp-api-core/src",
            "fcp-core/src",
            "fcp-presence/src",
            "fcp-cluster/src",
        ] {
            let mut stack = vec![std::path::PathBuf::from(format!("{root}/{dir}"))];
            while let Some(path) = stack.pop() {
                if path.is_dir() {
                    for entry in std::fs::read_dir(&path).unwrap() {
                        stack.push(entry.unwrap().path());
                    }
                    continue;
                }
                if path.extension().is_none_or(|e| e != "rs") {
                    continue;
                }
                let text = std::fs::read_to_string(&path).unwrap();
                let lines: Vec<&str> = text.lines().collect();
                for (i, line) in lines.iter().enumerate() {
                    if !(line.contains("derive") && line.contains("ToSchema")) {
                        continue;
                    }
                    let mut renamed = None;
                    for next in &lines[i + 1..] {
                        let t = next.trim();
                        if let Some(rest) = t.strip_prefix("#[schema(as = ") {
                            renamed = Some(rest.trim_end_matches(")]").replace("::", "."));
                        }
                        let item = t
                            .trim_start_matches("pub(crate) ")
                            .trim_start_matches("pub ");
                        if let Some(rest) = item
                            .strip_prefix("struct ")
                            .or_else(|| item.strip_prefix("enum "))
                        {
                            let name: String = rest
                                .chars()
                                .take_while(|c| c.is_alphanumeric() || *c == '_')
                                .collect();
                            names
                                .entry(renamed.clone().unwrap_or(name))
                                .or_default()
                                .push(path.display().to_string());
                            break;
                        }
                    }
                }
            }
        }
        let clashes: Vec<_> = names.iter().filter(|(_, at)| at.len() > 1).collect();
        assert!(clashes.is_empty(), "schema names used twice: {clashes:#?}");
    }

    /// A parameter "in path" names a segment of the path; one that does not
    /// is a query parameter declared in the wrong place.
    #[test]
    fn path_parameters_are_in_the_path() {
        let spec = serde_json::to_value(ApiDoc::openapi()).unwrap();
        let mut wrong = Vec::new();
        for (path, item) in spec["paths"].as_object().unwrap() {
            for (method, op) in item.as_object().unwrap() {
                for p in op["parameters"].as_array().into_iter().flatten() {
                    let name = p["name"].as_str().unwrap_or_default();
                    if p["in"] == "path" && !path.contains(&format!("{{{name}}}")) {
                        wrong.push(format!("{method} {path}: {name}"));
                    }
                }
            }
        }
        assert!(
            wrong.is_empty(),
            "path parameters not in the path: {wrong:#?}"
        );
    }
}
