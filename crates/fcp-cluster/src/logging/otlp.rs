//! Lines sent to an operator's OpenTelemetry collector over OTLP/HTTP
//! (`docs/LOG_VIEWER.md`, L6).
//!
//! The same records the Logs page reads (redacted, with the tenant and
//! call lifted from spans) go as OTLP log records to the endpoint the
//! standard variables name:
//!
//! - `OTEL_EXPORTER_OTLP_LOGS_ENDPOINT` (the full URL), else
//!   `OTEL_EXPORTER_OTLP_ENDPOINT` with `/v1/logs` added. Neither set: no
//!   export.
//! - `OTEL_EXPORTER_OTLP_[LOGS_]PROTOCOL`: `http/protobuf` (the default)
//!   or `http/json`. `grpc` is refused at startup.
//! - `OTEL_EXPORTER_OTLP_[LOGS_]HEADERS` (`k=v,k2=v2`, values
//!   percent-encoded), `…_TIMEOUT` (milliseconds, 10,000),
//!   `…_COMPRESSION` (`gzip` or `none`), `…_CERTIFICATE` (a CA bundle),
//!   `…_CLIENT_CERTIFICATE` and `…_CLIENT_KEY` (mutual TLS).
//! - `OTEL_SERVICE_NAME` and `OTEL_RESOURCE_ATTRIBUTES` over the resource
//!   FCP gives (`service.name` = `fcp-<kind>`, `service.instance.id` = the
//!   beacon's id, `service.version`, `service.namespace` = `fcp`).
//! - `OTEL_SDK_DISABLED=true` turns it off.
//! - `FCP_LOG_OTLP_LEVEL`: the least severe level sent (`info`; `off` for
//!   none). A level raised from the page raises it too, for the hour.
//!
//! The exporter runs on a thread of its own, so a process without a
//! runtime or Redis still exports, and its HTTP client's own lines are
//! never exported. A collector away loses lines, never the process: the
//! queue is bounded, a batch is retried three times on 429, 502–504 or a
//! failed connection, then dropped and counted in the next batch.

use std::cell::Cell;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context as _};
use tokio::sync::mpsc;
use tracing::Level;

use super::LogRecord;

/// How records are encoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protocol {
    Protobuf,
    Json,
}

/// Where and how lines are exported; from the environment, read once.
#[derive(Debug, Clone)]
pub struct OtlpConfig {
    pub endpoint: String,
    pub protocol: Protocol,
    pub headers: Vec<(String, String)>,
    pub timeout: Duration,
    pub gzip: bool,
    pub level: Level,
    pub certificate: Option<String>,
    pub client_identity: Option<(String, String)>,
    pub service_name: Option<String>,
    pub resource: Vec<(String, String)>,
}

