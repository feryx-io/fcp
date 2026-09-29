//! One shape for every response under `/api/v1` (docs/DEVELOPER_SURFACE.md
//! §3, phase 2): errors as JSON whoever produced them, and a request id on
//! every response.

use crate::error::ErrorResponse;
use axum::{
    body::Body,
    extract::Request,
    http::{header, HeaderValue},
    middleware::Next,
    response::Response,
};

/// The header carrying a request's id: the caller's when it sent one, else
/// one made here. It is on every response, so a caller can quote it.
pub const REQUEST_ID_HEADER: &str = "x-request-id";

/// How much of a plain-text error body is kept as the JSON error's detail.
const MAX_DETAIL_BYTES: usize = 16 * 1024;

/// Rewrite an error answered as plain text (or with no body at all) as the
/// API's JSON error: axum's own rejections of a malformed body, path or
/// query, a method the route does not take, a path nothing routes. A JSON
/// error passes untouched, as does anything outside `/api/v1`.
pub async fn json_errors(req: Request, next: Next) -> Response {
    let api = req.uri().path().starts_with("/api/v1/") || req.uri().path() == "/metrics";
    let response = next.run(req).await;
    let status = response.status();
    if !api || !(status.is_client_error() || status.is_server_error()) {
        return response;
    }
    let is_json = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|ct| ct.starts_with("application/json"));
    if is_json {
        return response;
    }
    let (mut parts, body) = response.into_parts();
    let bytes = axum::body::to_bytes(body, MAX_DETAIL_BYTES)
        .await
        .unwrap_or_default();
    let text = String::from_utf8_lossy(&bytes).trim().to_string();
    let error = ErrorResponse::for_status(status, (!text.is_empty()).then_some(text));
    let body = serde_json::to_vec(&error).unwrap_or_default();
    parts.headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    parts.headers.remove(header::CONTENT_LENGTH);
    Response::from_parts(parts, Body::from(body))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{http::StatusCode, routing::post, Json, Router};
    use tower::ServiceExt;

    async fn call(app: Router, req: axum::http::Request<Body>) -> (StatusCode, serde_json::Value) {
        let res = app.oneshot(req).await.unwrap();
        let status = res.status();
        assert_eq!(
            res.headers().get(header::CONTENT_TYPE).unwrap(),
            "application/json"
        );
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, serde_json::from_slice(&bytes).unwrap())
    }

    fn app() -> Router {
        Router::new()
            .route(
                "/api/v1/things",
                post(|Json(_): Json<serde_json::Value>| async { "ok" }),
            )
            .layer(axum::middleware::from_fn(json_errors))
    }

    #[tokio::test]
    async fn a_malformed_body_is_a_json_error() {
        let (status, body) = call(
            app(),
            axum::http::Request::post("/api/v1/things")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from("{not json"))
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["code"], "bad_request");
        assert!(body["details"].as_str().unwrap().contains("JSON"), "{body}");
    }

    #[tokio::test]
    async fn a_wrong_method_and_an_unknown_path_are_json_errors() {
        let (status, body) = call(
            app(),
            axum::http::Request::get("/api/v1/things")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
        assert_eq!(body["code"], "method_not_allowed");
        let (status, body) = call(
            app(),
            axum::http::Request::get("/api/v1/nothing")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body["error"], "Not Found");
    }
}
