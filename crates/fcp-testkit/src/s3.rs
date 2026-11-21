//! A stand-in for an S3-compatible bucket, path-style: `PUT`, `GET`,
//! `HEAD` and `DELETE` of `/<bucket>/<key>`, kept in memory with the
//! content type and `Cache-Control` each object was put with. Every request
//! must carry a SigV4 `Authorization` header naming the access key; the
//! signature itself is not checked (`fcp-cdr/tests/s3.rs` does that
//! against a real store). For tests that run where no store does.

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::any;
use axum::Router;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

pub const ACCESS_KEY_ID: &str = "STANDINKEY";
pub const SECRET_ACCESS_KEY: &str = "stand-in-secret";

/// An object the stand-in holds.
#[derive(Debug, Clone)]
pub struct Object {
    pub body: Vec<u8>,
    pub content_type: Option<String>,
    pub cache_control: Option<String>,
}

struct Shared {
    bucket: String,
    objects: Mutex<BTreeMap<String, Object>>,
}

pub struct StandInS3 {
    url: String,
    state: Arc<Shared>,
}

impl StandInS3 {
    /// A bucket named `bucket`, served on a free loopback port.
    pub async fn start(bucket: &str) -> anyhow::Result<Self> {
        let state = Arc::new(Shared {
            bucket: bucket.to_string(),
            objects: Mutex::new(BTreeMap::new()),
        });
        let app = Router::new()
            .route("/:bucket/*key", any(object))
            .with_state(Arc::clone(&state));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("http://{}", listener.local_addr()?);
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        Ok(Self { url, state })
    }

    /// The endpoint, for `endpoint` with `path_style = true`.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// The object at `key`.
    pub fn object(&self, key: &str) -> Option<Object> {
        self.state.objects.lock().unwrap().get(key).cloned()
    }

    /// Every key, in order.
    pub fn keys(&self) -> Vec<String> {
        self.state.objects.lock().unwrap().keys().cloned().collect()
    }
}

async fn object(
    State(state): State<Arc<Shared>>,
    Path((bucket, key)): Path<(String, String)>,
    method: Method,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let header = |name: &str| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string)
    };
    let signed = header("authorization").is_some_and(|a| {
        a.starts_with("AWS4-HMAC-SHA256 ") && a.contains(&format!("Credential={ACCESS_KEY_ID}/"))
    });
    if !signed {
        return (
            StatusCode::FORBIDDEN,
            "<Error><Code>AccessDenied</Code></Error>",
        )
            .into_response();
    }
    if bucket != state.bucket {
        return (
            StatusCode::NOT_FOUND,
            "<Error><Code>NoSuchBucket</Code></Error>",
        )
            .into_response();
    }
    let mut objects = state.objects.lock().unwrap();
    match method {
        Method::PUT => {
            objects.insert(
                key,
                Object {
                    body: body.to_vec(),
                    content_type: header("content-type"),
                    cache_control: header("cache-control"),
                },
            );
            StatusCode::OK.into_response()
        }
        Method::GET | Method::HEAD => match objects.get(&key) {
            Some(o) => {
                let mut response = if method == Method::GET {
                    o.body.clone().into_response()
                } else {
                    StatusCode::OK.into_response()
                };
                if let Some(t) = &o.content_type {
                    if let Ok(v) = t.parse() {
                        response.headers_mut().insert("content-type", v);
                    }
                }
                response
            }
            None => (
                StatusCode::NOT_FOUND,
                "<Error><Code>NoSuchKey</Code></Error>",
            )
                .into_response(),
        },
        Method::DELETE => {
            objects.remove(&key);
            StatusCode::NO_CONTENT.into_response()
        }
        _ => StatusCode::METHOD_NOT_ALLOWED.into_response(),
    }
}
