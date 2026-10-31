//! Which permission each admin route needs (docs/DEVELOPER_SURFACE.md §3,
//! phase 1).
//!
//! Every route under `/api/v1` is in [`ROUTES`]: the self-service and
//! session routes as [`Gate::Open`] (their handlers find the caller
//! themselves), the admin routes by the permission they need. The
//! [`enforce`] middleware looks the matched route up and checks the
//! caller against it, so an API token reaches the routes its permissions
//! name and no more, and a role reaches what its permissions say. A route
//! missing from the table is refused to everyone but the static token, and
//! `every_route_names_a_gate` fails when one is added without an entry.
//!
//! Handlers that already check a permission keep their check; the table
//! names the same one, and a handler that needs a second (a routing rule
//! naming a trunk) still asks for it.

use crate::auth::{check_permission, AuthIdentity};
use crate::state::ApiState;
use axum::{
    extract::{MatchedPath, Request, State},
    http::Method,
    middleware::Next,
    response::{IntoResponse, Response},
};
use fcp_core::permissions::Permission;
use fcp_core::permissions::Permission::*;

/// What a route asks of its caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gate {
    /// Any authenticated caller; the handler decides (self-service and the
    /// session's own routes).
    Open,
    /// Any administrator, whatever its permissions (the dashboard).
    AnyAdmin,
    /// A caller holding this permission.
    Needs(Permission),
}

use Gate::{AnyAdmin, Needs, Open};

const GET: &str = "GET";
const POST: &str = "POST";
const PUT: &str = "PUT";
const PATCH: &str = "PATCH";
const DELETE: &str = "DELETE";

