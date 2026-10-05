use crate::Error;
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

/// Serde default helper that returns `true`.
pub(crate) fn default_true() -> bool {
    true
}

/// A key whose value is a secret: masked wherever a configuration is
/// printed. Matched on the key's name, case-insensitively.
fn is_secret_key(key: &str) -> bool {
    let k = key.to_ascii_lowercase();
    [
        "token",
        "secret",
        "password",
        "passwd",
        "credential",
        "private_key",
        "master_key",
        "api_key",
        "jwt",
        "signing_key",
    ]
    .iter()
    .any(|needle| k.contains(needle))
        && !k.ends_with("_file")
        && !k.ends_with("_path")
        && !k.ends_with("_secs")
        && !k.ends_with("_enabled")
        && !k.ends_with("_required")
        && !k.starts_with("require_")
        && !k.starts_with("allow_")
        && !k.starts_with("disable_")
}

/// The password in a `scheme://user:password@host…` URL replaced by
/// `***`; any other string as it was.
fn mask_url_password(s: &str) -> Option<String> {
    let (scheme, rest) = s.split_once("://")?;
    if scheme.is_empty()
        || !scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c))
    {
        return None;
    }
    let (userinfo, host) = rest.split_once('@')?;
    if userinfo.contains('/') {
        return None;
    }
    let (user, password) = userinfo.split_once(':')?;
    if password.is_empty() {
        return None;
    }
    Some(format!("{scheme}://{user}:***@{host}"))
}

/// Mask every secret in a TOML value tree in place: a string under a
/// secret key becomes `***`, an array of strings under one becomes
/// `["***", …]`, and the password in any URL becomes `***`; an empty
/// string stays empty, so "unset" still reads as unset.
pub fn mask_toml_value(value: &mut toml::Value) {
    match value {
        toml::Value::Table(table) => {
            for (key, v) in table.iter_mut() {
                if is_secret_key(key) {
                    match v {
                        toml::Value::String(s) if !s.is_empty() => *s = "***".to_string(),
                        toml::Value::Array(items) => {
                            for item in items.iter_mut() {
                                if let toml::Value::String(s) = item {
                                    if !s.is_empty() {
                                        *s = "***".to_string();
                                    }
                                }
                            }
                        }
                        other => mask_toml_value(other),
                    }
                } else {
                    mask_toml_value(v);
                }
            }
        }
        toml::Value::Array(items) => {
            for item in items.iter_mut() {
                mask_toml_value(item);
            }
        }
        toml::Value::String(s) => {
            if let Some(masked) = mask_url_password(s) {
                *s = masked;
            }
        }
        _ => {}
    }
}

/// The effective configuration as TOML with every secret masked: what
/// `--check-config` prints on every binary (`docs/SHIP_AND_OPERATE.md`
/// phase 1), so an operator sees what the node will run with, defaults
/// filled in, and never a token.
pub fn masked_toml<T: Serialize>(config: &T) -> std::result::Result<String, String> {
    let mut value = toml::Value::try_from(config).map_err(|e| e.to_string())?;
    mask_toml_value(&mut value);
    toml::to_string_pretty(&value).map_err(|e| e.to_string())
}

/// A `${NAME}` in a configuration string that could not be resolved
/// (`docs/PRODUCTION_STACK.md` phase 2): the environment does not set
/// the name, or what is between the braces is not a name.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ReferenceError {
    /// `${NAME}` where `NAME` is not set in the environment.
    #[error("{key} refers to ${{{name}}}, which is not set in the environment")]
    Unset { name: String, key: String },
    /// `${…}` where `…` is not an environment variable name, or has no
    /// closing brace.
    #[error("{key} has `${{{text}` where an environment variable name was expected")]
    Malformed { text: String, key: String },
}

