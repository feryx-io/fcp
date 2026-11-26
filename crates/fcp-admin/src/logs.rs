//! The services' logs, from the cluster's Redis (`docs/LOG_VIEWER.md`
//! phase L3).
//!
//! Every process ships its lines to `fcp:logs:<service id>`
//! (`fcp_cluster::logging`). These routes read them back:
//!
//! - `GET /logs` — newest first, filtered by service, node type, level (at
//!   least), call, text and time; a platform administrator may also filter
//!   by tenant (`none` for the platform's own lines). Anyone else sees only
//!   lines naming their own tenant (`log_read`).
//! - `GET /logs/export` — the same lines as JSON Lines, audited.
//! - `GET /logs/levels` and `PUT /logs/levels/{service}` — a platform
//!   administrator sets a service's level for an hour; the service's beacon
//!   takes it up within seconds.

use axum::{
    extract::{Extension, Path, Query},
    http::{header, StatusCode},
    response::IntoResponse,
    routing::{get, put},
    Json, Router,
};
use fcp_api_core::auth::{AuthIdentity, TenantContext};
use fcp_api_core::error::ApiError;
use fcp_api_core::page::{Page, PageQuery};
use fcp_cluster::logging::{self, LogRecord};
use fcp_storage_redis::{RedisConfig, RedisStorage};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use tokio::sync::OnceCell;

/// Lines read from one stream at a time while looking for matches.
const BATCH: usize = 500;
/// The most lines one request looks through, across every stream.
const SCAN_LIMIT: usize = 50_000;
/// The most lines one export holds.
const EXPORT_LIMIT: usize = 50_000;

/// Where the lines are.
#[derive(Clone)]
pub struct LogsApiState {
    pub redis: Option<RedisConfig>,
    pub conn: std::sync::Arc<OnceCell<RedisStorage>>,
}

impl LogsApiState {
    pub fn new(redis: Option<RedisConfig>) -> Self {
        Self {
            redis,
            conn: std::sync::Arc::new(OnceCell::new()),
        }
    }

    pub(crate) async fn storage(&self) -> Result<&RedisStorage, ApiError> {
        let config = self.redis.clone().ok_or_else(|| {
            ApiError::ServiceUnavailable(
                "Logs are not kept here: add [cluster] redis_url to the admin's configuration"
                    .to_string(),
            )
        })?;
        self.conn
            .get_or_try_init(|| async move { RedisStorage::new(config).await })
            .await
            .map_err(|e| ApiError::ServiceUnavailable(format!("Redis: {e:#}")))
    }
}

/// What to look for.
#[derive(Debug, Default, Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct LogQuery {
    /// One service, by id.
    pub service: Option<String>,
    /// Services of this node type (`call_manager`, `sbc`…).
    pub kind: Option<String>,
    /// This level and worse: `error`, `warn`, `info`, `debug`, `trace`.
    pub level: Option<String>,
    /// Platform administrators: a tenant's lines, or `none` for the
    /// platform's own.
    pub tenant: Option<String>,
    /// Lines about this call.
    pub call_id: Option<String>,
    /// Text in the message or a field, any case.
    pub q: Option<String>,
    /// RFC 3339: lines at or after.
    pub since: Option<String>,
    /// RFC 3339: lines before.
    pub until: Option<String>,
    pub limit: Option<usize>,
    pub cursor: Option<String>,
}

/// One line.
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct LogLine {
    /// Its id in the service's stream; a page's cursor.
    pub id: String,
    pub service: String,
    /// The service's node type, when it is up.
    pub kind: Option<String>,
    #[schema(value_type = String, format = DateTime)]
    pub time: String,
    pub level: String,
    pub target: String,
    pub message: String,
    pub fields: BTreeMap<String, String>,
    pub tenant_id: Option<String>,
    pub call_id: Option<String>,
}

/// A service's level, as set from here for now.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct LogLevel {
    pub service: String,
    /// Set for now; null is the service's configured level.
    pub level: Option<String>,
    /// Seconds before it goes back.
    pub expires_in_secs: Option<u64>,
}

/// A level to set for an hour; null goes back now.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct SetLogLevel {
    pub level: Option<String>,
}

