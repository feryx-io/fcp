# FCP Phase 1: MVP Implementation Plan
## "Rust Asterisk-lite" - Months 1-4 (16 weeks)

---

## Executive Summary

**Goal:** Build a single-node PBX that can:
- Register SIP endpoints (phones, softphones, gateways)
- Route calls between registered endpoints
- Terminate SIP trunks (connect to PSTN via SIP providers)
- Provide a REST API for basic operations
- Expose Prometheus metrics

**End State Demo:**
```
┌──────────────┐         ┌─────────────┐         ┌──────────────┐
│  Odin     │◄───────►│             │◄───────►│   Odin    │
│  (Odin) │   SIP    │     FCP     │   SIP    │  (Odin)  │
│  Ext 1001  │◄───RTP──►│   Server    │◄───RTP──►│  Ext 1002  │
└──────────────┘         │             │         └──────────────┘
                         │             │
                         │             │         ┌──────────────┐
                         │             │◄───────►│  SIP Trunk   │
                         │             │   SIP    │  (Telnyx/    │
                         │             │◄───RTP──►│   Twilio)    │
                         └─────────────┘         └──────────────┘
                               │
                               │ REST API
                               ▼
                         ┌─────────────┐
                         │   Admin/    │
                         │   Client    │
                         └─────────────┘
```

---

## Sprint Overview (8 x 2-week sprints)

```
Week  1-2:  Sprint 1 - Foundation (fcp-core, siphon-core parser)
Week  3-4:  Sprint 2 - SIP Messages (complete parser, serializer)
Week  5-6:  Sprint 3 - Transport (UDP, connection handling)
Week  7-8:  Sprint 4 - Transactions (state machines, timers)
Week  9-10: Sprint 5 - Registrar & Proxy (registration, basic routing)
Week 11-12: Sprint 6 - B2BUA & RTP (call control, media relay)
Week 13-14: Sprint 7 - Storage & API (SQLite, REST endpoints)
Week 15-16: Sprint 8 - Integration & Polish (CLI, Docker, testing)
```

---

## Dependency Graph

```
                                    ┌─────────────┐
                                    │  fcp-core   │
                                    └──────┬──────┘
                                           │
                    ┌──────────────────────┼──────────────────────┐
                    │                      │                      │
                    ▼                      ▼                      ▼
             ┌─────────────┐       ┌─────────────┐       ┌─────────────┐
             │siphon-core  │       │  fcp-rtp    │       │fcp-storage  │
             │  (parser)   │       │  (media)    │       │  (sqlite)   │
             └──────┬──────┘       └──────┬──────┘       └──────┬──────┘
                    │                     │                     │
                    ▼                     │                     │
             ┌─────────────┐              │                     │
             │  siphon-    │              │                     │
             │  transport  │              │                     │
             └──────┬──────┘              │                     │
                    │                     │                     │
                    ▼                     │                     │
             ┌─────────────┐              │                     │
             │  siphon-    │              │                     │
             │ transaction │              │                     │
             └──────┬──────┘              │                     │
                    │                     │                     │
                    ▼                     │                     │
             ┌─────────────┐              │                     │
             │  siphon-    │              │                     │
             │   dialog    │              │                     │
             └──────┬──────┘              │                     │
                    │                     │                     │
         ┌─────────┬┴─────────┐          │                     │
         │         │          │          │                     │
         ▼         ▼          ▼          │                     │
   ┌──────────┐ ┌──────────┐ ┌──────────┐│                     │
   │registrar │ │  proxy   │ │  b2bua   │◄─────────────────────┘
   └────┬─────┘ └────┬─────┘ └────┬─────┘│
        │            │            │      │
        └────────────┼────────────┘      │
                     │                   │
                     ▼                   │
              ┌─────────────┐            │
              │ fcp-server  │◄───────────┘
              │  (runtime)  │
              └──────┬──────┘
                     │
          ┌──────────┼──────────┐
          │          │          │
          ▼          ▼          ▼
    ┌──────────┐ ┌──────────┐ ┌──────────┐
    │ fcp-api  │ │ fcp-cli  │ │  Docker  │
    │  (rest)  │ │          │ │  Image   │
    └──────────┘ └──────────┘ └──────────┘
```

---

## Sprint 1: Foundation (Weeks 1-2)

### Goals
- Set up repository structure and CI/CD
- Implement `fcp-core` with all foundational types
- Begin `siphon-core` SIP parser (headers, URIs)

### 1.1 Repository Setup

```bash
# Directory structure after Sprint 1
fcp/
├── .github/
│   └── workflows/
│       ├── ci.yml           # Build, test, lint on PR
│       └── release.yml      # Release builds
├── Cargo.toml               # Workspace root
├── rust-toolchain.toml      # Pin Rust version
├── clippy.toml              # Clippy configuration
├── rustfmt.toml             # Format configuration
├── deny.toml                # cargo-deny config
├── crates/
│   ├── fcp-core/
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── error.rs
│   │       ├── identifiers.rs
│   │       ├── config.rs
│   │       └── ...
│   └── siphon/
│       └── siphon-core/
│           ├── Cargo.toml
│           └── src/
│               ├── lib.rs
│               ├── uri.rs
│               ├── headers.rs
│               ├── message.rs
│               └── parser/
│                   ├── mod.rs
│                   ├── uri.rs
│                   └── headers.rs
└── tests/
    └── integration/
```

### 1.2 fcp-core Specification

```rust
// crates/fcp-core/src/lib.rs

//! FCP Core - Foundation types and traits
//!
//! This crate provides the shared abstractions used throughout FCP.

pub mod error;
pub mod identifiers;
pub mod config;
pub mod service;
pub mod codec;
pub mod transport;
pub mod time;

// Re-exports
pub use error::{Error, Result};
pub use identifiers::*;
```

#### Error Types

```rust
// crates/fcp-core/src/error.rs

use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Error, Debug)]
pub enum Error {
    // SIP Errors
    #[error("SIP parse error: {0}")]
    Parse(#[from] ParseError),
    
    #[error("SIP protocol error: {0}")]
    Protocol(String),
    
    #[error("Transaction error: {0}")]
    Transaction(#[from] TransactionError),
    
    #[error("Dialog error: {0}")]
    Dialog(#[from] DialogError),
    
    // Transport Errors
    #[error("Transport error: {0}")]
    Transport(#[from] TransportError),
    
    #[error("Connection refused: {0}")]
    ConnectionRefused(String),
    
    #[error("Connection timeout")]
    ConnectionTimeout,
    
    // Storage Errors
    #[error("Storage error: {0}")]
    Storage(#[from] StorageError),
    
    #[error("Not found: {0}")]
    NotFound(String),
    
    #[error("Already exists: {0}")]
    AlreadyExists(String),
    
    // Media Errors
    #[error("Media error: {0}")]
    Media(#[from] MediaError),
    
    #[error("Port exhausted")]
    PortExhausted,
    
    // Config Errors
    #[error("Configuration error: {0}")]
    Config(String),
    
    // Generic
    #[error("Internal error: {0}")]
    Internal(String),
    
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

#[derive(Error, Debug)]
pub enum ParseError {
    #[error("Invalid SIP message")]
    InvalidMessage,
    
    #[error("Invalid URI: {0}")]
    InvalidUri(String),
    
    #[error("Invalid header: {0}")]
    InvalidHeader(String),
    
    #[error("Missing required header: {0}")]
    MissingHeader(&'static str),
    
    #[error("Invalid method: {0}")]
    InvalidMethod(String),
    
    #[error("Invalid status code: {0}")]
    InvalidStatusCode(u16),
    
    #[error("Incomplete message")]
    Incomplete,
    
    #[error("UTF-8 error: {0}")]
    Utf8(#[from] std::str::Utf8Error),
}

#[derive(Error, Debug)]
pub enum TransactionError {
    #[error("Transaction timeout")]
    Timeout,
    
    #[error("Transaction already exists")]
    AlreadyExists,
    
    #[error("Transaction not found")]
    NotFound,
    
    #[error("Invalid state transition: {from:?} -> {to:?}")]
    InvalidStateTransition {
        from: String,
        to: String,
    },
}

#[derive(Error, Debug)]
pub enum DialogError {
    #[error("Dialog not found")]
    NotFound,
    
    #[error("Dialog already exists")]
    AlreadyExists,
    
    #[error("Invalid dialog state")]
    InvalidState,
    
    #[error("Sequence number error: expected {expected}, got {got}")]
    SequenceError { expected: u32, got: u32 },
}

#[derive(Error, Debug)]
pub enum TransportError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    
    #[error("DNS resolution failed: {0}")]
    DnsResolution(String),
    
    #[error("Message too large: {size} bytes (max {max})")]
    MessageTooLarge { size: usize, max: usize },
    
    #[error("Connection closed")]
    ConnectionClosed,
}

#[derive(Error, Debug)]
pub enum StorageError {
    #[error("Database error: {0}")]
    Database(String),
    
    #[error("Serialization error: {0}")]
    Serialization(String),
    
    #[error("Migration failed: {0}")]
    Migration(String),
}

#[derive(Error, Debug)]
pub enum MediaError {
    #[error("RTP error: {0}")]
    Rtp(String),
    
    #[error("RTCP error: {0}")]
    Rtcp(String),
    
    #[error("Codec not supported: {0}")]
    UnsupportedCodec(String),
    
    #[error("SDP error: {0}")]
    Sdp(String),
}
```