fn is_reference_name(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Every `${NAME}` in `text` replaced by what `lookup` returns for it;
/// `$${` is the way to write a literal `${`. Any other `$` is itself.
/// `key` names the value in the error, for the operator.
pub fn resolve_references(
    text: &str,
    key: &str,
    lookup: &dyn Fn(&str) -> Option<String>,
) -> std::result::Result<String, ReferenceError> {
    if !text.contains('$') {
        return Ok(text.to_string());
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find('$') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        if let Some(tail) = after.strip_prefix("${") {
            // `$${…}`: a literal `${`, the reference not resolved.
            out.push_str("${");
            rest = tail;
        } else if let Some(tail) = after.strip_prefix('{') {
            let Some(end) = tail.find('}') else {
                return Err(ReferenceError::Malformed {
                    text: tail.to_string(),
                    key: key.to_string(),
                });
            };
            let name = &tail[..end];
            if !is_reference_name(name) {
                return Err(ReferenceError::Malformed {
                    text: format!("{name}}}"),
                    key: key.to_string(),
                });
            }
            match lookup(name) {
                Some(value) => out.push_str(&value),
                None => {
                    return Err(ReferenceError::Unset {
                        name: name.to_string(),
                        key: key.to_string(),
                    })
                }
            }
            rest = &tail[end + 1..];
        } else {
            out.push('$');
            rest = after;
        }
    }
    out.push_str(rest);
    Ok(out)
}

fn resolve_value_references(
    value: &mut toml::Value,
    key: &mut String,
    lookup: &dyn Fn(&str) -> Option<String>,
) -> std::result::Result<(), ReferenceError> {
    match value {
        toml::Value::Table(table) => {
            for (k, v) in table.iter_mut() {
                let len = key.len();
                if !key.is_empty() {
                    key.push('.');
                }
                key.push_str(k);
                resolve_value_references(v, key, lookup)?;
                key.truncate(len);
            }
        }
        toml::Value::Array(items) => {
            for (i, item) in items.iter_mut().enumerate() {
                let len = key.len();
                key.push_str(&format!("[{i}]"));
                resolve_value_references(item, key, lookup)?;
                key.truncate(len);
            }
        }
        toml::Value::String(s) if s.contains('$') => {
            *s = resolve_references(s, key, lookup)?;
        }
        _ => {}
    }
    Ok(())
}

/// Every string in a TOML value tree with its `${NAME}` references
/// resolved through `lookup`, in place; the first that cannot be is the
/// error, naming the key.
pub fn resolve_toml_references(
    value: &mut toml::Value,
    lookup: &dyn Fn(&str) -> Option<String>,
) -> std::result::Result<(), ReferenceError> {
    resolve_value_references(value, &mut String::new(), lookup)
}

/// A binary's configuration from its file's text: parsed, every
/// `${NAME}` in every string resolved from the process environment,
/// then deserialized — so a nested credential no per-field variable
/// reaches can still come from the environment. A name the environment
/// lacks is a refusal that names the key; bad TOML, an unknown key or a
/// wrong type is a parse failure, as before.
pub fn parse_toml<T: serde::de::DeserializeOwned>(text: &str) -> anyhow::Result<T> {
    let mut value: toml::Value = toml::from_str(text)?;
    resolve_toml_references(&mut value, &|name| std::env::var(name).ok())?;
    Ok(value.try_into::<T>()?)
}

/// Exit code of `--check-config` when the file does not parse — an
/// unknown key, a wrong type, bad TOML.
pub const CHECK_CONFIG_PARSE_FAILED: i32 = 2;
/// Exit code of `--check-config` when the file parses but a startup rule
/// refuses it — a public bind without its token, say.
pub const CHECK_CONFIG_REFUSED: i32 = 1;

/// One stage of loading a configuration under `--check-config`: an
/// error is printed as the refusal it is and the process exits with
/// [`CHECK_CONFIG_PARSE_FAILED`] when a TOML error is in the chain,
/// [`CHECK_CONFIG_REFUSED`] otherwise. Without `--check-config` the
/// error is returned as it was, so startup fails as it always did.
pub fn check_stage<T>(result: anyhow::Result<T>, checking: bool) -> anyhow::Result<T> {
    match result {
        Ok(value) => Ok(value),
        Err(e) if checking => {
            let parse_failed = e
                .chain()
                .any(|cause| cause.downcast_ref::<toml::de::Error>().is_some());
            eprintln!(
                "configuration {}: {e:#}",
                if parse_failed {
                    "does not parse"
                } else {
                    "refused"
                }
            );
            std::process::exit(if parse_failed {
                CHECK_CONFIG_PARSE_FAILED
            } else {
                CHECK_CONFIG_REFUSED
            });
        }
        Err(e) => Err(e),
    }
}

const MAX_SIP_MESSAGE_SIZE: usize = 65_535;
const MAX_CONNECTION_POOL_SIZE: usize = 10_000;
const MAX_RTP_PORT_SPAN: u16 = 10_000;

/// Complete runtime configuration for FCP.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FcpConfig {
    /// Deployment mode (carrier, service_provider, enterprise)
    #[serde(default)]
    pub deployment_mode: crate::DeploymentMode,

    /// Node configuration
    pub node: NodeConfig,

    /// Transport configuration
    pub transport: TransportConfig,

    /// Media configuration
    pub media: MediaConfig,

    /// Storage configuration
    pub storage: StorageConfig,

    /// Registrar configuration
    pub registrar: RegistrarConfig,

    /// API configuration
    pub api: ApiConfig,

    /// Observability configuration
    pub observability: ObservabilityConfig,

    /// Routing configuration
    pub routing: RoutingConfig,
}

impl FcpConfig {
    /// Validate configuration values for common bounds and CRLF safety.
    pub fn validate(&self) -> Result<(), Error> {
        self.node.validate().map_err(Error::Config)?;
        self.transport.validate().map_err(Error::Config)?;
        self.media.validate().map_err(Error::Config)?;
        self.storage.validate().map_err(Error::Config)?;
        self.registrar.validate().map_err(Error::Config)?;
        self.api.validate().map_err(Error::Config)?;
        self.observability.validate().map_err(Error::Config)?;
        self.routing.validate().map_err(Error::Config)?;
        Ok(())
    }
}

/// Node-level configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeConfig {
    /// Unique node identifier (auto-generated if not provided)
    pub node_id: Option<String>,

    /// Human-friendly node name
    pub name: String,

    /// Local domain for SIP URIs (e.g., "pbx.example.com")
    pub local_domain: String,
}

impl Default for NodeConfig {
    fn default() -> Self {
        Self {
            node_id: None,
            name: "fcp-node".to_string(),
            local_domain: "fcp.local".to_string(),
        }
    }
}

