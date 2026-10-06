//! Stand-ins for vendors' redirection services (`docs/PROVISIONING.md`
//! §10): Yealink RPS's JSON API, checking each request's signature as its
//! guide describes, and Poly ZTP's XML API, checking the account in each.
//! Both keep the devices registered with them, for a test to read.

use axum::body::Bytes;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::Engine;
use hmac::{Hmac, Mac};
use serde_json::{json, Value};
use sha2::Sha256;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// A device as Yealink RPS keeps it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RpsDevice {
    pub id: String,
    pub mac: String,
    pub url: String,
    pub auth_name: Option<String>,
    pub password: Option<String>,
    pub remark: String,
}

struct RpsState {
    key_id: String,
    secret: String,
    devices: Mutex<HashMap<String, RpsDevice>>,
    deleted: Mutex<Vec<String>>,
    /// How long each answer waits, as a slow service's does.
    delay_ms: std::sync::atomic::AtomicU64,
}

/// A stand-in Yealink RPS: `url()` is what `[api.redirection]
/// yealink_rps_url` names.
pub struct StandInRps {
    url: String,
    state: Arc<RpsState>,
}

fn refused(code: u16, msg: &str) -> Json<Value> {
    Json(
        json!({"ret": -1, "data": null, "error": {"msg": msg, "errorCode": code, "fieldErrors": []}}),
    )
}

fn header<'a>(headers: &'a HeaderMap, name: &str) -> &'a str {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
}

/// Check a request as RPS does (its guide, §1.3): the key, the body's MD5,
/// and the HMAC-SHA256 signature over method, headers, path and query.
fn rps_signed(
    s: &RpsState,
    method: &str,
    path: &str,
    query: &[(String, String)],
    headers: &HeaderMap,
    body: Option<&[u8]>,
) -> bool {
    if header(headers, "x-ca-key") != s.key_id {
        return false;
    }
    let mut to_sign = format!("{method}\n");
    if let Some(body) = body {
        let md5 = base64::engine::general_purpose::STANDARD.encode(md5::compute(body).0);
        if header(headers, "content-md5") != md5 {
            return false;
        }
        to_sign.push_str(&format!("Content-MD5:{md5}\n"));
    }
    to_sign.push_str(&format!(
        "X-Ca-Key:{}\nX-Ca-Nonce:{}\nX-Ca-Timestamp:{}\n{}",
        header(headers, "x-ca-key"),
        header(headers, "x-ca-nonce"),
        header(headers, "x-ca-timestamp"),
        path.trim_start_matches('/')
    ));
    if body.is_none() && !query.is_empty() {
        let mut q = query.to_vec();
        q.sort();
        let joined: Vec<String> = q.iter().map(|(k, v)| format!("{k}={v}")).collect();
        to_sign.push('\n');
        to_sign.push_str(&joined.join("&"));
    }
    let mut mac = Hmac::<Sha256>::new_from_slice(s.secret.as_bytes()).unwrap();
    mac.update(to_sign.as_bytes());
    let want = base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes());
    header(headers, "x-ca-signature") == want
}