fn rank(level: &str) -> u8 {
    match level.to_ascii_uppercase().as_str() {
        "ERROR" => 1,
        "WARN" | "WARNING" => 2,
        "INFO" => 3,
        "DEBUG" => 4,
        "TRACE" => 5,
        _ => 0,
    }
}

fn stream_id_ms(id: &str) -> u64 {
    id.split('-')
        .next()
        .and_then(|m| m.parse().ok())
        .unwrap_or(0)
}

/// `(ms, seq)` of a stream id, for ordering across streams.
fn order_key(id: &str) -> (u64, u64) {
    let mut parts = id.split('-');
    let ms = parts.next().and_then(|m| m.parse().ok()).unwrap_or(0);
    let seq = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    (ms, seq)
}

fn rfc3339_ms(text: &str) -> Result<u64, ApiError> {
    chrono::DateTime::parse_from_rfc3339(text)
        .map(|t| t.timestamp_millis().max(0) as u64)
        .map_err(|e| ApiError::BadRequest(format!("{text:?} is not an RFC 3339 time: {e}")))
}

/// The filter a request may use: a tenant's caller is held to its tenant.
struct Filter {
    services: Option<Vec<String>>,
    min_rank: u8,
    /// `Some(None)`: the platform's own lines; `Some(Some(t))`: tenant t's.
    tenant: Option<Option<String>>,
    call_id: Option<String>,
    q: Option<String>,
    since_ms: Option<u64>,
}

impl Filter {
    fn of(
        query: &LogQuery,
        identity: &AuthIdentity,
        tenant: &TenantContext,
        kinds: &HashMap<String, String>,
    ) -> Result<Self, ApiError> {
        let tenant = if identity.is_platform_admin() {
            match query.tenant.as_deref().map(str::trim) {
                None | Some("") | Some("all") => None,
                Some("none") | Some("platform") => Some(None),
                Some(t) => Some(Some(plain_tenant(t))),
            }
        } else {
            // A tenant's caller sees its own tenant's lines and nothing else.
            Some(Some(tenant.0.as_uuid().to_string()))
        };
        let min_rank = match query.level.as_deref() {
            None | Some("") => 5,
            Some(l) => match rank(l) {
                0 => {
                    return Err(ApiError::BadRequest(format!(
                        "level {l:?}: error, warn, info, debug or trace"
                    )))
                }
                r => r,
            },
        };
        let services = match (&query.service, &query.kind) {
            (Some(s), _) => Some(vec![s.clone()]),
            (None, Some(k)) => Some(
                kinds
                    .iter()
                    .filter(|(_, kind)| *kind == k)
                    .map(|(id, _)| id.clone())
                    .collect(),
            ),
            (None, None) => None,
        };
        Ok(Self {
            services,
            min_rank,
            tenant,
            call_id: query.call_id.clone().filter(|c| !c.is_empty()),
            q: query
                .q
                .as_deref()
                .map(str::trim)
                .filter(|q| !q.is_empty())
                .map(str::to_lowercase),
            since_ms: query.since.as_deref().map(rfc3339_ms).transpose()?,
        })
    }

    fn keeps(&self, r: &LogRecord) -> bool {
        if rank(&r.level) > self.min_rank {
            return false;
        }
        if let Some(t) = &self.tenant {
            if r.tenant_id.as_deref() != t.as_deref() {
                return false;
            }
        }
        if let Some(c) = &self.call_id {
            if r.call_id.as_deref() != Some(c.as_str()) {
                return false;
            }
        }
        if let Some(q) = &self.q {
            let hit = r.message.to_lowercase().contains(q)
                || r.target.to_lowercase().contains(q)
                || r.fields
                    .values()
                    .any(|v| v.to_lowercase().contains(q.as_str()));
            if !hit {
                return false;
            }
        }
        true
    }
}

fn plain_tenant(t: &str) -> String {
    t.strip_prefix("tenant-").unwrap_or(t).to_string()
}

