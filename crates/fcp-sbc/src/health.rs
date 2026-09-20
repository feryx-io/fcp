//! HTTP health, readiness, and metrics endpoints for the SBC.

use crate::metrics::SbcRuntimeMetrics;
use crate::recording_store::RecordingStore;
use crate::stir_shaken::{StirMetrics, StirVerifier};
use axum::body::Body;
use axum::extract::Request;
use axum::http::header;
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::{extract::State, routing::get, Json, Router};
use fcp_sbc_core::admission::AdmissionController;
use fcp_sbc_core::trunk_health::TrunkHealthMonitor;
use fcp_sbc_core::trunk_registration::TrunkRegistrar;
use std::fmt::Write;
use std::sync::Arc;
use tracing::info;

/// Shared state for the HTTP health server and admin API.
pub struct HealthState {
    pub admission: Arc<AdmissionController>,
    pub health_monitor: Arc<TrunkHealthMonitor>,
    pub metrics: Arc<SbcRuntimeMetrics>,
    pub call_manager: Arc<fcp_b2bua_core::CallManager>,
    pub trunk_configs: Vec<fcp_sbc_core::TrunkConfig>,
    pub trunk_registrar: Option<Arc<TrunkRegistrar>>,
    pub uac: Option<Arc<fcp_sip::uac::SipUserAgentClient>>,
    pub dialog_manager: Arc<sip_dialog::DialogManager>,
    pub recording_store: Option<Arc<RecordingStore>>,
    pub quality_tracker: Option<Arc<fcp_sbc_core::TrunkQualityTracker>>,
    pub lcr_routes: Vec<fcp_sbc_core::LcrRouteEntry>,
    pub stir_metrics: Option<Arc<StirMetrics>>,
    pub stir_verifier: Option<Arc<StirVerifier>>,
    pub api_token: Option<String>,
}

fn extract_bearer_token(value: &str) -> Option<&str> {
    let (scheme, token) = value.split_once(' ')?;
    if scheme.eq_ignore_ascii_case("bearer") {
        Some(token.trim())
    } else {
        None
    }
}

fn constant_time_compare(left: &str, right: &str) -> bool {
    if left.len() != right.len() {
        return false;
    }

    let mut diff = 0u8;
    for (a, b) in left.as_bytes().iter().zip(right.as_bytes()) {
        diff |= a ^ b;
    }
    diff == 0
}

async fn require_api_token(
    State(state): State<Arc<HealthState>>,
    request: Request<Body>,
    next: Next,
) -> Response {
    let Some(expected_token) = state.api_token.as_deref() else {
        return next.run(request).await;
    };

    let provided_token = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(extract_bearer_token);

    match provided_token {
        Some(token) if constant_time_compare(token, expected_token) => next.run(request).await,
        _ => (
            axum::http::StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"error": "Invalid or missing authentication token"})),
        )
            .into_response(),
    }
}

fn public_router() -> Router<Arc<HealthState>> {
    Router::new()
        .route("/health", get(health_check))
        .route("/ready", get(readiness_check))
}

fn protected_router() -> Router<Arc<HealthState>> {
    Router::new()
        .route("/metrics", get(metrics_snapshot))
        .nest("/admin", crate::admin::admin_router())
}

/// Build the health/readiness/metrics router with admin API.
pub fn health_router(state: Arc<HealthState>) -> Router {
    public_router()
        .merge(protected_router().layer(middleware::from_fn_with_state(
            Arc::clone(&state),
            require_api_token,
        )))
        .with_state(state)
}

/// Run the HTTP health server.
pub async fn run_health_server(
    bind_addr: std::net::SocketAddr,
    state: Arc<HealthState>,
) -> anyhow::Result<()> {
    let app = health_router(state);
    let listener = tokio::net::TcpListener::bind(bind_addr).await?;
    info!(%bind_addr, "SBC health server started");
    axum::serve(listener, app).await?;
    Ok(())
}

async fn health_check() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "healthy",
        "service": "fcp-sbc"
    }))
}

async fn readiness_check(State(state): State<Arc<HealthState>>) -> Json<serde_json::Value> {
    let ready = state.health_monitor.any_healthy();
    let status = if ready { "ready" } else { "not_ready" };
    Json(serde_json::json!({
        "status": status,
        "service": "fcp-sbc",
        "trunks_healthy": ready,
    }))
}