impl StandInRps {
    /// An RPS that accepts the AccessKey `key_id` and `secret`.
    pub async fn start(key_id: &str, secret: &str) -> anyhow::Result<Self> {
        let state = Arc::new(RpsState {
            key_id: key_id.to_string(),
            secret: secret.to_string(),
            devices: Mutex::new(HashMap::new()),
            deleted: Mutex::new(Vec::new()),
            delay_ms: std::sync::atomic::AtomicU64::new(0),
        });
        let body_route = |path: &'static str| {
            move |State(s): State<Arc<RpsState>>, headers: HeaderMap, body: Bytes| async move {
                // Up to the delay, at random: requests overtake each other.
                let delay = s.delay_ms.load(std::sync::atomic::Ordering::Relaxed);
                if delay > 0 {
                    let wait = rand::random::<u64>() % (delay + 1);
                    tokio::time::sleep(std::time::Duration::from_millis(wait)).await;
                }
                if !rps_signed(&s, "POST", path, &[], &headers, Some(&body)) {
                    return refused(401, "Error user key.");
                }
                let req: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
                let mut devices = s.devices.lock().unwrap();
                match path {
                    "/api/open/v1/device/add" => {
                        let mut added = Vec::new();
                        for mac in req["macs"].as_array().into_iter().flatten() {
                            let mac = mac.as_str().unwrap_or("").to_ascii_lowercase();
                            if devices.values().any(|d| d.mac == mac) {
                                return refused(409, "device.exists");
                            }
                            let device = RpsDevice {
                                id: uuid::Uuid::new_v4().simple().to_string(),
                                mac,
                                url: req["uniqueServerUrl"].as_str().unwrap_or("").to_string(),
                                auth_name: req["authName"].as_str().map(String::from),
                                password: req["password"].as_str().map(String::from),
                                remark: req["remark"].as_str().unwrap_or("").to_string(),
                            };
                            added.push(json!({"id": device.id, "mac": device.mac}));
                            devices.insert(device.id.clone(), device);
                        }
                        Json(json!({"ret": 1, "data": added, "error": null}))
                    }
                    "/api/open/v1/device/list" => {
                        let key = req["key"].as_str().unwrap_or("").to_ascii_lowercase();
                        let found: Vec<Value> = devices
                            .values()
                            .filter(|d| d.mac.contains(&key))
                            .map(|d| json!({"id": d.id, "mac": d.mac}))
                            .collect();
                        Json(
                            json!({"ret": 1, "data": {"total": found.len(), "data": found}, "error": null}),
                        )
                    }
                    "/api/open/v1/device/edit" => {
                        let id = req["id"].as_str().unwrap_or("");
                        match devices.get_mut(id) {
                            Some(d) => {
                                if let Some(url) = req["uniqueServerUrl"].as_str() {
                                    d.url = url.to_string();
                                }
                                d.auth_name = req["authName"].as_str().map(String::from);
                                d.password = req["password"].as_str().map(String::from);
                                Json(json!({"ret": 1, "data": {"id": id}, "error": null}))
                            }
                            None => refused(404, "device.not.found"),
                        }
                    }
                    "/api/open/v1/device/delete" => {
                        for id in req["ids"].as_array().into_iter().flatten() {
                            if let Some(d) = devices.remove(id.as_str().unwrap_or("")) {
                                s.deleted.lock().unwrap().push(d.mac);
                            }
                        }
                        Json(json!({"ret": 0, "data": null, "error": null}))
                    }
                    _ => refused(404, "not.found"),
                }
            }
        };
        let app = Router::new()
            .route("/api/open/v1/device/add", post(body_route("/api/open/v1/device/add")))
            .route("/api/open/v1/device/list", post(body_route("/api/open/v1/device/list")))
            .route("/api/open/v1/device/edit", post(body_route("/api/open/v1/device/edit")))
            .route("/api/open/v1/device/delete", post(body_route("/api/open/v1/device/delete")))
            .route(
                "/api/open/v1/device/checkMac",
                get(
                    |State(s): State<Arc<RpsState>>,
                     headers: HeaderMap,
                     Query(q): Query<Vec<(String, String)>>| async move {
                        if !rps_signed(&s, "GET", "/api/open/v1/device/checkMac", &q, &headers, None) {
                            return refused(401, "Error user key.");
                        }
                        Json(json!({"ret": 1, "data": {"existed": false, "self": null}, "error": null}))
                    },
                ),
            )
            .with_state(Arc::clone(&state));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("http://{}", listener.local_addr()?);
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        Ok(Self { url, state })
    }

    /// Answer each request after a random wait of up to `delay`, as a slow
    /// service does: two requests about one phone overlap, and the later
    /// one may arrive first.
    pub fn slow(&self, delay: std::time::Duration) {
        self.state.delay_ms.store(
            delay.as_millis() as u64,
            std::sync::atomic::Ordering::Relaxed,
        );
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    /// The device RPS has for a MAC.
    pub fn device(&self, mac: &str) -> Option<RpsDevice> {
        self.state
            .devices
            .lock()
            .unwrap()
            .values()
            .find(|d| d.mac == mac)
            .cloned()
    }

    /// The MACs deleted, in order.
    pub fn deleted(&self) -> Vec<String> {
        self.state.deleted.lock().unwrap().clone()
    }
}