#### Identifiers

```rust
// crates/fcp-core/src/identifiers.rs

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
use uuid::Uuid;

/// Macro to define strongly-typed UUID identifiers
macro_rules! define_id {
    ($name:ident, $prefix:expr) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(Uuid);

        impl $name {
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }

            pub fn from_uuid(uuid: Uuid) -> Self {
                Self(uuid)
            }

            pub fn as_uuid(&self) -> &Uuid {
                &self.0
            }

            pub fn nil() -> Self {
                Self(Uuid::nil())
            }
            
            pub fn is_nil(&self) -> bool {
                self.0.is_nil()
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}-{}", $prefix, &self.0.to_string()[..8])
            }
        }

        impl FromStr for $name {
            type Err = uuid::Error;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                // Handle both "prefix-uuid" and raw UUID formats
                let uuid_str = s.strip_prefix(concat!($prefix, "-")).unwrap_or(s);
                Ok(Self(Uuid::parse_str(uuid_str)?))
            }
        }
    };
}

// Core identifiers with prefixes for debugging
define_id!(CallId, "call");
define_id!(SessionId, "sess");
define_id!(DialogId, "dlg");
define_id!(TransactionId, "txn");
define_id!(RegistrationId, "reg");
define_id!(NodeId, "node");
define_id!(TenantId, "tenant");

/// SIP Call-ID header value (not our internal CallId)
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SipCallId(String);

impl SipCallId {
    pub fn new() -> Self {
        // Format: uuid@host
        Self(format!("{}@fcp.local", Uuid::new_v4()))
    }

    pub fn from_string(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for SipCallId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for SipCallId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<String> for SipCallId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<&str> for SipCallId {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

/// Address of Record - canonical SIP identity
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Aor {
    pub user: String,
    pub host: String,
}

impl Aor {
    pub fn new(user: impl Into<String>, host: impl Into<String>) -> Self {
        Self {
            user: user.into(),
            host: host.into(),
        }
    }

    /// Parse from "user@host" or "sip:user@host" format
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.strip_prefix("sip:").or_else(|| s.strip_prefix("sips:")).unwrap_or(s);
        let parts: Vec<&str> = s.split('@').collect();
        if parts.len() == 2 {
            Some(Self {
                user: parts[0].to_string(),
                host: parts[1].to_string(),
            })
        } else {
            None
        }
    }

    pub fn to_uri(&self) -> String {
        format!("sip:{}@{}", self.user, self.host)
    }
}

impl fmt::Display for Aor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}@{}", self.user, self.host)
    }
}

impl FromStr for Aor {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s).ok_or("Invalid AOR format")
    }
}
```

### 1.3 siphon-core: URI Parser

