//! The admin API's contract, served (docs/DEVELOPER_SURFACE.md §3, phase 3):
//! the spec at `/api/v1/openapi.json`, and a reference page at
//! `/admin/api` that renders it. Neither needs a token: the contract holds
//! no secret, and a developer reads it before they have one.

use axum::{
    http::header,
    response::{Html, IntoResponse},
    routing::get,
    Router,
};
use utoipa::OpenApi;

/// The whole contract: the core API and this server's cluster routes.
pub fn spec() -> utoipa::openapi::OpenApi {
    let mut doc = fcp_api_core::openapi::ApiDoc::openapi();
    doc.merge(crate::cluster_api::ClusterDoc::openapi());
    doc
}

/// `GET /api/v1/openapi.json` and `GET /admin/api`.
pub fn router<S: Clone + Send + Sync + 'static>() -> Router<S> {
    let json = fcp_api_core::openapi::json(&spec());
    Router::new()
        // Nested like the core API's other public routes, beside its nest.
        .nest(
            "/api/v1",
            Router::new().route(
                "/openapi.json",
                get(move || {
                    let json = json.clone();
                    async move { ([(header::CONTENT_TYPE, "application/json")], json) }
                }),
            ),
        )
        .route("/admin/api", get(reference))
}

/// The reference page: Scalar's renderer over the spec above.
async fn reference() -> impl IntoResponse {
    (
        [(
            header::CONTENT_SECURITY_POLICY,
            "default-src 'self'; script-src 'self' https://cdn.jsdelivr.net 'unsafe-inline'; \
             style-src 'self' https://cdn.jsdelivr.net https://fonts.googleapis.com 'unsafe-inline'; \
             font-src 'self' https://fonts.gstatic.com https://cdn.jsdelivr.net data:; \
             img-src 'self' data:; connect-src 'self'",
        )],
        Html(REFERENCE),
    )
}

const REFERENCE: &str = r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>FCP admin API</title>
</head>
<body>
  <script id="api-reference" data-url="/api/v1/openapi.json"></script>
  <script src="https://cdn.jsdelivr.net/npm/@scalar/api-reference@1"></script>
</body>
</html>
"#;

#[cfg(test)]
mod tests {
    use super::*;

    /// `docs/openapi.json` is the spec. When a route or a type changes,
    /// run this with `FCP_WRITE_OPENAPI=1` and commit the file.
    #[test]
    fn docs_openapi_json_is_the_spec() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/openapi.json");
        let generated = fcp_api_core::openapi::json(&spec());
        if std::env::var_os("FCP_WRITE_OPENAPI").is_some() {
            std::fs::write(path, &generated).expect("write docs/openapi.json");
            return;
        }
        let committed = std::fs::read_to_string(path).unwrap_or_default();
        assert!(
            committed == generated,
            "docs/openapi.json is not the spec the code makes: run \
             `FCP_WRITE_OPENAPI=1 cargo test -p fcp-admin docs_openapi_json_is_the_spec` \
             and commit it"
        );
    }

    #[test]
    fn the_cluster_routes_are_in_the_contract() {
        let spec = serde_json::to_value(spec()).unwrap();
        for path in [
            "/api/v1/cluster/nodes",
            "/api/v1/cluster/stats",
            "/api/v1/cluster/sites",
        ] {
            assert!(spec["paths"][path]["get"].is_object(), "{path}");
        }
    }

    #[tokio::test]
    async fn the_spec_and_the_page_are_served_without_a_token() {
        use tower::ServiceExt;
        let app: Router = router();
        for (path, kind) in [
            ("/api/v1/openapi.json", "application/json"),
            ("/admin/api", "text/html"),
        ] {
            let res = app
                .clone()
                .oneshot(
                    axum::http::Request::get(path)
                        .body(axum::body::Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(res.status(), 200, "{path}");
            let ct = res.headers()[header::CONTENT_TYPE]
                .to_str()
                .unwrap()
                .to_string();
            assert!(ct.starts_with(kind), "{path}: {ct}");
        }
    }
}
