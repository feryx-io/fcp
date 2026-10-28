//! A user's own contacts (`docs/RECORD_AND_CONTACTS.md` §3): `/me/contacts`,
//! kept so that every app the user signs in to has them, and synced with
//! the tenant's sites both ways. A delete leaves a tombstone, so it
//! reaches the other side; reads never show one.

use crate::error::ApiError;
use crate::state::ApiState;
use crate::v1::auth_session::Me;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use fcp_core::contact::{
    ContactNumber, ContactStore, PersonalContact, MAX_CONTACTS_PER_USER, MAX_NUMBERS,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::SystemTime;

/// The longest name, company, email or label taken.
const MAX_TEXT: usize = 200;
/// The longest notes taken.
const MAX_NOTES: usize = 4_000;
/// The longest number taken.
const MAX_NUMBER: usize = 64;

/// A contact, as written.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct PersonalContactRequest {
    pub name: String,
    #[serde(default)]
    pub company: Option<String>,
    #[serde(default)]
    pub numbers: Vec<ContactNumber>,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
    /// The ringtone the user's apps ring with for this contact, by the
    /// app's id for it; `null` rings with the app's own choice, and left
    /// out a change keeps the one set (an app or page that has none to
    /// offer does not clear it).
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<String>)]
    pub ringtone: Option<Option<String>>,
}

/// A field that may be left out (`None`), `null` (`Some(None)`) or set.
fn present<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<Option<String>>, D::Error> {
    Option::<String>::deserialize(d).map(Some)
}

/// A contact.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct PersonalContactResponse {
    pub id: String,
    pub name: String,
    pub company: Option<String>,
    pub numbers: Vec<ContactNumber>,
    pub email: Option<String>,
    pub notes: Option<String>,
    pub ringtone: Option<String>,
    #[serde(with = "crate::time::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub created_at: SystemTime,
    #[serde(with = "crate::time::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub updated_at: SystemTime,
}

