//! SCIM 2.0 (RFC 7643/7644): the tenant's provisioner — Entra ID, Okta —
//! makes, changes and removes users at `/scim/v2` (`docs/SCIM.md`,
//! `docs/IDENTITY_AND_DEVICES.md` phase 6).
//!
//! A request carries an API token holding `scim_provision`; the tenant is
//! the token's. A user is the SCIM resource of the same id. A PATCH is
//! applied to the user's SCIM document and the result taken as a replace,
//! so a patch and a replace mean the same thing whichever shape a
//! provisioner sends. A delete deprovisions (`docs/USERS.md` §4).

use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::{
    body::Bytes,
    extract::{Path, Query, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Extension, Router,
};
use fcp_core::permissions::Permission;
use fcp_core::{AccountStatus, CredentialStore, User, UserId, UserRole};
use fcp_events::{PlatformEvent, SystemEvent, UserEvent};
use serde::Deserialize;
use serde_json::{json, Map, Value};

use crate::auth::{AuthIdentity, TenantContext};
use crate::error::ApiError;
use crate::state::ApiState;

pub const USER_SCHEMA: &str = "urn:ietf:params:scim:schemas:core:2.0:User";
pub const ENTERPRISE_SCHEMA: &str = "urn:ietf:params:scim:schemas:extension:enterprise:2.0:User";
/// FCP's own attributes of a user: extension, role, location.
pub const FCP_USER_SCHEMA: &str = "urn:ietf:params:scim:schemas:extension:fcp:2.0:User";
pub const GROUP_SCHEMA: &str = "urn:ietf:params:scim:schemas:core:2.0:Group";
/// FCP's own attribute of a group: the role its members hold.
pub const FCP_GROUP_SCHEMA: &str = "urn:ietf:params:scim:schemas:extension:fcp:2.0:Group";
const LIST_SCHEMA: &str = "urn:ietf:params:scim:api:messages:2.0:ListResponse";
const PATCH_SCHEMA: &str = "urn:ietf:params:scim:api:messages:2.0:PatchOp";
const ERROR_SCHEMA: &str = "urn:ietf:params:scim:api:messages:2.0:Error";
const SCIM_JSON: &str = "application/scim+json";
/// The most resources one page lists.
const MAX_COUNT: usize = 1000;

/// The `/scim/v2` routes, behind the bearer layer.
pub fn router() -> Router<ApiState> {
    Router::new()
        .route("/Users", get(list_users).post(create_user))
        .route(
            "/Users/:id",
            get(get_user)
                .put(replace_user)
                .patch(patch_user)
                .delete(delete_user),
        )
        .route("/Groups", get(list_groups).post(create_group))
        .route(
            "/Groups/:id",
            get(get_group)
                .put(replace_group)
                .patch(patch_group)
                .delete(delete_group),
        )
        .route("/ServiceProviderConfig", get(service_provider_config))
        .route("/ResourceTypes", get(resource_types))
        .route("/Schemas", get(schemas))
}

// ---- Errors and responses ----

/// A SCIM error (RFC 7644 §3.12).
#[derive(Debug)]
pub struct ScimError {
    status: StatusCode,
    scim_type: Option<&'static str>,
    detail: String,
}

impl ScimError {
    fn new(status: StatusCode, scim_type: Option<&'static str>, detail: impl Into<String>) -> Self {
        Self {
            status,
            scim_type,
            detail: detail.into(),
        }
    }
    fn bad(scim_type: &'static str, detail: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, Some(scim_type), detail)
    }
    fn not_found(detail: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, None, detail)
    }
    fn uniqueness(detail: impl Into<String>) -> Self {
        Self::new(StatusCode::CONFLICT, Some("uniqueness"), detail)
    }
}

impl From<ApiError> for ScimError {
    fn from(e: ApiError) -> Self {
        let status = e.status();
        let scim_type = match status {
            StatusCode::CONFLICT => Some("uniqueness"),
            StatusCode::BAD_REQUEST => Some("invalidValue"),
            _ => None,
        };
        let detail = match e {
            ApiError::NotFound(d)
            | ApiError::BadRequest(d)
            | ApiError::Unauthorized(d)
            | ApiError::Forbidden(d)
            | ApiError::Internal(d)
            | ApiError::Conflict(d)
            | ApiError::ServiceUnavailable(d)
            | ApiError::Gone(d) => d,
        };
        Self::new(status, scim_type, detail)
    }
}

impl From<anyhow::Error> for ScimError {
    fn from(e: anyhow::Error) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, None, e.to_string())
    }
}

impl IntoResponse for ScimError {
    fn into_response(self) -> Response {
        let mut body = json!({
            "schemas": [ERROR_SCHEMA],
            "status": self.status.as_u16().to_string(),
            "detail": self.detail,
        });
        if let Some(t) = self.scim_type {
            body["scimType"] = json!(t);
        }
        scim_json(self.status, &body, None)
    }
}

fn scim_json(status: StatusCode, body: &Value, etag: Option<&str>) -> Response {
    let mut response = (status, serde_json::to_vec(body).unwrap_or_default()).into_response();
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(SCIM_JSON));
    if let Some(etag) = etag.and_then(|e| HeaderValue::from_str(e).ok()) {
        headers.insert(header::ETAG, etag);
    }
    response
}

type ScimResult = Result<Response, ScimError>;

// ---- Who is asking ----

/// The provisioner's tenant, once its token is checked: `scim_provision`,
/// and not at a site — SCIM goes to the cloud, and users reach a site
/// through its link (`docs/IDENTITY_AND_DEVICES.md` Q11).
fn provisioner(
    state: &ApiState,
    identity: &AuthIdentity,
    tenant: &TenantContext,
) -> Result<fcp_core::TenantId, ScimError> {
    crate::auth::check_permission(identity, Permission::ScimProvision, state.deployment_mode)
        .map_err(|_| {
            ScimError::new(
                StatusCode::FORBIDDEN,
                None,
                "This token does not hold scim_provision",
            )
        })?;
    if state.managed_by.is_some() {
        return Err(ScimError::new(
            StatusCode::FORBIDDEN,
            None,
            "SCIM provisions the cloud; a site's users come from it",
        ));
    }
    Ok(tenant.0)
}

fn storage(state: &ApiState) -> Result<&fcp_core::DynStorage, ScimError> {
    state.storage.as_deref().ok_or_else(|| {
        ScimError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            None,
            "Storage is not configured",
        )
    })
}

fn parse_body(body: &Bytes) -> Result<Value, ScimError> {
    serde_json::from_slice(body).map_err(|e| ScimError::bad("invalidSyntax", e.to_string()))
}

// ---- The user resource ----

fn millis(t: SystemTime) -> u128 {
    t.duration_since(UNIX_EPOCH).unwrap_or_default().as_millis()
}