```rust
// crates/siphon/siphon-core/src/uri.rs

use std::collections::HashMap;
use std::fmt;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::str::FromStr;

/// SIP URI scheme
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Scheme {
    Sip,
    Sips,
}

impl Scheme {
    pub fn as_str(&self) -> &'static str {
        match self {
            Scheme::Sip => "sip",
            Scheme::Sips => "sips",
        }
    }
    
    pub fn default_port(&self) -> u16 {
        match self {
            Scheme::Sip => 5060,
            Scheme::Sips => 5061,
        }
    }
}

impl fmt::Display for Scheme {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Host part of a URI
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Host {
    Domain(String),
    Ipv4(Ipv4Addr),
    Ipv6(Ipv6Addr),
}

impl Host {
    pub fn parse(s: &str) -> Result<Self, ParseError> {
        // Try IPv6 first (with brackets)
        if s.starts_with('[') && s.ends_with(']') {
            let inner = &s[1..s.len()-1];
            return inner.parse::<Ipv6Addr>()
                .map(Host::Ipv6)
                .map_err(|_| ParseError::InvalidUri(format!("Invalid IPv6: {}", s)));
        }
        
        // Try IPv4
        if let Ok(ip) = s.parse::<Ipv4Addr>() {
            return Ok(Host::Ipv4(ip));
        }
        
        // Domain name
        // Basic validation: non-empty, valid characters
        if s.is_empty() {
            return Err(ParseError::InvalidUri("Empty host".to_string()));
        }
        
        Ok(Host::Domain(s.to_lowercase()))
    }
}

impl fmt::Display for Host {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Host::Domain(d) => write!(f, "{}", d),
            Host::Ipv4(ip) => write!(f, "{}", ip),
            Host::Ipv6(ip) => write!(f, "[{}]", ip),
        }
    }
}

/// Transport protocol parameter
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Transport {
    #[default]
    Udp,
    Tcp,
    Tls,
    Sctp,
    Ws,
    Wss,
}

impl Transport {
    pub fn as_str(&self) -> &'static str {
        match self {
            Transport::Udp => "udp",
            Transport::Tcp => "tcp",
            Transport::Tls => "tls",
            Transport::Sctp => "sctp",
            Transport::Ws => "ws",
            Transport::Wss => "wss",
        }
    }
    
    pub fn default_port(&self) -> u16 {
        match self {
            Transport::Udp | Transport::Tcp | Transport::Sctp => 5060,
            Transport::Tls => 5061,
            Transport::Ws => 80,
            Transport::Wss => 443,
        }
    }
    
    pub fn is_reliable(&self) -> bool {
        !matches!(self, Transport::Udp)
    }
    
    pub fn is_secure(&self) -> bool {
        matches!(self, Transport::Tls | Transport::Wss)
    }
}

impl FromStr for Transport {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "udp" => Ok(Transport::Udp),
            "tcp" => Ok(Transport::Tcp),
            "tls" => Ok(Transport::Tls),
            "sctp" => Ok(Transport::Sctp),
            "ws" => Ok(Transport::Ws),
            "wss" => Ok(Transport::Wss),
            _ => Err(ParseError::InvalidUri(format!("Unknown transport: {}", s))),
        }
    }
}

impl fmt::Display for Transport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// URI parameters
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UriParams {
    /// Transport parameter
    pub transport: Option<Transport>,
    /// User parameter (phone, ip)
    pub user: Option<String>,
    /// Method parameter
    pub method: Option<String>,
    /// TTL parameter
    pub ttl: Option<u8>,
    /// Multicast address
    pub maddr: Option<Host>,
    /// Loose routing flag
    pub lr: bool,
    /// Other parameters
    pub other: HashMap<String, Option<String>>,
}

impl UriParams {
    pub fn new() -> Self {
        Self::default()
    }
    
    /// Parse parameters from ";param=value;param2" format
    pub fn parse(s: &str) -> Result<Self, ParseError> {
        let mut params = Self::default();
        
        for part in s.split(';').filter(|p| !p.is_empty()) {
            let (name, value) = if let Some(eq_pos) = part.find('=') {
                (&part[..eq_pos], Some(&part[eq_pos+1..]))
            } else {
                (part, None)
            };
            
            match name.to_lowercase().as_str() {
                "transport" => {
                    params.transport = Some(value
                        .ok_or_else(|| ParseError::InvalidUri("transport requires value".into()))?
                        .parse()?);
                }
                "user" => {
                    params.user = value.map(|v| v.to_string());
                }
                "method" => {
                    params.method = value.map(|v| v.to_uppercase());
                }
                "ttl" => {
                    params.ttl = value
                        .and_then(|v| v.parse().ok());
                }
                "maddr" => {
                    params.maddr = value
                        .map(Host::parse)
                        .transpose()?;
                }
                "lr" => {
                    params.lr = true;
                }
                other => {
                    params.other.insert(
                        other.to_lowercase(),
                        value.map(|v| v.to_string())
                    );
                }
            }
        }
        
        Ok(params)
    }
    
    pub fn is_empty(&self) -> bool {
        self.transport.is_none()
            && self.user.is_none()
            && self.method.is_none()
            && self.ttl.is_none()
            && self.maddr.is_none()
            && !self.lr
            && self.other.is_empty()
    }
}

impl fmt::Display for UriParams {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(ref t) = self.transport {
            write!(f, ";transport={}", t)?;
        }
        if let Some(ref u) = self.user {
            write!(f, ";user={}", u)?;
        }
        if let Some(ref m) = self.method {
            write!(f, ";method={}", m)?;
        }
        if let Some(ttl) = self.ttl {
            write!(f, ";ttl={}", ttl)?;
        }
        if let Some(ref m) = self.maddr {
            write!(f, ";maddr={}", m)?;
        }
        if self.lr {
            write!(f, ";lr")?;
        }
        for (k, v) in &self.other {
            if let Some(val) = v {
                write!(f, ";{}={}", k, val)?;
            } else {
                write!(f, ";{}", k)?;
            }
        }
        Ok(())
    }
}

/// Complete SIP URI
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Uri {
    /// Scheme (sip or sips)
    pub scheme: Scheme,
    /// User part
    pub user: Option<String>,
    /// Password (rarely used)
    pub password: Option<String>,
    /// Host
    pub host: Host,
    /// Port
    pub port: Option<u16>,
    /// URI parameters
    pub params: UriParams,
    /// URI headers (after ?)
    pub headers: HashMap<String, String>,
}

impl Uri {
    /// Create a new SIP URI
    pub fn new(host: Host) -> Self {
        Self {
            scheme: Scheme::Sip,
            user: None,
            password: None,
            host,
            port: None,
            params: UriParams::default(),
            headers: HashMap::new(),
        }
    }
    
    /// Create a SIP URI with user and host
    pub fn sip(user: impl Into<String>, host: impl Into<String>) -> Self {
        Self {
            scheme: Scheme::Sip,
            user: Some(user.into()),
            password: None,
            host: Host::Domain(host.into()),
            port: None,
            params: UriParams::default(),
            headers: HashMap::new(),
        }
    }
    
    /// Parse a URI from string
    pub fn parse(input: &str) -> Result<Self, ParseError> {
        UriParser::parse(input)
    }
    
    /// Get effective port (explicit or default for scheme/transport)
    pub fn effective_port(&self) -> u16 {
        self.port.unwrap_or_else(|| {
            self.params.transport
                .map(|t| t.default_port())
                .unwrap_or_else(|| self.scheme.default_port())
        })
    }
    
    /// Get the Address of Record (strips parameters/headers)
    pub fn aor(&self) -> Self {
        Self {
            scheme: self.scheme,
            user: self.user.clone(),
            password: None,
            host: self.host.clone(),
            port: None,
            params: UriParams::default(),
            headers: HashMap::new(),
        }
    }
    
    /// Check if this URI matches another for routing
    pub fn matches(&self, other: &Uri) -> bool {
        self.scheme == other.scheme
            && self.user == other.user
            && self.host == other.host
            && self.effective_port() == other.effective_port()
    }
    
    /// Builder pattern
    pub fn with_user(mut self, user: impl Into<String>) -> Self {
        self.user = Some(user.into());
        self
    }
    
    pub fn with_port(mut self, port: u16) -> Self {
        self.port = Some(port);
        self
    }
    
    pub fn with_transport(mut self, transport: Transport) -> Self {
        self.params.transport = Some(transport);
        self
    }
    
    pub fn with_lr(mut self) -> Self {
        self.params.lr = true;
        self
    }
}

impl fmt::Display for Uri {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:", self.scheme)?;
        
        if let Some(ref user) = self.user {
            write!(f, "{}", user)?;
            if let Some(ref pass) = self.password {
                write!(f, ":{}", pass)?;
            }
            write!(f, "@")?;
        }
        
        write!(f, "{}", self.host)?;
        
        if let Some(port) = self.port {
            write!(f, ":{}", port)?;
        }
        
        write!(f, "{}", self.params)?;
        
        if !self.headers.is_empty() {
            write!(f, "?")?;
            let mut first = true;
            for (k, v) in &self.headers {
                if !first {
                    write!(f, "&")?;
                }
                write!(f, "{}={}", k, v)?;
                first = false;
            }
        }
        
        Ok(())
    }
}

impl FromStr for Uri {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Uri::parse(s)
    }
}

/// URI Parser (nom-based or hand-written)
struct UriParser;

impl UriParser {
    pub fn parse(input: &str) -> Result<Uri, ParseError> {
        // Format: scheme:user:password@host:port;params?headers
        
        // 1. Parse scheme
        let (scheme, rest) = if input.starts_with("sips:") {
            (Scheme::Sips, &input[5..])
        } else if input.starts_with("sip:") {
            (Scheme::Sip, &input[4..])
        } else {
            return Err(ParseError::InvalidUri("Missing or invalid scheme".into()));
        };
        
        // 2. Split off headers (after ?)
        let (rest, headers) = if let Some(q_pos) = rest.find('?') {
            let headers_str = &rest[q_pos+1..];
            let headers = Self::parse_headers(headers_str)?;
            (&rest[..q_pos], headers)
        } else {
            (rest, HashMap::new())
        };
        
        // 3. Split off params (after ;)
        let (rest, params) = if let Some(semi_pos) = rest.find(';') {
            let params = UriParams::parse(&rest[semi_pos..])?;
            (&rest[..semi_pos], params)
        } else {
            (rest, UriParams::default())
        };
        
        // 4. Parse userinfo and hostport
        let (user, password, hostport) = if let Some(at_pos) = rest.find('@') {
            let userinfo = &rest[..at_pos];
            let hostport = &rest[at_pos+1..];
            
            let (user, password) = if let Some(colon_pos) = userinfo.find(':') {
                (
                    Some(userinfo[..colon_pos].to_string()),
                    Some(userinfo[colon_pos+1..].to_string())
                )
            } else {
                (Some(userinfo.to_string()), None)
            };
            
            (user, password, hostport)
        } else {
            (None, None, rest)
        };
        
        // 5. Parse host:port
        let (host, port) = Self::parse_hostport(hostport)?;
        
        Ok(Uri {
            scheme,
            user,
            password,
            host,
            port,
            params,
            headers,
        })
    }
    
    fn parse_hostport(s: &str) -> Result<(Host, Option<u16>), ParseError> {
        // Handle IPv6 with brackets
        if s.starts_with('[') {
            if let Some(bracket_end) = s.find(']') {
                let host = Host::parse(&s[..=bracket_end])?;
                let port = if s.len() > bracket_end + 1 && s.as_bytes()[bracket_end + 1] == b':' {
                    Some(s[bracket_end+2..].parse()
                        .map_err(|_| ParseError::InvalidUri("Invalid port".into()))?)
                } else {
                    None
                };
                return Ok((host, port));
            }
        }
        
        // Regular host:port
        if let Some(colon_pos) = s.rfind(':') {
            // Check if it's actually a port (all digits after colon)
            let potential_port = &s[colon_pos+1..];
            if potential_port.chars().all(|c| c.is_ascii_digit()) {
                let host = Host::parse(&s[..colon_pos])?;
                let port = potential_port.parse()
                    .map_err(|_| ParseError::InvalidUri("Invalid port".into()))?;
                return Ok((host, Some(port)));
            }
        }
        
        // Just host, no port
        Ok((Host::parse(s)?, None))
    }
    
    fn parse_headers(s: &str) -> Result<HashMap<String, String>, ParseError> {
        let mut headers = HashMap::new();
        for part in s.split('&') {
            if let Some(eq_pos) = part.find('=') {
                let key = &part[..eq_pos];
                let value = &part[eq_pos+1..];
                headers.insert(key.to_string(), value.to_string());
            }
        }
        Ok(headers)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_uri() {
        let uri = Uri::parse("sip:alice@example.com").unwrap();
        assert_eq!(uri.scheme, Scheme::Sip);
        assert_eq!(uri.user, Some("alice".to_string()));
        assert_eq!(uri.host, Host::Domain("example.com".to_string()));
        assert_eq!(uri.port, None);
    }

    #[test]
    fn test_parse_uri_with_port() {
        let uri = Uri::parse("sip:alice@example.com:5080").unwrap();
        assert_eq!(uri.port, Some(5080));
    }

    #[test]
    fn test_parse_uri_with_params() {
        let uri = Uri::parse("sip:alice@example.com;transport=tcp;lr").unwrap();
        assert_eq!(uri.params.transport, Some(Transport::Tcp));
        assert!(uri.params.lr);
    }

    #[test]
    fn test_parse_uri_ipv4() {
        let uri = Uri::parse("sip:100@192.168.1.1:5060").unwrap();
        assert_eq!(uri.host, Host::Ipv4("192.168.1.1".parse().unwrap()));
    }

    #[test]
    fn test_parse_uri_ipv6() {
        let uri = Uri::parse("sip:user@[::1]:5060").unwrap();
        assert_eq!(uri.host, Host::Ipv6("::1".parse().unwrap()));
    }

    #[test]
    fn test_uri_display() {
        let uri = Uri::sip("alice", "example.com")
            .with_port(5080)
            .with_transport(Transport::Tcp)
            .with_lr();
        assert_eq!(uri.to_string(), "sip:alice@example.com:5080;transport=tcp;lr");
    }
    
    #[test]
    fn test_uri_aor() {
        let uri = Uri::parse("sip:alice@example.com:5080;transport=tcp").unwrap();
        let aor = uri.aor();
        assert_eq!(aor.to_string(), "sip:alice@example.com");
    }
}
```

### 1.4 Sprint 1 Tasks

| Task | Est. Hours | Owner | Status |
|------|-----------|-------|--------|
| Repository setup (Cargo workspace, CI/CD) | 4 | - | ☐ |
| `fcp-core`: Error types | 4 | - | ☐ |
| `fcp-core`: Identifiers | 4 | - | ☐ |
| `fcp-core`: Config structs | 6 | - | ☐ |
| `fcp-core`: Transport enums | 2 | - | ☐ |
| `fcp-core`: Codec definitions | 2 | - | ☐ |
| `siphon-core`: URI types | 8 | - | ☐ |
| `siphon-core`: URI parser | 12 | - | ☐ |
| `siphon-core`: URI tests | 6 | - | ☐ |
| Documentation | 4 | - | ☐ |
| **Total** | **52** | | |