fn reject_crlf(field: &str, value: &str) -> Result<(), String> {
    if value.bytes().any(|b| matches!(b, b'\r' | b'\n')) {
        Err(format!("{field} must not contain CR or LF characters"))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    fn env(name: &str) -> Option<String> {
        match name {
            "TOKEN" => Some("s3cret".to_string()),
            "HOST" => Some("carrier.example.net".to_string()),
            _ => None,
        }
    }

    #[test]
    fn references_resolve_anywhere_in_a_string() {
        assert_eq!(resolve_references("${TOKEN}", "k", &env).unwrap(), "s3cret");
        assert_eq!(
            resolve_references("sip:${HOST}:5060;x=${TOKEN}", "k", &env).unwrap(),
            "sip:carrier.example.net:5060;x=s3cret"
        );
        assert_eq!(
            resolve_references("no reference", "k", &env).unwrap(),
            "no reference"
        );
    }

    #[test]
    fn a_doubled_dollar_is_a_literal_reference_and_a_lone_one_is_itself() {
        assert_eq!(
            resolve_references("$${TOKEN} costs $5 and $TOKEN", "k", &env).unwrap(),
            "${TOKEN} costs $5 and $TOKEN"
        );
    }

    #[test]
    fn an_unset_name_and_a_malformed_reference_are_errors_naming_the_key() {
        assert_eq!(
            resolve_references("${MISSING}", "http.api_token", &env),
            Err(ReferenceError::Unset {
                name: "MISSING".into(),
                key: "http.api_token".into()
            })
        );
        let e = resolve_references("${MISSING}", "http.api_token", &env).unwrap_err();
        assert_eq!(
            e.to_string(),
            "http.api_token refers to ${MISSING}, which is not set in the environment"
        );
        assert!(matches!(
            resolve_references("${not a name}", "k", &env),
            Err(ReferenceError::Malformed { .. })
        ));
        assert!(matches!(
            resolve_references("${TOKEN", "k", &env),
            Err(ReferenceError::Malformed { .. })
        ));
        assert!(matches!(
            resolve_references("${}", "k", &env),
            Err(ReferenceError::Malformed { .. })
        ));
    }

    #[test]
    fn a_toml_tree_is_resolved_in_every_string_and_the_key_is_a_path() {
        let mut value: toml::Value = toml::from_str(
            r#"
[http]
api_token = "${TOKEN}"
port = 8080
[[trunks]]
name = "a"
hosts = ["${HOST}:5060", "b"]
"#,
        )
        .unwrap();
        resolve_toml_references(&mut value, &env).unwrap();
        assert_eq!(value["http"]["api_token"].as_str(), Some("s3cret"));
        assert_eq!(
            value["trunks"][0]["hosts"][0].as_str(),
            Some("carrier.example.net:5060")
        );

        let mut value: toml::Value =
            toml::from_str("[[trunks]]\nhosts = [\"x\", \"${NOPE}\"]\n").unwrap();
        let e = resolve_toml_references(&mut value, &env).unwrap_err();
        assert_eq!(
            e,
            ReferenceError::Unset {
                name: "NOPE".into(),
                key: "trunks[0].hosts[1]".into()
            }
        );
    }

    #[test]
    fn parse_toml_resolves_from_the_environment_and_still_refuses_unknown_keys() {
        #[derive(Debug, serde::Deserialize)]
        #[serde(deny_unknown_fields)]
        struct C {
            token: String,
        }
        std::env::set_var("FCP_CORE_TEST_REFERENCE", "from-env");
        let c: C = parse_toml("token = \"${FCP_CORE_TEST_REFERENCE}\"").unwrap();
        assert_eq!(c.token, "from-env");

        let e = parse_toml::<C>("token = \"${FCP_CORE_TEST_UNSET}\"").unwrap_err();
        assert!(e.downcast_ref::<ReferenceError>().is_some(), "{e}");
        assert!(e.to_string().contains("FCP_CORE_TEST_UNSET"), "{e}");

        let e = parse_toml::<C>("token = \"x\"\nother = 1").unwrap_err();
        assert!(e.downcast_ref::<toml::de::Error>().is_some(), "{e}");
        assert!(e.to_string().contains("other"), "{e}");
    }
    use super::*;

    #[test]
    fn default_config_validates() {
        let config = FcpConfig::default();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn validate_rejects_bad_values() {
        let mut config = FcpConfig::default();
        config.transport.max_message_size = MAX_SIP_MESSAGE_SIZE + 1;
        assert!(matches!(config.validate(), Err(Error::Config(_))));

        config.transport.max_message_size = MAX_SIP_MESSAGE_SIZE;
        config.node.name = "bad\r\nname".to_string();
        assert!(matches!(config.validate(), Err(Error::Config(_))));
    }
}
impl NodeConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.name.is_empty() {
            return Err("node.name must not be empty".to_string());
        }
        if self.local_domain.is_empty() {
            return Err("node.local_domain must not be empty".to_string());
        }
        reject_crlf("node.name", &self.name)?;
        reject_crlf("node.local_domain", &self.local_domain)?;
        Ok(())
    }
}

/// Transport layer configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransportConfig {
    /// UDP bind address
    pub udp_bind: Option<SocketAddr>,

    /// TCP bind address
    pub tcp_bind: Option<SocketAddr>,

    /// TLS bind address (SIPS)
    pub tls_bind: Option<SocketAddr>,

    /// TLS certificate path (PEM format)
    pub tls_cert: Option<PathBuf>,

    /// TLS private key path (PEM format)
    pub tls_key: Option<PathBuf>,

    /// Maximum message size (bytes)
    pub max_message_size: usize,

    /// Connection pool size for TCP/TLS
    pub connection_pool_size: usize,

    /// Connection idle timeout
    pub connection_timeout: Duration,

    /// Local IP address for SDP (if not set, derived from bind address)
    pub local_ip: Option<String>,

    /// RTP port for media
    pub rtp_port: u16,

    /// Enable SDP support for calls
    pub enable_sdp: bool,
}

impl Default for TransportConfig {
    fn default() -> Self {
        Self {
            udp_bind: Some("0.0.0.0:5060".parse().unwrap()),
            tcp_bind: Some("0.0.0.0:5060".parse().unwrap()),
            tls_bind: None,
            tls_cert: None,
            tls_key: None,
            max_message_size: 65535,
            connection_pool_size: 1000,
            connection_timeout: Duration::from_secs(300),
            local_ip: None,
            rtp_port: 10000,
            enable_sdp: true,
        }
    }
}

