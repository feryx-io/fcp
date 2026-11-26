//! One logging setup for every binary, and its lines shipped to the
//! cluster's Redis for the admin's Logs page (`docs/LOG_VIEWER.md`, L1).
//!
//! [`init`] installs what each binary used to build itself: an `EnvFilter`
//! from `RUST_LOG` or the configured level, and stdout as `pretty`,
//! `compact` or `json`. It adds two things:
//!
//! - **Redaction**, on stdout and on what is shipped: the values of fields
//!   named like secrets, bearer tokens, sealed values and tokens in URLs
//!   ([`redact`]).
//! - **Shipping**: each line at or above the ship level (`info` unless
//!   `FCP_LOG_SHIP` says otherwise, `off` for none) becomes a record —
//!   time, level, target, message, fields, and the `tenant_id` and
//!   `call_id` lifted from the line or the nearest span naming them — and
//!   waits in a bounded queue. When the process's service beacon starts
//!   ([`crate::ServiceBeacon::start`]) it knows the service's id and the
//!   Redis, and the queue drains to the stream `fcp:logs:<service id>`,
//!   trimmed to the last `FCP_LOG_SHIP_LINES` (100,000) lines and
//!   `FCP_LOG_SHIP_HOURS` (24) hours. At most `FCP_LOG_SHIP_RATE` (500)
//!   lines a second are shipped; what is dropped is counted and said.
//!
//! Losing Redis loses recent lines, never the process: stdout is
//! unchanged, and the queue drops the oldest when full.

use std::collections::BTreeMap;
use std::io::Write;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{reload, EnvFilter, Layer};

/// The stream a service's lines are kept in, under the key prefix.
pub fn stream_key(service_id: &str) -> String {
    format!("logs:{service_id}")
}

/// The key a platform administrator sets, with an expiry, to change a
/// service's level for a while (`docs/LOG_VIEWER.md` §2): `error`, `warn`,
/// `info`, `debug` or `trace`. Gone, the configured level is back.
pub fn level_key(service_id: &str) -> String {
    format!("logs:level:{service_id}")
}

/// How long a level set from the page lasts.
pub const LEVEL_SECS: u64 = 3600;

/// The process's level, as configured and as set for now.
struct Levels {
    configured: String,
    configured_ship: Option<Level>,
    now: Mutex<Option<String>>,
    reload: Box<dyn Fn(EnvFilter) -> Result<(), String> + Send + Sync>,
}

static LEVELS: OnceLock<Levels> = OnceLock::new();

/// The ship layer's level, changed with the process's (0 = off).
static SHIP_LEVEL: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

fn level_rank(level: Option<Level>) -> u8 {
    match level {
        None => 0,
        Some(Level::ERROR) => 1,
        Some(Level::WARN) => 2,
        Some(Level::INFO) => 3,
        Some(Level::DEBUG) => 4,
        Some(Level::TRACE) => 5,
    }
}

/// Apply a level set from the page, or with `None` go back to the
/// configured one. A level that does not parse is ignored.
pub fn set_level(level: Option<&str>) {
    let Some(levels) = LEVELS.get() else { return };
    let mut now = levels.now.lock().unwrap_or_else(|e| e.into_inner());
    if now.as_deref() == level {
        return;
    }
    let (filter, ship) = match level {
        Some(l) => match (l.parse::<Level>(), EnvFilter::try_new(l)) {
            (Ok(parsed), Ok(filter)) => (filter, Some(parsed)),
            _ => return,
        },
        None => match EnvFilter::try_new(&levels.configured) {
            Ok(filter) => (filter, levels.configured_ship),
            Err(_) => return,
        },
    };
    if (levels.reload)(filter).is_ok() {
        // Shipping never goes quieter than it was configured to.
        let ship = ship.max(levels.configured_ship);
        SHIP_LEVEL.store(
            if levels.configured_ship.is_some() {
                level_rank(ship)
            } else {
                0
            },
            Ordering::Relaxed,
        );
        tracing::warn!(
            level = level.unwrap_or(levels.configured.as_str()),
            "Log level changed{}",
            if level.is_some() {
                " for an hour, from the admin"
            } else {
                ": back to the configured level"
            }
        );
        *now = level.map(str::to_string);
    }
}