### 1.5 Sprint 1 Definition of Done

- [ ] All code compiles with no warnings
- [ ] `cargo clippy` passes with no warnings
- [ ] `cargo fmt --check` passes
- [ ] Unit test coverage > 80% for parsers
- [ ] CI pipeline passing (build, test, lint)
- [ ] URI parser handles RFC 3261 examples
- [ ] Documentation for all public APIs

---

## Sprint 2: SIP Messages (Weeks 3-4)

### Goals
- Complete SIP message parser (headers, bodies)
- Implement message serialization
- Define all SIP methods and status codes

### 2.1 Header Types

```rust
// crates/siphon/siphon-core/src/headers.rs

use crate::uri::Uri;
use std::fmt;

/// Via header (RFC 3261 Section 20.42)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Via {
    /// Protocol version (always "SIP/2.0")
    pub version: SipVersion,
    /// Transport protocol
    pub transport: Transport,
    /// Sent-by host
    pub host: Host,
    /// Sent-by port
    pub port: Option<u16>,
    /// Via parameters
    pub params: ViaParams,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ViaParams {
    /// Branch parameter (transaction ID, starts with z9hG4bK)
    pub branch: String,
    /// Received parameter (actual source IP)
    pub received: Option<String>,
    /// RPort parameter (actual source port)
    pub rport: Option<u16>,
    /// RPort flag (;rport with no value)
    pub rport_flag: bool,
    /// TTL for multicast
    pub ttl: Option<u8>,
    /// Multicast address
    pub maddr: Option<Host>,
    /// Other parameters
    pub other: HashMap<String, Option<String>>,
}

impl Via {
    pub fn new(transport: Transport, host: Host) -> Self {
        Self {
            version: SipVersion::V2_0,
            transport,
            host,
            port: None,
            params: ViaParams {
                branch: generate_branch(),
                ..Default::default()
            },
        }
    }
    
    /// Get sent-by address for routing responses
    pub fn sent_by(&self) -> (&Host, u16) {
        let port = self.port.unwrap_or(self.transport.default_port());
        (&self.host, port)
    }
    
    /// Get response destination (received/rport if present, else sent-by)
    pub fn response_addr(&self) -> (String, u16) {
        let host = self.params.received
            .clone()
            .unwrap_or_else(|| self.host.to_string());
        let port = self.params.rport
            .or(self.port)
            .unwrap_or(self.transport.default_port());
        (host, port)
    }
}

impl fmt::Display for Via {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{} {}", self.version, self.transport.as_str().to_uppercase(), self.host)?;
        if let Some(port) = self.port {
            write!(f, ":{}", port)?;
        }
        write!(f, ";branch={}", self.params.branch)?;
        if let Some(ref r) = self.params.received {
            write!(f, ";received={}", r)?;
        }
        if let Some(rport) = self.params.rport {
            write!(f, ";rport={}", rport)?;
        } else if self.params.rport_flag {
            write!(f, ";rport")?;
        }
        Ok(())
    }
}

/// Generate RFC 3261 compliant branch ID
pub fn generate_branch() -> String {
    format!("z9hG4bK{}", uuid::Uuid::new_v4().simple())
}

/// Generate tag for From/To headers
pub fn generate_tag() -> String {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    format!("{:08x}", rng.gen::<u32>())
}

/// Name-Addr: display-name + URI + parameters
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NameAddr {
    /// Display name (optional)
    pub display_name: Option<String>,
    /// URI
    pub uri: Uri,
    /// Header parameters (like tag for From/To)
    pub params: HashMap<String, Option<String>>,
}

impl NameAddr {
    pub fn new(uri: Uri) -> Self {
        Self {
            display_name: None,
            uri,
            params: HashMap::new(),
        }
    }
    
    pub fn with_display_name(mut self, name: impl Into<String>) -> Self {
        self.display_name = Some(name.into());
        self
    }
    
    pub fn with_tag(mut self, tag: impl Into<String>) -> Self {
        self.params.insert("tag".to_string(), Some(tag.into()));
        self
    }
    
    pub fn tag(&self) -> Option<&str> {
        self.params.get("tag").and_then(|v| v.as_deref())
    }
    
    pub fn set_tag(&mut self, tag: impl Into<String>) {
        self.params.insert("tag".to_string(), Some(tag.into()));
    }
    
    pub fn generate_tag(&mut self) {
        self.set_tag(generate_tag());
    }
}

impl fmt::Display for NameAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(ref name) = self.display_name {
            // Quote display name if it contains special chars
            if name.contains(|c: char| !c.is_alphanumeric() && c != ' ') {
                write!(f, "\"{}\" ", name)?;
            } else {
                write!(f, "{} ", name)?;
            }
            write!(f, "<{}>", self.uri)?;
        } else {
            write!(f, "<{}>", self.uri)?;
        }
        
        for (key, value) in &self.params {
            if let Some(val) = value {
                write!(f, ";{}={}", key, val)?;
            } else {
                write!(f, ";{}", key)?;
            }
        }
        
        Ok(())
    }
}

/// Contact header
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Contact {
    /// Address
    pub addr: NameAddr,
    /// Expiration time
    pub expires: Option<u32>,
    /// Q-value (priority, 0.0-1.0)
    pub q: Option<f32>,
}

impl Contact {
    pub fn new(uri: Uri) -> Self {
        Self {
            addr: NameAddr::new(uri),
            expires: None,
            q: None,
        }
    }
    
    /// Create wildcard contact (* for unregister)
    pub fn wildcard() -> Self {
        Self {
            addr: NameAddr {
                display_name: None,
                uri: Uri {
                    scheme: Scheme::Sip,
                    user: None,
                    password: None,
                    host: Host::Domain("*".to_string()),
                    port: None,
                    params: UriParams::default(),
                    headers: HashMap::new(),
                },
                params: HashMap::new(),
            },
            expires: Some(0),
            q: None,
        }
    }
    
    pub fn is_wildcard(&self) -> bool {
        matches!(&self.addr.uri.host, Host::Domain(d) if d == "*")
    }
    
    pub fn with_expires(mut self, expires: u32) -> Self {
        self.expires = Some(expires);
        self
    }
}

impl fmt::Display for Contact {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_wildcard() {
            write!(f, "*")?;
        } else {
            write!(f, "{}", self.addr)?;
        }
        if let Some(exp) = self.expires {
            write!(f, ";expires={}", exp)?;
        }
        if let Some(q) = self.q {
            write!(f, ";q={:.1}", q)?;
        }
        Ok(())
    }
}

/// CSeq header
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CSeq {
    /// Sequence number
    pub seq: u32,
    /// Method
    pub method: Method,
}

impl CSeq {
    pub fn new(seq: u32, method: Method) -> Self {
        Self { seq, method }
    }
    
    pub fn increment(&mut self) {
        self.seq = self.seq.wrapping_add(1);
    }
}

impl fmt::Display for CSeq {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.seq, self.method)
    }
}

/// Content-Type header
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentType {
    pub media_type: String,
    pub media_subtype: String,
    pub params: HashMap<String, String>,
}

impl ContentType {
    pub fn new(media_type: &str, media_subtype: &str) -> Self {
        Self {
            media_type: media_type.to_string(),
            media_subtype: media_subtype.to_string(),
            params: HashMap::new(),
        }
    }
    
    pub fn sdp() -> Self {
        Self::new("application", "sdp")
    }
    
    pub fn is_sdp(&self) -> bool {
        self.media_type == "application" && self.media_subtype == "sdp"
    }
}

impl fmt::Display for ContentType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.media_type, self.media_subtype)?;
        for (k, v) in &self.params {
            write!(f, ";{}={}", k, v)?;
        }
        Ok(())
    }
}

/// WWW-Authenticate / Authorization challenge/response
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthChallenge {
    pub scheme: AuthScheme,
    pub realm: String,
    pub nonce: Option<String>,
    pub opaque: Option<String>,
    pub algorithm: Option<String>,
    pub qop: Option<Vec<String>>,
    pub stale: bool,
    pub domain: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthScheme {
    Digest,
    Basic,
    Other(String),
}

/// Authorization response
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthResponse {
    pub scheme: AuthScheme,
    pub username: String,
    pub realm: String,
    pub nonce: String,
    pub uri: String,
    pub response: String,
    pub algorithm: Option<String>,
    pub cnonce: Option<String>,
    pub opaque: Option<String>,
    pub qop: Option<String>,
    pub nc: Option<String>,
}

/// Generic header map for non-standard headers
#[derive(Debug, Clone, Default)]
pub struct HeaderMap {
    headers: HashMap<String, Vec<String>>,
}

impl HeaderMap {
    pub fn new() -> Self {
        Self::default()
    }
    
    pub fn get(&self, name: &str) -> Option<&str> {
        self.headers
            .get(&Self::normalize_name(name))
            .and_then(|v| v.first())
            .map(|s| s.as_str())
    }
    
    pub fn get_all(&self, name: &str) -> Option<&[String]> {
        self.headers
            .get(&Self::normalize_name(name))
            .map(|v| v.as_slice())
    }
    
    pub fn set(&mut self, name: impl Into<String>, value: impl Into<String>) {
        let name = Self::normalize_name(&name.into());
        self.headers.insert(name, vec![value.into()]);
    }
    
    pub fn add(&mut self, name: impl Into<String>, value: impl Into<String>) {
        let name = Self::normalize_name(&name.into());
        self.headers
            .entry(name)
            .or_default()
            .push(value.into());
    }
    
    pub fn remove(&mut self, name: &str) -> Option<Vec<String>> {
        self.headers.remove(&Self::normalize_name(name))
    }
    
    pub fn contains(&self, name: &str) -> bool {
        self.headers.contains_key(&Self::normalize_name(name))
    }
    
    pub fn iter(&self) -> impl Iterator<Item = (&String, &Vec<String>)> {
        self.headers.iter()
    }
    
    fn normalize_name(name: &str) -> String {
        // SIP headers are case-insensitive
        name.to_lowercase()
    }
}
```