/// One environment variable, empty taken as unset.
fn var(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// The logs signal's variable, else the general one.
fn signal_var(suffix: &str) -> Option<String> {
    var(&format!("OTEL_EXPORTER_OTLP_LOGS_{suffix}"))
        .or_else(|| var(&format!("OTEL_EXPORTER_OTLP_{suffix}")))
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
            if let Ok(b) = u8::from_str_radix(hex, 16) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// `k=v,k2=v2` as W3C Baggage writes it (the form OTel's variables use).
fn pairs(list: &str) -> anyhow::Result<Vec<(String, String)>> {
    let mut out = Vec::new();
    for item in list.split(',').map(str::trim).filter(|i| !i.is_empty()) {
        let (k, v) = item
            .split_once('=')
            .ok_or_else(|| anyhow!("{item:?} is not key=value"))?;
        let k = percent_decode(k.trim());
        if k.is_empty() {
            bail!("{item:?} has no key");
        }
        out.push((k, percent_decode(v.trim())));
    }
    Ok(out)
}

impl OtlpConfig {
    /// The export the environment asks for, `None` for none, an error for
    /// one that cannot work (refused at startup rather than silently off).
    pub fn from_env() -> anyhow::Result<Option<Self>> {
        if var("OTEL_SDK_DISABLED").is_some_and(|v| v.eq_ignore_ascii_case("true")) {
            return Ok(None);
        }
        let level = match var("FCP_LOG_OTLP_LEVEL").as_deref() {
            Some("off") | Some("none") => return Ok(None),
            Some(l) => l
                .parse::<Level>()
                .map_err(|_| anyhow!("FCP_LOG_OTLP_LEVEL {l:?} is not a level"))?,
            None => Level::INFO,
        };
        let endpoint = match var("OTEL_EXPORTER_OTLP_LOGS_ENDPOINT") {
            Some(e) => e,
            None => match var("OTEL_EXPORTER_OTLP_ENDPOINT") {
                Some(e) => format!("{}/v1/logs", e.trim_end_matches('/')),
                None => return Ok(None),
            },
        };
        let url = reqwest::Url::parse(&endpoint)
            .with_context(|| format!("OTLP logs endpoint {endpoint:?}"))?;
        if !matches!(url.scheme(), "http" | "https") {
            bail!("OTLP logs endpoint {endpoint:?}: only http and https are supported");
        }
        let protocol = match signal_var("PROTOCOL").as_deref() {
            None | Some("http/protobuf") => Protocol::Protobuf,
            Some("http/json") => Protocol::Json,
            Some("grpc") => bail!(
                "OTLP over gRPC is not supported for logs: set \
                 OTEL_EXPORTER_OTLP_PROTOCOL=http/protobuf (the collector's port 4318)"
            ),
            Some(p) => bail!("OTLP protocol {p:?}: use http/protobuf or http/json"),
        };
        // The general headers, then the logs signal's over them.
        let mut headers = Vec::new();
        for name in [
            "OTEL_EXPORTER_OTLP_HEADERS",
            "OTEL_EXPORTER_OTLP_LOGS_HEADERS",
        ] {
            if let Some(list) = var(name) {
                for (k, v) in pairs(&list).with_context(|| name.to_string())? {
                    headers.retain(|(h, _): &(String, String)| !h.eq_ignore_ascii_case(&k));
                    headers.push((k, v));
                }
            }
        }
        let timeout = match signal_var("TIMEOUT") {
            Some(ms) => Duration::from_millis(
                ms.parse::<u64>()
                    .map_err(|_| anyhow!("OTLP timeout {ms:?} is not milliseconds"))?
                    .max(100),
            ),
            None => Duration::from_secs(10),
        };
        let gzip = match signal_var("COMPRESSION").as_deref() {
            None | Some("none") => false,
            Some("gzip") => true,
            Some(c) => bail!("OTLP compression {c:?}: use gzip or none"),
        };
        let certificate = signal_var("CERTIFICATE");
        let client_identity = match (signal_var("CLIENT_CERTIFICATE"), signal_var("CLIENT_KEY")) {
            (Some(c), Some(k)) => Some((c, k)),
            (None, None) => None,
            _ => bail!("OTLP mutual TLS needs both a client certificate and its key"),
        };
        let resource = match var("OTEL_RESOURCE_ATTRIBUTES") {
            Some(list) => pairs(&list).context("OTEL_RESOURCE_ATTRIBUTES")?,
            None => Vec::new(),
        };
        Ok(Some(Self {
            endpoint,
            protocol,
            headers,
            timeout,
            gzip,
            level,
            certificate,
            client_identity,
            service_name: var("OTEL_SERVICE_NAME"),
            resource,
        }))
    }

    fn client(&self) -> anyhow::Result<reqwest::Client> {
        let mut headers = reqwest::header::HeaderMap::new();
        for (k, v) in &self.headers {
            let name = reqwest::header::HeaderName::from_bytes(k.as_bytes())
                .with_context(|| format!("OTLP header name {k:?}"))?;
            let mut value = reqwest::header::HeaderValue::from_str(v)
                .with_context(|| format!("OTLP header {k:?}'s value"))?;
            value.set_sensitive(true);
            headers.insert(name, value);
        }
        let mut builder = reqwest::Client::builder()
            .timeout(self.timeout)
            .default_headers(headers)
            .user_agent(concat!("fcp/", env!("CARGO_PKG_VERSION")));
        if let Some(path) = &self.certificate {
            let pem = std::fs::read(path).with_context(|| format!("OTLP certificate {path}"))?;
            for cert in reqwest::Certificate::from_pem_bundle(&pem)
                .with_context(|| format!("OTLP certificate {path}"))?
            {
                builder = builder.add_root_certificate(cert);
            }
        }
        if let Some((cert, key)) = &self.client_identity {
            let mut pem =
                std::fs::read(cert).with_context(|| format!("OTLP client certificate {cert}"))?;
            pem.push(b'\n');
            pem.extend(std::fs::read(key).with_context(|| format!("OTLP client key {key}"))?);
            builder =
                builder
                    .identity(reqwest::Identity::from_pem(&pem).with_context(|| {
                        format!("OTLP client certificate {cert} and key {key}")
                    })?);
        }
        builder.build().context("OTLP client")
    }
}

/// Who this process is, once its service beacon says.
#[derive(Debug, Clone)]
pub(crate) struct Identity {
    pub id: String,
    pub kind: String,
    pub version: String,
    pub region: Option<String>,
}

static IDENTITY: OnceLock<Identity> = OnceLock::new();

/// Named by the service beacon when it starts.
pub(crate) fn set_identity(identity: Identity) {
    let _ = IDENTITY.set(identity);
}

thread_local! {
    /// Set on the exporter's thread: its own lines (the HTTP client's)
    /// are never exported, or each export would make more.
    static ON_EXPORTER: Cell<bool> = const { Cell::new(false) };
}

/// Whether the line being written comes from the exporter itself.
pub(crate) fn on_exporter() -> bool {
    ON_EXPORTER.with(Cell::get)
}

/// Lines waiting for the collector.
const QUEUE: usize = 10_000;
/// Most records in one export.
const BATCH: usize = 512;
/// Longest a line waits for its batch to fill.
const LINGER: Duration = Duration::from_secs(1);
/// Retries of a batch the collector could not take for now.
const RETRIES: u32 = 3;

pub(crate) struct Exporter {
    pub tx: mpsc::Sender<LogRecord>,
    pub dropped: AtomicU64,
}

static EXPORTER: OnceLock<Exporter> = OnceLock::new();

pub(crate) fn exporter() -> Option<&'static Exporter> {
    EXPORTER.get()
}

/// Start exporting on a thread of its own. A second call does nothing.
pub(crate) fn start(config: OtlpConfig) -> anyhow::Result<()> {
    if EXPORTER.get().is_some() {
        return Ok(());
    }
    let client = config.client()?;
    let (tx, rx) = mpsc::channel(QUEUE);
    if EXPORTER
        .set(Exporter {
            tx,
            dropped: AtomicU64::new(0),
        })
        .is_err()
    {
        return Ok(());
    }
    std::thread::Builder::new()
        .name("fcp-otlp".into())
        .spawn(move || {
            ON_EXPORTER.with(|f| f.set(true));
            let runtime = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("OTLP log export not started: {e}");
                    return;
                }
            };
            runtime.block_on(run(config, client, rx));
        })
        .context("OTLP exporter thread")?;
    Ok(())
}

