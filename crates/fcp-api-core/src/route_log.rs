//! The operations a test run reaches (`docs/DEEP_TESTING.md` D1).
//!
//! With `FCP_ROUTE_LOG` naming a file, every request that matched an
//! `/api/v1` route appends `METHOD TEMPLATE STATUS` to it, once per
//! method, template and status in a process. Unset, as in any deployment,
//! the layer only reads a cached `None`.
//!
//! It is server-side so it sees every client alike: the Postgres suite's
//! in-process router, the admin binaries other suites start, and the SDKs'
//! examples run against them. `scripts/check-routes-called.py` compares
//! the union of a run's logs with the contract.

use std::collections::HashSet;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::sync::{Mutex, OnceLock};

use axum::{
    extract::{MatchedPath, Request},
    middleware::Next,
    response::Response,
};

/// The environment variable naming the log file.
pub const ENV: &str = "FCP_ROUTE_LOG";

/// A request carrying this header is not logged: the D3 probes call every
/// route with ids that name nothing to see who is refused, and reaching a
/// handler that way is not testing it.
pub const PROBE_HEADER: &str = "x-fcp-route-probe";

struct Log {
    file: File,
    seen: HashSet<String>,
}

fn log() -> Option<&'static Mutex<Log>> {
    static LOG: OnceLock<Option<Mutex<Log>>> = OnceLock::new();
    LOG.get_or_init(|| {
        let path = std::env::var_os(ENV)?;
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|e| tracing::warn!(error = %e, "{ENV} cannot be opened; routes not logged"))
            .ok()?;
        Some(Mutex::new(Log {
            file,
            seen: HashSet::new(),
        }))
    })
    .as_ref()
}

/// Middleware: note the matched route and the status it answered.
pub async fn record(req: Request, next: Next) -> Response {
    let Some(log) = log() else {
        return next.run(req).await;
    };
    if req.headers().contains_key(PROBE_HEADER) {
        return next.run(req).await;
    }
    let method = req.method().clone();
    let template = req
        .extensions()
        .get::<MatchedPath>()
        .map(|m| m.as_str().to_string());
    let response = next.run(req).await;
    if let Some(template) = template {
        let line = format!("{method} {template} {}\n", response.status().as_u16());
        let mut log = log.lock().unwrap_or_else(|e| e.into_inner());
        if log.seen.insert(line.clone()) {
            // One write per line, appended: test processes running at once
            // share the file without interleaving.
            let _ = log.file.write_all(line.as_bytes());
        }
    }
    response
}