### 2.2 SIP Methods and Status Codes

```rust
// crates/siphon/siphon-core/src/method.rs

use std::fmt;
use std::str::FromStr;

/// SIP request methods
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Method {
    // RFC 3261 core methods
    Invite,
    Ack,
    Bye,
    Cancel,
    Register,
    Options,
    
    // RFC 3262 - Reliability of Provisional Responses
    Prack,
    
    // RFC 3311 - UPDATE method
    Update,
    
    // RFC 3428 - Instant Messaging
    Message,
    
    // RFC 3515 - REFER method
    Refer,
    
    // RFC 3903 - PUBLISH method
    Publish,
    
    // RFC 6665 - Event Notification
    Subscribe,
    Notify,
    
    // RFC 2976 - INFO method
    Info,
}

impl Method {
    pub fn as_str(&self) -> &'static str {
        match self {
            Method::Invite => "INVITE",
            Method::Ack => "ACK",
            Method::Bye => "BYE",
            Method::Cancel => "CANCEL",
            Method::Register => "REGISTER",
            Method::Options => "OPTIONS",
            Method::Prack => "PRACK",
            Method::Update => "UPDATE",
            Method::Message => "MESSAGE",
            Method::Refer => "REFER",
            Method::Publish => "PUBLISH",
            Method::Subscribe => "SUBSCRIBE",
            Method::Notify => "NOTIFY",
            Method::Info => "INFO",
        }
    }
    
    /// Does this method create a dialog?
    pub fn creates_dialog(&self) -> bool {
        matches!(self, Method::Invite | Method::Subscribe)
    }
    
    /// Does this method require a response?
    pub fn requires_response(&self) -> bool {
        !matches!(self, Method::Ack)
    }
    
    /// Can this method be cancelled?
    pub fn can_cancel(&self) -> bool {
        matches!(self, Method::Invite)
    }
    
    /// Is this a target refresh method?
    pub fn is_target_refresh(&self) -> bool {
        matches!(self, Method::Invite | Method::Update | Method::Subscribe | Method::Notify | Method::Refer)
    }
}

impl fmt::Display for Method {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl FromStr for Method {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_uppercase().as_str() {
            "INVITE" => Ok(Method::Invite),
            "ACK" => Ok(Method::Ack),
            "BYE" => Ok(Method::Bye),
            "CANCEL" => Ok(Method::Cancel),
            "REGISTER" => Ok(Method::Register),
            "OPTIONS" => Ok(Method::Options),
            "PRACK" => Ok(Method::Prack),
            "UPDATE" => Ok(Method::Update),
            "MESSAGE" => Ok(Method::Message),
            "REFER" => Ok(Method::Refer),
            "PUBLISH" => Ok(Method::Publish),
            "SUBSCRIBE" => Ok(Method::Subscribe),
            "NOTIFY" => Ok(Method::Notify),
            "INFO" => Ok(Method::Info),
            _ => Err(ParseError::InvalidMethod(s.to_string())),
        }
    }
}

// crates/siphon/siphon-core/src/status.rs

/// SIP response status code
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StatusCode(u16);

impl StatusCode {
    // === Provisional 1xx ===
    pub const TRYING: Self = Self(100);
    pub const RINGING: Self = Self(180);
    pub const CALL_IS_BEING_FORWARDED: Self = Self(181);
    pub const QUEUED: Self = Self(182);
    pub const SESSION_PROGRESS: Self = Self(183);
    pub const EARLY_DIALOG_TERMINATED: Self = Self(199);
    
    // === Success 2xx ===
    pub const OK: Self = Self(200);
    pub const ACCEPTED: Self = Self(202);
    pub const NO_NOTIFICATION: Self = Self(204);
    
    // === Redirection 3xx ===
    pub const MULTIPLE_CHOICES: Self = Self(300);
    pub const MOVED_PERMANENTLY: Self = Self(301);
    pub const MOVED_TEMPORARILY: Self = Self(302);
    pub const USE_PROXY: Self = Self(305);
    pub const ALTERNATIVE_SERVICE: Self = Self(380);
    
    // === Client Error 4xx ===
    pub const BAD_REQUEST: Self = Self(400);
    pub const UNAUTHORIZED: Self = Self(401);
    pub const PAYMENT_REQUIRED: Self = Self(402);
    pub const FORBIDDEN: Self = Self(403);
    pub const NOT_FOUND: Self = Self(404);
    pub const METHOD_NOT_ALLOWED: Self = Self(405);
    pub const NOT_ACCEPTABLE: Self = Self(406);
    pub const PROXY_AUTHENTICATION_REQUIRED: Self = Self(407);
    pub const REQUEST_TIMEOUT: Self = Self(408);
    pub const CONFLICT: Self = Self(409);
    pub const GONE: Self = Self(410);
    pub const LENGTH_REQUIRED: Self = Self(411);
    pub const CONDITIONAL_REQUEST_FAILED: Self = Self(412);
    pub const REQUEST_ENTITY_TOO_LARGE: Self = Self(413);
    pub const REQUEST_URI_TOO_LONG: Self = Self(414);
    pub const UNSUPPORTED_MEDIA_TYPE: Self = Self(415);
    pub const UNSUPPORTED_URI_SCHEME: Self = Self(416);
    pub const UNKNOWN_RESOURCE_PRIORITY: Self = Self(417);
    pub const BAD_EXTENSION: Self = Self(420);
    pub const EXTENSION_REQUIRED: Self = Self(421);
    pub const SESSION_INTERVAL_TOO_SMALL: Self = Self(422);
    pub const INTERVAL_TOO_BRIEF: Self = Self(423);
    pub const BAD_LOCATION_INFORMATION: Self = Self(424);
    pub const USE_IDENTITY_HEADER: Self = Self(428);
    pub const PROVIDE_REFERRER_IDENTITY: Self = Self(429);
    pub const FLOW_FAILED: Self = Self(430);
    pub const ANONYMITY_DISALLOWED: Self = Self(433);
    pub const BAD_IDENTITY_INFO: Self = Self(436);
    pub const UNSUPPORTED_CERTIFICATE: Self = Self(437);
    pub const INVALID_IDENTITY_HEADER: Self = Self(438);
    pub const FIRST_HOP_LACKS_OUTBOUND_SUPPORT: Self = Self(439);
    pub const MAX_BREADTH_EXCEEDED: Self = Self(440);
    pub const BAD_INFO_PACKAGE: Self = Self(469);
    pub const CONSENT_NEEDED: Self = Self(470);
    pub const TEMPORARILY_UNAVAILABLE: Self = Self(480);
    pub const CALL_TRANSACTION_DOES_NOT_EXIST: Self = Self(481);
    pub const LOOP_DETECTED: Self = Self(482);
    pub const TOO_MANY_HOPS: Self = Self(483);
    pub const ADDRESS_INCOMPLETE: Self = Self(484);
    pub const AMBIGUOUS: Self = Self(485);
    pub const BUSY_HERE: Self = Self(486);
    pub const REQUEST_TERMINATED: Self = Self(487);
    pub const NOT_ACCEPTABLE_HERE: Self = Self(488);
    pub const BAD_EVENT: Self = Self(489);
    pub const REQUEST_PENDING: Self = Self(491);
    pub const UNDECIPHERABLE: Self = Self(493);
    pub const SECURITY_AGREEMENT_REQUIRED: Self = Self(494);
    
    // === Server Error 5xx ===
    pub const SERVER_INTERNAL_ERROR: Self = Self(500);
    pub const NOT_IMPLEMENTED: Self = Self(501);
    pub const BAD_GATEWAY: Self = Self(502);
    pub const SERVICE_UNAVAILABLE: Self = Self(503);
    pub const SERVER_TIMEOUT: Self = Self(504);
    pub const VERSION_NOT_SUPPORTED: Self = Self(505);
    pub const MESSAGE_TOO_LARGE: Self = Self(513);
    pub const PUSH_NOTIFICATION_SERVICE_NOT_SUPPORTED: Self = Self(555);
    pub const PRECONDITION_FAILURE: Self = Self(580);
    
    // === Global Failure 6xx ===
    pub const BUSY_EVERYWHERE: Self = Self(600);
    pub const DECLINE: Self = Self(603);
    pub const DOES_NOT_EXIST_ANYWHERE: Self = Self(604);
    pub const NOT_ACCEPTABLE_GLOBAL: Self = Self(606);
    pub const UNWANTED: Self = Self(607);
    pub const REJECTED: Self = Self(608);
    
    pub fn new(code: u16) -> Option<Self> {
        if (100..700).contains(&code) {
            Some(Self(code))
        } else {
            None
        }
    }
    
    pub fn code(&self) -> u16 {
        self.0
    }
    
    pub fn is_provisional(&self) -> bool {
        (100..200).contains(&self.0)
    }
    
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.0)
    }
    
    pub fn is_redirect(&self) -> bool {
        (300..400).contains(&self.0)
    }
    
    pub fn is_client_error(&self) -> bool {
        (400..500).contains(&self.0)
    }
    
    pub fn is_server_error(&self) -> bool {
        (500..600).contains(&self.0)
    }
    
    pub fn is_global_failure(&self) -> bool {
        (600..700).contains(&self.0)
    }
    
    pub fn is_failure(&self) -> bool {
        self.0 >= 400
    }
    
    pub fn is_final(&self) -> bool {
        self.0 >= 200
    }
    
    pub fn reason_phrase(&self) -> &'static str {
        match self.0 {
            100 => "Trying",
            180 => "Ringing",
            181 => "Call Is Being Forwarded",
            182 => "Queued",
            183 => "Session Progress",
            199 => "Early Dialog Terminated",
            200 => "OK",
            202 => "Accepted",
            204 => "No Notification",
            300 => "Multiple Choices",
            301 => "Moved Permanently",
            302 => "Moved Temporarily",
            305 => "Use Proxy",
            380 => "Alternative Service",
            400 => "Bad Request",
            401 => "Unauthorized",
            402 => "Payment Required",
            403 => "Forbidden",
            404 => "Not Found",
            405 => "Method Not Allowed",
            406 => "Not Acceptable",
            407 => "Proxy Authentication Required",
            408 => "Request Timeout",
            410 => "Gone",
            413 => "Request Entity Too Large",
            414 => "Request-URI Too Long",
            415 => "Unsupported Media Type",
            416 => "Unsupported URI Scheme",
            420 => "Bad Extension",
            421 => "Extension Required",
            423 => "Interval Too Brief",
            480 => "Temporarily Unavailable",
            481 => "Call/Transaction Does Not Exist",
            482 => "Loop Detected",
            483 => "Too Many Hops",
            484 => "Address Incomplete",
            485 => "Ambiguous",
            486 => "Busy Here",
            487 => "Request Terminated",
            488 => "Not Acceptable Here",
            489 => "Bad Event",
            491 => "Request Pending",
            493 => "Undecipherable",
            500 => "Server Internal Error",
            501 => "Not Implemented",
            502 => "Bad Gateway",
            503 => "Service Unavailable",
            504 => "Server Time-out",
            505 => "Version Not Supported",
            513 => "Message Too Large",
            580 => "Precondition Failure",
            600 => "Busy Everywhere",
            603 => "Decline",
            604 => "Does Not Exist Anywhere",
            606 => "Not Acceptable",
            607 => "Unwanted",
            _ => "Unknown",
        }
    }
}

impl fmt::Display for StatusCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}
```