impl TransportConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.max_message_size == 0 || self.max_message_size > MAX_SIP_MESSAGE_SIZE {
            return Err(format!(
                "transport.max_message_size must be between 1 and {} bytes",
                MAX_SIP_MESSAGE_SIZE
            ));
        }
        if self.connection_pool_size == 0 || self.connection_pool_size > MAX_CONNECTION_POOL_SIZE {
            return Err(format!(
                "transport.connection_pool_size must be between 1 and {}",
                MAX_CONNECTION_POOL_SIZE
            ));
        }
        if self.connection_timeout.is_zero() {
            return Err("transport.connection_timeout must be greater than zero".to_string());
        }
        if self.rtp_port == 0 {
            return Err("transport.rtp_port must be greater than zero".to_string());
        }
        if let Some(ip) = &self.local_ip {
            reject_crlf("transport.local_ip", ip)?;
        }

        // Validate TLS configuration if TLS is enabled
        if self.tls_bind.is_some() {
            // If TLS is enabled, both cert and key must be provided
            if self.tls_cert.is_none() || self.tls_key.is_none() {
                return Err(
                    "transport.tls_cert and transport.tls_key must be provided when TLS is enabled"
                        .to_string(),
                );
            }

            // Verify certificate file exists
            if let Some(cert_path) = &self.tls_cert {
                if !cert_path.exists() {
                    return Err(format!(
                        "TLS certificate file not found: {}",
                        cert_path.display()
                    ));
                }
                if !cert_path.is_file() {
                    return Err(format!(
                        "TLS certificate path is not a file: {}",
                        cert_path.display()
                    ));
                }
            }

            // Verify key file exists
            if let Some(key_path) = &self.tls_key {
                if !key_path.exists() {
                    return Err(format!(
                        "TLS private key file not found: {}",
                        key_path.display()
                    ));
                }
                if !key_path.is_file() {
                    return Err(format!(
                        "TLS private key path is not a file: {}",
                        key_path.display()
                    ));
                }
            }
        }

        Ok(())
    }
}

/// Registrar configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistrarConfig {
    /// Enable REGISTER request handling
    pub enabled: bool,

    /// Enable authentication for REGISTER requests
    pub auth_enabled: bool,

    /// Authentication realm
    pub realm: String,

    /// Data directory for file-based storage
    pub data_dir: PathBuf,

    /// Path to users file (JSON format, optional)
    /// If specified, users will be loaded from this file on startup.
    /// Example format: [{"username": "alice", "password": "secret123", "display_name": "Alice", "enabled": true}]
    pub users_file: Option<PathBuf>,

    /// Default registration expiry (seconds)
    pub default_expires: u32,

    /// Minimum registration expiry (seconds)
    pub min_expires: u32,

    /// Maximum registration expiry (seconds)
    pub max_expires: u32,
}

impl Default for RegistrarConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            auth_enabled: false,
            realm: "fcp.local".to_string(),
            data_dir: PathBuf::from("./data"),
            users_file: None,
            default_expires: 3600,
            min_expires: 60,
            max_expires: 86400,
        }
    }
}

impl RegistrarConfig {
    pub fn validate(&self) -> Result<(), String> {
        reject_crlf("registrar.realm", &self.realm)?;
        if self.min_expires == 0 {
            return Err("registrar.min_expires must be greater than zero".to_string());
        }
        if self.min_expires > self.max_expires {
            return Err("registrar.min_expires cannot exceed registrar.max_expires".to_string());
        }
        if self.default_expires < self.min_expires || self.default_expires > self.max_expires {
            return Err(
                "registrar.default_expires must fall between min_expires and max_expires"
                    .to_string(),
            );
        }
        Ok(())
    }
}

/// Storage configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageConfig {
    /// Storage backend type
    pub backend: StorageBackend,

    /// Database connection string
    pub connection_string: String,

    /// Maximum number of database connections
    pub max_connections: u32,

    /// Connection timeout
    pub connection_timeout: Duration,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            backend: StorageBackend::Sqlite,
            connection_string: "sqlite://fcp.db".to_string(),
            max_connections: 10,
            connection_timeout: Duration::from_secs(30),
        }
    }
}

impl StorageConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.max_connections == 0 {
            return Err("storage.max_connections must be greater than zero".to_string());
        }
        if self.connection_timeout.is_zero() {
            return Err("storage.connection_timeout must be greater than zero".to_string());
        }
        Ok(())
    }
}

/// Storage backend types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StorageBackend {
    /// SQLite (for development and single-node)
    Sqlite,

    /// PostgreSQL (for production)
    Postgres,

    /// MySQL (for existing MySQL infrastructure)
    Mysql,

    /// MongoDB (for large-scale NoSQL)
    Mongodb,

    /// CockroachDB (for distributed deployments) - FUTURE
    Cockroach,
}

/// API server configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiConfig {
    /// Enable REST API
    pub enabled: bool,

    /// REST API bind address
    pub bind: SocketAddr,

    /// Enable Prometheus metrics endpoint
    pub metrics_enabled: bool,

    /// Metrics endpoint path
    pub metrics_path: String,

    /// Enable API authentication
    pub auth_enabled: bool,

    /// API authentication token (if auth enabled)
    pub auth_token: Option<String>,

    /// Require HTTPS (via `X-Forwarded-Proto`) for the provisioning endpoint.
    ///
    /// Provisioning responses may contain SIP digest credentials (HA1 hashes)
    /// that are reusable authentication material.  Serving them over plain HTTP
    /// exposes credentials to network observers.  Defaults to `true`; set to
    /// `false` only for local development or when TLS is terminated at the
    /// application layer rather than a reverse proxy.
    #[serde(default = "default_true")]
    pub provisioning_require_https: bool,

    /// Allow unauthenticated tenant self-service signup.
    ///
    /// Disabled by default so `/api/v1/auth/signup` is not exposed unless a
    /// deployment explicitly opts into public onboarding.
    #[serde(default)]
    pub allow_public_signup: bool,

    /// Deployment mode for permission evaluation.
    /// Controls which permissions are available (e.g., trunk management
    /// is only enabled in Carrier/ServiceProvider modes).
    #[serde(default)]
    pub deployment_mode: crate::permissions::DeploymentMode,

    /// Request limits for API tokens (docs/DEVELOPER_SURFACE.md §3,
    /// phase 2).
    #[serde(default)]
    pub rate_limit: ApiRateLimitConfig,

    /// Sign-in sessions (`docs/IDENTITY_AND_DEVICES.md` phase 1).
    #[serde(default)]
    pub session: ApiSessionConfig,

    /// Single sign-on through tenants' identity providers (phase 3).
    #[serde(default)]
    pub sso: ApiSsoConfig,

    /// What provisioned phones are told (phase 7).
    #[serde(default)]
    pub provisioning: ApiProvisioningConfig,

    /// Where vendors' redirection services are reached (phase 8).
    #[serde(default)]
    pub redirection: ApiRedirectionConfig,

    /// Where firmware images are kept (phase 9).
    #[serde(default)]
    pub firmware: ApiFirmwareConfig,
}