/// One line as kept in Redis and read by the admin.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LogRecord {
    /// Unix milliseconds.
    pub ts: u64,
    /// `ERROR`, `WARN`, `INFO`, `DEBUG`, `TRACE`.
    pub level: String,
    pub target: String,
    pub message: String,
    /// The line's other fields, redacted.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub fields: BTreeMap<String, String>,
    /// The tenant the line is about, from the line or a span around it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tenant_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call_id: Option<String>,
}

/// What redaction hides, before a line reaches stdout or Redis.
pub mod redact {
    use regex::Regex;
    use std::borrow::Cow;
    use std::sync::OnceLock;

    /// Field names whose values are secrets.
    const SECRET_NAMES: &[&str] = &[
        "token",
        "auth_token",
        "access_token",
        "refresh_token",
        "api_key",
        "apikey",
        "password",
        "passwd",
        "secret",
        "client_secret",
        "join_secret",
        "signing_key",
        "secret_access_key",
        "authorization",
        "pin",
    ];

    /// Whether a field so named holds a secret.
    pub fn is_secret_field(name: &str) -> bool {
        let name = name.to_ascii_lowercase();
        SECRET_NAMES
            .iter()
            .any(|s| name == *s || name.ends_with(&format!("_{s}")))
    }

    fn patterns() -> &'static [(Regex, &'static str)] {
        static P: OnceLock<Vec<(Regex, &'static str)>> = OnceLock::new();
        P.get_or_init(|| {
            // An Authorization header is the scheme rule's: its value
            // starts with the scheme, which must stay readable.
            let names = SECRET_NAMES
                .iter()
                .filter(|n| **n != "authorization")
                .copied()
                .collect::<Vec<_>>()
                .join("|");
            vec![
                // `password=x`, `"token": "x"`, `api_key: x`
                (
                    Regex::new(&format!(
                        r#"(?i)\b((?:[a-z0-9]+_)*(?:{names}))("?\s*[=:]\s*"?)([^\s"',&}}\]]+)"#
                    ))
                    .unwrap(),
                    "$1$2***",
                ),
                (
                    Regex::new(r"(?i)\b(bearer|basic|digest)\s+[A-Za-z0-9._~+/=-]{8,}").unwrap(),
                    "$1 ***",
                ),
                (Regex::new(r"enc:v1:[^\s,;'}\]]+").unwrap(), "enc:v1:***"),
            ]
        })
    }

    /// `text` with secrets hidden.
    /// Whether `text` could hold anything [`scrub`] hides: one pass, so the
    /// patterns run only on the few lines that might (every line a busy
    /// node writes goes through here).
    fn may_hold_secret(text: &str) -> bool {
        static ANY: OnceLock<Regex> = OnceLock::new();
        ANY.get_or_init(|| {
            Regex::new(r"(?i)token|passw|secret|api_?key|pin|bearer|basic|digest|enc:v1:").unwrap()
        })
        .is_match(text)
    }

    pub fn scrub(text: &str) -> Cow<'_, str> {
        if !may_hold_secret(text) {
            return Cow::Borrowed(text);
        }
        let mut out = Cow::Borrowed(text);
        for (re, with) in patterns() {
            if re.is_match(&out) {
                out = Cow::Owned(re.replace_all(&out, *with).into_owned());
            }
        }
        out
    }
}

/// How lines are shipped; from the environment, read once.
#[derive(Debug, Clone)]
struct ShipConfig {
    level: Option<Level>,
    max_lines: u64,
    max_age: Duration,
    rate: u32,
}

impl ShipConfig {
    fn from_env() -> Self {
        let var = |name: &str| std::env::var(name).ok().filter(|v| !v.trim().is_empty());
        let level = match var("FCP_LOG_SHIP").as_deref().map(str::trim) {
            Some("off") | Some("none") => None,
            Some(l) => l.parse().ok().or(Some(Level::INFO)),
            None => Some(Level::INFO),
        };
        let num = |name: &str, default: u64| {
            var(name)
                .and_then(|v| v.trim().parse().ok())
                .unwrap_or(default)
        };
        Self {
            level,
            max_lines: num("FCP_LOG_SHIP_LINES", 100_000).max(1_000),
            max_age: Duration::from_secs(num("FCP_LOG_SHIP_HOURS", 24).max(1) * 3600),
            rate: num("FCP_LOG_SHIP_RATE", 500).max(1) as u32,
        }
    }
}