### 2.3 Complete Message Types

```rust
// crates/siphon/siphon-core/src/message.rs

use bytes::{Bytes, BytesMut, BufMut};

/// SIP protocol version
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct SipVersion {
    pub major: u8,
    pub minor: u8,
}

impl SipVersion {
    pub const V2_0: Self = Self { major: 2, minor: 0 };
}

impl fmt::Display for SipVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SIP/{}.{}", self.major, self.minor)
    }
}

/// SIP message (request or response)
#[derive(Debug, Clone)]
pub enum SipMessage {
    Request(Request),
    Response(Response),
}

impl SipMessage {
    /// Parse from bytes
    pub fn parse(data: &[u8]) -> Result<Self, ParseError> {
        MessageParser::parse(data)
    }
    
    /// Serialize to bytes
    pub fn to_bytes(&self) -> Bytes {
        let mut buf = BytesMut::with_capacity(2048);
        match self {
            SipMessage::Request(r) => r.write_to(&mut buf),
            SipMessage::Response(r) => r.write_to(&mut buf),
        }
        buf.freeze()
    }
    
    /// Get Call-ID
    pub fn call_id(&self) -> &SipCallId {
        match self {
            SipMessage::Request(r) => &r.call_id,
            SipMessage::Response(r) => &r.call_id,
        }
    }
    
    /// Get CSeq
    pub fn cseq(&self) -> &CSeq {
        match self {
            SipMessage::Request(r) => &r.cseq,
            SipMessage::Response(r) => &r.cseq,
        }
    }
    
    /// Get Via headers
    pub fn via(&self) -> &[Via] {
        match self {
            SipMessage::Request(r) => &r.via,
            SipMessage::Response(r) => &r.via,
        }
    }
    
    /// Get From header
    pub fn from(&self) -> &NameAddr {
        match self {
            SipMessage::Request(r) => &r.from,
            SipMessage::Response(r) => &r.from,
        }
    }
    
    /// Get To header
    pub fn to(&self) -> &NameAddr {
        match self {
            SipMessage::Request(r) => &r.to,
            SipMessage::Response(r) => &r.to,
        }
    }
}

/// SIP Request
#[derive(Debug, Clone)]
pub struct Request {
    // Request line
    pub method: Method,
    pub uri: Uri,
    pub version: SipVersion,
    
    // Required headers
    pub via: Vec<Via>,
    pub from: NameAddr,
    pub to: NameAddr,
    pub call_id: SipCallId,
    pub cseq: CSeq,
    pub max_forwards: u8,
    
    // Common optional headers
    pub contact: Vec<Contact>,
    pub route: Vec<NameAddr>,
    pub record_route: Vec<NameAddr>,
    pub content_type: Option<ContentType>,
    pub content_length: usize,
    pub expires: Option<u32>,
    pub authorization: Option<AuthResponse>,
    pub proxy_authorization: Option<AuthResponse>,
    
    // Other headers
    pub headers: HeaderMap,
    
    // Body
    pub body: Option<Bytes>,
}

impl Request {
    /// Create a new request
    pub fn new(method: Method, uri: Uri) -> Self {
        Self {
            method,
            uri,
            version: SipVersion::V2_0,
            via: Vec::new(),
            from: NameAddr::new(Uri::new(Host::Domain("localhost".into()))),
            to: NameAddr::new(Uri::new(Host::Domain("localhost".into()))),
            call_id: SipCallId::new(),
            cseq: CSeq::new(1, method),
            max_forwards: 70,
            contact: Vec::new(),
            route: Vec::new(),
            record_route: Vec::new(),
            content_type: None,
            content_length: 0,
            expires: None,
            authorization: None,
            proxy_authorization: None,
            headers: HeaderMap::new(),
            body: None,
        }
    }
    
    /// Builder for requests
    pub fn builder(method: Method, uri: Uri) -> RequestBuilder {
        RequestBuilder::new(method, uri)
    }
    
    /// Create REGISTER request
    pub fn register(aor: &Uri, contact: &Uri) -> Self {
        let mut req = Self::new(Method::Register, aor.aor());
        req.from = NameAddr::new(aor.aor());
        req.from.generate_tag();
        req.to = NameAddr::new(aor.aor());
        req.contact.push(Contact::new(contact.clone()));
        req
    }
    
    /// Create INVITE request
    pub fn invite(from: &Uri, to: &Uri) -> Self {
        let mut req = Self::new(Method::Invite, to.clone());
        req.from = NameAddr::new(from.clone());
        req.from.generate_tag();
        req.to = NameAddr::new(to.clone());
        req
    }
    
    /// Create BYE request
    pub fn bye(dialog: &Dialog) -> Self {
        dialog.create_request(Method::Bye)
    }
    
    /// Create CANCEL request for this INVITE
    pub fn cancel(&self) -> Self {
        assert_eq!(self.method, Method::Invite, "Can only CANCEL an INVITE");
        
        let mut cancel = Self::new(Method::Cancel, self.uri.clone());
        cancel.from = self.from.clone();
        cancel.to = self.to.clone();
        cancel.call_id = self.call_id.clone();
        cancel.cseq = CSeq::new(self.cseq.seq, Method::Cancel);
        cancel.via = vec![self.via.first().cloned().expect("Request must have Via")];
        cancel.route = self.route.clone();
        cancel.max_forwards = self.max_forwards;
        cancel
    }
    
    /// Create ACK for 2xx response
    pub fn ack_for_2xx(&self, response: &Response) -> Self {
        assert_eq!(self.method, Method::Invite, "ACK is for INVITE");
        
        let mut ack = Self::new(Method::Ack, self.uri.clone());
        ack.from = self.from.clone();
        ack.to = response.to.clone();
        ack.call_id = self.call_id.clone();
        ack.cseq = CSeq::new(self.cseq.seq, Method::Ack);
        ack.via = vec![self.via.first().cloned().expect("Request must have Via")];
        // Route from Record-Route in response (reversed)
        ack.route = response.record_route.iter().rev().cloned().collect();
        ack.max_forwards = self.max_forwards;
        ack
    }
    
    /// Create ACK for non-2xx response (within transaction)
    pub fn ack_for_error(&self, response: &Response) -> Self {
        let mut ack = Self::new(Method::Ack, self.uri.clone());
        ack.from = self.from.clone();
        ack.to = response.to.clone();
        ack.call_id = self.call_id.clone();
        ack.cseq = CSeq::new(self.cseq.seq, Method::Ack);
        ack.via = vec![self.via.first().cloned().expect("Request must have Via")];
        ack.route = self.route.clone();
        ack.max_forwards = self.max_forwards;
        ack
    }
    
    /// Set body with content type
    pub fn set_body(&mut self, content_type: ContentType, body: impl Into<Bytes>) {
        let body = body.into();
        self.content_length = body.len();
        self.content_type = Some(content_type);
        self.body = Some(body);
    }
    
    /// Set SDP body
    pub fn set_sdp(&mut self, sdp: impl Into<Bytes>) {
        self.set_body(ContentType::sdp(), sdp);
    }
    
    /// Write to buffer
    pub fn write_to(&self, buf: &mut BytesMut) {
        // Request line
        buf.put_slice(format!("{} {} {}\r\n", self.method, self.uri, self.version).as_bytes());
        
        // Via headers
        for via in &self.via {
            buf.put_slice(format!("Via: {}\r\n", via).as_bytes());
        }
        
        // Required headers
        buf.put_slice(format!("From: {}\r\n", self.from).as_bytes());
        buf.put_slice(format!("To: {}\r\n", self.to).as_bytes());
        buf.put_slice(format!("Call-ID: {}\r\n", self.call_id).as_bytes());
        buf.put_slice(format!("CSeq: {}\r\n", self.cseq).as_bytes());
        buf.put_slice(format!("Max-Forwards: {}\r\n", self.max_forwards).as_bytes());
        
        // Contact
        for contact in &self.contact {
            buf.put_slice(format!("Contact: {}\r\n", contact).as_bytes());
        }
        
        // Route
        for route in &self.route {
            buf.put_slice(format!("Route: {}\r\n", route).as_bytes());
        }
        
        // Record-Route
        for rr in &self.record_route {
            buf.put_slice(format!("Record-Route: {}\r\n", rr).as_bytes());
        }
        
        // Expires
        if let Some(exp) = self.expires {
            buf.put_slice(format!("Expires: {}\r\n", exp).as_bytes());
        }
        
        // Content-Type and Content-Length
        if let Some(ref ct) = self.content_type {
            buf.put_slice(format!("Content-Type: {}\r\n", ct).as_bytes());
        }
        buf.put_slice(format!("Content-Length: {}\r\n", self.content_length).as_bytes());
        
        // Other headers
        for (name, values) in self.headers.iter() {
            for value in values {
                buf.put_slice(format!("{}: {}\r\n", capitalize_header(name), value).as_bytes());
            }
        }
        
        // Empty line
        buf.put_slice(b"\r\n");
        
        // Body
        if let Some(ref body) = self.body {
            buf.put_slice(body);
        }
    }
}

/// SIP Response
#[derive(Debug, Clone)]
pub struct Response {
    // Status line
    pub version: SipVersion,
    pub status: StatusCode,
    pub reason: String,
    
    // Headers (mirror of request)
    pub via: Vec<Via>,
    pub from: NameAddr,
    pub to: NameAddr,
    pub call_id: SipCallId,
    pub cseq: CSeq,
    
    // Optional headers
    pub contact: Vec<Contact>,
    pub record_route: Vec<NameAddr>,
    pub content_type: Option<ContentType>,
    pub content_length: usize,
    pub www_authenticate: Option<AuthChallenge>,
    pub proxy_authenticate: Option<AuthChallenge>,
    
    // Other headers
    pub headers: HeaderMap,
    
    // Body
    pub body: Option<Bytes>,
}

impl Response {
    /// Create response for a request
    pub fn new(status: StatusCode) -> Self {
        Self {
            version: SipVersion::V2_0,
            status,
            reason: status.reason_phrase().to_string(),
            via: Vec::new(),
            from: NameAddr::new(Uri::new(Host::Domain("localhost".into()))),
            to: NameAddr::new(Uri::new(Host::Domain("localhost".into()))),
            call_id: SipCallId::default(),
            cseq: CSeq::new(1, Method::Invite),
            contact: Vec::new(),
            record_route: Vec::new(),
            content_type: None,
            content_length: 0,
            www_authenticate: None,
            proxy_authenticate: None,
            headers: HeaderMap::new(),
            body: None,
        }
    }
    
    /// Create response from request
    pub fn for_request(request: &Request, status: StatusCode) -> Self {
        let mut response = Self::new(status);
        response.via = request.via.clone();
        response.from = request.from.clone();
        response.to = request.to.clone();
        response.call_id = request.call_id.clone();
        response.cseq = request.cseq.clone();
        response
    }
    
    // Status check helpers
    pub fn is_provisional(&self) -> bool { self.status.is_provisional() }
    pub fn is_success(&self) -> bool { self.status.is_success() }
    pub fn is_redirect(&self) -> bool { self.status.is_redirect() }
    pub fn is_failure(&self) -> bool { self.status.is_failure() }
    pub fn is_final(&self) -> bool { self.status.is_final() }
    
    /// Set body
    pub fn set_body(&mut self, content_type: ContentType, body: impl Into<Bytes>) {
        let body = body.into();
        self.content_length = body.len();
        self.content_type = Some(content_type);
        self.body = Some(body);
    }
    
    /// Set SDP body
    pub fn set_sdp(&mut self, sdp: impl Into<Bytes>) {
        self.set_body(ContentType::sdp(), sdp);
    }
    
    /// Write to buffer
    pub fn write_to(&self, buf: &mut BytesMut) {
        // Status line
        buf.put_slice(format!("{} {} {}\r\n", self.version, self.status, self.reason).as_bytes());
        
        // Via headers
        for via in &self.via {
            buf.put_slice(format!("Via: {}\r\n", via).as_bytes());
        }
        
        // Required headers
        buf.put_slice(format!("From: {}\r\n", self.from).as_bytes());
        buf.put_slice(format!("To: {}\r\n", self.to).as_bytes());
        buf.put_slice(format!("Call-ID: {}\r\n", self.call_id).as_bytes());
        buf.put_slice(format!("CSeq: {}\r\n", self.cseq).as_bytes());
        
        // Contact
        for contact in &self.contact {
            buf.put_slice(format!("Contact: {}\r\n", contact).as_bytes());
        }
        
        // Record-Route
        for rr in &self.record_route {
            buf.put_slice(format!("Record-Route: {}\r\n", rr).as_bytes());
        }
        
        // Content headers
        if let Some(ref ct) = self.content_type {
            buf.put_slice(format!("Content-Type: {}\r\n", ct).as_bytes());
        }
        buf.put_slice(format!("Content-Length: {}\r\n", self.content_length).as_bytes());
        
        // Other headers
        for (name, values) in self.headers.iter() {
            for value in values {
                buf.put_slice(format!("{}: {}\r\n", capitalize_header(name), value).as_bytes());
            }
        }
        
        // Empty line
        buf.put_slice(b"\r\n");
        
        // Body
        if let Some(ref body) = self.body {
            buf.put_slice(body);
        }
    }
}

/// Capitalize header name (content-type -> Content-Type)
fn capitalize_header(name: &str) -> String {
    name.split('-')
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                None => String::new(),
                Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
            }
        })
        .collect::<Vec<_>>()
        .join("-")
}

/// Request builder for ergonomic construction
pub struct RequestBuilder {
    request: Request,
}

impl RequestBuilder {
    pub fn new(method: Method, uri: Uri) -> Self {
        Self {
            request: Request::new(method, uri),
        }
    }
    
    pub fn from(mut self, from: NameAddr) -> Self {
        self.request.from = from;
        self
    }
    
    pub fn to(mut self, to: NameAddr) -> Self {
        self.request.to = to;
        self
    }
    
    pub fn call_id(mut self, call_id: SipCallId) -> Self {
        self.request.call_id = call_id;
        self
    }
    
    pub fn cseq(mut self, seq: u32) -> Self {
        self.request.cseq.seq = seq;
        self
    }
    
    pub fn via(mut self, via: Via) -> Self {
        self.request.via.push(via);
        self
    }
    
    pub fn contact(mut self, contact: Contact) -> Self {
        self.request.contact.push(contact);
        self
    }
    
    pub fn route(mut self, route: NameAddr) -> Self {
        self.request.route.push(route);
        self
    }
    
    pub fn expires(mut self, expires: u32) -> Self {
        self.request.expires = Some(expires);
        self
    }
    
    pub fn header(mut self, name: &str, value: &str) -> Self {
        self.request.headers.add(name, value);
        self
    }
    
    pub fn body(mut self, content_type: ContentType, body: impl Into<Bytes>) -> Self {
        self.request.set_body(content_type, body);
        self
    }
    
    pub fn sdp(mut self, sdp: impl Into<Bytes>) -> Self {
        self.request.set_sdp(sdp);
        self
    }
    
    pub fn build(self) -> Request {
        self.request
    }
}
```