/// Every service with lines kept, by id.
async fn streams(storage: &RedisStorage) -> Result<Vec<String>, ApiError> {
    let prefix = storage.key(&logging::stream_key(""));
    let mut conn = storage.connection();
    let mut keys = Vec::new();
    let mut cursor: u64 = 0;
    loop {
        let (next, batch): (u64, Vec<String>) = redis::cmd("SCAN")
            .arg(cursor)
            .arg("MATCH")
            .arg(format!("{prefix}*"))
            .arg("COUNT")
            .arg(500)
            .query_async(&mut conn)
            .await
            .map_err(|e| ApiError::ServiceUnavailable(format!("Redis: {e}")))?;
        keys.extend(batch);
        cursor = next;
        if cursor == 0 {
            break;
        }
    }
    // `logs:level:<id>` keys share the prefix and are not streams.
    let level_prefix = storage.key(&logging::level_key(""));
    let mut ids: Vec<String> = keys
        .into_iter()
        .filter(|k| !k.starts_with(&level_prefix))
        .filter_map(|k| k.strip_prefix(&prefix).map(str::to_string))
        .collect();
    ids.sort();
    Ok(ids)
}

/// The node type of each live service.
async fn kinds(storage: &RedisStorage) -> HashMap<String, String> {
    fcp_cluster::ServiceBeacon::read_all(storage)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|r| (r.id, r.kind))
        .collect()
}

/// Up to `want` lines matching `filter`, newest first, older than `before`
/// (a stream id) and not older than `until_ms`.
async fn find(
    storage: &RedisStorage,
    filter: &Filter,
    kinds: &HashMap<String, String>,
    before: Option<&str>,
    until_ms: Option<u64>,
    want: usize,
) -> Result<(Vec<LogLine>, bool), ApiError> {
    let mut services = streams(storage).await?;
    if let Some(only) = &filter.services {
        services.retain(|s| only.contains(s));
    }
    let mut conn = storage.connection();
    let mut found: Vec<LogLine> = Vec::new();
    let mut scanned = 0usize;
    let mut more = false;
    for service in services {
        let key = storage.key(&logging::stream_key(&service));
        let mut end = match (before, until_ms) {
            (Some(b), _) => format!("({b}"),
            (None, Some(u)) => format!("({u}"),
            (None, None) => "+".to_string(),
        };
        let start = filter
            .since_ms
            .map(|s| s.to_string())
            .unwrap_or_else(|| "-".to_string());
        let mut from_here = 0usize;
        loop {
            let entries: Vec<(String, Vec<(String, String)>)> = redis::cmd("XREVRANGE")
                .arg(&key)
                .arg(&end)
                .arg(&start)
                .arg("COUNT")
                .arg(BATCH)
                .query_async(&mut conn)
                .await
                .map_err(|e| ApiError::ServiceUnavailable(format!("Redis: {e}")))?;
            let read = entries.len();
            scanned += read;
            for (id, fields) in &entries {
                let Some(json) = fields.iter().find(|(k, _)| k == "r").map(|(_, v)| v) else {
                    continue;
                };
                let Ok(record) = serde_json::from_str::<LogRecord>(json) else {
                    continue;
                };
                if !filter.keeps(&record) {
                    continue;
                }
                from_here += 1;
                found.push(LogLine {
                    id: id.clone(),
                    service: service.clone(),
                    kind: kinds.get(&service).cloned(),
                    time: chrono::DateTime::<chrono::Utc>::from_timestamp_millis(
                        record.ts.max(stream_id_ms(id)) as i64,
                    )
                    .unwrap_or_default()
                    .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
                    level: record.level,
                    target: record.target,
                    message: record.message,
                    fields: record.fields,
                    tenant_id: record.tenant_id,
                    call_id: record.call_id,
                });
            }
            // Enough from this stream for a page, or none left.
            if read < BATCH || from_here > want {
                if read == BATCH {
                    more = true;
                }
                break;
            }
            if scanned >= SCAN_LIMIT {
                more = true;
                break;
            }
            let last = &entries.last().expect("a full batch").0;
            end = format!("({last}");
        }
        if scanned >= SCAN_LIMIT {
            more = true;
            break;
        }
    }
    found.sort_by_key(|l| std::cmp::Reverse(order_key(&l.id)));
    if found.len() > want {
        found.truncate(want);
        more = true;
    }
    Ok((found, more))
}