/// Lines waiting for Redis. Bounded: a process that cannot ship keeps the
/// newest [`QUEUE`] and drops the rest.
const QUEUE: usize = 10_000;

struct Shipping {
    config: ShipConfig,
    tx: mpsc::Sender<LogRecord>,
    rx: Mutex<Option<mpsc::Receiver<LogRecord>>>,
    dropped: AtomicU64,
}

static SHIPPING: OnceLock<Shipping> = OnceLock::new();

/// Install this process's logging: `level` unless `RUST_LOG` says
/// otherwise, `format` (`json`, `compact`, else `pretty`), and the target
/// shown or not. Shipping waits for the service beacon. Calling it twice
/// leaves the first in place.
pub fn init(level: &str, format: &str, with_target: bool) -> anyhow::Result<()> {
    let configured = std::env::var("RUST_LOG")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| level.to_string());
    let filter = EnvFilter::try_new(&configured)
        .map_err(|e| anyhow::anyhow!("log level {configured:?}: {e}"))?;
    let (filter, reload_handle) = reload::Layer::new(filter);
    let config = ShipConfig::from_env();
    SHIP_LEVEL.store(level_rank(config.level), Ordering::Relaxed);
    let _ = LEVELS.set(Levels {
        configured,
        configured_ship: config.level,
        now: Mutex::new(None),
        reload: Box::new(move |f| reload_handle.reload(f).map_err(|e| e.to_string())),
    });
    let ship = config.level.map(|level| {
        let shipping = SHIPPING.get_or_init(|| {
            let (tx, rx) = mpsc::channel(QUEUE);
            Shipping {
                config: config.clone(),
                tx,
                rx: Mutex::new(Some(rx)),
                dropped: AtomicU64::new(0),
            }
        });
        let _ = level;
        ShipLayer {
            tx: shipping.tx.clone(),
        }
    });
    let stdout = tracing_subscriber::fmt::layer()
        .with_target(with_target)
        .with_writer(|| RedactingStdout);
    let registry = tracing_subscriber::registry().with(filter).with(ship);
    let installed = match format {
        "json" => registry.with(stdout.json()).try_init(),
        "compact" => registry.with(stdout.compact()).try_init(),
        _ => registry.with(stdout).try_init(),
    };
    // A second init (a test, a library) keeps the first.
    let _ = installed;
    Ok(())
}

/// Stdout with secrets hidden. The formatter writes each line in one call.
struct RedactingStdout;

impl Write for RedactingStdout {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let text = String::from_utf8_lossy(buf);
        std::io::stdout().write_all(redact::scrub(&text).as_bytes())?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        std::io::stdout().flush()
    }
}

/// The tenant and call a span names, kept on it for the lines inside.
#[derive(Default, Clone)]
struct Tags {
    tenant_id: Option<String>,
    call_id: Option<String>,
}

#[derive(Default)]
struct Fields {
    message: String,
    fields: BTreeMap<String, String>,
    tags: Tags,
}

impl Fields {
    fn put(&mut self, field: &Field, value: String) {
        match field.name() {
            "message" => self.message = value,
            "tenant_id" | "tenant" => self.tags.tenant_id = Some(value),
            "call_id" => {
                self.tags.call_id = Some(value.clone());
                self.fields.insert("call_id".into(), value);
            }
            name if redact::is_secret_field(name) => {
                self.fields.insert(name.into(), "***".into());
            }
            name if name.starts_with("log.") => {}
            name => {
                self.fields.insert(name.into(), value);
            }
        }
    }
}

impl Visit for Fields {
    fn record_str(&mut self, field: &Field, value: &str) {
        self.put(field, value.to_string());
    }

    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.put(field, format!("{value:?}"));
    }
}