/// `[api.firmware]`: where uploaded firmware images are kept — a
/// directory, or an S3-compatible bucket (`[api.firmware.s3]`) — and how
/// large one may be (`docs/PROVISIONING.md` §11). Neither set, firmware
/// cannot be uploaded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApiFirmwareConfig {
    /// The images, when they are kept on this admin's disk; with a bucket,
    /// where an upload is staged before it goes up (the system's temporary
    /// directory when unset).
    #[serde(default)]
    pub directory: Option<String>,
    #[serde(default = "default_max_image_mb")]
    pub max_image_mb: u64,
    /// The images in a bucket instead: every admin of the deployment then
    /// serves the same ones.
    #[serde(default)]
    pub s3: Option<FirmwareS3Config>,
}

/// `[api.firmware.s3]`: the bucket images are kept in. Any store that
/// speaks S3 with SigV4 (AWS, MinIO, Garage, Ceph, R2).
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FirmwareS3Config {
    pub bucket: String,
    #[serde(default = "default_s3_region")]
    pub region: String,
    /// The store's base URL; AWS's regional endpoint when unset.
    #[serde(default)]
    pub endpoint: Option<String>,
    /// Address the bucket in the path rather than the host name; most
    /// self-hosted stores want this.
    #[serde(default)]
    pub path_style: bool,
    /// Unset, `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY` and
    /// `AWS_SESSION_TOKEN` from the environment.
    #[serde(default)]
    pub access_key_id: Option<String>,
    #[serde(default)]
    pub secret_access_key: Option<String>,
    #[serde(default)]
    pub session_token: Option<String>,
    /// Keys are `<prefix><image>/<file>`.
    #[serde(default = "default_firmware_prefix")]
    pub prefix: String,
}

impl std::fmt::Debug for FirmwareS3Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FirmwareS3Config")
            .field("bucket", &self.bucket)
            .field("region", &self.region)
            .field("endpoint", &self.endpoint)
            .field("path_style", &self.path_style)
            .field("prefix", &self.prefix)
            .finish_non_exhaustive()
    }
}

fn default_s3_region() -> String {
    "us-east-1".to_string()
}

fn default_firmware_prefix() -> String {
    "firmware/".to_string()
}

fn default_max_image_mb() -> u64 {
    1024
}

impl Default for ApiFirmwareConfig {
    fn default() -> Self {
        Self {
            directory: None,
            max_image_mb: default_max_image_mb(),
            s3: None,
        }
    }
}

impl ApiFirmwareConfig {
    pub fn validate(&self) -> std::result::Result<(), String> {
        if self.max_image_mb == 0 || self.max_image_mb > 16 * 1024 {
            return Err("api.firmware.max_image_mb must be 1 to 16384".to_string());
        }
        if self
            .directory
            .as_deref()
            .is_some_and(|d| d.trim().is_empty())
        {
            return Err("api.firmware.directory must not be empty".to_string());
        }
        if let Some(s3) = &self.s3 {
            if s3.bucket.trim().is_empty() {
                return Err("api.firmware.s3.bucket must be set".to_string());
            }
            if s3
                .endpoint
                .as_deref()
                .is_some_and(|e| !(e.starts_with("https://") || e.starts_with("http://")))
            {
                return Err("api.firmware.s3.endpoint must be an http(s):// address".to_string());
            }
        }
        Ok(())
    }
}

/// `[api.redirection]`: where vendors' redirection services answer
/// (`docs/PROVISIONING.md` §10). Set by the operator, not a tenant, so a
/// tenant's account cannot point FCP at an address of its choosing; the
/// defaults are the vendors' own.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApiRedirectionConfig {
    #[serde(default = "default_yealink_rps_url")]
    pub yealink_rps_url: String,
    #[serde(default = "default_poly_ztp_url")]
    pub poly_ztp_url: String,
}

fn default_yealink_rps_url() -> String {
    "https://api-dm.yealink.com:8443".to_string()
}

fn default_poly_ztp_url() -> String {
    "https://ztpconsole.polycom.com/inboundservlet/GenericServlet".to_string()
}

impl Default for ApiRedirectionConfig {
    fn default() -> Self {
        Self {
            yealink_rps_url: default_yealink_rps_url(),
            poly_ztp_url: default_poly_ztp_url(),
        }
    }
}

impl ApiRedirectionConfig {
    pub fn validate(&self) -> std::result::Result<(), String> {
        for (key, url) in [
            ("yealink_rps_url", &self.yealink_rps_url),
            ("poly_ztp_url", &self.poly_ztp_url),
        ] {
            if !(url.starts_with("https://") || url.starts_with("http://"))
                || url.chars().any(|c| c.is_whitespace() || c.is_control())
            {
                return Err(format!(
                    "api.redirection.{key} must be an http(s):// address"
                ));
            }
        }
        Ok(())
    }
}

