use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
use uuid::Uuid;

/// Macro to define strongly typed UUID wrappers with friendly display prefixes.
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
                write!(f, "{}-{}", $prefix, self.0)
            }
        }

        impl FromStr for $name {
            type Err = uuid::Error;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                let uuid_str = s.strip_prefix(concat!($prefix, "-")).unwrap_or(s);
                Ok(Self(Uuid::parse_str(uuid_str)?))
            }
        }
    };
}

define_id!(CallId, "call");
define_id!(SessionId, "sess");
define_id!(DialogId, "dlg");
define_id!(TransactionId, "txn");
define_id!(RegistrationId, "reg");
define_id!(NodeId, "node");
define_id!(TenantId, "tenant");

/// SIP Call-ID header value (not our internal CallId).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SipCallId(String);

impl SipCallId {
    pub fn new() -> Self {
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

/// Address of Record - canonical SIP identity.
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

    pub fn parse(s: &str) -> Option<Self> {
        let s = s
            .strip_prefix("sip:")
            .or_else(|| s.strip_prefix("sips:"))
            .unwrap_or(s);
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