### 2.4 Sprint 2 Tasks

| Task | Est. Hours | Status |
|------|-----------|--------|
| Header types (Via, NameAddr, Contact, CSeq) | 8 | ☐ |
| Header parsing | 12 | ☐ |
| Method enum and parsing | 4 | ☐ |
| StatusCode with all codes | 4 | ☐ |
| Request struct and builder | 8 | ☐ |
| Response struct | 6 | ☐ |
| Message parser (complete) | 16 | ☐ |
| Message serializer | 8 | ☐ |
| Parser tests (RFC 3261 torture tests) | 12 | ☐ |
| Documentation | 4 | ☐ |
| **Total** | **82** | |

---

## Sprint 3-8 Overview

*Due to length, I'll provide condensed specs for remaining sprints.*

### Sprint 3: Transport (Weeks 5-6)

```rust
// Key interfaces

#[async_trait]
pub trait Transport: Send + Sync {
    fn transport_type(&self) -> TransportType;
    fn local_addr(&self) -> SocketAddr;
    fn is_reliable(&self) -> bool;
    
    async fn send(&self, msg: &SipMessage, dest: SocketAddr) -> Result<()>;
    async fn recv(&self) -> Result<(SipMessage, SocketAddr)>;
}

pub struct UdpTransport {
    socket: Arc<UdpSocket>,
    recv_buf_size: usize,
}

pub struct TransportManager {
    udp: Option<UdpTransport>,
    tcp: Option<TcpTransport>,  // Future
    tls: Option<TlsTransport>,  // Future
}
```