/// `[api.provisioning]`: what provisioned phones are told — where they
/// register and how, their time server, and the URL they fetch from
/// (`docs/PROVISIONING.md` §5).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApiProvisioningConfig {
    /// The address phones register to, e.g. `sip.example.com`; unset,
    /// the SIP realm.
    #[serde(default)]
    pub sip_server: Option<String>,
    /// Its port; unset, 5061 for TLS and 5060 otherwise.
    #[serde(default)]
    pub sip_port: Option<u16>,
    /// `udp` (the default), `tcp` or `tls`.
    #[serde(default)]
    pub sip_transport: Option<String>,
    /// The time server phones use; unset, `pool.ntp.org`.
    #[serde(default)]
    pub ntp_server: Option<String>,
    /// Where phones reach this admin, e.g. `https://pbx.example.com`, for
    /// the `provisioning_url` a template can hand them; unset,
    /// `[api.sso] public_url`, then the address the request came to.
    #[serde(default)]
    pub public_url: Option<String>,
}

impl ApiProvisioningConfig {
    /// `udp`, `tcp` or `tls`.
    pub fn transport(&self) -> &str {
        self.sip_transport
            .as_deref()
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .unwrap_or("udp")
    }

    pub fn port(&self) -> u16 {
        self.sip_port.unwrap_or(if self.transport() == "tls" {
            5061
        } else {
            5060
        })
    }

    pub fn ntp_server(&self) -> &str {
        self.ntp_server
            .as_deref()
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .unwrap_or("pool.ntp.org")
    }

    pub fn validate(&self) -> std::result::Result<(), String> {
        let transport = self
            .sip_transport
            .as_deref()
            .map(|t| t.trim().to_ascii_lowercase());
        if let Some(t) = &transport {
            if !matches!(t.as_str(), "udp" | "tcp" | "tls") {
                return Err(format!(
                    "api.provisioning.sip_transport `{t}` must be udp, tcp or tls"
                ));
            }
        }
        if self
            .sip_transport
            .as_deref()
            .is_some_and(|t| t != t.trim().to_ascii_lowercase())
        {
            return Err("api.provisioning.sip_transport must be lower case".to_string());
        }
        for (key, value) in [
            ("sip_server", &self.sip_server),
            ("ntp_server", &self.ntp_server),
            ("public_url", &self.public_url),
        ] {
            if let Some(v) = value {
                if v.chars().any(|c| c.is_control() || c.is_whitespace()) {
                    return Err(format!("api.provisioning.{key} must be one word"));
                }
            }
        }
        if let Some(url) = &self.public_url {
            if !(url.starts_with("https://") || url.starts_with("http://")) {
                return Err("api.provisioning.public_url must be an http(s):// address".to_string());
            }
        }
        if self.sip_port == Some(0) {
            return Err("api.provisioning.sip_port must not be 0".to_string());
        }
        Ok(())
    }
}

/// `[api.sso]`: sign-in through a tenant's identity provider
/// (`docs/IDENTITY_AND_DEVICES.md` phase 3).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApiSsoConfig {
    /// Where browsers reach this admin, e.g. `https://pbx.example.com`:
    /// a provider sends them back to `<public_url>/api/v1/auth/sso/callback`,
    /// the address registered with it. Unset, nobody signs in through a
    /// provider (they can still be configured).
    #[serde(default)]
    pub public_url: Option<String>,
    /// Let an issuer be a private or loopback address, over plain HTTP: a
    /// lab's own provider. Off, an issuer must be public and HTTPS.
    #[serde(default)]
    pub allow_private_issuers: bool,
}

impl ApiSsoConfig {
    /// `public_url` without its trailing slash, if set.
    pub fn public_url(&self) -> Option<String> {
        self.public_url
            .as_deref()
            .map(|u| u.trim().trim_end_matches('/').to_string())
            .filter(|u| !u.is_empty())
    }

    pub fn validate(&self) -> std::result::Result<(), String> {
        if let Some(url) = self.public_url() {
            let ok = (url.starts_with("https://") || url.starts_with("http://"))
                && !url[url.find("://").unwrap_or(0) + 3..].contains(['?', '#']);
            if !ok {
                return Err("api.sso.public_url must be an http(s):// address".to_string());
            }
        }
        Ok(())
    }
}

/// `[api.session]`: sign-in sessions. A signed-in browser holds an access
/// token, refreshed from an HttpOnly cookie until the session ends.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApiSessionConfig {
    /// The key access tokens are signed with, 32 characters or more. Unset,
    /// it is derived from `api.auth_token`, as before this setting existed,
    /// and changing that token signs everyone out.
    #[serde(default)]
    pub signing_key: Option<String>,
    /// Keys a rotation replaced: tokens they signed are still accepted
    /// until they expire, so a rotation signs nobody out.
    #[serde(default)]
    pub previous_signing_keys: Vec<String>,
    /// How long a session lasts, however often it is refreshed.
    #[serde(default = "default_session_ttl_secs")]
    pub ttl_secs: u64,
    /// How long an access token lasts; the UI refreshes it before then.
    #[serde(default = "default_session_access_ttl_secs")]
    pub access_ttl_secs: u64,
}

fn default_session_ttl_secs() -> u64 {
    crate::session::DEFAULT_SESSION_TTL.as_secs()
}

fn default_session_access_ttl_secs() -> u64 {
    crate::session::DEFAULT_ACCESS_TTL.as_secs()
}

impl Default for ApiSessionConfig {
    fn default() -> Self {
        Self {
            signing_key: None,
            previous_signing_keys: Vec::new(),
            ttl_secs: default_session_ttl_secs(),
            access_ttl_secs: default_session_access_ttl_secs(),
        }
    }
}