async fn metrics_snapshot(State(state): State<Arc<HealthState>>) -> impl IntoResponse {
    let snap = state.metrics.snapshot();
    let mut buf = String::with_capacity(1024);

    // Gauges (current values)
    prom_gauge(&mut buf, "fcp_sbc_up", "SBC server status (1 = up)", 1);
    prom_gauge(
        &mut buf,
        "fcp_sbc_current_cps",
        "Current calls per second",
        snap.current_cps,
    );
    prom_gauge(
        &mut buf,
        "fcp_sbc_peak_cps",
        "Peak calls per second observed since startup",
        snap.peak_cps,
    );
    prom_gauge(
        &mut buf,
        "fcp_sbc_concurrent_calls",
        "Current concurrent active calls",
        snap.concurrent_calls,
    );
    prom_gauge(
        &mut buf,
        "fcp_sbc_concurrent_registrations",
        "Current concurrent registrations",
        snap.concurrent_registrations,
    );

    // Counters (monotonically increasing totals)
    prom_counter(
        &mut buf,
        "fcp_sbc_calls_attempted_total",
        "Total call attempts since startup",
        snap.total_calls_attempted,
    );
    prom_counter(
        &mut buf,
        "fcp_sbc_calls_completed_total",
        "Total calls completed (BYE exchanged)",
        snap.total_calls_completed,
    );
    prom_counter(
        &mut buf,
        "fcp_sbc_calls_rejected_total",
        "Total calls rejected (4xx/5xx before answer)",
        snap.total_calls_rejected,
    );
    prom_counter(
        &mut buf,
        "fcp_sbc_admission_rejections_total",
        "Total calls rejected by admission control",
        snap.total_admission_rejections,
    );
    prom_counter(
        &mut buf,
        "fcp_sbc_rate_limit_rejections_total",
        "Total calls rejected by rate limiting",
        snap.total_rate_limit_rejections,
    );
    prom_counter(
        &mut buf,
        "fcp_sbc_security_rejections_total",
        "Total calls rejected by security policy",
        snap.total_security_rejections,
    );

    // Per-trunk quality metrics (if quality tracker is available)
    if let Some(ref tracker) = state.quality_tracker {
        let _ = writeln!(buf);
        let _ = writeln!(
            buf,
            "# HELP fcp_sbc_trunk_asr Answer seizure ratio per trunk (0.0-1.0)"
        );
        let _ = writeln!(buf, "# TYPE fcp_sbc_trunk_asr gauge");
        let _ = writeln!(
            buf,
            "# HELP fcp_sbc_trunk_acd_secs Average call duration per trunk"
        );
        let _ = writeln!(buf, "# TYPE fcp_sbc_trunk_acd_secs gauge");
        let _ = writeln!(
            buf,
            "# HELP fcp_sbc_trunk_pdd_ms Average post-dial delay per trunk"
        );
        let _ = writeln!(buf, "# TYPE fcp_sbc_trunk_pdd_ms gauge");
        let _ = writeln!(
            buf,
            "# HELP fcp_sbc_trunk_calls_total Total calls in quality window per trunk"
        );
        let _ = writeln!(buf, "# TYPE fcp_sbc_trunk_calls_total gauge");

        for snap_entry in tracker.all_snapshots() {
            let (name, snap) = snap_entry;
            let _ = writeln!(buf, "fcp_sbc_trunk_asr{{trunk=\"{name}\"}} {:.4}", snap.asr);
            let _ = writeln!(
                buf,
                "fcp_sbc_trunk_acd_secs{{trunk=\"{name}\"}} {:.1}",
                snap.acd_secs
            );
            let _ = writeln!(
                buf,
                "fcp_sbc_trunk_pdd_ms{{trunk=\"{name}\"}} {:.0}",
                snap.avg_pdd_ms
            );
            let _ = writeln!(
                buf,
                "fcp_sbc_trunk_calls_total{{trunk=\"{name}\"}} {}",
                snap.total_calls
            );
        }
    }

    // STIR/SHAKEN metrics
    if let Some(ref stir_metrics) = state.stir_metrics {
        let stir_snap = stir_metrics.snapshot();
        let _ = writeln!(buf);
        prom_counter(
            &mut buf,
            "fcp_sbc_stir_verifications_success_total",
            "Total STIR/SHAKEN verifications succeeded",
            stir_snap.verifications_success,
        );
        prom_counter(
            &mut buf,
            "fcp_sbc_stir_verifications_failed_total",
            "Total STIR/SHAKEN verifications failed",
            stir_snap.verifications_failed,
        );
        prom_counter(
            &mut buf,
            "fcp_sbc_stir_verifications_no_identity_total",
            "Total STIR/SHAKEN calls with no Identity header",
            stir_snap.verifications_no_identity,
        );
        prom_counter(
            &mut buf,
            "fcp_sbc_stir_signings_success_total",
            "Total STIR/SHAKEN signings succeeded",
            stir_snap.signings_success,
        );
        prom_counter(
            &mut buf,
            "fcp_sbc_stir_signings_failed_total",
            "Total STIR/SHAKEN signings failed",
            stir_snap.signings_failed,
        );

        if let Some(ref verifier) = state.stir_verifier {
            prom_gauge(
                &mut buf,
                "fcp_sbc_stir_cert_cache_entries",
                "Current STIR/SHAKEN certificate cache entries",
                verifier.cache().len() as u64,
            );
        }
    }

    ([(header::CONTENT_TYPE, "text/plain; version=0.0.4")], buf)
}

