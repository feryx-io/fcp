//! FCP Admin UI - Embedded React SPA served by Axum.
//!
//! This crate embeds the built frontend assets into the binary using `include_dir`,
//! and exports an Axum router that serves the SPA at `/admin/`.

use axum::{
    body::Body,
    extract::Path,
    http::{header, StatusCode},
    response::Response,
    routing::get,
    Router,
};
use include_dir::{include_dir, Dir};

/// Embedded frontend build output.
static DIST_DIR: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/dist");

/// Returns an Axum router that serves the admin SPA.
///
/// - `/admin/assets/*` serves static files (JS, CSS, images)
/// - `/admin/*` serves `index.html` for client-side routing
pub fn admin_ui_router<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    Router::new()
        .route("/admin/assets/*path", get(serve_asset))
        .route("/admin", get(serve_index))
        .route("/admin/", get(serve_index))
        .route("/admin/*path", get(serve_spa_fallback))
        // Login and password change pages at root
        .route("/login", get(serve_index))
        .route("/change-password", get(serve_index))
        // User portal (same SPA, different route prefix)
        .route("/user", get(serve_index))
        .route("/user/", get(serve_index))
        .route("/user/*path", get(serve_spa_fallback))
}

/// Security headers applied to all responses (including errors).
const SECURITY_HEADERS: [(&str, &str); 4] = [
    ("X-Content-Type-Options", "nosniff"),
    ("Referrer-Policy", "strict-origin-when-cross-origin"),
    ("X-Frame-Options", "DENY"),
    ("Content-Security-Policy", "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; media-src 'self' blob:; connect-src *; font-src 'self'; object-src 'none'; frame-ancestors 'none'; base-uri 'self'; form-action 'self'"),
];

fn add_security_headers(builder: axum::http::response::Builder) -> axum::http::response::Builder {
    let mut b = builder;
    for (name, value) in &SECURITY_HEADERS {
        b = b.header(*name, *value);
    }
    b
}

/// Build an error response with security headers applied.
fn error_response(status: StatusCode, body: &str) -> Response {
    let builder = Response::builder().status(status);
    add_security_headers(builder)
        .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .body(Body::from(body.to_string()))
        .unwrap()
}

/// Serve a static asset from the embedded dist directory.
async fn serve_asset(Path(path): Path<String>) -> Response {
    let asset_path = format!("assets/{}", path);
    match DIST_DIR.get_file(&asset_path) {
        Some(file) => {
            let mime = mime_guess::from_path(&asset_path)
                .first_or_octet_stream()
                .to_string();

            let builder = Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, mime)
                // Cache static assets aggressively (they have content hashes in filenames)
                .header(header::CACHE_CONTROL, "public, max-age=31536000, immutable");

            add_security_headers(builder)
                .body(Body::from(file.contents().to_vec()))
                .unwrap()
        }
        None => error_response(StatusCode::NOT_FOUND, "Not Found"),
    }
}

/// Serve the SPA index.html.
async fn serve_index() -> Response {
    serve_index_html()
}

/// Serve index.html for any unmatched admin route (SPA client-side routing).
async fn serve_spa_fallback(Path(_path): Path<String>) -> Response {
    serve_index_html()
}

/// Helper to serve the index.html file.
fn serve_index_html() -> Response {
    match DIST_DIR.get_file("index.html") {
        Some(file) => {
            let content = std::str::from_utf8(file.contents()).unwrap_or("<!-- error -->");

            let builder = Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
                // index.html must not be cached (SPA routing, deploys)
                .header(header::CACHE_CONTROL, "no-cache, no-store, must-revalidate");

            add_security_headers(builder)
                .body(Body::from(content.to_string()))
                .unwrap()
        }
        None => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Admin UI not built. Run: cd crates/fcp-admin-ui/ui && npm run build",
        ),
    }
}