impl ApiSessionConfig {
    /// The keys, from the configured key or else the static token.
    pub fn keys(&self, auth_token: Option<&str>) -> Option<crate::session::SessionKeys> {
        crate::session::SessionKeys::from_config(
            self.signing_key.as_deref().map(str::trim),
            &self.previous_signing_keys,
            auth_token.map(str::trim).filter(|t| !t.is_empty()),
        )
    }

    pub fn validate(&self, auth_token: Option<&str>) -> Result<(), String> {
        if let Some(key) = &self.signing_key {
            reject_crlf("api.session.signing_key", key)?;
            if key.trim().len() < 32 {
                return Err("api.session.signing_key must be 32 characters or more".to_string());
            }
            if auth_token.map(str::trim) == Some(key.trim()) {
                return Err(
                    "api.session.signing_key must be distinct from api.auth_token".to_string(),
                );
            }
        }
        for key in &self.previous_signing_keys {
            reject_crlf("api.session.previous_signing_keys", key)?;
            if key.trim().len() < 32 {
                return Err(
                    "api.session.previous_signing_keys: each key is 32 characters or more"
                        .to_string(),
                );
            }
        }
        if !(60..=3600).contains(&self.access_ttl_secs) {
            return Err("api.session.access_ttl_secs must be between 60 and 3600".to_string());
        }
        if self.ttl_secs < self.access_ttl_secs || self.ttl_secs > 30 * 86_400 {
            return Err(
                "api.session.ttl_secs must be at least access_ttl_secs and at most 30 days"
                    .to_string(),
            );
        }
        Ok(())
    }
}

/// `[api.rate_limit]`: how many requests an API token, and all of a
/// tenant's tokens together, may make a minute. Counted in the admin's
/// Redis when it has one (so every admin node counts together), else in
/// the process. Sessions and the static token are not limited: they are
/// people at the UI and the platform itself.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApiRateLimitConfig {
    /// Whether tokens are limited at all.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Requests one token may make a minute.
    #[serde(default = "default_per_token_per_minute")]
    pub per_token_per_minute: u32,
    /// Requests all of one tenant's tokens may make a minute.
    #[serde(default = "default_per_tenant_per_minute")]
    pub per_tenant_per_minute: u32,
}

fn default_per_token_per_minute() -> u32 {
    600
}

fn default_per_tenant_per_minute() -> u32 {
    3000
}

impl Default for ApiRateLimitConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            per_token_per_minute: default_per_token_per_minute(),
            per_tenant_per_minute: default_per_tenant_per_minute(),
        }
    }
}

impl Default for ApiConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            bind: "127.0.0.1:8080".parse().unwrap(),
            metrics_enabled: true,
            metrics_path: "/metrics".to_string(),
            auth_enabled: false,
            auth_token: None,
            provisioning_require_https: true,
            allow_public_signup: false,
            deployment_mode: crate::permissions::DeploymentMode::default(),
            rate_limit: ApiRateLimitConfig::default(),
            session: ApiSessionConfig::default(),
            sso: ApiSsoConfig::default(),
            provisioning: ApiProvisioningConfig::default(),
            redirection: ApiRedirectionConfig::default(),
            firmware: ApiFirmwareConfig::default(),
        }
    }
}

impl ApiConfig {
    pub fn validate(&self) -> Result<(), String> {
        reject_crlf("api.metrics_path", &self.metrics_path)?;
        if !self.metrics_path.starts_with('/') {
            return Err("api.metrics_path must start with '/'".to_string());
        }
        let has_token = self
            .auth_token
            .as_deref()
            .map(str::trim)
            .is_some_and(|token| !token.is_empty());
        if let Some(token) = &self.auth_token {
            reject_crlf("api.auth_token", token)?;
        }
        if self.allow_public_signup && !has_token {
            return Err(
                "api.auth_token must be set when api.allow_public_signup = true".to_string(),
            );
        }
        self.session.validate(self.auth_token.as_deref())?;
        self.provisioning.validate()?;
        self.redirection.validate()?;
        self.firmware.validate()?;
        Ok(())
    }
}

/// Observability configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObservabilityConfig {
    /// Log level (trace, debug, info, warn, error)
    pub log_level: String,

    /// Log format (json, pretty, compact)
    pub log_format: LogFormat,

    /// Enable OpenTelemetry tracing
    pub tracing_enabled: bool,

    /// OpenTelemetry collector endpoint
    pub tracing_endpoint: Option<String>,

    /// Service name for tracing
    pub service_name: String,
}

impl Default for ObservabilityConfig {
    fn default() -> Self {
        Self {
            log_level: "info".to_string(),
            log_format: LogFormat::Pretty,
            tracing_enabled: false,
            tracing_endpoint: None,
            service_name: "fcp".to_string(),
        }
    }
}

impl ObservabilityConfig {
    pub fn validate(&self) -> Result<(), String> {
        reject_crlf("observability.log_level", &self.log_level)?;
        reject_crlf("observability.service_name", &self.service_name)?;
        if let Some(endpoint) = &self.tracing_endpoint {
            reject_crlf("observability.tracing_endpoint", endpoint)?;
        }
        Ok(())
    }
}

/// Log output format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogFormat {
    /// Human-readable format
    Pretty,

    /// JSON format (for log aggregation)
    Json,

    /// Compact format
    Compact,
}

/// Routing configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingConfig {
    /// Enable routing policy enforcement
    pub enabled: bool,

    /// Routing backend (YAML file or database)
    pub backend: RoutingBackend,

    /// Default tenant ID for single-tenant deployments
    pub default_tenant_id: String,
}

impl Default for RoutingConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            backend: RoutingBackend::Yaml {
                path: PathBuf::from("routing.yaml"),
                watch: true,
            },
            default_tenant_id: "default".to_string(),
        }
    }
}

