//! HTTP health, readiness, and metrics endpoints for the SBC.

use crate::admission::AdmissionController;
use crate::metrics::SbcRuntimeMetrics;
use crate::recording_store::RecordingStore;
use fcp_sbc_core::trunk_health::TrunkHealthMonitor;
use fcp_sbc_core::trunk_registration::TrunkRegistrar;
use axum::http::header;
use axum::response::IntoResponse;
use axum::{extract::State, routing::get, Json, Router};
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
}

/// Build the health/readiness/metrics router with admin API.
pub fn health_router(state: Arc<HealthState>) -> Router {
    Router::new()
        .route("/health", get(health_check))
        .route("/ready", get(readiness_check))
        .route("/metrics", get(metrics_snapshot))
        .nest("/admin", crate::admin::admin_router())
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
        let _ = writeln!(buf, "# HELP fcp_sbc_trunk_asr Answer seizure ratio per trunk (0.0-1.0)");
        let _ = writeln!(buf, "# TYPE fcp_sbc_trunk_asr gauge");
        let _ = writeln!(buf, "# HELP fcp_sbc_trunk_acd_secs Average call duration per trunk");
        let _ = writeln!(buf, "# TYPE fcp_sbc_trunk_acd_secs gauge");
        let _ = writeln!(buf, "# HELP fcp_sbc_trunk_pdd_ms Average post-dial delay per trunk");
        let _ = writeln!(buf, "# TYPE fcp_sbc_trunk_pdd_ms gauge");
        let _ = writeln!(buf, "# HELP fcp_sbc_trunk_calls_total Total calls in quality window per trunk");
        let _ = writeln!(buf, "# TYPE fcp_sbc_trunk_calls_total gauge");

        for snap_entry in tracker.all_snapshots() {
            let (name, snap) = snap_entry;
            let _ = writeln!(buf, "fcp_sbc_trunk_asr{{trunk=\"{name}\"}} {:.4}", snap.asr);
            let _ = writeln!(buf, "fcp_sbc_trunk_acd_secs{{trunk=\"{name}\"}} {:.1}", snap.acd_secs);
            let _ = writeln!(buf, "fcp_sbc_trunk_pdd_ms{{trunk=\"{name}\"}} {:.0}", snap.avg_pdd_ms);
            let _ = writeln!(buf, "fcp_sbc_trunk_calls_total{{trunk=\"{name}\"}} {}", snap.total_calls);
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
            admission: Arc::new(crate::admission::AdmissionController::new(Default::default())),
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