impl From<PersonalContact> for PersonalContactResponse {
    fn from(c: PersonalContact) -> Self {
        Self {
            id: c.id,
            name: c.name,
            company: c.company,
            numbers: c.numbers,
            email: c.email,
            notes: c.notes,
            ringtone: c.ringtone,
            created_at: c.created_at,
            updated_at: c.updated_at,
        }
    }
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct PersonalContactsQuery {
    pub limit: Option<usize>,
    pub cursor: Option<String>,
    /// A word in a name, company, email or number.
    pub q: Option<String>,
}

fn store(state: &ApiState) -> Result<&Arc<dyn ContactStore>, ApiError> {
    state
        .storage
        .as_ref()
        .and_then(|s| s.contact_store())
        .ok_or_else(|| ApiError::NotFound("Contacts are not kept here".to_string()))
}

async fn me(state: &ApiState, me: &Me) -> Result<fcp_core::storage::User, ApiError> {
    let storage = state
        .storage
        .as_ref()
        .ok_or_else(|| ApiError::NotFound("Storage is required but not configured".into()))?;
    crate::v1::self_service::load_claims_user(storage, &me.0, state).await
}

fn trimmed(v: Option<String>, field: &str, max: usize) -> Result<Option<String>, ApiError> {
    let v = v.map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
    if v.as_ref().is_some_and(|v| v.chars().count() > max) {
        return Err(ApiError::BadRequest(format!(
            "{field} may be at most {max} characters"
        )));
    }
    Ok(v)
}

/// The request checked and tidied: a name, numbers that are not empty.
fn checked(req: PersonalContactRequest) -> Result<PersonalContactRequest, ApiError> {
    let name = trimmed(Some(req.name), "name", MAX_TEXT)?
        .ok_or_else(|| ApiError::BadRequest("A contact needs a name".into()))?;
    let mut numbers = Vec::new();
    for n in req.numbers {
        let number = n.number.trim().to_string();
        if number.is_empty() {
            continue;
        }
        if number.chars().count() > MAX_NUMBER {
            return Err(ApiError::BadRequest(format!(
                "A number may be at most {MAX_NUMBER} characters"
            )));
        }
        let label = trimmed(Some(n.label), "label", MAX_TEXT)?.unwrap_or_default();
        numbers.push(ContactNumber { label, number });
    }
    if numbers.len() > MAX_NUMBERS {
        return Err(ApiError::BadRequest(format!(
            "A contact may have at most {MAX_NUMBERS} numbers"
        )));
    }
    Ok(PersonalContactRequest {
        name,
        company: trimmed(req.company, "company", MAX_TEXT)?,
        numbers,
        email: trimmed(req.email, "email", MAX_TEXT)?,
        notes: trimmed(req.notes, "notes", MAX_NOTES)?,
        ringtone: match req.ringtone {
            Some(r) => Some(trimmed(r, "ringtone", MAX_TEXT)?),
            None => None,
        },
    })
}

/// The user's contact `id`, not deleted; 404 for anyone else's.
async fn mine(
    store: &Arc<dyn ContactStore>,
    user: &fcp_core::storage::User,
    id: &str,
) -> Result<PersonalContact, ApiError> {
    store
        .get_contact(id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .filter(|c| c.owner_user_id == user.id && c.deleted_at.is_none())
        .ok_or_else(|| ApiError::NotFound(format!("Contact {id} not found")))
}

/// Write `contact`, newer than what is kept; a write that lost (the same
/// contact changed elsewhere a moment later) is a conflict.
async fn write(store: &Arc<dyn ContactStore>, contact: &PersonalContact) -> Result<(), ApiError> {
    let written = store
        .put_contact_if_newer(contact)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    if !written {
        return Err(ApiError::Conflict(
            "The contact was changed elsewhere at the same time; read it again".into(),
        ));
    }
    Ok(())
}

/// Tell the user's apps a contact changed (`user.contact_changed`), so
/// they read their contacts again.
pub async fn announce(
    events: &dyn fcp_events::EventPublisher,
    contact: &PersonalContact,
    change: &str,
) {
    let event = fcp_events::PlatformEvent::User(fcp_events::UserEvent::ContactChanged {
        user_id: contact.owner_user_id.to_string(),
        tenant_id: contact.tenant_id,
        contact_id: contact.id.clone(),
        change: change.to_string(),
        timestamp: SystemTime::now(),
    });
    if let Err(e) = events.publish(event).await {
        tracing::warn!(error = %e, "A contact's change not announced");
    }
}

/// GET /me/contacts — the user's contacts, by name.
#[utoipa::path(
    get,
    path = "/api/v1/me/contacts",
    tag = "self_service",
    operation_id = "self_service_list_contacts",
    params(PersonalContactsQuery),
    responses(
        (status = 200, description = "OK", body = crate::page::Page<PersonalContactResponse>),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn list_contacts(
    State(state): State<ApiState>,
    me_: Me,
    Query(query): Query<PersonalContactsQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let user = me(&state, &me_).await?;
    let store = store(&state)?;
    let page = crate::page::PageQuery {
        limit: query.limit,
        cursor: query.cursor,
    };
    let limit = page.limit()?;
    let after = page.after()?;
    let q = query
        .q
        .as_deref()
        .map(str::trim)
        .filter(|q| !q.is_empty())
        .map(str::to_lowercase);
    let mut rows = store
        .list_contacts(&user.id, q.as_deref(), after.as_deref(), limit + 1)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let more = rows.len() > limit;
    rows.truncate(limit);
    let next = more.then(|| rows.last().map(|c| c.id.clone())).flatten();
    let data: Vec<PersonalContactResponse> = rows.into_iter().map(Into::into).collect();
    Ok(Json(crate::page::Page::new(data, next, None)))
}

/// POST /me/contacts — keep a new contact.
#[utoipa::path(
    post,
    path = "/api/v1/me/contacts",
    tag = "self_service",
    operation_id = "self_service_create_contact",
    request_body = PersonalContactRequest,
    responses(
        (status = 201, description = "Kept", body = PersonalContactResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn create_contact(
    State(state): State<ApiState>,
    me_: Me,
    Json(req): Json<PersonalContactRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let user = me(&state, &me_).await?;
    let store = store(&state)?;
    let req = checked(req)?;
    let kept = store
        .count_contacts(&user.id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    if kept >= MAX_CONTACTS_PER_USER {
        return Err(ApiError::Conflict(format!(
            "A user may keep at most {MAX_CONTACTS_PER_USER} contacts"
        )));
    }
    let now = SystemTime::now();
    let contact = PersonalContact {
        id: fcp_core::contact::new_contact_id(),
        tenant_id: user.tenant_id,
        owner_user_id: user.id,
        name: req.name,
        company: req.company,
        numbers: req.numbers,
        email: req.email,
        notes: req.notes,
        ringtone: req.ringtone.flatten(),
        created_at: now,
        updated_at: now,
        deleted_at: None,
    };
    write(store, &contact).await?;
    announce(state.event_bus.as_ref(), &contact, "created").await;
    let kept = mine(store, &user, &contact.id).await?;
    Ok((
        StatusCode::CREATED,
        Json(PersonalContactResponse::from(kept)),
    ))
}

/// GET /me/contacts/:id
#[utoipa::path(
    get,
    path = "/api/v1/me/contacts/{id}",
    tag = "self_service",
    operation_id = "self_service_get_contact",
    params(("id" = String, Path)),
    responses(
        (status = 200, description = "OK", body = PersonalContactResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn get_contact(
    State(state): State<ApiState>,
    me_: Me,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let user = me(&state, &me_).await?;
    let contact = mine(store(&state)?, &user, &id).await?;
    Ok(Json(PersonalContactResponse::from(contact)))
}

/// PUT /me/contacts/:id — replace a contact.
#[utoipa::path(
    put,
    path = "/api/v1/me/contacts/{id}",
    tag = "self_service",
    operation_id = "self_service_update_contact",
    params(("id" = String, Path)),
    request_body = PersonalContactRequest,
    responses(
        (status = 200, description = "Kept", body = PersonalContactResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn update_contact(
    State(state): State<ApiState>,
    me_: Me,
    Path(id): Path<String>,
    Json(req): Json<PersonalContactRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let user = me(&state, &me_).await?;
    let store = store(&state)?;
    let req = checked(req)?;
    let kept = mine(store, &user, &id).await?;
    let contact = PersonalContact {
        name: req.name,
        company: req.company,
        numbers: req.numbers,
        email: req.email,
        notes: req.notes,
        ringtone: req.ringtone.unwrap_or_else(|| kept.ringtone.clone()),
        updated_at: SystemTime::now().max(kept.updated_at + std::time::Duration::from_micros(1)),
        ..kept
    };
    write(store, &contact).await?;
    announce(state.event_bus.as_ref(), &contact, "updated").await;
    Ok(Json(PersonalContactResponse::from(
        mine(store, &user, &id).await?,
    )))
}

/// DELETE /me/contacts/:id — the contact goes from every app; its
/// tombstone carries the delete to the tenant's sites.
#[utoipa::path(
    delete,
    path = "/api/v1/me/contacts/{id}",
    tag = "self_service",
    operation_id = "self_service_delete_contact",
    params(("id" = String, Path)),
    responses(
        (status = 204, description = "Deleted"),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn delete_contact(
    State(state): State<ApiState>,
    me_: Me,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let user = me(&state, &me_).await?;
    let store = store(&state)?;
    let kept = mine(store, &user, &id).await?;
    let now = SystemTime::now().max(kept.updated_at + std::time::Duration::from_micros(1));
    let tombstone = PersonalContact {
        updated_at: now,
        deleted_at: Some(now),
        ..kept
    };
    write(store, &tombstone).await?;
    announce(state.event_bus.as_ref(), &tombstone, "deleted").await;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ContactFileQuery {
    /// `vcard` or `csv`. On import, left out, the file says (`BEGIN:VCARD`
    /// is vCard, anything else CSV); on export, vCard.
    pub format: Option<String>,
}

fn format_of(q: &ContactFileQuery) -> Result<Option<fcp_core::contact_file::Format>, ApiError> {
    q.format
        .as_deref()
        .map(|f| {
            fcp_core::contact_file::Format::parse(f)
                .ok_or_else(|| ApiError::BadRequest(format!("{f:?} is not vcard or csv")))
        })
        .transpose()
}

/// All of the user's contacts, by name.
async fn all_of(
    store: &Arc<dyn ContactStore>,
    user: &fcp_core::storage::User,
) -> Result<Vec<PersonalContact>, ApiError> {
    let mut all = Vec::new();
    let mut after: Option<String> = None;
    loop {
        let page = store
            .list_contacts(&user.id, None, after.as_deref(), 500)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        let more = page.len() == 500;
        after = page.last().map(|c| c.id.clone());
        all.extend(page);
        if !more {
            return Ok(all);
        }
    }
}

/// GET /me/contacts/export — the user's contacts as one file, vCard 3.0 or
/// CSV (Google's columns), to take to another phone or address book.
#[utoipa::path(
    get,
    path = "/api/v1/me/contacts/export",
    tag = "self_service",
    operation_id = "self_service_export_contacts",
    params(ContactFileQuery),
    responses(
        (status = 200, description = "The file", content_type = "text/vcard", body = String),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn export_contacts(
    State(state): State<ApiState>,
    me_: Me,
    Query(query): Query<ContactFileQuery>,
) -> Result<impl IntoResponse, ApiError> {
    use fcp_core::contact_file::Format;
    let format = format_of(&query)?.unwrap_or(Format::VCard);
    let user = me(&state, &me_).await?;
    let contacts = all_of(store(&state)?, &user).await?;
    let (content_type, file) = match format {
        Format::VCard => ("text/vcard; charset=utf-8", "contacts.vcf"),
        Format::Csv => ("text/csv; charset=utf-8", "contacts.csv"),
    };
    Ok((
        [
            (axum::http::header::CONTENT_TYPE, content_type.to_string()),
            (
                axum::http::header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{file}\""),
            ),
        ],
        fcp_core::contact_file::write(format, &contacts),
    ))
}

/// What an import did.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ContactImportResponse {
    /// Contacts made.
    pub imported: usize,
    /// Entries already among the user's contacts (the same name and
    /// numbers), or twice in the file: not made again.
    pub duplicates: usize,
    /// Entries not made, each with its place in the file (a vCard's
    /// number, a CSV row's line) and why.
    pub skipped: Vec<ContactImportSkip>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ContactImportSkip {
    pub entry: usize,
    pub reason: String,
}

/// POST /me/contacts/import — contacts from a vCard or CSV file (the body),
/// kept on the user's account. One already kept, with the same name and
/// numbers, is not made again. The user's apps hear one
/// `user.contact_changed` (`change`: `imported`) for the lot.
#[utoipa::path(
    post,
    path = "/api/v1/me/contacts/import",
    tag = "self_service",
    operation_id = "self_service_import_contacts",
    params(ContactFileQuery),
    request_body(
        description = "The file: vCard or CSV",
        content(
            (Vec<u8> = "application/octet-stream"),
            (String = "text/vcard"),
            (String = "text/csv"),
        ),
    ),
    responses(
        (status = 200, description = "Imported", body = ContactImportResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn import_contacts(
    State(state): State<ApiState>,
    me_: Me,
    Query(query): Query<ContactFileQuery>,
    body: String,
) -> Result<impl IntoResponse, ApiError> {
    use fcp_core::contact_file::{self, Format, ImportedContact};
    let user = me(&state, &me_).await?;
    let store = store(&state)?;
    let format = format_of(&query)?.unwrap_or_else(|| Format::guess(&body));
    let read = contact_file::read(format, &body);
    let mut kept: Vec<ImportedContact> = all_of(store, &user)
        .await?
        .into_iter()
        .map(|c| ImportedContact {
            name: c.name,
            numbers: c.numbers,
            ..Default::default()
        })
        .collect();
    let mut room = MAX_CONTACTS_PER_USER.saturating_sub(kept.len() as u64);
    let mut answer = ContactImportResponse {
        imported: 0,
        duplicates: 0,
        skipped: read
            .skipped
            .into_iter()
            .map(|(entry, reason)| ContactImportSkip { entry, reason })
            .collect(),
    };
    let mut first: Option<PersonalContact> = None;
    for (i, found) in read.contacts.into_iter().enumerate() {
        if kept.iter().any(|k| found.same_as(&k.name, &k.numbers)) {
            answer.duplicates += 1;
            continue;
        }
        let entry = i + 1;
        if room == 0 {
            answer.skipped.push(ContactImportSkip {
                entry,
                reason: format!("a user may keep at most {MAX_CONTACTS_PER_USER} contacts"),
            });
            continue;
        }
        let req = match checked(PersonalContactRequest {
            name: found.name.clone(),
            company: found.company.clone(),
            numbers: found.numbers.clone(),
            email: found.email.clone(),
            notes: found.notes.clone(),
            ringtone: None,
        }) {
            Ok(r) => r,
            Err(e) => {
                answer.skipped.push(ContactImportSkip {
                    entry,
                    reason: e.message().to_string(),
                });
                continue;
            }
        };
        let now = SystemTime::now();
        let contact = PersonalContact {
            id: fcp_core::contact::new_contact_id(),
            tenant_id: user.tenant_id,
            owner_user_id: user.id,
            name: req.name,
            company: req.company,
            numbers: req.numbers,
            email: req.email,
            notes: req.notes,
            ringtone: None,
            created_at: now,
            updated_at: now,
            deleted_at: None,
        };
        write(store, &contact).await?;
        kept.push(found);
        room -= 1;
        answer.imported += 1;
        first.get_or_insert(contact);
    }
    if let Some(first) = &first {
        announce(state.event_bus.as_ref(), first, "imported").await;
    }
    tracing::info!(
        user = %user.username,
        imported = answer.imported,
        duplicates = answer.duplicates,
        skipped = answer.skipped.len(),
        "Contacts imported"
    );
    Ok(Json(answer))
}