impl RoutingConfig {
    pub fn validate(&self) -> Result<(), String> {
        reject_crlf("routing.default_tenant_id", &self.default_tenant_id)?;
        match &self.backend {
            RoutingBackend::Yaml { path, .. } => {
                if let Some(path_str) = path.to_str() {
                    reject_crlf("routing.backend.yaml.path", path_str)?;
                }
            }
            RoutingBackend::Database { connection_string } => {
                reject_crlf(
                    "routing.backend.database.connection_string",
                    connection_string,
                )?;
            }
        }
        Ok(())
    }
}

/// Routing backend types.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum RoutingBackend {
    /// YAML file-based storage (for small deployments)
    Yaml {
        /// Path to routing.yaml file
        path: PathBuf,
        /// Watch for file changes and hot-reload
        watch: bool,
    },
    /// Database-backed storage (for large deployments) - FUTURE
    Database {
        /// Database connection string
        connection_string: String,
    },
}

/// Media configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaConfig {
    /// Enable media anchoring (RTP flows through FCP server)
    /// If false, all calls use pass-through mode (RTP flows direct between endpoints)
    pub enabled: bool,

    /// RTP port range start
    pub rtp_port_start: u16,

    /// RTP port range end
    pub rtp_port_end: u16,

    /// Enable SRTP encryption
    pub enable_srtp: bool,

    /// Enable transcoding between different codecs
    pub enable_transcoding: bool,

    /// Enable call recording
    pub enable_recording: bool,

    /// Enable DTMF detection and relay
    pub enable_dtmf: bool,

    /// Public IP address for RTP (if behind NAT)
    pub public_ip: Option<String>,

    /// STUN server for NAT traversal (optional)
    pub stun_server: Option<String>,

    /// TURN server for NAT traversal (optional)
    pub turn_server: Option<String>,

    /// Holds answered by the node with music, which anchors every call
    /// (`docs/ENTERPRISE_CALLING.md` §3.3).
    #[serde(default)]
    pub hold_music: bool,
}

impl Default for MediaConfig {
    fn default() -> Self {
        Self {
            enabled: false, // Disabled by default - use pass-through mode
            rtp_port_start: 10000,
            rtp_port_end: 20000,
            enable_srtp: false,
            enable_transcoding: false,
            enable_recording: false,
            enable_dtmf: true,
            public_ip: None,
            stun_server: None,
            turn_server: None,
            hold_music: false,
        }
    }
}

impl MediaConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.rtp_port_start == 0 || self.rtp_port_end == 0 {
            return Err(
                "media.rtp_port_start and media.rtp_port_end must be greater than zero".to_string(),
            );
        }
        if self.rtp_port_start > self.rtp_port_end {
            return Err("media.rtp_port_start cannot exceed media.rtp_port_end".to_string());
        }
        if self.rtp_port_end - self.rtp_port_start > MAX_RTP_PORT_SPAN {
            return Err(format!(
                "media RTP port span is too large (max {} ports)",
                MAX_RTP_PORT_SPAN
            ));
        }
        if let Some(ip) = &self.public_ip {
            reject_crlf("media.public_ip", ip)?;
        }
        if let Some(stun) = &self.stun_server {
            reject_crlf("media.stun_server", stun)?;
        }
        if let Some(turn) = &self.turn_server {
            reject_crlf("media.turn_server", turn)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod masking_tests {
    use super::*;

    #[test]
    fn secrets_are_masked_and_nothing_else_is() {
        #[derive(Serialize)]
        struct Http {
            bind_address: String,
            api_token: Option<String>,
            token_ttl_secs: u64,
            require_token: bool,
            token_file: Option<String>,
        }
        #[derive(Serialize)]
        struct Auth {
            tokens: Vec<String>,
            password: String,
            jwt_secret: String,
        }
        #[derive(Serialize)]
        struct Cfg {
            http: Http,
            auth: Auth,
        }
        let cfg = Cfg {
            http: Http {
                bind_address: "127.0.0.1:8084".into(),
                api_token: Some("hunter2".into()),
                token_ttl_secs: 3600,
                require_token: true,
                token_file: Some("/run/secrets/t".into()),
            },
            auth: Auth {
                tokens: vec!["a".into(), "".into()],
                password: "".into(),
                jwt_secret: "s".into(),
            },
        };
        let out = masked_toml(&cfg).unwrap();
        assert!(!out.contains("hunter2"), "{out}");
        assert!(out.contains("api_token = \"***\""), "{out}");
        assert!(out.contains("bind_address = \"127.0.0.1:8084\""), "{out}");
        assert!(out.contains("token_ttl_secs = 3600"), "{out}");
        assert!(out.contains("require_token = true"), "{out}");
        assert!(out.contains("token_file = \"/run/secrets/t\""), "{out}");
        assert!(!out.contains("\"a\""), "{out}");
        assert!(out.contains("\"***\""), "{out}");
        assert!(out.contains("password = \"\""), "unset stays unset: {out}");
        assert!(out.contains("jwt_secret = \"***\""), "{out}");
    }

    #[test]
    fn a_url_keeps_its_user_and_loses_its_password() {
        assert_eq!(
            mask_url_password("postgresql://fcp:fcp_password@localhost:5432/fcp").as_deref(),
            Some("postgresql://fcp:***@localhost:5432/fcp")
        );
        assert_eq!(
            mask_url_password("redis://:secret@redis:6379/1").as_deref(),
            Some("redis://:***@redis:6379/1")
        );
        assert_eq!(mask_url_password("redis://redis:6379"), None);
        assert_eq!(mask_url_password("http://host/a:b@c"), None);
        assert_eq!(mask_url_password("sip:alice@example.com"), None);
    }
}