/// Every authenticated route under `/api/v1`, by method and path template.
pub const ROUTES: &[(&str, &str, Gate)] = &[
    // The session and self-service (`v1::router`'s user routes).
    (GET, "/auth/me", Open),
    (POST, "/auth/app/approve", Open),
    (POST, "/auth/app/sessions", Open),
    (GET, "/me/profile", Open),
    (PUT, "/me/profile", Open),
    (GET, "/me/profile/avatar", Open),
    (POST, "/me/profile/avatar", Open),
    (DELETE, "/me/profile/avatar", Open),
    (PUT, "/me/password", Open),
    (GET, "/me/totp", Open),
    (POST, "/me/totp/enroll", Open),
    (POST, "/me/totp/confirm", Open),
    (POST, "/me/totp/disable", Open),
    (POST, "/me/totp/recovery-codes", Open),
    (GET, "/me/voicemail", Open),
    (PUT, "/me/voicemail", Open),
    (GET, "/me/voicemail/stats", Open),
    (GET, "/me/voicemail/messages", Open),
    (PUT, "/me/voicemail/messages/:id", Open),
    (DELETE, "/me/voicemail/messages/:id", Open),
    (GET, "/me/voicemail/messages/:id/audio", Open),
    (GET, "/me/voicemail/greetings", Open),
    (POST, "/me/voicemail/greetings", Open),
    (POST, "/me/voicemail/greetings/synthesize", Open),
    (DELETE, "/me/voicemail/greetings/:greeting_type", Open),
    (PUT, "/me/voicemail/greetings/:greeting_type/active", Open),
    (GET, "/me/voicemail/greetings/:greeting_type/audio", Open),
    (GET, "/me/location", Open),
    (PUT, "/me/location", Open),
    (DELETE, "/me/location", Open),
    (GET, "/me/softphone", Open),
    (PUT, "/me/lines/:line/label", Open),
    (POST, "/me/softphone/rotate-password", Open),
    (GET, "/me/devices", Open),
    (DELETE, "/me/devices/:id", Open),
    (POST, "/me/devices/:id/captures", Open),
    (GET, "/me/captures", Open),
    (GET, "/me/calls", Open),
    (POST, "/me/calls/:call_id/recording/start", Open),
    (POST, "/me/calls/:call_id/recording/stop", Open),
    (POST, "/me/calls/:call_id/quality", Open),
    (GET, "/me/directory", Open),
    (GET, "/me/contacts", Open),
    (POST, "/me/contacts", Open),
    (GET, "/me/contacts/export", Open),
    (POST, "/me/contacts/import", Open),
    (GET, "/me/contacts/:id", Open),
    (PUT, "/me/contacts/:id", Open),
    (DELETE, "/me/contacts/:id", Open),
    (GET, "/me/presence", Open),
    (PUT, "/me/presence", Open),
    (GET, "/me/presence/statuses", Open),
    (POST, "/me/presence/statuses", Open),
    (PUT, "/me/presence/statuses/order", Open),
    (PUT, "/me/presence/statuses/:id", Open),
    (DELETE, "/me/presence/statuses/:id", Open),
    (GET, "/me/calling", Open),
    (PUT, "/me/calling", Open),
    (GET, "/me/executives", Open),
    (PUT, "/me/executives/:user_id", Open),
    (GET, "/me/hoteling", Open),
    (DELETE, "/me/hoteling", Open),
    (GET, "/me/conferences", Open),
    (GET, "/me/conferences/:id", Open),
    (PATCH, "/me/conferences/:id", Open),
    (POST, "/me/conferences/:id/dial-out", Open),
    (POST, "/me/conferences/:id/join-tokens", Open),
    (GET, "/me/api-tokens", Open),
    (POST, "/me/api-tokens", Open),
    (DELETE, "/me/api-tokens/:id", Open),
    (POST, "/me/api-tokens/:id/rotate", Open),
    // Dashboard and status.
    (GET, "/dashboard/summary", AnyAdmin),
    (GET, "/status", AnyAdmin),
    // Encryption at rest (the handlers also require a platform admin).
    (GET, "/system/encryption", Needs(SystemConfig)),
    (POST, "/system/encryption/rotate", Needs(SystemConfig)),
    (POST, "/system/encryption/reencrypt", Needs(SystemConfig)),
    (POST, "/system/encryption/rewrap", Needs(SystemConfig)),
    (POST, "/system/encryption/seal", Needs(SystemConfig)),
    // Emergency calling.
    (GET, "/emergency/config", Needs(EmergencyRead)),
    (PUT, "/emergency/config", Needs(EmergencyManage)),
    (GET, "/emergency/readiness", Needs(EmergencyRead)),
    (GET, "/emergency/networks", Needs(EmergencyRead)),
    (POST, "/emergency/networks", Needs(EmergencyManage)),
    (GET, "/emergency/networks/:id", Needs(EmergencyRead)),
    (PATCH, "/emergency/networks/:id", Needs(EmergencyManage)),
    (DELETE, "/emergency/networks/:id", Needs(EmergencyManage)),
    (GET, "/emergency/calls", Needs(EmergencyRead)),
    (POST, "/emergency/notify-call", Needs(EmergencyManage)),
    (
        GET,
        "/emergency/calls/:call_id/notifications",
        Needs(EmergencyRead),
    ),
    (POST, "/emergency/validate-address", Needs(EmergencyManage)),
    (
        POST,
        "/phone-numbers/:id/e911/validate",
        Needs(EmergencyManage),
    ),
    (POST, "/locations/:id/e911/validate", Needs(EmergencyManage)),
    (POST, "/devices/:id/e911/validate", Needs(EmergencyManage)),
    // Usage and billing.
    (GET, "/usage", Needs(BillingRead)),
    (GET, "/usage/by-number", Needs(BillingRead)),
    (GET, "/billing/summary", Needs(BillingRead)),
    (GET, "/billing/balance", Needs(BillingRead)),
    (PUT, "/billing/balance", Needs(BillingManage)),
    (GET, "/billing/balance/transactions", Needs(BillingRead)),
    (POST, "/billing/balance/transactions", Needs(BillingManage)),
    (GET, "/rate-plans", Needs(BillingRead)),
    (POST, "/rate-plans", Needs(BillingManage)),
    (GET, "/rate-plans/:id", Needs(BillingRead)),
    (PATCH, "/rate-plans/:id", Needs(BillingManage)),
    (DELETE, "/rate-plans/:id", Needs(BillingManage)),
    (GET, "/rate-plans/:id/rates", Needs(BillingRead)),
    (POST, "/rate-plans/:id/rates", Needs(BillingManage)),
    (
        DELETE,
        "/rate-plans/:id/rates/:rate_id",
        Needs(BillingManage),
    ),
    // Debugging.
    (GET, "/debug/summary", Needs(DebugView)),
    (GET, "/debug/dialogs", Needs(DebugView)),
    (GET, "/debug/subscriptions", Needs(DebugView)),
    (GET, "/debug/calls", Needs(DebugView)),
    (GET, "/debug/rate-limits", Needs(DebugView)),
    // Tenants.
    (GET, "/tenants", Needs(TenantRead)),
    (POST, "/tenants", Needs(TenantCreate)),
    (GET, "/tenants/:id", Needs(TenantRead)),
    (PUT, "/tenants/:id", Needs(TenantUpdate)),
    (DELETE, "/tenants/:id", Needs(TenantDelete)),
    (GET, "/tenants/:id/deletion", Needs(TenantRead)),
    // Sites.
    (GET, "/sites", Needs(SiteRead)),
    (POST, "/sites", Needs(SiteManage)),
    (GET, "/sites/:id", Needs(SiteRead)),
    (DELETE, "/sites/:id", Needs(SiteManage)),
    (POST, "/sites/:id/enrollment-token", Needs(SiteManage)),
    (POST, "/sites/:id/disable", Needs(SiteManage)),
    (POST, "/sites/:id/enable", Needs(SiteManage)),
    // Users.
    (GET, "/users", Needs(UserRead)),
    (POST, "/users", Needs(UserCreate)),
    (GET, "/users/:username", Needs(UserRead)),
    (GET, "/users/by-id/:id", Needs(UserRead)),
    (DELETE, "/users/:username/totp", Needs(UserUpdate)),
    (GET, "/users/:username/identities", Needs(UserRead)),
    (GET, "/users/:username/groups", Needs(GroupRead)),
    (GET, "/groups", Needs(GroupRead)),
    (POST, "/groups", Needs(GroupManage)),
    (GET, "/groups/:id", Needs(GroupRead)),
    (PUT, "/groups/:id", Needs(GroupManage)),
    (DELETE, "/groups/:id", Needs(GroupManage)),
    (GET, "/groups/:id/members", Needs(GroupRead)),
    (POST, "/groups/:id/members", Needs(GroupManage)),
    (DELETE, "/groups/:id/members/:username", Needs(GroupManage)),
    (
        DELETE,
        "/users/:username/identities/:provider_id",
        Needs(UserUpdate),
    ),
    (GET, "/identity-providers", Needs(IdentityProviderManage)),
    (POST, "/identity-providers", Needs(IdentityProviderManage)),
    (
        GET,
        "/identity-providers/:id",
        Needs(IdentityProviderManage),
    ),
    (
        PUT,
        "/identity-providers/:id",
        Needs(IdentityProviderManage),
    ),
    (
        DELETE,
        "/identity-providers/:id",
        Needs(IdentityProviderManage),
    ),
    (
        POST,
        "/identity-providers/:id/check",
        Needs(IdentityProviderManage),
    ),
    (
        POST,
        "/identity-providers/:id/sp-key",
        Needs(IdentityProviderManage),
    ),
    (
        POST,
        "/identity-providers/:id/refresh-metadata",
        Needs(IdentityProviderManage),
    ),
    (PUT, "/users/:username", Needs(UserUpdate)),
    (DELETE, "/users/:username", Needs(UserDelete)),
    (PUT, "/users/:username/password", Needs(UserSetPassword)),
    (POST, "/users/:username/unlock", Needs(UserUpdate)),
    (POST, "/users/:username/restore", Needs(UserDelete)),
    (POST, "/users/:username/purge", Needs(UserDelete)),
    (
        PUT,
        "/users/:username/digest-password",
        Needs(UserSetPassword),
    ),
    (
        DELETE,
        "/users/:username/digest-password",
        Needs(UserSetPassword),
    ),
    (GET, "/users/:username/calling", Needs(UserRead)),
    (PUT, "/users/:username/calling", Needs(UserUpdate)),
    (GET, "/users/:username/presence", Needs(UserRead)),
    (GET, "/users/:username/devices", Needs(DeviceRead)),
    // Registrations.
    (GET, "/registrations", Needs(RegistrationView)),
    (GET, "/registrations/:aor", Needs(RegistrationView)),
    (DELETE, "/registrations/:aor", Needs(RegistrationManage)),
    (
        DELETE,
        "/registrations/:aor/:contact",
        Needs(RegistrationManage),
    ),
    // Calls.
    (GET, "/calls", Needs(CallView)),
    (GET, "/calls/:call_id", Needs(CallView)),
    (DELETE, "/calls/:call_id", Needs(CallControl)),
    (POST, "/calls/:call_id/recording/start", Needs(CallControl)),
    (POST, "/calls/:call_id/recording/stop", Needs(CallControl)),
    (POST, "/calls/:call_id/hold", Needs(CallControl)),
    (POST, "/calls/:call_id/resume", Needs(CallControl)),
    (POST, "/calls/:call_id/transfer", Needs(CallControl)),
    (POST, "/calls/:call_id/redirect", Needs(CallControl)),
    (POST, "/calls/:call_id/play", Needs(CallControl)),
    (POST, "/calls/:call_id/say", Needs(CallControl)),
    (POST, "/calls/:call_id/hangup", Needs(CallControl)),
    (POST, "/calls/:call_id/monitor", Needs(CallSupervise)),
    (POST, "/calls/:call_id/whisper", Needs(CallSupervise)),
    (POST, "/calls/:call_id/barge", Needs(CallSupervise)),
    (GET, "/calls/:call_id/supervisor", Needs(CallSupervise)),
    (DELETE, "/calls/:call_id/supervisor", Needs(CallSupervise)),
    (GET, "/calls/:call_id/sip-messages", Needs(DebugView)),
    // Compliance.
    (GET, "/compliance/policy", Needs(ComplianceManage)),
    (PUT, "/compliance/policy", Needs(ComplianceManage)),
    (POST, "/compliance/retention/run", Needs(ComplianceManage)),
    (GET, "/compliance/retention/runs", Needs(ComplianceManage)),
    (GET, "/audit-log", Needs(AuditView)),
    (GET, "/audit-log/export", Needs(AuditView)),
    (GET, "/compliance/deletions", Needs(AuditView)),
    (GET, "/compliance/holds", Needs(LegalHoldManage)),
    (POST, "/compliance/holds", Needs(LegalHoldManage)),
    (GET, "/compliance/holds/:id", Needs(LegalHoldManage)),
    (
        POST,
        "/compliance/holds/:id/release",
        Needs(LegalHoldManage),
    ),
    (
        GET,
        "/compliance/subject-requests",
        Needs(SubjectRequestManage),
    ),
    (
        POST,
        "/compliance/subject-requests",
        Needs(SubjectRequestManage),
    ),
    (
        GET,
        "/compliance/subject-requests/:id",
        Needs(SubjectRequestManage),
    ),
    (
        GET,
        "/compliance/subject-requests/:id/bundle",
        Needs(SubjectRequestManage),
    ),
    (
        POST,
        "/compliance/subject-requests/:id/confirm",
        Needs(SubjectRequestManage),
    ),
    // Recordings and call records.
    (GET, "/recordings", Needs(CallRecordingAccess)),
    (GET, "/recordings/:id", Needs(CallRecordingAccess)),
    (DELETE, "/recordings/:id", Needs(CallRecordingAccess)),
    (GET, "/recordings/:id/download", Needs(CallRecordingAccess)),
    (GET, "/cdrs", Needs(CdrRead)),
    (GET, "/cdrs/export", Needs(CdrRead)),
    (GET, "/cdrs/:call_id", Needs(CdrRead)),
    (GET, "/cdrs/:call_id/quality", Needs(CdrRead)),
    // Flows.
    (GET, "/flow-traces", Needs(FlowRead)),
    (GET, "/flow-traces/:call_id", Needs(FlowRead)),
    (GET, "/flows", Needs(FlowRead)),
    (POST, "/flows", Needs(FlowManage)),
    (GET, "/flows/:id", Needs(FlowRead)),
    (PUT, "/flows/:id", Needs(FlowManage)),
    (DELETE, "/flows/:id", Needs(FlowManage)),
    (GET, "/flows/:id/versions", Needs(FlowRead)),
    (GET, "/flows/:id/versions/:version", Needs(FlowRead)),
    // Numbers, lines and extensions.
    (GET, "/phone-numbers", Needs(PhoneNumberRead)),
    (POST, "/phone-numbers", Needs(PhoneNumberCreate)),
    (GET, "/phone-numbers/available", Needs(PhoneNumberRead)),
    (GET, "/phone-numbers/:id", Needs(PhoneNumberRead)),
    (PATCH, "/phone-numbers/:id", Needs(PhoneNumberUpdate)),
    (DELETE, "/phone-numbers/:id", Needs(PhoneNumberDelete)),
    (
        POST,
        "/phone-numbers/:id/activate",
        Needs(PhoneNumberUpdate),
    ),
    (
        POST,
        "/phone-numbers/:id/purchase",
        Needs(PhoneNumberCreate),
    ),
    (POST, "/phone-numbers/:id/release", Needs(PhoneNumberDelete)),
    (GET, "/line-bindings", Needs(PhoneNumberRead)),
    (POST, "/line-bindings", Needs(PhoneNumberUpdate)),
    (GET, "/line-bindings/:id", Needs(PhoneNumberRead)),
    (PATCH, "/line-bindings/:id", Needs(PhoneNumberUpdate)),
    (DELETE, "/line-bindings/:id", Needs(PhoneNumberUpdate)),
    (GET, "/extensions", Needs(ExtensionRead)),
    (POST, "/extensions", Needs(ExtensionManage)),
    (GET, "/extensions/:number", Needs(ExtensionRead)),
    (PATCH, "/extensions/:number", Needs(ExtensionManage)),
    (DELETE, "/extensions/:number", Needs(ExtensionManage)),
    // Devices, hoteling and provisioning templates.
    (GET, "/devices", Needs(DeviceRead)),
    (POST, "/devices", Needs(DeviceCreate)),
    (GET, "/devices/:id", Needs(DeviceRead)),
    (PATCH, "/devices/:id", Needs(DeviceUpdate)),
    (DELETE, "/devices/:id", Needs(DeviceDelete)),
    (GET, "/devices/:id/fetches", Needs(DeviceRead)),
    (POST, "/devices/:id/render", Needs(DeviceUpdate)),
    (POST, "/devices/:id/resync", Needs(DeviceUpdate)),
    (POST, "/devices/:id/push-test", Needs(DeviceUpdate)),
    (POST, "/devices/import", Needs(DeviceCreate)),
    (GET, "/captures", Needs(DeviceRead)),
    (GET, "/captures/allows", Needs(DeviceRead)),
    (POST, "/captures/allows", Needs(DeviceUpdate)),
    (DELETE, "/captures/allows/:id", Needs(DeviceUpdate)),
    (DELETE, "/captures/:id", Needs(DeviceDelete)),
    (GET, "/captures/:id/file", Needs(DeviceUpdate)),
    (GET, "/provisioning/key", Needs(DeviceUpdate)),
    (POST, "/provisioning/key", Needs(DeviceUpdate)),
    (GET, "/provisioning/pending", Needs(DeviceRead)),
    (GET, "/firmware", Needs(DeviceRead)),
    (POST, "/firmware", Needs(SystemConfig)),
    (DELETE, "/firmware/:id", Needs(SystemConfig)),
    (GET, "/firmware/targets", Needs(DeviceRead)),
    (POST, "/firmware/targets", Needs(DeviceUpdate)),
    (PUT, "/firmware/targets/:id", Needs(DeviceUpdate)),
    (DELETE, "/firmware/targets/:id", Needs(DeviceUpdate)),
    (GET, "/branding", Needs(BrandingManage)),
    (PUT, "/branding", Needs(BrandingManage)),
    (DELETE, "/branding", Needs(BrandingManage)),
    (GET, "/branding/assets", Needs(BrandingManage)),
    (POST, "/branding/assets", Needs(BrandingManage)),
    (DELETE, "/branding/assets/:sha256", Needs(BrandingManage)),
    (GET, "/branding/deployment-profile", Needs(BrandingManage)),
    (GET, "/provisioning/redirection", Needs(DeviceRead)),
    (
        PUT,
        "/provisioning/redirection/:service",
        Needs(DeviceUpdate),
    ),
    (
        DELETE,
        "/provisioning/redirection/:service",
        Needs(DeviceUpdate),
    ),
    (
        POST,
        "/provisioning/redirection/:service/check",
        Needs(DeviceUpdate),
    ),
    (
        POST,
        "/provisioning/redirection/:service/sync",
        Needs(DeviceUpdate),
    ),
    (GET, "/devices/:id/redirection", Needs(DeviceRead)),
    (POST, "/devices/:id/redirection", Needs(DeviceUpdate)),
    (DELETE, "/provisioning/pending/:mac", Needs(DeviceDelete)),
    (
        POST,
        "/provisioning/pending/:mac/claim",
        Needs(DeviceCreate),
    ),
    (GET, "/hoteling/sessions", Needs(DeviceRead)),
    (DELETE, "/hoteling/sessions/:id", Needs(DeviceUpdate)),
    (GET, "/config-templates", Needs(DeviceRead)),
    (POST, "/config-templates", Needs(DeviceCreate)),
    (GET, "/config-templates/library", Needs(DeviceRead)),
    (POST, "/config-templates/preview", Needs(DeviceRead)),
    (POST, "/config-templates/:id/resync", Needs(DeviceUpdate)),
    (
        POST,
        "/config-templates/library/:vendor",
        Needs(DeviceCreate),
    ),
    (GET, "/config-templates/:id", Needs(DeviceRead)),
    (PATCH, "/config-templates/:id", Needs(DeviceUpdate)),
    (DELETE, "/config-templates/:id", Needs(DeviceDelete)),
    // Locations.
    (GET, "/locations", Needs(LocationRead)),
    (POST, "/locations", Needs(LocationManage)),
    (GET, "/locations/:id", Needs(LocationRead)),
    (PATCH, "/locations/:id", Needs(LocationManage)),
    (DELETE, "/locations/:id", Needs(LocationManage)),
    // Live queues and their agents.
    (GET, "/queues", Needs(QueueRead)),
    (GET, "/queues/stats", Needs(QueueRead)),
    (GET, "/queues/:name", Needs(QueueRead)),
    (GET, "/queues/:name/stats", Needs(QueueRead)),
    (GET, "/queues/:name/agents", Needs(QueueRead)),
    (POST, "/queues/:name/agents", Needs(QueueManage)),
    (DELETE, "/queues/:name/agents/:id", Needs(QueueManage)),
    (PATCH, "/queues/:name/agents/:id", Needs(QueueManage)),
    (POST, "/queues/:name/agents/:id/pause", Needs(QueueManage)),
    (POST, "/queues/:name/agents/:id/resume", Needs(QueueManage)),
    // Presence.
    (GET, "/presence", Needs(PresenceRead)),
    (GET, "/presence/stats", Needs(PresenceRead)),
    (GET, "/presence/:aor", Needs(PresenceRead)),
    (PUT, "/presence/:aor", Needs(PresenceManage)),
    (DELETE, "/presence/:aor", Needs(PresenceManage)),
    (GET, "/presence/:aor/watchers", Needs(PresenceRead)),
    (
        DELETE,
        "/presence/:aor/watchers/:subscription_id",
        Needs(PresenceManage),
    ),
    (GET, "/presence/:aor/policy", Needs(PresenceRead)),
    (PUT, "/presence/:aor/policy", Needs(PresenceManage)),
    (DELETE, "/presence/:aor/policy", Needs(PresenceManage)),
    (GET, "/presence/:aor/pres-rules", Needs(PresenceRead)),
    (PUT, "/presence/:aor/pres-rules", Needs(PresenceManage)),
    // The services' configuration is the platform's, read and changed
    // only by its configuration permission: the nodes seed it with their
    // own credentials (the media server's API tokens, an AI key, a Redis
    // URL), which no tenant's administrator may read.
    (GET, "/config", Needs(SystemConfig)),
    (GET, "/config/audit", Needs(SystemConfig)),
    (GET, "/config/:service", Needs(SystemConfig)),
    (GET, "/config/:service/:section", Needs(SystemConfig)),
    (PUT, "/config/:service/:section", Needs(SystemConfig)),
    (DELETE, "/config/:service/:section", Needs(SystemConfig)),
    (
        POST,
        "/config/:service/:section/validate",
        Needs(SystemConfig),
    ),
    // Prompts.
    (GET, "/prompts", Needs(PromptRead)),
    (POST, "/prompts", Needs(PromptManage)),
    (GET, "/prompts/:id", Needs(PromptRead)),
    (PUT, "/prompts/:id", Needs(PromptManage)),
    (DELETE, "/prompts/:id", Needs(PromptManage)),
    (GET, "/prompts/:id/audio", Needs(PromptRead)),
    (POST, "/prompts/:id/audio", Needs(PromptManage)),
    (POST, "/prompts/:id/synthesize", Needs(PromptManage)),
    // Voicemail.
    (GET, "/voicemail/mailboxes", Needs(VoicemailRead)),
    (POST, "/voicemail/mailboxes", Needs(VoicemailManage)),
    (GET, "/voicemail/mailboxes/:id", Needs(VoicemailRead)),
    (PUT, "/voicemail/mailboxes/:id", Needs(VoicemailManage)),
    (DELETE, "/voicemail/mailboxes/:id", Needs(VoicemailManage)),
    (
        POST,
        "/voicemail/mailboxes/:id/enable",
        Needs(VoicemailManage),
    ),
    (
        POST,
        "/voicemail/mailboxes/:id/disable",
        Needs(VoicemailManage),
    ),
    (GET, "/voicemail/mailboxes/:id/stats", Needs(VoicemailRead)),
    (
        GET,
        "/voicemail/mailboxes/:mailbox_id/messages",
        Needs(VoicemailRead),
    ),
    (GET, "/voicemail/messages/:id", Needs(VoicemailRead)),
    (PUT, "/voicemail/messages/:id", Needs(VoicemailManage)),
    (DELETE, "/voicemail/messages/:id", Needs(VoicemailManage)),
    (
        GET,
        "/voicemail/mailboxes/:id/greetings",
        Needs(VoicemailRead),
    ),
    (
        POST,
        "/voicemail/mailboxes/:id/greetings",
        Needs(VoicemailManage),
    ),
    (
        DELETE,
        "/voicemail/mailboxes/:id/greetings/:greeting_type",
        Needs(VoicemailManage),
    ),
    (
        PUT,
        "/voicemail/mailboxes/:id/greetings/:greeting_type/active",
        Needs(VoicemailManage),
    ),
    (
        GET,
        "/voicemail/mailboxes/:id/greetings/:greeting_type/audio",
        Needs(VoicemailRead),
    ),
    // API tokens.
    (GET, "/api-tokens", Needs(ApiTokenManage)),
    (POST, "/api-tokens", Needs(ApiTokenManage)),
    (GET, "/api-tokens/permissions", Needs(ApiTokenManage)),
    (DELETE, "/api-tokens/:id", Needs(ApiTokenManage)),
    (POST, "/api-tokens/:id/rotate", Needs(ApiTokenManage)),
    // Webhooks.
    (GET, "/webhooks", Needs(WebhookManage)),
    (POST, "/webhooks", Needs(WebhookManage)),
    (GET, "/webhooks/event-types", Needs(WebhookManage)),
    (GET, "/webhooks/:id", Needs(WebhookManage)),
    (PATCH, "/webhooks/:id", Needs(WebhookManage)),
    (DELETE, "/webhooks/:id", Needs(WebhookManage)),
    (POST, "/webhooks/:id/rotate-secret", Needs(WebhookManage)),
    (POST, "/webhooks/:id/test", Needs(WebhookManage)),
    (GET, "/webhooks/:id/deliveries", Needs(WebhookManage)),
    (
        POST,
        "/webhooks/:id/deliveries/:delivery_id/replay",
        Needs(WebhookManage),
    ),
    // Routing: least-cost routes price trunks, so they take the trunk
    // permissions; classes of service and schedules are call policy.
    (GET, "/lcr-routes", Needs(TrunkRead)),
    (POST, "/lcr-routes", Needs(TrunkUpdate)),
    (GET, "/lcr-routes/:id", Needs(TrunkRead)),
    (PATCH, "/lcr-routes/:id", Needs(TrunkUpdate)),
    (DELETE, "/lcr-routes/:id", Needs(TrunkUpdate)),
    (GET, "/routing/preview", Needs(RoutingRuleRead)),
    (GET, "/classes-of-service", Needs(RoutingRuleRead)),
    (POST, "/classes-of-service", Needs(RoutingRuleManage)),
    (GET, "/classes-of-service/classify", Needs(RoutingRuleRead)),
    (GET, "/classes-of-service/:id", Needs(RoutingRuleRead)),
    (PUT, "/classes-of-service/:id", Needs(RoutingRuleManage)),
    (DELETE, "/classes-of-service/:id", Needs(RoutingRuleManage)),
    (GET, "/schedules", Needs(RoutingRuleRead)),
    (POST, "/schedules", Needs(RoutingRuleManage)),
    (GET, "/schedules/:id", Needs(RoutingRuleRead)),
    (PUT, "/schedules/:id", Needs(RoutingRuleManage)),
    (DELETE, "/schedules/:id", Needs(RoutingRuleManage)),
    // Parking lots (docs/PARKING.md).
    (GET, "/parking/lots", Needs(ParkingRead)),
    (POST, "/parking/lots", Needs(ParkingManage)),
    (GET, "/parking/lots/:id", Needs(ParkingRead)),
    (PUT, "/parking/lots/:id", Needs(ParkingManage)),
    (DELETE, "/parking/lots/:id", Needs(ParkingManage)),
    (GET, "/parking/calls", Needs(ParkingRead)),
    (POST, "/parking/calls/:slot/retrieve", Needs(ParkingManage)),
    (DELETE, "/parking/calls/:slot", Needs(ParkingManage)),
    (GET, "/routing/rules", Needs(RoutingRuleRead)),
    (POST, "/routing/rules", Needs(RoutingRuleManage)),
    (GET, "/routing/rules/:id", Needs(RoutingRuleRead)),
    (PATCH, "/routing/rules/:id", Needs(RoutingRuleManage)),
    (DELETE, "/routing/rules/:id", Needs(RoutingRuleManage)),
    // Queue configuration.
    (GET, "/queue-configs", Needs(QueueRead)),
    (POST, "/queue-configs", Needs(QueueManage)),
    (GET, "/queue-configs/:id", Needs(QueueRead)),
    (PATCH, "/queue-configs/:id", Needs(QueueManage)),
    (DELETE, "/queue-configs/:id", Needs(QueueManage)),
    // Trunks.
    (GET, "/trunks", Needs(TrunkRead)),
    (POST, "/trunks", Needs(TrunkCreate)),
    (GET, "/trunks/:id", Needs(TrunkRead)),
    (PATCH, "/trunks/:id", Needs(TrunkUpdate)),
    (DELETE, "/trunks/:id", Needs(TrunkDelete)),
    (POST, "/trunks/:id/test", Needs(TrunkUpdate)),
    (POST, "/trunks/:id/clone", Needs(TrunkCreate)),
    (GET, "/trunks/:id/stats", Needs(TrunkRead)),
    // Conferences.
    (GET, "/conferences/meetings", Needs(ConferenceRead)),
    (POST, "/conferences/meetings", Needs(ConferenceManage)),
    (GET, "/conferences/meetings/:id", Needs(ConferenceRead)),
    (PATCH, "/conferences/meetings/:id", Needs(ConferenceManage)),
    (DELETE, "/conferences/meetings/:id", Needs(ConferenceManage)),
    (
        POST,
        "/conferences/meetings/:id/enable",
        Needs(ConferenceManage),
    ),
    (
        POST,
        "/conferences/meetings/:id/disable",
        Needs(ConferenceManage),
    ),
    (
        POST,
        "/conferences/meetings/:id/join-tokens",
        Needs(ConferenceManage),
    ),
    (GET, "/conferences/live/:room/video", Needs(ConferenceRead)),
    (
        PUT,
        "/conferences/live/:room/video/layout",
        Needs(ConferenceManage),
    ),
    (
        POST,
        "/conferences/live/:room/video/pin/:pid",
        Needs(ConferenceManage),
    ),
    (
        DELETE,
        "/conferences/live/:room/video/pin",
        Needs(ConferenceManage),
    ),
    (
        POST,
        "/conferences/live/:room/video/spotlight/:pid",
        Needs(ConferenceManage),
    ),
    (
        DELETE,
        "/conferences/live/:room/video/spotlight",
        Needs(ConferenceManage),
    ),
    (
        POST,
        "/conferences/live/:room/participants/:pid/video/:state",
        Needs(ConferenceManage),
    ),
    (
        POST,
        "/conferences/live/:room/video/content/stop",
        Needs(ConferenceManage),
    ),
    (
        PUT,
        "/conferences/live/:room/video/content-layout",
        Needs(ConferenceManage),
    ),
    (
        POST,
        "/conferences/live/:room/dial-out",
        Needs(ConferenceManage),
    ),
    // fcp-admin's cluster view (`cluster_api.rs`, merged into the nest).
    (GET, "/cluster/nodes", Needs(SystemViewLogs)),
    (GET, "/cluster/stats", Needs(SystemViewLogs)),
    (GET, "/cluster/sites", Needs(SiteRead)),
    // fcp-admin's services (`services.rs`, docs/SERVICES.md), behind the
    // platform administrator's gate as well.
    (GET, "/system/services", Needs(SystemViewLogs)),
    (GET, "/system/services/summary", Needs(SystemViewLogs)),
    (GET, "/system/services/:id", Needs(SystemViewLogs)),
    (DELETE, "/system/services/:id", Needs(SystemConfig)),
];