/// GET /logs
#[utoipa::path(
    get,
    path = "/api/v1/logs",
    tag = "logs",
    operation_id = "logs_list",
    params(LogQuery),
    responses(
        (status = 200, description = "OK", body = Page<LogLine>),
        (status = "default", description = "An error", body = fcp_api_core::error::ErrorResponse),
    ),
)]
pub async fn list_logs(
    Extension(state): Extension<LogsApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: TenantContext,
    Query(query): Query<LogQuery>,
) -> Result<Json<Page<LogLine>>, ApiError> {
    let storage = state.storage().await?;
    let kinds = kinds(storage).await;
    let filter = Filter::of(&query, &identity, &tenant, &kinds)?;
    let page = PageQuery {
        limit: query.limit,
        cursor: query.cursor.clone(),
    };
    let limit = page.limit()?;
    let before = page.after()?;
    let until_ms = query.until.as_deref().map(rfc3339_ms).transpose()?;
    let (lines, more) = find(storage, &filter, &kinds, before.as_deref(), until_ms, limit).await?;
    let next = more.then(|| lines.last().map(|l| l.id.clone())).flatten();
    Ok(Json(Page::new(lines, next, None)))
}

/// GET /logs/export — JSON Lines, audited.
#[utoipa::path(
    get,
    path = "/api/v1/logs/export",
    tag = "logs",
    operation_id = "logs_export",
    params(LogQuery),
    responses(
        (status = 200, description = "JSON Lines, one LogLine each", content_type = "application/x-ndjson", body = String),
        (status = "default", description = "An error", body = fcp_api_core::error::ErrorResponse),
    ),
)]
pub async fn export_logs(
    Extension(state): Extension<LogsApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: TenantContext,
    Query(query): Query<LogQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let storage = state.storage().await?;
    let kinds = kinds(storage).await;
    let filter = Filter::of(&query, &identity, &tenant, &kinds)?;
    let until_ms = query.until.as_deref().map(rfc3339_ms).transpose()?;
    let (lines, _) = find(storage, &filter, &kinds, None, until_ms, EXPORT_LIMIT).await?;
    let mut body = String::new();
    for line in lines {
        if let Ok(json) = serde_json::to_string(&line) {
            body.push_str(&json);
            body.push('\n');
        }
    }
    Ok((
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "application/x-ndjson"),
            (
                header::CONTENT_DISPOSITION,
                "attachment; filename=\"fcp-logs.jsonl\"",
            ),
        ],
        body,
    ))
}

fn platform_only(identity: &AuthIdentity) -> Result<(), ApiError> {
    if identity.is_platform_admin() {
        Ok(())
    } else {
        Err(ApiError::Forbidden(
            "Only a platform administrator changes a service's level".to_string(),
        ))
    }
}

/// GET /logs/levels — the levels set for now.
#[utoipa::path(
    get,
    path = "/api/v1/logs/levels",
    tag = "logs",
    operation_id = "logs_levels",
    responses(
        (status = 200, description = "OK", body = Vec<LogLevel>),
        (status = "default", description = "An error", body = fcp_api_core::error::ErrorResponse),
    ),
)]
pub async fn list_levels(
    Extension(state): Extension<LogsApiState>,
    Extension(identity): Extension<AuthIdentity>,
) -> Result<Json<Vec<LogLevel>>, ApiError> {
    platform_only(&identity)?;
    let storage = state.storage().await?;
    let mut conn = storage.connection();
    let mut out = Vec::new();
    for service in streams(storage).await? {
        let key = storage.key(&logging::level_key(&service));
        let (level, ttl): (Option<String>, i64) = redis::pipe()
            .cmd("GET")
            .arg(&key)
            .cmd("TTL")
            .arg(&key)
            .query_async(&mut conn)
            .await
            .map_err(|e| ApiError::ServiceUnavailable(format!("Redis: {e}")))?;
        out.push(LogLevel {
            service,
            expires_in_secs: level.as_ref().and(u64::try_from(ttl).ok()),
            level,
        });
    }
    Ok(Json(out))
}