/// A device as Poly ZTP associates it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZtpDevice {
    pub account_id: String,
    pub profile: String,
}

struct ZtpState {
    user: String,
    password: String,
    accounts: Vec<String>,
    devices: Mutex<HashMap<String, ZtpDevice>>,
}

/// A stand-in Poly ZTP: `url()` is what `[api.redirection] poly_ztp_url`
/// names.
pub struct StandInZtp {
    url: String,
    state: Arc<ZtpState>,
}

fn attribute(document: &str, element: &str, name: &str) -> Option<String> {
    let start = document.find(&format!("<{element}"))?;
    let tag = &document[start..start + document[start..].find('>')?];
    let at = tag.find(&format!("{name}=\""))? + name.len() + 2;
    let end = tag[at..].find('"')?;
    Some(
        tag[at..at + end]
            .replace("&amp;", "&")
            .replace("&quot;", "\""),
    )
}

fn text(document: &str, element: &str) -> Option<String> {
    let open = format!("<{element}>");
    let start = document.find(&open)? + open.len();
    let end = document[start..].find(&format!("</{element}>"))?;
    Some(document[start..start + end].to_string())
}

fn ztp_answer(code: &str, message: &str) -> (StatusCode, String) {
    (
        StatusCode::OK,
        format!(
            "<?xml version='1.0'?><response xmlns='http://schemas.alopa.com/Inbound'>\
             <status ErrorCode='{code}' ErrorMessage='{message}'/><response-details/></response>"
        ),
    )
}

impl StandInZtp {
    /// A ZTP accepting the API account `user`/`password`, with the
    /// Reference IDs `accounts`.
    pub async fn start(user: &str, password: &str, accounts: &[&str]) -> anyhow::Result<Self> {
        let state = Arc::new(ZtpState {
            user: user.to_string(),
            password: password.to_string(),
            accounts: accounts.iter().map(|a| a.to_string()).collect(),
            devices: Mutex::new(HashMap::new()),
        });
        let app = Router::new()
            .route(
                "/",
                post(|State(s): State<Arc<ZtpState>>, body: String| async move {
                    if attribute(&body, "request", "userid").as_deref() != Some(&s.user)
                        || attribute(&body, "request", "password").as_deref() != Some(&s.password)
                    {
                        return ztp_answer("1000", "Authentication failed.");
                    }
                    let mut devices = s.devices.lock().unwrap();
                    if body.contains("<add-sip-device") {
                        let account =
                            attribute(&body, "add-sip-device", "account-id").unwrap_or_default();
                        let mac = text(&body, "deviceId").unwrap_or_default();
                        if !s.accounts.contains(&account) {
                            return ztp_answer("2001", "Account not found.");
                        }
                        if devices.contains_key(&mac) {
                            return ztp_answer("3001", "Device already exists.");
                        }
                        devices.insert(
                            mac,
                            ZtpDevice {
                                account_id: account,
                                profile: text(&body, "templateCriteria").unwrap_or_default(),
                            },
                        );
                        ztp_answer("0000", "Successful.")
                    } else if body.contains("<delete-sip-device") {
                        let mac = text(&body, "deviceId").unwrap_or_default();
                        match devices.remove(&mac) {
                            Some(_) => ztp_answer("0000", "Successful."),
                            None => ztp_answer("3002", "Device not found."),
                        }
                    } else if body.contains("<query-sip-device") {
                        let account =
                            attribute(&body, "query-sip-device", "accountid").unwrap_or_default();
                        if s.accounts.contains(&account) {
                            ztp_answer("0000", "Successful.")
                        } else {
                            ztp_answer("2001", "Account not found.")
                        }
                    } else {
                        ztp_answer("9999", "Unknown command.")
                    }
                }),
            )
            .with_state(Arc::clone(&state));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("http://{}/", listener.local_addr()?);
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        Ok(Self { url, state })
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    pub fn device(&self, mac: &str) -> Option<ZtpDevice> {
        self.state.devices.lock().unwrap().get(mac).cloned()
    }
}