async fn run(config: OtlpConfig, client: reqwest::Client, mut rx: mpsc::Receiver<LogRecord>) {
    let started = Instant::now();
    let exporter = EXPORTER.get().expect("set before the thread starts");
    let mut batch: Vec<LogRecord> = Vec::with_capacity(BATCH);
    let mut last_complaint: Option<Instant> = None;
    loop {
        batch.clear();
        let Some(first) = rx.recv().await else { return };
        batch.push(first);
        let deadline = tokio::time::Instant::now() + LINGER;
        while batch.len() < BATCH {
            match tokio::time::timeout_at(deadline, rx.recv()).await {
                Ok(Some(r)) => batch.push(r),
                Ok(None) | Err(_) => break,
            }
        }
        // The first lines wait a little for the beacon to say who this is.
        while IDENTITY.get().is_none() && started.elapsed() < Duration::from_secs(10) {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let dropped = exporter.dropped.swap(0, Ordering::Relaxed);
        if dropped > 0 {
            batch.push(LogRecord {
                ts: super::unix_ms(),
                level: "WARN".into(),
                target: "fcp_cluster::logging".into(),
                message: format!(
                    "{dropped} log lines were not exported (queue full or collector away)"
                ),
                fields: BTreeMap::new(),
                tenant_id: None,
                call_id: None,
            });
        }
        let resource = resource(&config, IDENTITY.get());
        let body = match config.protocol {
            Protocol::Protobuf => encode_protobuf(&resource, &batch),
            Protocol::Json => encode_json(&resource, &batch).into_bytes(),
        };
        let lines = batch.len() as u64;
        if let Err(why) = send(&config, &client, body).await {
            exporter.dropped.fetch_add(lines, Ordering::Relaxed);
            // Said on stdout (never exported) at most once a minute.
            if last_complaint.is_none_or(|t| t.elapsed() >= Duration::from_secs(60)) {
                last_complaint = Some(Instant::now());
                tracing::warn!(endpoint = %config.endpoint, "OTLP log export failed: {why}");
            }
        }
    }
}

/// Send one export, retrying what the collector says to retry.
async fn send(config: &OtlpConfig, client: &reqwest::Client, body: Vec<u8>) -> Result<(), String> {
    let body = if config.gzip {
        use std::io::Write;
        let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        enc.write_all(&body).map_err(|e| e.to_string())?;
        enc.finish().map_err(|e| e.to_string())?
    } else {
        body
    };
    let content_type = match config.protocol {
        Protocol::Protobuf => "application/x-protobuf",
        Protocol::Json => "application/json",
    };
    let mut attempt = 0;
    loop {
        let mut request = client
            .post(&config.endpoint)
            .header(reqwest::header::CONTENT_TYPE, content_type)
            .body(body.clone());
        if config.gzip {
            request = request.header(reqwest::header::CONTENT_ENCODING, "gzip");
        }
        let (why, wait) = match request.send().await {
            Ok(r) if r.status().is_success() => return Ok(()),
            Ok(r) => {
                let status = r.status();
                if !matches!(status.as_u16(), 429 | 502 | 503 | 504) {
                    return Err(format!("the collector answered {status}"));
                }
                let after = r
                    .headers()
                    .get(reqwest::header::RETRY_AFTER)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.trim().parse::<u64>().ok())
                    .map(|s| Duration::from_secs(s.min(30)));
                (format!("the collector answered {status}"), after)
            }
            Err(e) => (format!("{e}"), None),
        };
        if attempt >= RETRIES {
            return Err(why);
        }
        tokio::time::sleep(wait.unwrap_or(Duration::from_secs(1 << attempt))).await;
        attempt += 1;
    }
}