/// The gate for a route, by method and path template relative to
/// `/api/v1`.
pub fn gate(method: &Method, path: &str) -> Option<Gate> {
    ROUTES
        .iter()
        .find(|(m, p, _)| *m == method.as_str() && *p == path)
        .map(|(_, _, gate)| *gate)
}

fn refuse(message: &str) -> Response {
    crate::error::ApiError::Forbidden(message.to_string()).into_response()
}

/// Check the caller against the matched route's gate. A caller without an
/// identity is let through (the bearer layer, not this one, decides who is
/// authenticated), as is a request that matched no route (it is a 404).
pub async fn enforce(State(state): State<ApiState>, req: Request, next: Next) -> Response {
    let Some(identity) = req.extensions().get::<AuthIdentity>().cloned() else {
        return next.run(req).await;
    };
    let Some(matched) = req.extensions().get::<MatchedPath>() else {
        return next.run(req).await;
    };
    let path = matched
        .as_str()
        .strip_prefix("/api/v1")
        .unwrap_or(matched.as_str());
    let gate = gate(req.method(), path);

    // A personal token reaches its maker's own routes and nothing else.
    if let AuthIdentity::Personal { .. } = identity {
        return if gate == Some(Open) && path.starts_with("/me/") {
            next.run(req).await
        } else {
            refuse("A personal token reaches only /me routes")
        };
    }

    match gate {
        Some(Open) => next.run(req).await,
        Some(AnyAdmin) => {
            if identity.is_admin() {
                next.run(req).await
            } else {
                refuse("Admin access required")
            }
        }
        Some(Needs(permission)) => {
            match check_permission(&identity, permission, state.deployment_mode) {
                Ok(()) => next.run(req).await,
                Err(e) => e.into_response(),
            }
        }
        None => {
            if matches!(identity, AuthIdentity::StaticToken) {
                return next.run(req).await;
            }
            tracing::error!(
                method = %req.method(),
                path,
                "Route has no permission in route_permissions::ROUTES; refused"
            );
            refuse("This route names no permission")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// The `(method, path)` pairs a router source file binds: each
    /// `.route("path", …)` call's method routers (`get(…)`, `post(…)`, …,
    /// also as `axum::routing::delete(…)`).
    fn bound_routes(source: &str) -> Vec<(String, String)> {
        let mut out = Vec::new();
        let mut rest = source;
        while let Some(at) = rest.find(".route(") {
            rest = &rest[at + ".route(".len()..];
            // The call's argument list, up to its closing parenthesis.
            let mut depth = 1usize;
            let mut end = 0;
            for (i, c) in rest.char_indices() {
                match c {
                    '(' => depth += 1,
                    ')' => {
                        depth -= 1;
                        if depth == 0 {
                            end = i;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            let args = &rest[..end];
            let open = args.find('"').expect("route path");
            let close = open + 1 + args[open + 1..].find('"').expect("route path end");
            let path = args[open + 1..close].to_string();
            let handlers = &args[close + 1..];
            for (method, call) in [
                ("GET", "get("),
                ("POST", "post("),
                ("PUT", "put("),
                ("PATCH", "patch("),
                ("DELETE", "delete("),
            ] {
                let mut from = 0;
                while let Some(i) = handlers[from..].find(call) {
                    let pos = from + i;
                    let before = handlers[..pos].chars().last();
                    // `get(` but not `…_get(` or `forget(`.
                    if !before.is_some_and(|c| c.is_alphanumeric() || c == '_') {
                        out.push((method.to_string(), path.clone()));
                    }
                    from = pos + call.len();
                }
            }
            rest = &rest[end..];
        }
        out
    }

    #[test]
    fn every_route_names_a_gate() {
        let v1 = include_str!("v1/mod.rs");
        // The user and admin routes; the pre-auth and WebSocket routers
        // come before them and are not behind the bearer layer.
        let start = v1.find("let user_routes").expect("user routes");
        let mut bound = bound_routes(&v1[start..]);
        bound.extend(bound_routes(include_str!(
            "../../fcp-admin/src/cluster_api.rs"
        )));
        bound.extend(bound_routes(include_str!(
            "../../fcp-admin/src/services.rs"
        )));
        assert!(bound.len() > 250, "parsed only {} routes", bound.len());

        let mut missing = Vec::new();
        for (method, path) in &bound {
            let method: Method = method.parse().unwrap();
            if gate(&method, path).is_none() {
                missing.push(format!("{method} {path}"));
            }
        }
        assert!(missing.is_empty(), "routes with no gate: {missing:#?}");

        // And the table names nothing the router does not bind.
        let bound: HashSet<(String, String)> = bound.into_iter().collect();
        let stale: Vec<_> = ROUTES
            .iter()
            .filter(|(m, p, _)| !bound.contains(&(m.to_string(), p.to_string())))
            .collect();
        assert!(stale.is_empty(), "gates for routes not bound: {stale:#?}");
    }

    #[test]
    fn every_route_is_listed_once() {
        let mut seen = HashSet::new();
        for (m, p, _) in ROUTES {
            assert!(seen.insert((m, p)), "{m} {p} listed twice");
        }
    }

    #[test]
    fn only_self_service_and_the_session_are_open() {
        for (m, p, g) in ROUTES {
            if *g == Open {
                assert!(
                    p.starts_with("/me/") || p.starts_with("/auth/"),
                    "{m} {p} is open"
                );
            }
        }
    }
}