fn prom_gauge(buf: &mut String, name: &str, help: &str, value: u64) {
    let _ = writeln!(buf, "# HELP {name} {help}");
    let _ = writeln!(buf, "# TYPE {name} gauge");
    let _ = writeln!(buf, "{name} {value}");
}

fn prom_counter(buf: &mut String, name: &str, help: &str, value: u64) {
    let _ = writeln!(buf, "# HELP {name} {help}");
    let _ = writeln!(buf, "# TYPE {name} counter");
    let _ = writeln!(buf, "{name} {value}");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::SbcRuntimeMetrics;

    #[tokio::test]
    async fn test_metrics_prometheus_format() {
        let metrics = Arc::new(SbcRuntimeMetrics::new());
        metrics.record_call_attempt();
        metrics.record_call_attempt();
        metrics.record_call_completed();
        metrics.record_call_rejected();
        metrics.record_admission_rejection();
        metrics.record_rate_limit_rejection();
        metrics.record_security_rejection();
        metrics.update_snapshot(42, 10, 5);

        let state = Arc::new(HealthState {
            admission: Arc::new(fcp_sbc_core::admission::AdmissionController::new(
                Default::default(),
            )),
            health_monitor: Arc::new(fcp_sbc_core::trunk_health::TrunkHealthMonitor::new(&[])),
            metrics,
            call_manager: Arc::new(fcp_b2bua_core::CallManager::new()),
            trunk_configs: vec![],
            trunk_registrar: None,
            uac: None,
            dialog_manager: Arc::new(sip_dialog::DialogManager::new()),
            recording_store: None,
            quality_tracker: None,
            lcr_routes: vec![],
            stir_metrics: None,
            stir_verifier: None,
            api_token: None,
        });

        let resp = metrics_snapshot(State(state)).await.into_response();

        // Check content type
        let ct = resp.headers().get(header::CONTENT_TYPE).unwrap();
        assert_eq!(ct, "text/plain; version=0.0.4");

        // Extract body text
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let text = String::from_utf8(body.to_vec()).unwrap();

        // Gauges
        assert!(text.contains("# TYPE fcp_sbc_up gauge"));
        assert!(text.contains("fcp_sbc_up 1"));
        assert!(text.contains("# TYPE fcp_sbc_current_cps gauge"));
        assert!(text.contains("fcp_sbc_current_cps 42"));
        assert!(text.contains("# TYPE fcp_sbc_peak_cps gauge"));
        assert!(text.contains("fcp_sbc_peak_cps 42"));
        assert!(text.contains("# TYPE fcp_sbc_concurrent_calls gauge"));
        assert!(text.contains("fcp_sbc_concurrent_calls 10"));
        assert!(text.contains("# TYPE fcp_sbc_concurrent_registrations gauge"));
        assert!(text.contains("fcp_sbc_concurrent_registrations 5"));

        // Counters
        assert!(text.contains("# TYPE fcp_sbc_calls_attempted_total counter"));
        assert!(text.contains("fcp_sbc_calls_attempted_total 2"));
        assert!(text.contains("# TYPE fcp_sbc_calls_completed_total counter"));
        assert!(text.contains("fcp_sbc_calls_completed_total 1"));
        assert!(text.contains("# TYPE fcp_sbc_calls_rejected_total counter"));
        assert!(text.contains("fcp_sbc_calls_rejected_total 1"));
        assert!(text.contains("# TYPE fcp_sbc_admission_rejections_total counter"));
        assert!(text.contains("fcp_sbc_admission_rejections_total 1"));
        assert!(text.contains("# TYPE fcp_sbc_rate_limit_rejections_total counter"));
        assert!(text.contains("fcp_sbc_rate_limit_rejections_total 1"));
        assert!(text.contains("# TYPE fcp_sbc_security_rejections_total counter"));
        assert!(text.contains("fcp_sbc_security_rejections_total 1"));

        // Every metric line should have HELP + TYPE + value
        for line in text.lines() {
            assert!(
                line.starts_with("# HELP ")
                    || line.starts_with("# TYPE ")
                    || line.split_whitespace().count() == 2,
                "unexpected line format: {line}"
            );
        }
    }
}