**Deliverable:** UDP transport that can send/receive SIP messages.

### Sprint 4: Transactions (Weeks 7-8)

```rust
// Transaction state machines per RFC 3261

pub struct TransactionLayer {
    client_transactions: DashMap<TransactionKey, ClientTransaction>,
    server_transactions: DashMap<TransactionKey, ServerTransaction>,
    transport: Arc<TransportManager>,
    timer_wheel: TimerWheel,
}

pub enum ClientTransaction {
    InviteClient(InviteClientTransaction),
    NonInviteClient(NonInviteClientTransaction),
}

pub enum ServerTransaction {
    InviteServer(InviteServerTransaction),
    NonInviteServer(NonInviteServerTransaction),
}

// Timer constants
const T1: Duration = Duration::from_millis(500);
const T2: Duration = Duration::from_secs(4);
const T4: Duration = Duration::from_secs(5);
```

**Deliverable:** Complete transaction state machines with retransmission.

### Sprint 5: Registrar & Proxy (Weeks 9-10)

```rust
// Registration store
pub struct RegistrationStore {
    bindings: DashMap<Aor, Vec<Binding>>,
}

pub struct Binding {
    pub contact: Uri,
    pub expires: Instant,
    pub call_id: SipCallId,
    pub cseq: u32,
    pub user_agent: Option<String>,
    pub received: SocketAddr,
}

// Registrar service
pub struct Registrar {
    store: Arc<RegistrationStore>,
    config: RegistrarConfig,
}

impl Registrar {
    pub async fn handle_register(&self, request: Request) -> Response;
}

// Stateless proxy
pub struct Proxy {
    registrar: Arc<RegistrationStore>,
    config: ProxyConfig,
}

impl Proxy {
    pub async fn route(&self, request: Request) -> Result<Vec<SocketAddr>>;
}
```

**Deliverable:** Working registration and call routing between registered endpoints.

### Sprint 6: B2BUA & RTP (Weeks 11-12)

```rust
// B2BUA for call control
pub struct B2BUA {
    calls: DashMap<CallId, Call>,
    dialog_mgr: Arc<DialogManager>,
    media_mgr: Arc<MediaManager>,
}

pub struct Call {
    pub id: CallId,
    pub leg_a: CallLeg,
    pub leg_b: Option<CallLeg>,
    pub state: CallState,
    pub created_at: Instant,
}

// RTP relay
pub struct RtpRelay {
    sessions: DashMap<SessionId, RtpSession>,
    port_allocator: PortAllocator,
}

pub struct RtpSession {
    pub id: SessionId,
    pub local_rtp: UdpSocket,
    pub local_rtcp: UdpSocket,
    pub remote_addr: Option<SocketAddr>,
}
```

**Deliverable:** Complete calls between two endpoints with media.

### Sprint 7: Storage & API (Weeks 13-14)

```rust
// SQLite storage
pub struct SqliteStore {
    pool: SqlitePool,
}

impl SqliteStore {
    pub async fn save_registration(&self, aor: &Aor, binding: &Binding) -> Result<()>;
    pub async fn load_registrations(&self, aor: &Aor) -> Result<Vec<Binding>>;
    pub async fn save_cdr(&self, cdr: &Cdr) -> Result<()>;
}

// REST API
pub fn api_routes() -> Router {
    Router::new()
        .route("/v1/registrations", get(list_registrations))
        .route("/v1/registrations/:aor", get(get_registration).delete(delete_registration))
        .route("/v1/calls", get(list_calls).post(create_call))
        .route("/v1/calls/:id", get(get_call).delete(hangup_call))
        .route("/v1/health", get(health_check))
        .route("/metrics", get(prometheus_metrics))
}
```

**Deliverable:** Persistent storage and REST API for management.

### Sprint 8: Integration & Polish (Weeks 15-16)

- CLI tool for server management
- Docker image with health checks
- Integration tests with real softphones
- Performance testing
- Documentation
- Example configurations

**Deliverable:** Production-ready single-node PBX.

---

## Testing Strategy

### Unit Tests
- Parser tests with RFC 3261 torture test messages
- State machine tests for all transaction types
- Timer behavior tests

### Integration Tests
```rust
#[tokio::test]
async fn test_registration_flow() {
    let server = TestServer::start().await;
    let client = SipClient::new().await;
    
    // Register
    let response = client.register("sip:alice@test.local", 3600).await;
    assert_eq!(response.status, StatusCode::OK);
    
    // Verify registration
    let bindings = server.lookup("alice@test.local").await;
    assert_eq!(bindings.len(), 1);
}

#[tokio::test]
async fn test_basic_call() {
    let server = TestServer::start().await;
    
    // Register two endpoints
    let alice = SipClient::register(&server, "alice").await;
    let bob = SipClient::register(&server, "bob").await;
    
    // Alice calls Bob
    let call = alice.invite("sip:bob@test.local").await;
    
    // Bob answers
    let incoming = bob.wait_for_invite().await;
    bob.answer(incoming).await;
    
    // Verify call established
    assert!(call.is_established());
    
    // Hangup
    alice.hangup(call).await;
}
```

### Interop Tests
- Test with Odin, Odin, Odin
- Test with SIP trunks (Telnyx, Twilio)
- Test with hardware phones (Polycom, Yealink)

---

## Risk Mitigation

| Risk | Impact | Mitigation |
|------|--------|------------|
| SIP parser complexity | High | Use RFC torture tests early, fuzz testing |
| Timer accuracy | Medium | Use tokio-timer, test under load |
| UDP packet loss | Medium | Implement retransmission correctly |
| NAT traversal | High | Start with simple topology, add STUN/rport later |
| Media quality | Medium | Focus on relay first, transcoding later |

---

## Success Criteria

### Milestone 1 (Week 4): "First SIP Message"
- [ ] Parse and serialize any valid SIP message
- [ ] Send/receive over UDP

### Milestone 2 (Week 8): "First Registration"
- [ ] Register a softphone
- [ ] See registration in logs

### Milestone 3 (Week 12): "First Call"
- [ ] Complete call between two softphones
- [ ] Hear audio in both directions

### Milestone 4 (Week 16): "Production Ready"
- [ ] Docker deployment working
- [ ] REST API functional
- [ ] Prometheus metrics exposed
- [ ] SIP trunk connected
- [ ] Documentation complete

---

## Resource Requirements

### Development Environment
- Rust 1.75+ (async traits, RPITIT)
- Linux or macOS (Windows via WSL2)
- Docker for testing
- Wireshark for SIP/RTP debugging

### Test Equipment
- 2+ softphones (Odin, Odin, Odin)
- SIP trunk account (Telnyx, Twilio, or similar)
- Optional: hardware IP phone

### CI/CD
- GitHub Actions or GitLab CI
- cargo-deny for dependency auditing
- cargo-llvm-cov for coverage
- cargo-fuzz for fuzz testing

---

*Document Version: 1.0*
*Phase 1 Planning Complete*