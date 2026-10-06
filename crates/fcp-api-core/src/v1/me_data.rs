//! What a user sees of their own calls and of the people they work with
//! (`docs/SOFTPHONE.md` §2.7, phase F4): `GET /me/calls`, their call
//! history, and `GET /me/directory`, the tenant's people and extensions
//! with each one's presence.

use crate::error::ApiError;
use crate::state::ApiState;
use crate::v1::auth_session::Me;
use axum::{
    extract::{Query, State},
    response::IntoResponse,
    Json,
};
use fcp_core::legal_hold::{self, SubjectIdentifier};
use fcp_core::storage::{
    CallDetailRecord, CallDisposition, CdrCursor, CdrKind, CdrQuery, CdrStore, CredentialStore,
    ExtensionStore, TenantStore, UserFilter,
};
use serde::{Deserialize, Serialize};
use std::time::UNIX_EPOCH;

macro_rules! require_storage {
    ($state:expr) => {
        $state.storage.as_ref().ok_or_else(|| {
            ApiError::NotFound("Storage is required but not configured".to_string())
        })?
    };
}

fn epoch(t: std::time::SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// One of the user's calls, from their side.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct MyCallResponse {
    /// The record's id, as `/cdrs` names it.
    pub cdr_id: String,
    pub call_id: String,
    /// `outgoing` when the user placed it, else `incoming`.
    pub direction: &'static str,
    /// The other party's address.
    pub other_party: String,
    pub other_party_name: Option<String>,
    /// `answered`, `busy`, `no_answer`, `failed` or `cancelled`.
    pub disposition: String,
    /// An incoming call nobody answered.
    pub missed: bool,
    #[serde(with = "crate::time::epoch_secs")]
    #[schema(value_type = String, format = DateTime)]
    pub start_time: u64,
    #[serde(with = "crate::time::epoch_secs_opt")]
    #[schema(value_type = Option<String>, format = DateTime)]
    pub answer_time: Option<u64>,
    #[serde(with = "crate::time::epoch_secs_opt")]
    #[schema(value_type = Option<String>, format = DateTime)]
    pub end_time: Option<u64>,
    /// Talk time, from the answer.
    pub duration_seconds: Option<u64>,
}