/// The resource's attributes: FCP's, then the operator's over them.
pub(crate) fn resource(config: &OtlpConfig, identity: Option<&Identity>) -> Vec<(String, String)> {
    let mut attrs: Vec<(String, String)> = Vec::new();
    let put = |attrs: &mut Vec<(String, String)>, k: &str, v: String| {
        attrs.retain(|(a, _)| a != k);
        attrs.push((k.to_string(), v));
    };
    let exe = std::env::current_exe()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "fcp".into());
    match identity {
        Some(id) => {
            put(&mut attrs, "service.name", format!("fcp-{}", id.kind));
            put(&mut attrs, "service.instance.id", id.id.clone());
            put(&mut attrs, "service.version", id.version.clone());
            put(&mut attrs, "fcp.service.kind", id.kind.clone());
            if let Some(region) = &id.region {
                put(&mut attrs, "fcp.region", region.clone());
            }
        }
        None => {
            put(&mut attrs, "service.name", exe);
            put(
                &mut attrs,
                "service.version",
                env!("CARGO_PKG_VERSION").into(),
            );
        }
    }
    put(&mut attrs, "service.namespace", "fcp".into());
    if let Some(host) = var("HOSTNAME").or_else(|| {
        std::fs::read_to_string("/etc/hostname")
            .ok()
            .map(|h| h.trim().to_string())
            .filter(|h| !h.is_empty())
    }) {
        put(&mut attrs, "host.name", host);
    }
    for (k, v) in &config.resource {
        put(&mut attrs, k, v.clone());
    }
    if let Some(name) = &config.service_name {
        put(&mut attrs, "service.name", name.clone());
    }
    attrs
}

