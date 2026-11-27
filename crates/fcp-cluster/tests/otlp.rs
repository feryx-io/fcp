//! A process's lines reach an OpenTelemetry collector over OTLP/HTTP
//! (`docs/LOG_VIEWER.md`, L6): the standard variables, gzip, headers, a
//! retry when the collector is busy, redaction, the tenant from a span,
//! and the level raised from the page raising the export.

use std::io::Read;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::post;
use opentelemetry_proto::tonic::collector::logs::v1::ExportLogsServiceRequest;
use opentelemetry_proto::tonic::common::v1::{any_value, KeyValue};
use opentelemetry_proto::tonic::logs::v1::LogRecord;
use prost::Message;

#[derive(Default)]
struct Collector {
    calls: usize,
    seen: Vec<(HeaderMap, ExportLogsServiceRequest)>,
}

async fn logs(
    State(c): State<Arc<Mutex<Collector>>>,
    headers: HeaderMap,
    body: Bytes,
) -> StatusCode {
    let mut c = c.lock().unwrap();
    c.calls += 1;
    // Busy at first: the exporter must try again.
    if c.calls == 1 {
        return StatusCode::SERVICE_UNAVAILABLE;
    }
    let mut raw = Vec::new();
    if headers.get("content-encoding").map(|v| v.as_bytes()) == Some(b"gzip") {
        flate2::read::GzDecoder::new(&body[..])
            .read_to_end(&mut raw)
            .unwrap();
    } else {
        raw = body.to_vec();
    }
    let req = ExportLogsServiceRequest::decode(raw.as_slice()).expect("protobuf");
    c.seen.push((headers, req));
    StatusCode::OK
}

fn text(kvs: &[KeyValue], key: &str) -> Option<String> {
    kvs.iter()
        .find(|kv| kv.key == key)
        .and_then(|kv| match kv.value.as_ref()?.value.as_ref()? {
            any_value::Value::StringValue(s) => Some(s.clone()),
            _ => None,
        })
}

fn body(r: &LogRecord) -> String {
    match r.body.as_ref().and_then(|b| b.value.as_ref()) {
        Some(any_value::Value::StringValue(s)) => s.clone(),
        _ => String::new(),
    }
}

/// Every record the collector has taken, with its resource.
fn records(c: &Mutex<Collector>) -> Vec<(Vec<KeyValue>, String, LogRecord)> {
    let c = c.lock().unwrap();
    let mut out = Vec::new();
    for (_, req) in &c.seen {
        for rl in &req.resource_logs {
            let res = rl.resource.clone().unwrap_or_default().attributes;
            for sl in &rl.scope_logs {
                let scope = sl.scope.clone().unwrap_or_default().name;
                for r in &sl.log_records {
                    out.push((res.clone(), scope.clone(), r.clone()));
                }
            }
        }
    }
    out
}

async fn wait_for(c: &Mutex<Collector>, message: &str) -> (Vec<KeyValue>, String, LogRecord) {
    for _ in 0..150 {
        if let Some(found) = records(c)
            .into_iter()
            .find(|(_, _, r)| body(r).contains(message))
        {
            return found;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("{message:?} never reached the collector");
}

#[tokio::test]
async fn lines_reach_the_operator_s_collector() {
    let collector = Arc::new(Mutex::new(Collector::default()));
    let app = axum::Router::new()
        .route("/otel/v1/logs", post(logs))
        .with_state(collector.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await });

    std::env::set_var("FCP_LOG_SHIP", "off");
    std::env::set_var(
        "OTEL_EXPORTER_OTLP_ENDPOINT",
        format!("http://{addr}/otel/"),
    );
    std::env::set_var("OTEL_EXPORTER_OTLP_HEADERS", "x-scope-orgid=lab%201");
    std::env::set_var("OTEL_EXPORTER_OTLP_COMPRESSION", "gzip");
    std::env::set_var("OTEL_RESOURCE_ATTRIBUTES", "deployment.environment=lab");
    std::env::remove_var("RUST_LOG");
    fcp_cluster::logging::init("info", "compact", true).unwrap();

    {
        let span = tracing::info_span!(
            "call",
            tenant_id = "tenant-7d0c6a3e-0000-4000-8000-000000000001"
        );
        let _in = span.enter();
        tracing::info!(password = "hunter2", "caller joined with token=abc123");
        tracing::debug!("a debug line, below the export's level");
    }
    let (resource, scope, joined) = wait_for(&collector, "caller joined").await;
    assert_eq!(body(&joined), "caller joined with token=***");
    assert_eq!(text(&joined.attributes, "password").as_deref(), Some("***"));
    assert_eq!(
        text(&joined.attributes, "tenant_id").as_deref(),
        Some("7d0c6a3e-0000-4000-8000-000000000001")
    );
    assert_eq!(joined.severity_number, 9);
    assert_eq!(scope, "otlp");
    assert_eq!(
        text(&resource, "deployment.environment").as_deref(),
        Some("lab")
    );
    assert_eq!(text(&resource, "service.namespace").as_deref(), Some("fcp"));
    {
        let c = collector.lock().unwrap();
        assert!(c.calls >= 2, "the busy answer was retried");
        let (headers, _) = &c.seen[0];
        assert_eq!(headers["x-scope-orgid"], "lab 1");
        assert_eq!(headers["content-type"], "application/x-protobuf");
    }
    assert!(!records(&collector)
        .iter()
        .any(|(_, _, r)| body(r).contains("a debug line")));

    // Raised from the page, the export follows.
    fcp_cluster::logging::set_level(Some("debug"));
    tracing::debug!("a debug line, raised");
    wait_for(&collector, "a debug line, raised").await;
}