/// PUT /logs/levels/{service} — a level for an hour, or null for back.
#[utoipa::path(
    put,
    path = "/api/v1/logs/levels/{service}",
    tag = "logs",
    operation_id = "logs_set_level",
    params(("service" = String, Path)),
    request_body = SetLogLevel,
    responses(
        (status = 200, description = "OK", body = LogLevel),
        (status = "default", description = "An error", body = fcp_api_core::error::ErrorResponse),
    ),
)]
pub async fn set_level(
    Extension(state): Extension<LogsApiState>,
    Extension(identity): Extension<AuthIdentity>,
    Path(service): Path<String>,
    Json(body): Json<SetLogLevel>,
) -> Result<Json<LogLevel>, ApiError> {
    platform_only(&identity)?;
    if service.is_empty() || service.contains(['*', '?', '[']) {
        return Err(ApiError::BadRequest("Name one service".to_string()));
    }
    let level = match body.level.as_deref().map(str::trim) {
        None | Some("") => None,
        Some(l) if rank(l) > 0 => Some(l.to_ascii_lowercase().replace("warning", "warn")),
        Some(l) => {
            return Err(ApiError::BadRequest(format!(
                "level {l:?}: error, warn, info, debug or trace"
            )))
        }
    };
    let storage = state.storage().await?;
    let key = storage.key(&logging::level_key(&service));
    let mut conn = storage.connection();
    let result: Result<(), _> = match &level {
        Some(l) => {
            redis::cmd("SET")
                .arg(&key)
                .arg(l)
                .arg("EX")
                .arg(logging::LEVEL_SECS)
                .query_async(&mut conn)
                .await
        }
        None => redis::cmd("DEL").arg(&key).query_async(&mut conn).await,
    };
    result.map_err(|e| ApiError::ServiceUnavailable(format!("Redis: {e}")))?;
    Ok(Json(LogLevel {
        service,
        expires_in_secs: level.as_ref().map(|_| logging::LEVEL_SECS),
        level,
    }))
}

/// The logs routes, under `/api/v1`.
pub fn logs_router<S>(state: LogsApiState) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    TenantContext: axum::extract::FromRequestParts<S>,
{
    Router::new()
        .route("/logs", get(list_logs))
        .route("/logs/export", get(export_logs))
        .route("/logs/levels", get(list_levels))
        .route("/logs/levels/:service", put(set_level))
        .layer(Extension(state))
}

/// The logs routes' part of the contract (`crate::api_docs`).
#[derive(utoipa::OpenApi)]
#[openapi(paths(list_logs, export_logs, list_levels, set_level))]
pub struct LogsDoc;

#[cfg(test)]
mod tests {
    use super::*;

    fn record(level: &str, tenant: Option<&str>, message: &str) -> LogRecord {
        LogRecord {
            ts: 1,
            level: level.into(),
            target: "fcp_sip::handler".into(),
            message: message.into(),
            fields: BTreeMap::new(),
            tenant_id: tenant.map(String::from),
            call_id: None,
        }
    }

    #[test]
    fn a_filter_keeps_its_level_tenant_and_text() {
        let filter = Filter {
            services: None,
            min_rank: rank("warn"),
            tenant: Some(Some("t-1".into())),
            call_id: None,
            q: Some("busy".into()),
            since_ms: None,
        };
        assert!(filter.keeps(&record("WARN", Some("t-1"), "Callee BUSY")));
        assert!(filter.keeps(&record("ERROR", Some("t-1"), "busy here")));
        assert!(
            !filter.keeps(&record("INFO", Some("t-1"), "busy")),
            "below the level"
        );
        assert!(
            !filter.keeps(&record("WARN", Some("t-2"), "busy")),
            "another tenant"
        );
        assert!(
            !filter.keeps(&record("WARN", None, "busy")),
            "the platform's"
        );
        assert!(
            !filter.keeps(&record("WARN", Some("t-1"), "idle")),
            "no text"
        );
        let platform = Filter {
            tenant: Some(None),
            q: None,
            ..filter
        };
        assert!(platform.keeps(&record("WARN", None, "x")));
        assert!(!platform.keeps(&record("WARN", Some("t-1"), "x")));
    }

    #[test]
    fn stream_ids_order_across_streams() {
        let mut ids = vec!["10-0", "9-5", "10-2", "100-0"];
        ids.sort_by_key(|id| std::cmp::Reverse(order_key(id)));
        assert_eq!(ids, vec!["100-0", "10-2", "10-0", "9-5"]);
    }
}