/// OTLP's severity number for a level's name.
pub(crate) fn severity_number(level: &str) -> u64 {
    match level {
        "TRACE" => 1,
        "DEBUG" => 5,
        "INFO" => 9,
        "WARN" => 13,
        "ERROR" => 17,
        _ => 0,
    }
}

/// A record's attributes: its fields, and its tenant and call.
fn attributes(r: &LogRecord) -> Vec<(&str, &str)> {
    let mut out: Vec<(&str, &str)> = r
        .fields
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    if let Some(t) = &r.tenant_id {
        out.push(("tenant_id", t));
    }
    if let Some(c) = &r.call_id {
        if !r.fields.contains_key("call_id") {
            out.push(("call_id", c));
        }
    }
    out
}

/// Records by target, each target an instrumentation scope, in the order
/// first seen.
fn by_scope(records: &[LogRecord]) -> Vec<(&str, Vec<&LogRecord>)> {
    let mut scopes: Vec<(&str, Vec<&LogRecord>)> = Vec::new();
    for r in records {
        match scopes.iter_mut().find(|(t, _)| *t == r.target) {
            Some((_, list)) => list.push(r),
            None => scopes.push((&r.target, vec![r])),
        }
    }
    scopes
}

// ── protobuf, by hand: the few messages of opentelemetry-proto's logs ──

mod pb {
    pub fn varint(out: &mut Vec<u8>, mut v: u64) {
        while v >= 0x80 {
            out.push((v as u8) | 0x80);
            v >>= 7;
        }
        out.push(v as u8);
    }

    fn key(out: &mut Vec<u8>, field: u32, wire: u8) {
        varint(out, ((field as u64) << 3) | wire as u64);
    }

    pub fn bytes(out: &mut Vec<u8>, field: u32, data: &[u8]) {
        key(out, field, 2);
        varint(out, data.len() as u64);
        out.extend_from_slice(data);
    }

    pub fn string(out: &mut Vec<u8>, field: u32, s: &str) {
        bytes(out, field, s.as_bytes());
    }

    pub fn uint(out: &mut Vec<u8>, field: u32, v: u64) {
        key(out, field, 0);
        varint(out, v);
    }

    pub fn fixed64(out: &mut Vec<u8>, field: u32, v: u64) {
        key(out, field, 1);
        out.extend_from_slice(&v.to_le_bytes());
    }

    /// `AnyValue { string_value = 1 }`
    pub fn any_string(s: &str) -> Vec<u8> {
        let mut out = Vec::with_capacity(s.len() + 2);
        string(&mut out, 1, s);
        out
    }

    /// `KeyValue { key = 1, value = 2 }`
    pub fn key_value(k: &str, v: &str) -> Vec<u8> {
        let mut out = Vec::with_capacity(k.len() + v.len() + 6);
        string(&mut out, 1, k);
        bytes(&mut out, 2, &any_string(v));
        out
    }
}