/// A tenant written `tenant-<uuid>` and `<uuid>` is one tenant.
fn plain_tenant(t: String) -> String {
    match t.strip_prefix("tenant-") {
        Some(rest) => rest.to_string(),
        None => t,
    }
}

struct ShipLayer {
    tx: mpsc::Sender<LogRecord>,
}

impl<S> Layer<S> for ShipLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        let mut seen = Fields::default();
        attrs.record(&mut seen);
        if let Some(span) = ctx.span(id) {
            span.extensions_mut().insert(seen.tags);
        }
    }

    fn on_record(&self, id: &Id, values: &Record<'_>, ctx: Context<'_, S>) {
        let mut seen = Fields::default();
        values.record(&mut seen);
        if let Some(span) = ctx.span(id) {
            let mut ext = span.extensions_mut();
            let tags = match ext.get_mut::<Tags>() {
                Some(tags) => tags,
                None => {
                    ext.insert(Tags::default());
                    ext.get_mut::<Tags>().expect("just inserted")
                }
            };
            if seen.tags.tenant_id.is_some() {
                tags.tenant_id = seen.tags.tenant_id;
            }
            if seen.tags.call_id.is_some() {
                tags.call_id = seen.tags.call_id;
            }
        }
    }

    fn on_event(&self, event: &Event<'_>, ctx: Context<'_, S>) {
        // Nowhere to put it (no beacon has started shipping, or Redis is
        // behind): building the record would only throw it away.
        if self.tx.capacity() == 0 {
            if let Some(shipping) = SHIPPING.get() {
                shipping.dropped.fetch_add(1, Ordering::Relaxed);
            }
            return;
        }
        let meta = event.metadata();
        if level_rank(Some(*meta.level())) > SHIP_LEVEL.load(Ordering::Relaxed) {
            return;
        }
        // Shipping must not ship its own client's lines.
        let target = meta.target();
        if target.starts_with("redis") || target.starts_with("fcp_cluster::logging") {
            return;
        }
        let mut seen = Fields::default();
        event.record(&mut seen);
        if seen.tags.tenant_id.is_none() || seen.tags.call_id.is_none() {
            if let Some(scope) = ctx.event_scope(event) {
                for span in scope {
                    if let Some(tags) = span.extensions().get::<Tags>() {
                        if seen.tags.tenant_id.is_none() {
                            seen.tags.tenant_id.clone_from(&tags.tenant_id);
                        }
                        if seen.tags.call_id.is_none() {
                            seen.tags.call_id.clone_from(&tags.call_id);
                        }
                    }
                }
            }
        }
        let record = LogRecord {
            ts: unix_ms(),
            level: meta.level().to_string(),
            target: target.to_string(),
            message: redact::scrub(&seen.message).into_owned(),
            fields: seen
                .fields
                .into_iter()
                .map(|(k, v)| (k, redact::scrub(&v).into_owned()))
                .collect(),
            tenant_id: seen.tags.tenant_id.map(plain_tenant),
            call_id: seen.tags.call_id,
        };
        if self.tx.try_send(record).is_err() {
            if let Some(shipping) = SHIPPING.get() {
                shipping.dropped.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

fn unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Drain the queue to `service_id`'s stream in the Redis `config` names.
/// Called once by the service beacon; later calls and a process that does
/// not ship do nothing.
pub(crate) fn start_shipping(service_id: &str, config: fcp_storage_redis::RedisConfig) {
    let Some(shipping) = SHIPPING.get() else {
        return;
    };
    let Some(rx) = shipping.rx.lock().unwrap_or_else(|e| e.into_inner()).take() else {
        return;
    };
    let service_id = service_id.to_string();
    tokio::spawn(ship(service_id, config, rx, shipping));
}

async fn ship(
    service_id: String,
    config: fcp_storage_redis::RedisConfig,
    mut rx: mpsc::Receiver<LogRecord>,
    shipping: &'static Shipping,
) {
    let conf = shipping.config.clone();
    let mut storage: Option<fcp_storage_redis::RedisStorage> = None;
    let mut window_start = std::time::Instant::now();
    let mut sent_in_window: u32 = 0;
    let mut over_rate: u64 = 0;
    let mut last_trim = std::time::Instant::now() - Duration::from_secs(3600);
    let mut batch: Vec<LogRecord> = Vec::with_capacity(256);
    loop {
        batch.clear();
        let Some(first) = rx.recv().await else { return };
        batch.push(first);
        while batch.len() < 256 {
            match rx.try_recv() {
                Ok(r) => batch.push(r),
                Err(_) => break,
            }
        }
        // The rate: what is over it is counted and said once a second.
        if window_start.elapsed() >= Duration::from_secs(1) {
            window_start = std::time::Instant::now();
            sent_in_window = 0;
            let dropped = over_rate + shipping.dropped.swap(0, Ordering::Relaxed);
            over_rate = 0;
            if dropped > 0 {
                batch.push(LogRecord {
                    ts: unix_ms(),
                    level: "WARN".into(),
                    target: "fcp_cluster::logging".into(),
                    message: format!("{dropped} log lines were not shipped (rate or queue full)"),
                    fields: BTreeMap::new(),
                    tenant_id: None,
                    call_id: None,
                });
            }
        }
        let room = conf.rate.saturating_sub(sent_in_window) as usize;
        if batch.len() > room {
            over_rate += (batch.len() - room) as u64;
            batch.truncate(room);
        }
        if batch.is_empty() {
            continue;
        }
        sent_in_window += batch.len() as u32;

        if storage.is_none() {
            match tokio::time::timeout(
                Duration::from_secs(3),
                fcp_storage_redis::RedisStorage::new(config.clone()),
            )
            .await
            {
                Ok(Ok(s)) => storage = Some(s),
                _ => {
                    // Redis away: these lines are lost; stdout has them.
                    shipping
                        .dropped
                        .fetch_add(batch.len() as u64, Ordering::Relaxed);
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    continue;
                }
            }
        }
        let Some(store) = &storage else { continue };
        let key = store.key(&stream_key(&service_id));
        let mut pipe = redis::pipe();
        for r in &batch {
            let Ok(json) = serde_json::to_string(r) else {
                continue;
            };
            pipe.cmd("XADD")
                .arg(&key)
                .arg("MAXLEN")
                .arg("~")
                .arg(conf.max_lines)
                .arg("*")
                .arg("r")
                .arg(json)
                .ignore();
        }
        if last_trim.elapsed() >= Duration::from_secs(60) {
            last_trim = std::time::Instant::now();
            let oldest = unix_ms().saturating_sub(conf.max_age.as_millis() as u64);
            pipe.cmd("XTRIM")
                .arg(&key)
                .arg("MINID")
                .arg("~")
                .arg(oldest)
                .ignore();
        }
        let mut conn = store.connection();
        if pipe.query_async::<()>(&mut conn).await.is_err() {
            shipping
                .dropped
                .fetch_add(batch.len() as u64, Ordering::Relaxed);
            storage = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::redact::{is_secret_field, scrub};

    #[test]
    fn secrets_are_hidden_wherever_they_are_written() {
        assert_eq!(scrub("token=abc123 rest"), "token=*** rest");
        assert_eq!(
            scrub(r#"{"auth_token":"s3cr3t","x":1}"#),
            r#"{"auth_token":"***","x":1}"#
        );
        assert_eq!(
            scrub("ws://h/ws/t?token=abc&x=1"),
            "ws://h/ws/t?token=***&x=1"
        );
        assert_eq!(
            scrub("Authorization: Bearer eyJhbGciOiJIUzI1NiJ9.x.y"),
            "Authorization: Bearer ***"
        );
        assert_eq!(scrub("key enc:v1:abc:def== used"), "key enc:v1:*** used");
        assert_eq!(scrub("client_secret: hunter22"), "client_secret: ***");
        // Ordinary words are left alone.
        assert_eq!(
            scrub("tokens refreshed for 3 users"),
            "tokens refreshed for 3 users"
        );
        assert_eq!(scrub("call_id=abc tenant=x"), "call_id=abc tenant=x");
        assert!(is_secret_field("api_key"));
        assert!(is_secret_field("events_auth_token"));
        assert!(!is_secret_field("token_count_total"));
        assert!(!is_secret_field("call_id"));
    }
}