fn rfc3339(t: SystemTime) -> String {
    chrono::DateTime::<chrono::Utc>::from(t).to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// A user's version: changes whenever they do.
fn version(user: &User) -> String {
    format!("W/\"{}\"", millis(user.updated_at))
}

fn role_str(role: UserRole) -> String {
    crate::v1::auth_session::role_to_string(&role)
}

/// The user as a SCIM resource.
pub fn user_resource(user: &User) -> Value {
    let mut r = json!({
        "schemas": [USER_SCHEMA, ENTERPRISE_SCHEMA, FCP_USER_SCHEMA],
        "id": user.id.to_string(),
        "userName": user.username,
        "active": !user.is_suspended(),
        "meta": {
            "resourceType": "User",
            "created": rfc3339(user.created_at),
            "lastModified": rfc3339(user.updated_at),
            "location": format!("/scim/v2/Users/{}", user.id),
            "version": version(user),
        },
        ENTERPRISE_SCHEMA: {},
        FCP_USER_SCHEMA: {
            "role": role_str(user.role),
            "sipUsername": user.sip_name(),
        },
    });
    if let Some(id) = &user.scim_external_id {
        r["externalId"] = json!(id);
    }
    if let Some(name) = &user.display_name {
        r["displayName"] = json!(name);
        r["name"] = json!({ "formatted": name });
    }
    if let Some(email) = &user.email {
        r["emails"] = json!([{ "value": email, "type": "work", "primary": true }]);
    }
    let mut phones = Vec::new();
    if let Some(p) = &user.phone_number {
        phones.push(json!({ "value": p, "type": "work" }));
    }
    if let Some(p) = &user.mobile_number {
        phones.push(json!({ "value": p, "type": "mobile" }));
    }
    if !phones.is_empty() {
        r["phoneNumbers"] = Value::Array(phones);
    }
    if let Some(t) = &user.job_title {
        r["title"] = json!(t);
    }
    if let Some(d) = &user.department {
        r[ENTERPRISE_SCHEMA]["department"] = json!(d);
    }
    if let Some(e) = &user.extension {
        r[FCP_USER_SCHEMA]["extension"] = json!(e);
    }
    if let Some(l) = &user.location_id {
        r[FCP_USER_SCHEMA]["locationId"] = json!(l.to_string());
    }
    r
}

fn str_of(v: Option<&Value>) -> Option<String> {
    match v? {
        Value::String(s) => Some(s.trim().to_string()).filter(|s| !s.is_empty()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// A boolean as provisioners send one: `true`, or Entra ID's `"True"`.
fn bool_of(v: &Value) -> Option<bool> {
    match v {
        Value::Bool(b) => Some(*b),
        Value::String(s) if s.eq_ignore_ascii_case("true") => Some(true),
        Value::String(s) if s.eq_ignore_ascii_case("false") => Some(false),
        _ => None,
    }
}

/// The value of the multi-valued `attr` entry of `kind` (`work`, `mobile`),
/// else the primary one, else the first.
fn multi_value(r: &Value, attr: &str, kind: Option<&str>) -> Option<String> {
    let items = r.get(attr)?.as_array()?;
    let typed = |t: &str| {
        items
            .iter()
            .find(|i| i.get("type").and_then(Value::as_str) == Some(t))
    };
    let pick = match kind {
        Some(k) => typed(k),
        None => items
            .iter()
            .find(|i| i.get("primary").and_then(bool_of) == Some(true))
            .or_else(|| typed("work"))
            .or_else(|| items.first()),
    };
    str_of(pick?.get("value"))
}

/// What a SCIM user document says, for FCP's fields.
#[derive(Debug, Default)]
struct Described {
    username: String,
    external_id: Option<String>,
    active: bool,
    display_name: Option<String>,
    email: Option<String>,
    phone: Option<String>,
    mobile: Option<String>,
    title: Option<String>,
    /// `None`: the enterprise extension was not sent, so not changed.
    department: Option<Option<String>>,
    /// `None`: FCP's extension was not sent, so not changed.
    fcp: Option<FcpAttributes>,
}

#[derive(Debug, Default)]
struct FcpAttributes {
    extension: Option<String>,
    role: Option<UserRole>,
    location_id: Option<fcp_core::LocationId>,
}

fn describe(r: &Value) -> Result<Described, ScimError> {
    let username = str_of(r.get("userName"))
        .ok_or_else(|| ScimError::bad("invalidValue", "userName is required"))?;
    let name = r.get("name");
    let display_name = str_of(r.get("displayName"))
        .or_else(|| str_of(name.and_then(|n| n.get("formatted"))))
        .or_else(|| {
            let given = str_of(name.and_then(|n| n.get("givenName")));
            let family = str_of(name.and_then(|n| n.get("familyName")));
            match (given, family) {
                (Some(g), Some(f)) => Some(format!("{g} {f}")),
                (g, f) => g.or(f),
            }
        });
    let active = match r.get("active") {
        None | Some(Value::Null) => true,
        Some(v) => {
            bool_of(v).ok_or_else(|| ScimError::bad("invalidValue", "active must be a boolean"))?
        }
    };
    let department = r
        .get(ENTERPRISE_SCHEMA)
        .map(|e| str_of(e.get("department")));
    let fcp = match r.get(FCP_USER_SCHEMA) {
        None | Some(Value::Null) => None,
        Some(ext) => Some(FcpAttributes {
            extension: str_of(ext.get("extension")),
            role: match str_of(ext.get("role")) {
                None => None,
                Some(role) => {
                    let role = UserRole::from_str(&role).map_err(|_| {
                        ScimError::bad("invalidValue", format!("Unknown role '{role}'"))
                    })?;
                    if role == UserRole::SuperAdmin {
                        return Err(ScimError::bad(
                            "invalidValue",
                            "A provisioner cannot make a platform admin",
                        ));
                    }
                    Some(role)
                }
            },
            location_id: match str_of(ext.get("locationId")) {
                None => None,
                Some(l) => Some(l.parse().map_err(|_| {
                    ScimError::bad("invalidValue", format!("Invalid locationId '{l}'"))
                })?),
            },
        }),
    };
    Ok(Described {
        username,
        external_id: str_of(r.get("externalId")),
        active,
        display_name,
        email: multi_value(r, "emails", None),
        phone: multi_value(r, "phoneNumbers", Some("work")),
        mobile: multi_value(r, "phoneNumbers", Some("mobile")),
        title: str_of(r.get("title")),
        department,
        fcp,
    })
}

/// Write what a document says onto a user (not the name or the status,
/// which have their own paths).
fn apply(user: &mut User, d: &Described) {
    user.scim_external_id = d.external_id.clone();
    user.display_name = d.display_name.clone();
    user.email = d.email.clone();
    user.phone_number = d.phone.clone();
    user.mobile_number = d.mobile.clone();
    user.job_title = d.title.clone();
    if let Some(dept) = &d.department {
        user.department = dept.clone();
    }
    if let Some(fcp) = &d.fcp {
        user.extension = fcp.extension.clone();
        if let Some(role) = fcp.role {
            user.role = role;
        }
        user.location_id = fcp.location_id;
    }
}

async fn ensure_external_id_free(
    storage: &fcp_core::DynStorage,
    tenant: &fcp_core::TenantId,
    external_id: Option<&str>,
    except: Option<&UserId>,
) -> Result<(), ScimError> {
    let Some(external_id) = external_id else {
        return Ok(());
    };
    let found = storage
        .list_users_in_tenant(
            tenant,
            &fcp_core::storage::UserFilter {
                external_id: Some(external_id.to_string()),
                ..Default::default()
            },
        )
        .await?;
    if found.iter().any(|u| Some(&u.id) != except) {
        return Err(ScimError::uniqueness(format!(
            "externalId '{external_id}' is another user's"
        )));
    }
    Ok(())
}

async fn announce_change(state: &ApiState, before: &User, after: &User) {
    crate::v1::users::status_changed(state, before, after).await;
    let _ = state
        .event_bus
        .publish(PlatformEvent::User(UserEvent::Updated {
            user_id: after.id.to_string(),
            username: after.username.clone(),
            tenant_id: after.tenant_id,
            changed_fields: crate::v1::users::changed_fields(before, after),
            timestamp: SystemTime::now(),
        }))
        .await;
    let _ = state
        .event_bus
        .publish(PlatformEvent::System(SystemEvent::ConfigChanged {
            entity_type: "user".to_string(),
            entity_id: after.id.to_string(),
            tenant_id: after.tenant_id,
            timestamp: SystemTime::now(),
        }))
        .await;
}

/// A user of the tenant by SCIM id; a deleted one is gone to SCIM.
async fn found(
    storage: &fcp_core::DynStorage,
    tenant: &fcp_core::TenantId,
    id: &str,
) -> Result<User, ScimError> {
    let gone = || ScimError::not_found(format!("User {id} not found"));
    let user_id: UserId = id.parse().map_err(|_| gone())?;
    storage
        .get_user_by_id(&user_id)
        .await?
        .filter(|u| u.tenant_id == *tenant && !u.is_deprovisioned())
        .ok_or_else(gone)
}

/// A request's `If-Match`, when it names another version: 412.
fn check_if_match(headers: &HeaderMap, user: &User) -> Result<(), ScimError> {
    let Some(wanted) = headers.get(header::IF_MATCH).and_then(|v| v.to_str().ok()) else {
        return Ok(());
    };
    let current = version(user);
    let matches = wanted.split(',').map(str::trim).any(|w| {
        w == "*" || w == current || w.trim_start_matches("W/") == current.trim_start_matches("W/")
    });
    if matches {
        Ok(())
    } else {
        Err(ScimError::new(
            StatusCode::PRECONDITION_FAILED,
            None,
            format!("The user has changed; its version is {current}"),
        ))
    }
}

fn resource_response(status: StatusCode, user: &User) -> Response {
    let mut response = scim_json(status, &user_resource(user), Some(&version(user)));
    if status == StatusCode::CREATED {
        if let Ok(location) = HeaderValue::from_str(&format!("/scim/v2/Users/{}", user.id)) {
            response.headers_mut().insert(header::LOCATION, location);
        }
    }
    response
}

// ---- Handlers ----

/// POST /scim/v2/Users
async fn create_user(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: TenantContext,
    body: Bytes,
) -> ScimResult {
    let tenant = provisioner(&state, &identity, &tenant)?;
    let storage = storage(&state)?;
    let d = describe(&parse_body(&body)?)?;
    crate::validation::validate_username(&d.username)?;
    crate::v1::users::refuse_id_shaped(&d.username)?;
    if storage
        .get_user_in_tenant(&d.username, &tenant)
        .await?
        .is_some()
        || crate::v1::users::sip_name_taken(storage, &tenant, &d.username, None).await?
    {
        return Err(ScimError::uniqueness(format!(
            "userName '{}' is taken",
            d.username
        )));
    }
    ensure_external_id_free(storage, &tenant, d.external_id.as_deref(), None).await?;
    crate::v1::users::ensure_room_for_a_user(&state, &tenant).await?;

    let role = d.fcp.as_ref().and_then(|f| f.role).unwrap_or_default();
    let mut user = crate::v1::users::user_without_password(
        &state.realm,
        tenant,
        &d.username,
        d.display_name.clone(),
        d.email.clone(),
        role,
    )?;
    apply(&mut user, &d);
    crate::v1::users::check_user_extension(
        storage,
        &user.tenant_id,
        &user.id,
        user.extension.as_deref(),
    )
    .await?;
    user.scim_managed = true;
    if !d.active {
        user.account_status = AccountStatus::Suspended;
        user.enabled = false;
    }
    user.normalize_status();
    storage.put_user(user.clone()).await?;
    crate::v1::users::sync_user_extension(storage, &user, None).await?;
    let _ = state
        .event_bus
        .publish(PlatformEvent::User(UserEvent::Created {
            user_id: user.id.to_string(),
            username: user.username.clone(),
            tenant_id: user.tenant_id,
            timestamp: SystemTime::now(),
        }))
        .await;
    tracing::info!(event = "scim_user_created", username = %user.username, tenant_id = %tenant, "User provisioned by SCIM");
    Ok(resource_response(StatusCode::CREATED, &user))
}

/// GET /scim/v2/Users/:id
async fn get_user(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: TenantContext,
    Path(id): Path<String>,
) -> ScimResult {
    let tenant = provisioner(&state, &identity, &tenant)?;
    let user = found(storage(&state)?, &tenant, &id).await?;
    Ok(resource_response(StatusCode::OK, &user))
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListQuery {
    filter: Option<String>,
    start_index: Option<usize>,
    count: Option<usize>,
    /// `members`, as Entra ID asks for a group without them.
    excluded_attributes: Option<String>,
}

/// One `attribute eq "value"` comparison, the filters provisioners send.
#[derive(Debug, PartialEq, Eq)]
pub enum Filter {
    UserName(String),
    ExternalId(String),
    Email(String),
    Id(String),
    DisplayName(String),
}

/// Parse a SCIM filter: one `eq` comparison on `userName`, `externalId`,
/// `id`, `displayName` or an email (`emails`, `emails.value`,
/// `emails[value eq "…"]`).
pub fn parse_filter(filter: &str) -> Result<Filter, ScimError> {
    let unsupported = || {
        ScimError::bad(
            "invalidFilter",
            format!("Unsupported filter '{filter}': one `eq` on userName, externalId, id, displayName or emails"),
        )
    };
    let f = filter.trim();
    // emails[value eq "x"]: inside the brackets, `value` is compared.
    if let Some(inner) = f
        .strip_prefix("emails[")
        .and_then(|rest| rest.strip_suffix(']'))
    {
        let (attr, value) = split_eq(inner).ok_or_else(unsupported)?;
        return if attr.eq_ignore_ascii_case("value") {
            Ok(Filter::Email(value))
        } else {
            Err(unsupported())
        };
    }
    let (attr, value) = split_eq(f).ok_or_else(unsupported)?;
    match attr.to_ascii_lowercase().as_str() {
        "username" => Ok(Filter::UserName(value)),
        "externalid" => Ok(Filter::ExternalId(value)),
        "id" => Ok(Filter::Id(value)),
        "displayname" => Ok(Filter::DisplayName(value)),
        "emails" | "emails.value" | "value" => Ok(Filter::Email(value)),
        _ => Err(unsupported()),
    }
}

/// `attr eq "value"` (the operator case aside), quotes unescaped.
fn split_eq(f: &str) -> Option<(String, String)> {
    let mut parts = f.trim().splitn(3, char::is_whitespace);
    let attr = parts.next()?.to_string();
    if !parts.next()?.eq_ignore_ascii_case("eq") {
        return None;
    }
    let value = parts.next()?.trim();
    let value = value.strip_prefix('"')?.strip_suffix('"')?;
    Some((attr, value.replace("\\\"", "\"").replace("\\\\", "\\")))
}

/// A page of resources, RFC 7644 §3.4.2.
fn list_response(all: Vec<Value>, q: &ListQuery) -> Response {
    let total = all.len();
    let start = q.start_index.unwrap_or(1).max(1);
    let count = q.count.unwrap_or(100).min(MAX_COUNT);
    let page: Vec<Value> = all.into_iter().skip(start - 1).take(count).collect();
    scim_json(
        StatusCode::OK,
        &json!({
            "schemas": [LIST_SCHEMA],
            "totalResults": total,
            "startIndex": start,
            "itemsPerPage": page.len(),
            "Resources": page,
        }),
        None,
    )
}

/// GET /scim/v2/Users
async fn list_users(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: TenantContext,
    Query(q): Query<ListQuery>,
) -> ScimResult {
    let tenant = provisioner(&state, &identity, &tenant)?;
    let storage = storage(&state)?;
    let mut filter = fcp_core::storage::UserFilter {
        exclude_deprovisioned: true,
        ..Default::default()
    };
    let mut only_id = None;
    match q.filter.as_deref().map(parse_filter).transpose()? {
        None => {}
        Some(Filter::UserName(v)) => filter.username = Some(v),
        Some(Filter::ExternalId(v)) => filter.external_id = Some(v),
        Some(Filter::Email(v)) => filter.email = Some(v),
        Some(Filter::Id(v)) => only_id = Some(v),
        Some(Filter::DisplayName(_)) => {
            return Err(ScimError::bad(
                "invalidFilter",
                "Users are not filtered by displayName",
            ))
        }
    }
    let users = storage.list_users_in_tenant(&tenant, &filter).await?;
    let resources = users
        .iter()
        .filter(|u| only_id.as_deref().is_none_or(|id| u.id.to_string() == id))
        .map(user_resource)
        .collect();
    Ok(list_response(resources, &q))
}

/// Take a whole document as the user now: their name (a rename), status,
/// and the rest.
async fn replace_with(
    state: &ApiState,
    storage: &fcp_core::DynStorage,
    before: User,
    doc: &Value,
) -> Result<User, ScimError> {
    let d = describe(doc)?;
    ensure_external_id_free(
        storage,
        &before.tenant_id,
        d.external_id.as_deref(),
        Some(&before.id),
    )
    .await?;
    let mut user = before.clone();
    if d.username != user.username {
        user = crate::v1::users::rename(state, storage, &user, &d.username).await?;
    }
    apply(&mut user, &d);
    crate::v1::users::check_user_extension(
        storage,
        &user.tenant_id,
        &user.id,
        user.extension.as_deref(),
    )
    .await?;
    match (d.active, user.is_suspended()) {
        (false, false) => {
            user.account_status = AccountStatus::Suspended;
            user.enabled = false;
        }
        (true, true) => {
            user.account_status = AccountStatus::Active;
            user.enabled = true;
        }
        _ => {}
    }
    user.normalize_status();
    user.updated_at = SystemTime::now();
    storage.put_user(user.clone()).await?;
    crate::v1::users::sync_user_extension(storage, &user, before.extension.as_deref()).await?;
    announce_change(state, &before, &user).await;
    Ok(user)
}

/// PUT /scim/v2/Users/:id
async fn replace_user(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: TenantContext,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> ScimResult {
    let tenant = provisioner(&state, &identity, &tenant)?;
    let storage = storage(&state)?;
    let before = found(storage, &tenant, &id).await?;
    check_if_match(&headers, &before)?;
    let user = replace_with(&state, storage, before, &parse_body(&body)?).await?;
    Ok(resource_response(StatusCode::OK, &user))
}

/// PATCH /scim/v2/Users/:id
async fn patch_user(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: TenantContext,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> ScimResult {
    let tenant = provisioner(&state, &identity, &tenant)?;
    let storage = storage(&state)?;
    let before = found(storage, &tenant, &id).await?;
    check_if_match(&headers, &before)?;
    let mut doc = user_resource(&before);
    apply_patch(&mut doc, &parse_body(&body)?)?;
    let user = replace_with(&state, storage, before, &doc).await?;
    Ok(resource_response(StatusCode::OK, &user))
}

/// DELETE /scim/v2/Users/:id — deprovisioned: suspended now, purged after
/// the tenant's grace period (`docs/USERS.md` §4).
async fn delete_user(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: TenantContext,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> ScimResult {
    let tenant = provisioner(&state, &identity, &tenant)?;
    let storage = storage(&state)?;
    let user = found(storage, &tenant, &id).await?;
    check_if_match(&headers, &user)?;
    crate::offboarding::deprovision(&state, user, &identity.actor()).await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

// ---- PATCH ----

/// Apply a PatchOp (RFC 7644 §3.5.2) to a resource document, as Entra ID
/// and Okta send them: `op` in any case, a value object with no path, a
/// path to an attribute, a sub-attribute (`name.givenName`), an extension
/// attribute (`urn:…:User:department`), or a filtered multi-valued one
/// (`emails[type eq "work"].value`).
pub fn apply_patch(doc: &mut Value, patch: &Value) -> Result<(), ScimError> {
    if !patch
        .get("schemas")
        .and_then(Value::as_array)
        .is_some_and(|s| s.iter().any(|v| v == PATCH_SCHEMA))
    {
        return Err(ScimError::bad("invalidSyntax", "Not a PatchOp"));
    }
    let ops = patch
        .get("Operations")
        .and_then(Value::as_array)
        .ok_or_else(|| ScimError::bad("invalidSyntax", "No Operations"))?;
    for op in ops {
        let kind = op
            .get("op")
            .and_then(Value::as_str)
            .map(str::to_ascii_lowercase)
            .ok_or_else(|| ScimError::bad("invalidSyntax", "An operation without op"))?;
        let path = op.get("path").and_then(Value::as_str);
        let value = op.get("value").cloned().unwrap_or(Value::Null);
        match (kind.as_str(), path) {
            ("add" | "replace", None) => {
                let Value::Object(fields) = value else {
                    return Err(ScimError::bad(
                        "invalidValue",
                        "A value without a path is an object",
                    ));
                };
                for (key, v) in fields {
                    set_path(doc, &key, v, &kind)?;
                }
            }
            ("add" | "replace", Some(path)) => set_path(doc, path, value, &kind)?,
            // Entra ID removes members as a value list on the path.
            ("remove", Some(path)) if value.is_array() => {
                let gone: Vec<Value> = value
                    .as_array()
                    .expect("an array")
                    .iter()
                    .filter_map(|v| v.get("value").cloned())
                    .collect();
                let (urn, attr) = split_path(path);
                let map = container(doc, urn)?;
                if let Some(Value::Array(list)) = map.get_mut(attr) {
                    list.retain(|i| !i.get("value").is_some_and(|v| gone.contains(v)));
                }
            }
            ("remove", Some(path)) => remove_path(doc, path)?,
            ("remove", None) => {
                return Err(ScimError::bad("noTarget", "A remove names its path"));
            }
            (other, _) => {
                return Err(ScimError::bad(
                    "invalidSyntax",
                    format!("Unknown op '{other}'"),
                ));
            }
        }
    }
    Ok(())
}

/// Where a path points: the object holding it (an extension's, or the
/// resource's) and the attribute in it.
fn split_path(path: &str) -> (Option<&str>, &str) {
    // `urn:…:User:department` — the extension is everything before the
    // last colon.
    if path.starts_with("urn:") {
        if let Some((urn, attr)) = path.rsplit_once(':') {
            if urn == ENTERPRISE_SCHEMA || urn == FCP_USER_SCHEMA || urn == FCP_GROUP_SCHEMA {
                return (Some(urn), attr);
            }
        }
        return (Some(path), "");
    }
    (None, path)
}

fn container<'a>(
    doc: &'a mut Value,
    urn: Option<&str>,
) -> Result<&'a mut Map<String, Value>, ScimError> {
    let target = match urn {
        Some(urn) => {
            if !doc.get(urn).is_some_and(Value::is_object) {
                doc[urn] = json!({});
            }
            &mut doc[urn]
        }
        None => doc,
    };
    target
        .as_object_mut()
        .ok_or_else(|| ScimError::bad("invalidPath", "Not an object"))
}

fn set_path(doc: &mut Value, path: &str, value: Value, kind: &str) -> Result<(), ScimError> {
    let (urn, attr) = split_path(path);
    // A whole extension, as a value object's key.
    if urn.is_some() && attr.is_empty() {
        let Value::Object(fields) = value else {
            return Err(ScimError::bad(
                "invalidValue",
                "An extension's value is an object",
            ));
        };
        let map = container(doc, urn)?;
        for (k, v) in fields {
            map.insert(k, v);
        }
        return Ok(());
    }
    // emails[type eq "work"].value
    if let Some((attr_name, filter, sub)) = parse_value_path(attr) {
        let map = container(doc, urn)?;
        let items = map
            .entry(attr_name.to_string())
            .or_insert_with(|| Value::Array(Vec::new()));
        if !items.is_array() {
            *items = Value::Array(Vec::new());
        }
        let list = items.as_array_mut().expect("an array");
        let (fattr, fvalue) = filter;
        let pos = list
            .iter()
            .position(|i| i.get(&fattr).and_then(Value::as_str) == Some(fvalue.as_str()));
        let idx = match pos {
            Some(i) => i,
            None => {
                list.push(json!({ fattr.clone(): fvalue }));
                list.len() - 1
            }
        };
        match sub {
            Some(sub) => {
                list[idx][sub] = value;
            }
            None => {
                if let Value::Object(fields) = value {
                    for (k, v) in fields {
                        list[idx][k] = v;
                    }
                }
            }
        }
        return Ok(());
    }
    // name.givenName
    if let Some((parent, child)) = attr.split_once('.') {
        let map = container(doc, urn)?;
        let parent_value = map.entry(parent.to_string()).or_insert_with(|| json!({}));
        if !parent_value.is_object() {
            *parent_value = json!({});
        }
        // A part of the name changed: the formatted one is made again. FCP
        // keeps one display name, so its parts are what it splits into at
        // the first space, where the provisioner has not said them.
        if parent == "name" && child != "formatted" {
            let name = parent_value.as_object_mut().expect("an object");
            if !name.contains_key("givenName") && !name.contains_key("familyName") {
                if let Some(formatted) = name.get("formatted").and_then(Value::as_str) {
                    let (given, family) = formatted
                        .split_once(' ')
                        .map(|(g, f)| (g.to_string(), f.to_string()))
                        .unwrap_or_else(|| (formatted.to_string(), String::new()));
                    name.insert("givenName".into(), json!(given));
                    if !family.is_empty() {
                        name.insert("familyName".into(), json!(family));
                    }
                }
            }
            name.remove("formatted");
            map.remove("displayName");
        }
        let parent_value = map.get_mut(parent).expect("just made");
        parent_value[child] = value;
        return Ok(());
    }
    let map = container(doc, urn)?;
    // `add` to a multi-valued attribute appends; `replace` replaces.
    match (kind, map.get_mut(attr), &value) {
        ("add", Some(Value::Array(existing)), Value::Array(more)) => {
            existing.extend(more.iter().cloned());
        }
        _ => {
            if attr == "name" {
                map.remove("displayName");
            }
            map.insert(attr.to_string(), value);
        }
    }
    Ok(())
}

fn remove_path(doc: &mut Value, path: &str) -> Result<(), ScimError> {
    let (urn, attr) = split_path(path);
    let map = container(doc, urn)?;
    if let Some((attr_name, (fattr, fvalue), sub)) = parse_value_path(attr) {
        if let Some(Value::Array(list)) = map.get_mut(attr_name) {
            match sub {
                Some(sub) => {
                    for item in list.iter_mut() {
                        if item.get(&fattr).and_then(Value::as_str) == Some(fvalue.as_str()) {
                            if let Some(o) = item.as_object_mut() {
                                o.remove(sub);
                            }
                        }
                    }
                    list.retain(|i| i.get("value").is_some());
                }
                None => {
                    list.retain(|i| i.get(&fattr).and_then(Value::as_str) != Some(fvalue.as_str()))
                }
            }
        }
        return Ok(());
    }
    if let Some((parent, child)) = attr.split_once('.') {
        if let Some(Value::Object(p)) = map.get_mut(parent) {
            p.remove(child);
        }
        return Ok(());
    }
    map.remove(attr);
    Ok(())
}

/// `attr[fattr eq "v"].sub` → (`attr`, (`fattr`, `v`), `sub`).
fn parse_value_path(path: &str) -> Option<(&str, (String, String), Option<&str>)> {
    let open = path.find('[')?;
    let close = path.find(']')?;
    let attr = &path[..open];
    let (fattr, fvalue) = split_eq(&path[open + 1..close])?;
    let sub = path[close + 1..].strip_prefix('.');
    Some((attr, (fattr, fvalue), sub))
}

// ---- Groups ----

fn group_version(group: &fcp_core::group::Group) -> String {
    format!("W/\"{}\"", millis(group.updated_at))
}

/// A group as a SCIM resource; its members unless left out.
async fn group_resource(
    storage: &fcp_core::DynStorage,
    group: &fcp_core::group::Group,
    with_members: bool,
) -> Result<Value, ScimError> {
    let mut r = json!({
        "schemas": [GROUP_SCHEMA, FCP_GROUP_SCHEMA],
        "id": group.id.to_string(),
        "displayName": group.name,
        "meta": {
            "resourceType": "Group",
            "created": rfc3339(group.created_at),
            "lastModified": rfc3339(group.updated_at),
            "location": format!("/scim/v2/Groups/{}", group.id),
            "version": group_version(group),
        },
        FCP_GROUP_SCHEMA: {},
    });
    if let Some(id) = &group.scim_external_id {
        r["externalId"] = json!(id);
    }
    if let Some(role) = group.role {
        r[FCP_GROUP_SCHEMA]["role"] = json!(role_str(role));
    }
    if with_members {
        let store = groups(storage)?;
        let mut members = Vec::new();
        for m in store.list_members(&group.id).await? {
            if let Some(user) = storage.get_user_by_id(&m.user_id).await? {
                members.push(json!({
                    "value": user.id.to_string(),
                    "display": user.username,
                    "$ref": format!("/scim/v2/Users/{}", user.id),
                }));
            }
        }
        r["members"] = Value::Array(members);
    }
    Ok(r)
}

fn groups(
    storage: &fcp_core::DynStorage,
) -> Result<&std::sync::Arc<dyn fcp_core::group::GroupStore>, ScimError> {
    storage.group_store().ok_or_else(|| {
        ScimError::new(
            StatusCode::NOT_IMPLEMENTED,
            None,
            "Groups are not kept here",
        )
    })
}

async fn found_group(
    storage: &fcp_core::DynStorage,
    tenant: &fcp_core::TenantId,
    id: &str,
) -> Result<fcp_core::group::Group, ScimError> {
    let gone = || ScimError::not_found(format!("Group {id} not found"));
    let group_id: fcp_core::GroupId = id.parse().map_err(|_| gone())?;
    groups(storage)?
        .get_group(&group_id)
        .await?
        .filter(|g| g.tenant_id == *tenant)
        .ok_or_else(gone)
}

fn check_group_if_match(
    headers: &HeaderMap,
    group: &fcp_core::group::Group,
) -> Result<(), ScimError> {
    let Some(wanted) = headers.get(header::IF_MATCH).and_then(|v| v.to_str().ok()) else {
        return Ok(());
    };
    let current = group_version(group);
    if wanted
        .split(',')
        .map(str::trim)
        .any(|w| w == "*" || w.trim_start_matches("W/") == current.trim_start_matches("W/"))
    {
        Ok(())
    } else {
        Err(ScimError::new(
            StatusCode::PRECONDITION_FAILED,
            None,
            format!("The group has changed; its version is {current}"),
        ))
    }
}

async fn group_response(
    storage: &fcp_core::DynStorage,
    status: StatusCode,
    group: &fcp_core::group::Group,
) -> ScimResult {
    let mut response = scim_json(
        status,
        &group_resource(storage, group, true).await?,
        Some(&group_version(group)),
    );
    if status == StatusCode::CREATED {
        if let Ok(location) = HeaderValue::from_str(&format!("/scim/v2/Groups/{}", group.id)) {
            response.headers_mut().insert(header::LOCATION, location);
        }
    }
    Ok(response)
}

/// What a group document says: its name, the provisioner's id, its role
/// when FCP's extension is sent, and its members (users of the tenant).
struct DescribedGroup {
    name: String,
    external_id: Option<String>,
    role: Option<Option<UserRole>>,
    members: Vec<UserId>,
}

async fn describe_group(
    storage: &fcp_core::DynStorage,
    tenant: &fcp_core::TenantId,
    r: &Value,
) -> Result<DescribedGroup, ScimError> {
    let name = str_of(r.get("displayName"))
        .ok_or_else(|| ScimError::bad("invalidValue", "displayName is required"))?;
    let role =
        match r.get(FCP_GROUP_SCHEMA) {
            None | Some(Value::Null) => None,
            Some(ext) => Some(match str_of(ext.get("role")) {
                None => None,
                Some(role) => Some(UserRole::from_str(&role).map_err(|_| {
                    ScimError::bad("invalidValue", format!("Unknown role '{role}'"))
                })?),
            }),
        };
    let mut members = Vec::new();
    if let Some(list) = r.get("members").and_then(Value::as_array) {
        for m in list {
            let Some(id) = str_of(m.get("value")) else {
                continue;
            };
            let user = match id.parse::<UserId>() {
                Ok(user_id) => storage.get_user_by_id(&user_id).await?,
                Err(_) => None,
            }
            .filter(|u| u.tenant_id == *tenant && !u.is_deprovisioned())
            .ok_or_else(|| {
                ScimError::bad("invalidValue", format!("No user {id} to be a member"))
            })?;
            if !members.contains(&user.id) {
                members.push(user.id);
            }
        }
    }
    Ok(DescribedGroup {
        name,
        external_id: str_of(r.get("externalId")),
        role,
        members,
    })
}

/// Make the group's members exactly `wanted`, announcing each change.
async fn set_members(
    state: &ApiState,
    storage: &fcp_core::DynStorage,
    group: &fcp_core::group::Group,
    wanted: &[UserId],
) -> Result<(), ScimError> {
    let store = groups(storage)?;
    let now: Vec<UserId> = store
        .list_members(&group.id)
        .await?
        .into_iter()
        .map(|m| m.user_id)
        .collect();
    for user_id in wanted.iter().filter(|u| !now.contains(u)) {
        store
            .add_member(&fcp_core::group::GroupMember {
                group_id: group.id,
                user_id: *user_id,
                tenant_id: group.tenant_id,
                added_at: SystemTime::now(),
            })
            .await?;
        if let Some(user) = storage.get_user_by_id(user_id).await? {
            crate::v1::groups::membership_changed(state, &user).await;
        }
    }
    for user_id in now.iter().filter(|u| !wanted.contains(u)) {
        store.remove_member(&group.id, user_id).await?;
        if let Some(user) = storage.get_user_by_id(user_id).await? {
            crate::v1::groups::membership_changed(state, &user).await;
        }
    }
    Ok(())
}

/// Write a group document over `group` (or a new one): checked, stored,
/// its members set.
async fn write_group(
    state: &ApiState,
    storage: &fcp_core::DynStorage,
    mut group: fcp_core::group::Group,
    doc: &Value,
) -> Result<fcp_core::group::Group, ScimError> {
    let d = describe_group(storage, &group.tenant_id, doc).await?;
    let store = groups(storage)?;
    let siblings = store.list_groups(&group.tenant_id).await?;
    if siblings
        .iter()
        .any(|g| g.id != group.id && g.name.eq_ignore_ascii_case(d.name.trim()))
    {
        return Err(ScimError::uniqueness(format!(
            "displayName '{}' is taken",
            d.name
        )));
    }
    if let Some(ext) = &d.external_id {
        if siblings
            .iter()
            .any(|g| g.id != group.id && g.scim_external_id.as_deref() == Some(ext))
        {
            return Err(ScimError::uniqueness(format!(
                "externalId '{ext}' is another group's"
            )));
        }
    }
    group.name = d.name;
    group.scim_external_id = d.external_id;
    if let Some(role) = d.role {
        group.role = role;
    }
    group.updated_at = SystemTime::now();
    let group = group
        .normalized()
        .map_err(|e| ScimError::bad("invalidValue", e))?;
    store.put_group(&group).await?;
    set_members(state, storage, &group, &d.members).await?;
    let _ = state
        .event_bus
        .publish(PlatformEvent::System(SystemEvent::ConfigChanged {
            entity_type: "group".to_string(),
            entity_id: group.id.to_string(),
            tenant_id: group.tenant_id,
            timestamp: SystemTime::now(),
        }))
        .await;
    Ok(group)
}

/// POST /scim/v2/Groups
async fn create_group(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: TenantContext,
    body: Bytes,
) -> ScimResult {
    let tenant = provisioner(&state, &identity, &tenant)?;
    let storage = storage(&state)?;
    let doc = parse_body(&body)?;
    let group = write_group(
        &state,
        storage,
        fcp_core::group::Group::new(tenant, String::new()),
        &doc,
    )
    .await?;
    tracing::info!(event = "scim_group_created", group = %group.name, tenant_id = %tenant, "Group provisioned by SCIM");
    group_response(storage, StatusCode::CREATED, &group).await
}

/// GET /scim/v2/Groups/:id
async fn get_group(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: TenantContext,
    Path(id): Path<String>,
    Query(q): Query<ListQuery>,
) -> ScimResult {
    let tenant = provisioner(&state, &identity, &tenant)?;
    let storage = storage(&state)?;
    let group = found_group(storage, &tenant, &id).await?;
    let with_members = !excludes_members(&q);
    Ok(scim_json(
        StatusCode::OK,
        &group_resource(storage, &group, with_members).await?,
        Some(&group_version(&group)),
    ))
}

fn excludes_members(q: &ListQuery) -> bool {
    q.excluded_attributes.as_deref().is_some_and(|a| {
        a.split(',')
            .any(|x| x.trim().eq_ignore_ascii_case("members"))
    })
}

/// GET /scim/v2/Groups
async fn list_groups(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: TenantContext,
    Query(q): Query<ListQuery>,
) -> ScimResult {
    let tenant = provisioner(&state, &identity, &tenant)?;
    let storage = storage(&state)?;
    let filter = q.filter.as_deref().map(parse_filter).transpose()?;
    let with_members = !excludes_members(&q);
    let mut resources = Vec::new();
    for g in groups(storage)?.list_groups(&tenant).await? {
        let keep = match &filter {
            None => true,
            Some(Filter::DisplayName(v)) => g.name.eq_ignore_ascii_case(v),
            Some(Filter::ExternalId(v)) => g.scim_external_id.as_deref() == Some(v.as_str()),
            Some(Filter::Id(v)) => g.id.to_string() == *v,
            Some(_) => {
                return Err(ScimError::bad(
                    "invalidFilter",
                    "Groups are filtered by displayName, externalId or id",
                ))
            }
        };
        if keep {
            resources.push(group_resource(storage, &g, with_members).await?);
        }
    }
    Ok(list_response(resources, &q))
}

/// PUT /scim/v2/Groups/:id
async fn replace_group(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: TenantContext,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> ScimResult {
    let tenant = provisioner(&state, &identity, &tenant)?;
    let storage = storage(&state)?;
    let group = found_group(storage, &tenant, &id).await?;
    check_group_if_match(&headers, &group)?;
    let group = write_group(&state, storage, group, &parse_body(&body)?).await?;
    group_response(storage, StatusCode::OK, &group).await
}

/// PATCH /scim/v2/Groups/:id — members added and removed the way Entra ID
/// and Okta send them, or the name.
async fn patch_group(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: TenantContext,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> ScimResult {
    let tenant = provisioner(&state, &identity, &tenant)?;
    let storage = storage(&state)?;
    let group = found_group(storage, &tenant, &id).await?;
    check_group_if_match(&headers, &group)?;
    let mut doc = group_resource(storage, &group, true).await?;
    apply_patch(&mut doc, &parse_body(&body)?)?;
    let group = write_group(&state, storage, group, &doc).await?;
    group_response(storage, StatusCode::OK, &group).await
}

/// DELETE /scim/v2/Groups/:id — the group goes; its members keep their
/// own roles.
async fn delete_group(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: TenantContext,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> ScimResult {
    let tenant = provisioner(&state, &identity, &tenant)?;
    let storage = storage(&state)?;
    let group = found_group(storage, &tenant, &id).await?;
    check_group_if_match(&headers, &group)?;
    let members: Vec<UserId> = groups(storage)?
        .list_members(&group.id)
        .await?
        .into_iter()
        .map(|m| m.user_id)
        .collect();
    groups(storage)?.delete_group(&group.id).await?;
    for user_id in members {
        if let Some(user) = storage.get_user_by_id(&user_id).await? {
            crate::v1::groups::membership_changed(&state, &user).await;
        }
    }
    Ok(StatusCode::NO_CONTENT.into_response())
}

// ---- Discovery ----

async fn service_provider_config() -> Response {
    scim_json(
        StatusCode::OK,
        &json!({
            "schemas": ["urn:ietf:params:scim:schemas:core:2.0:ServiceProviderConfig"],
            "documentationUri": "https://github.com/thevoiceguy/fcp/blob/main/docs/SCIM.md",
            "patch": { "supported": true },
            "bulk": { "supported": false, "maxOperations": 0, "maxPayloadSize": 0 },
            "filter": { "supported": true, "maxResults": MAX_COUNT },
            "changePassword": { "supported": false },
            "sort": { "supported": false },
            "etag": { "supported": true },
            "authenticationSchemes": [{
                "type": "oauthbearertoken",
                "name": "Bearer token",
                "description": "An FCP API token holding scim_provision",
                "primary": true,
            }],
            "meta": { "resourceType": "ServiceProviderConfig", "location": "/scim/v2/ServiceProviderConfig" },
        }),
        None,
    )
}

async fn resource_types() -> Response {
    let user = json!({
        "schemas": ["urn:ietf:params:scim:schemas:core:2.0:ResourceType"],
        "id": "User",
        "name": "User",
        "endpoint": "/Users",
        "schema": USER_SCHEMA,
        "schemaExtensions": [
            { "schema": ENTERPRISE_SCHEMA, "required": false },
            { "schema": FCP_USER_SCHEMA, "required": false },
        ],
        "meta": { "resourceType": "ResourceType", "location": "/scim/v2/ResourceTypes/User" },
    });
    let group = json!({
        "schemas": ["urn:ietf:params:scim:schemas:core:2.0:ResourceType"],
        "id": "Group",
        "name": "Group",
        "endpoint": "/Groups",
        "schema": GROUP_SCHEMA,
        "schemaExtensions": [{ "schema": FCP_GROUP_SCHEMA, "required": false }],
        "meta": { "resourceType": "ResourceType", "location": "/scim/v2/ResourceTypes/Group" },
    });
    list_response(vec![user, group], &ListQuery::default())
}

fn attribute(name: &str, kind: &str, multi: bool, required: bool) -> Value {
    json!({
        "name": name,
        "type": kind,
        "multiValued": multi,
        "required": required,
        "caseExact": false,
        "mutability": "readWrite",
        "returned": "default",
        "uniqueness": if name == "userName" { "server" } else { "none" },
    })
}

async fn schemas() -> Response {
    let user = json!({
        "id": USER_SCHEMA,
        "name": "User",
        "attributes": [
            attribute("userName", "string", false, true),
            attribute("externalId", "string", false, false),
            attribute("displayName", "string", false, false),
            attribute("name", "complex", false, false),
            attribute("emails", "complex", true, false),
            attribute("phoneNumbers", "complex", true, false),
            attribute("title", "string", false, false),
            attribute("active", "boolean", false, false),
        ],
    });
    let enterprise = json!({
        "id": ENTERPRISE_SCHEMA,
        "name": "EnterpriseUser",
        "attributes": [attribute("department", "string", false, false)],
    });
    let fcp = json!({
        "id": FCP_USER_SCHEMA,
        "name": "FcpUser",
        "description": "FCP's own attributes of a user",
        "attributes": [
            attribute("extension", "string", false, false),
            attribute("role", "string", false, false),
            attribute("locationId", "string", false, false),
            json!({
                "name": "sipUsername", "type": "string", "multiValued": false,
                "required": false, "caseExact": false, "mutability": "readOnly",
                "returned": "default", "uniqueness": "server",
            }),
        ],
    });
    let group = json!({
        "id": GROUP_SCHEMA,
        "name": "Group",
        "attributes": [
            attribute("displayName", "string", false, true),
            attribute("externalId", "string", false, false),
            attribute("members", "complex", true, false),
        ],
    });
    let fcp_group = json!({
        "id": FCP_GROUP_SCHEMA,
        "name": "FcpGroup",
        "description": "FCP's own attribute of a group",
        "attributes": [attribute("role", "string", false, false)],
    });
    let resources = [user, enterprise, fcp, group, fcp_group]
        .into_iter()
        .map(|mut s| {
            s["schemas"] = json!(["urn:ietf:params:scim:schemas:core:2.0:Schema"]);
            s
        })
        .collect();
    list_response(resources, &ListQuery::default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_as_provisioners_send_them() {
        assert_eq!(
            parse_filter(r#"userName eq "jane@acme.test""#).unwrap(),
            Filter::UserName("jane@acme.test".into())
        );
        assert_eq!(
            parse_filter(r#"externalId EQ "8f2a""#).unwrap(),
            Filter::ExternalId("8f2a".into())
        );
        assert_eq!(
            parse_filter(r#"emails[value eq "a@b.c"]"#).unwrap(),
            Filter::Email("a@b.c".into())
        );
        assert_eq!(
            parse_filter(r#"emails.value eq "a@b.c""#).unwrap(),
            Filter::Email("a@b.c".into())
        );
        assert!(parse_filter(r#"userName sw "j""#).is_err());
        assert!(parse_filter(r#"title eq "x""#).is_err());
    }

    fn doc() -> Value {
        json!({
            "schemas": [USER_SCHEMA],
            "userName": "jane",
            "active": true,
            "displayName": "Jane Doe",
            "name": { "formatted": "Jane Doe" },
            "emails": [{ "value": "jane@acme.test", "type": "work", "primary": true }],
        })
    }

    fn patch(ops: Value) -> Value {
        json!({ "schemas": [PATCH_SCHEMA], "Operations": ops })
    }

    #[test]
    fn entra_ids_patches() {
        let mut d = doc();
        apply_patch(
            &mut d,
            &patch(json!([
                { "op": "Replace", "path": "active", "value": "False" },
                { "op": "Replace", "path": "emails[type eq \"work\"].value", "value": "jane.doe@acme.test" },
                { "op": "Add", "path": "urn:ietf:params:scim:schemas:extension:enterprise:2.0:User:department", "value": "Sales" },
                { "op": "Replace", "path": "name.familyName", "value": "Smith" },
                { "op": "Replace", "path": "name.givenName", "value": "Jane" },
                { "op": "Add", "path": "phoneNumbers[type eq \"mobile\"].value", "value": "+15550001" },
            ])),
        )
        .unwrap();
        let described = describe(&d).unwrap();
        assert!(!described.active);
        assert_eq!(described.email.as_deref(), Some("jane.doe@acme.test"));
        assert_eq!(described.department, Some(Some("Sales".into())));
        assert_eq!(described.display_name.as_deref(), Some("Jane Smith"));
        assert_eq!(described.mobile.as_deref(), Some("+15550001"));
    }

    #[test]
    fn oktas_patches() {
        let mut d = doc();
        apply_patch(
            &mut d,
            &patch(json!([{ "op": "replace", "value": { "active": false } }])),
        )
        .unwrap();
        assert!(!describe(&d).unwrap().active);
        apply_patch(
            &mut d,
            &patch(json!([{ "op": "replace", "value": {
                "userName": "jane.doe",
                "name": { "givenName": "Jane", "familyName": "Doe" },
            } }])),
        )
        .unwrap();
        let described = describe(&d).unwrap();
        assert_eq!(described.username, "jane.doe");
        assert_eq!(described.display_name.as_deref(), Some("Jane Doe"));
        // Removing a value.
        apply_patch(
            &mut d,
            &patch(json!([{ "op": "remove", "path": "emails[type eq \"work\"]" }])),
        )
        .unwrap();
        assert_eq!(describe(&d).unwrap().email, None);
        assert!(apply_patch(&mut d, &json!({ "Operations": [] })).is_err());
    }
}