/// `ExportLogsServiceRequest` in protobuf.
pub(crate) fn encode_protobuf(resource: &[(String, String)], records: &[LogRecord]) -> Vec<u8> {
    // Resource { attributes = 1 }
    let mut res = Vec::new();
    for (k, v) in resource {
        pb::bytes(&mut res, 1, &pb::key_value(k, v));
    }
    // ResourceLogs { resource = 1, scope_logs = 2 }
    let mut resource_logs = Vec::new();
    pb::bytes(&mut resource_logs, 1, &res);
    for (target, list) in by_scope(records) {
        // ScopeLogs { scope = 1, log_records = 2 }
        let mut scope_logs = Vec::new();
        // InstrumentationScope { name = 1 }
        let mut scope = Vec::new();
        pb::string(&mut scope, 1, target);
        pb::bytes(&mut scope_logs, 1, &scope);
        for r in list {
            // LogRecord { time_unix_nano = 1 (fixed64), severity_number = 2,
            //   severity_text = 3, body = 5, attributes = 6,
            //   observed_time_unix_nano = 11 (fixed64) }
            let nanos = r.ts.saturating_mul(1_000_000);
            let mut rec = Vec::new();
            pb::fixed64(&mut rec, 1, nanos);
            pb::uint(&mut rec, 2, severity_number(&r.level));
            pb::string(&mut rec, 3, &r.level);
            pb::bytes(&mut rec, 5, &pb::any_string(&r.message));
            for (k, v) in attributes(r) {
                pb::bytes(&mut rec, 6, &pb::key_value(k, v));
            }
            pb::fixed64(&mut rec, 11, nanos);
            pb::bytes(&mut scope_logs, 2, &rec);
        }
        pb::bytes(&mut resource_logs, 2, &scope_logs);
    }
    // ExportLogsServiceRequest { resource_logs = 1 }
    let mut out = Vec::new();
    pb::bytes(&mut out, 1, &resource_logs);
    out
}