fn my_call(cdr: CallDetailRecord, outgoing: bool) -> MyCallResponse {
    let disposition = serde_json::to_value(cdr.disposition)
        .ok()
        .and_then(|v| v.as_str().map(String::from))
        .unwrap_or_default();
    let (other_party, other_party_name) = if outgoing {
        (cdr.to_uri, cdr.to_display_name)
    } else {
        (cdr.from_uri, cdr.from_display_name)
    };
    MyCallResponse {
        cdr_id: cdr.cdr_id.to_string(),
        call_id: cdr.call_id.to_string(),
        direction: if outgoing { "outgoing" } else { "incoming" },
        other_party,
        other_party_name,
        missed: !outgoing && cdr.disposition != CallDisposition::Answered,
        disposition,
        start_time: epoch(cdr.start_time),
        answer_time: cdr.answer_time.map(epoch),
        end_time: cdr.end_time.map(epoch),
        duration_seconds: cdr.call_duration_ms.map(|ms| ms / 1000),
    }
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct MyCallsQuery {
    pub limit: Option<usize>,
    pub cursor: Option<String>,
    /// Only the incoming calls nobody answered.
    #[serde(default)]
    pub missed: bool,
}

/// GET /me/calls — the user's calls, newest first: the records whose
/// caller or callee is one of their names, extensions, numbers or
/// addresses, as a subject export finds them.
#[utoipa::path(
    get,
    path = "/api/v1/me/calls",
    tag = "self_service",
    operation_id = "self_service_list_my_calls",
    params(MyCallsQuery),
    responses(
        (status = 200, description = "OK", body = crate::page::Page<MyCallResponse>),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn list_my_calls(
    State(state): State<ApiState>,
    Me(claims): Me,
    Query(query): Query<MyCallsQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let storage = require_storage!(state);
    let user = crate::v1::self_service::load_claims_user(storage, &claims, &state).await?;
    let limit = crate::page::PageQuery {
        limit: query.limit,
        cursor: None,
    }
    .limit()?;
    let mut cursor = match &query.cursor {
        Some(c) => Some(
            CdrCursor::decode(&crate::page::decode(c)?)
                .ok_or_else(|| ApiError::BadRequest("Invalid cursor".into()))?,
        ),
        None => None,
    };
    let expansion = legal_hold::expand(
        storage,
        &user.tenant_id,
        &[SubjectIdentifier::User {
            username: user.username.clone(),
        }],
    )
    .await?;
    let keys: Vec<String> = expansion
        .keys()
        .into_iter()
        .filter(|k| !k.starts_with("mailbox:"))
        .collect();
    let matcher = expansion.matcher();

    // The keys find candidates through the index; the matcher keeps the
    // user's own. Read in batches until a page is full or the records end.
    let batch = (limit * 2).max(50);
    let mut calls = Vec::new();
    let mut more = false;
    'read: loop {
        let records = storage
            .query_cdrs(CdrQuery {
                tenant_id: Some(user.tenant_id),
                kind: Some(CdrKind::Call),
                subject_keys: Some(keys.clone()),
                cursor,
                limit: Some(batch),
                ..Default::default()
            })
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        let exhausted = records.len() < batch;
        for cdr in records {
            cursor = Some(CdrCursor::after(&cdr));
            let outgoing = matcher.matches(&cdr.from_uri);
            if !outgoing && !matcher.matches(&cdr.to_uri) {
                continue;
            }
            let call = my_call(cdr, outgoing);
            if query.missed && !call.missed {
                continue;
            }
            if calls.len() == limit {
                more = true;
                break 'read;
            }
            calls.push((call, cursor.expect("set above")));
        }
        if exhausted {
            break;
        }
    }
    let next = more
        .then(|| calls.last().map(|(_, c)| c.encode()))
        .flatten();
    Ok(Json(crate::page::Page::new(
        calls.into_iter().map(|(c, _)| c).collect(),
        next,
        None,
    )))
}

/// Someone, or something, in the tenant's directory.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct DirectoryEntry {
    /// `user`, or `extension` for one that rings no user (a queue, a
    /// flow, a number).
    pub kind: &'static str,
    /// The user's id; unset for an extension.
    pub user_id: Option<String>,
    pub username: Option<String>,
    pub display_name: Option<String>,
    pub extension: Option<String>,
    pub email: Option<String>,
    pub department: Option<String>,
    pub job_title: Option<String>,
    /// Where they are: `open`, `closed`, `away`, `busy`, `on_the_phone`
    /// or `ringing`; unset when nothing is known.
    pub presence: Option<String>,
    pub presence_note: Option<String>,
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct DirectoryQuery {
    pub limit: Option<usize>,
    pub cursor: Option<String>,
    /// A word in a name, username, email or extension.
    pub q: Option<String>,
}

/// GET /me/directory — the tenant's people and the extensions that ring
/// no one person, each with its presence, while the tenant lets its users
/// see them (`directory.visible_to_users`; 403 otherwise).
#[utoipa::path(
    get,
    path = "/api/v1/me/directory",
    tag = "self_service",
    operation_id = "self_service_list_directory",
    params(DirectoryQuery),
    responses(
        (status = 200, description = "OK", body = crate::page::Page<DirectoryEntry>),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn list_directory(
    State(state): State<ApiState>,
    Me(claims): Me,
    Query(query): Query<DirectoryQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let storage = require_storage!(state);
    let me = crate::v1::self_service::load_claims_user(storage, &claims, &state).await?;
    let tenant = storage
        .get_tenant(&me.tenant_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| ApiError::NotFound("Tenant not found".to_string()))?;
    if !tenant.directory.visible_to_users {
        return Err(ApiError::Forbidden(
            "The tenant does not show its directory to its users".to_string(),
        ));
    }

    // Presence by the user part of each address the presence service holds.
    let mut presence = std::collections::HashMap::new();
    if let Some(pm) = &state.presence_manager {
        for p in pm.get_presences_by_tenant(&me.tenant_id) {
            let user_part = p
                .aor
                .trim_start_matches("sips:")
                .trim_start_matches("sip:")
                .split('@')
                .next()
                .unwrap_or_default()
                .to_lowercase();
            let state = serde_json::to_value(p.state)
                .ok()
                .and_then(|v| v.as_str().map(String::from));
            presence.entry(user_part).or_insert((state, p.note));
        }
    }
    let presence_of = |names: &[Option<&str>]| {
        names
            .iter()
            .flatten()
            .find_map(|n| presence.get(&n.to_lowercase()).cloned())
            .unwrap_or((None, None))
    };

    let needle = query
        .q
        .as_deref()
        .map(str::trim)
        .filter(|q| !q.is_empty())
        .map(str::to_lowercase);
    let found = |fields: &[Option<&str>]| match &needle {
        None => true,
        Some(n) => fields
            .iter()
            .flatten()
            .any(|f| f.to_lowercase().contains(n.as_str())),
    };

    let mut entries = Vec::new();
    let users = storage
        .list_users_in_tenant(&me.tenant_id, &UserFilter::default())
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    for u in users.iter().filter(|u| u.may_use_services()) {
        let fields = [
            Some(u.username.as_str()),
            u.display_name.as_deref(),
            u.email.as_deref(),
            u.extension.as_deref(),
        ];
        if !found(&fields) {
            continue;
        }
        let (state, note) = presence_of(&[Some(u.sip_name()), u.extension.as_deref()]);
        entries.push(DirectoryEntry {
            kind: "user",
            user_id: Some(u.id.to_string()),
            username: Some(u.username.clone()),
            display_name: u.display_name.clone(),
            extension: u.extension.clone(),
            email: u.email.clone(),
            department: u.department.clone(),
            job_title: u.job_title.clone(),
            presence: state,
            presence_note: note,
        });
    }
    let extensions = storage
        .list_extensions(&me.tenant_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    for x in extensions.iter().filter(|x| x.enabled) {
        if matches!(x.target, fcp_core::ExtensionTarget::User(_)) {
            continue;
        }
        if !found(&[Some(x.number.as_str()), x.description.as_deref()]) {
            continue;
        }
        let (state, note) = presence_of(&[Some(x.number.as_str())]);
        entries.push(DirectoryEntry {
            kind: "extension",
            user_id: None,
            username: None,
            display_name: x.description.clone(),
            extension: Some(x.number.clone()),
            email: None,
            department: None,
            job_title: None,
            presence: state,
            presence_note: note,
        });
    }
    let page = crate::page::PageQuery {
        limit: query.limit,
        cursor: query.cursor,
    };
    Ok(Json(crate::page::paginate(
        entries,
        &page,
        crate::page::Order::Ascending,
        |e| {
            format!(
                "{}/{}/{}",
                e.display_name
                    .as_deref()
                    .or(e.username.as_deref())
                    .unwrap_or_default()
                    .to_lowercase(),
                e.extension.as_deref().unwrap_or_default(),
                e.user_id.as_deref().unwrap_or_default()
            )
        },
    )?))
}