/// `ExportLogsServiceRequest` in OTLP's JSON (camelCase, 64-bit integers
/// as strings).
pub(crate) fn encode_json(resource: &[(String, String)], records: &[LogRecord]) -> String {
    use serde_json::{json, Value};
    let kv = |k: &str, v: &str| json!({"key": k, "value": {"stringValue": v}});
    let scopes: Vec<Value> = by_scope(records)
        .into_iter()
        .map(|(target, list)| {
            let logs: Vec<Value> = list
                .into_iter()
                .map(|r| {
                    let nanos = r.ts.saturating_mul(1_000_000).to_string();
                    json!({
                        "timeUnixNano": nanos,
                        "observedTimeUnixNano": nanos,
                        "severityNumber": severity_number(&r.level),
                        "severityText": r.level,
                        "body": {"stringValue": r.message},
                        "attributes": attributes(r)
                            .into_iter()
                            .map(|(k, v)| kv(k, v))
                            .collect::<Vec<_>>(),
                    })
                })
                .collect();
            json!({"scope": {"name": target}, "logRecords": logs})
        })
        .collect();
    json!({
        "resourceLogs": [{
            "resource": {
                "attributes": resource.iter().map(|(k, v)| kv(k, v)).collect::<Vec<_>>(),
            },
            "scopeLogs": scopes,
        }]
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use opentelemetry_proto::tonic::collector::logs::v1::ExportLogsServiceRequest;
    use opentelemetry_proto::tonic::common::v1::{any_value, KeyValue};
    use prost::Message;

    fn record(target: &str, level: &str, message: &str) -> LogRecord {
        LogRecord {
            ts: 1_764_000_000_123,
            level: level.into(),
            target: target.into(),
            message: message.into(),
            fields: BTreeMap::from([
                ("call_id".to_string(), "call-1".to_string()),
                ("from".to_string(), "sip:alice@x".to_string()),
            ]),
            tenant_id: Some("t1".into()),
            call_id: Some("call-1".into()),
        }
    }

    fn text(kvs: &[KeyValue], key: &str) -> Option<String> {
        kvs.iter().find(|kv| kv.key == key).and_then(|kv| {
            match kv.value.as_ref()?.value.as_ref()? {
                any_value::Value::StringValue(s) => Some(s.clone()),
                _ => None,
            }
        })
    }

    fn check(req: &ExportLogsServiceRequest) {
        assert_eq!(req.resource_logs.len(), 1);
        let rl = &req.resource_logs[0];
        let res = &rl.resource.as_ref().unwrap().attributes;
        assert_eq!(
            text(res, "service.name").as_deref(),
            Some("fcp-call-manager")
        );
        assert_eq!(text(res, "service.instance.id").as_deref(), Some("cm-1"));
        assert_eq!(text(res, "deployment.environment").as_deref(), Some("lab"));
        // Two targets, two scopes, in the order first seen.
        let names: Vec<_> = rl
            .scope_logs
            .iter()
            .map(|s| s.scope.as_ref().unwrap().name.clone())
            .collect();
        assert_eq!(names, ["fcp_sip::calls", "fcp_media"]);
        assert_eq!(rl.scope_logs[0].log_records.len(), 2);
        let r = &rl.scope_logs[0].log_records[1];
        assert_eq!(r.time_unix_nano, 1_764_000_000_123_000_000);
        assert_eq!(r.observed_time_unix_nano, r.time_unix_nano);
        assert_eq!(r.severity_number, 13);
        assert_eq!(r.severity_text, "WARN");
        match r.body.as_ref().unwrap().value.as_ref().unwrap() {
            any_value::Value::StringValue(s) => assert_eq!(s, "ringing timed out"),
            other => panic!("body {other:?}"),
        }
        assert_eq!(text(&r.attributes, "tenant_id").as_deref(), Some("t1"));
        assert_eq!(text(&r.attributes, "from").as_deref(), Some("sip:alice@x"));
        // The call is named once.
        assert_eq!(
            r.attributes.iter().filter(|kv| kv.key == "call_id").count(),
            1
        );
    }

    fn sample() -> (Vec<(String, String)>, Vec<LogRecord>) {
        let config = OtlpConfig {
            endpoint: "http://c/v1/logs".into(),
            protocol: Protocol::Protobuf,
            headers: vec![],
            timeout: Duration::from_secs(1),
            gzip: false,
            level: Level::INFO,
            certificate: None,
            client_identity: None,
            service_name: None,
            resource: vec![("deployment.environment".into(), "lab".into())],
        };
        let identity = Identity {
            id: "cm-1".into(),
            kind: "call-manager".into(),
            version: "0.0.1".into(),
            region: None,
        };
        let resource = resource(&config, Some(&identity));
        let records = vec![
            record("fcp_sip::calls", "INFO", "call placed"),
            record("fcp_media", "ERROR", "no ports"),
            record("fcp_sip::calls", "WARN", "ringing timed out"),
        ];
        (resource, records)
    }

    #[test]
    fn protobuf_is_what_opentelemetry_reads() {
        let (resource, records) = sample();
        let bytes = encode_protobuf(&resource, &records);
        let req = ExportLogsServiceRequest::decode(bytes.as_slice()).expect("decodes");
        check(&req);
    }

    #[test]
    fn json_is_what_opentelemetry_reads() {
        let (resource, records) = sample();
        let json = encode_json(&resource, &records);
        let req: ExportLogsServiceRequest = serde_json::from_str(&json).expect("decodes");
        check(&req);
    }

    #[test]
    fn the_operator_s_names_win() {
        let mut config = sample_config();
        config.resource = vec![("service.name".into(), "pbx".into())];
        let r = resource(&config, None);
        assert!(r.contains(&("service.name".into(), "pbx".into())));
        config.service_name = Some("edge".into());
        let r = resource(&config, None);
        assert!(r.contains(&("service.name".into(), "edge".into())));
        assert_eq!(r.iter().filter(|(k, _)| k == "service.name").count(), 1);
    }

    fn sample_config() -> OtlpConfig {
        OtlpConfig {
            endpoint: "http://c/v1/logs".into(),
            protocol: Protocol::Protobuf,
            headers: vec![],
            timeout: Duration::from_secs(1),
            gzip: false,
            level: Level::INFO,
            certificate: None,
            client_identity: None,
            service_name: None,
            resource: vec![],
        }
    }

    #[test]
    fn lists_are_baggage() {
        assert_eq!(
            pairs("api-key=a%2Cb, x = y ").unwrap(),
            [("api-key".into(), "a,b".into()), ("x".into(), "y".into())]
        );
        assert!(pairs("novalue").is_err());
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("a%zzb"), "a%zzb");
    }
}
