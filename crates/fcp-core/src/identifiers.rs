use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
use uuid::Uuid;

const MAX_AOR_LENGTH: usize = 256;

/// Macro to define strongly typed UUID wrappers with friendly display prefixes.
macro_rules! define_id {
    ($name:ident, $prefix:expr) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
        #[serde(transparent)]
        pub struct $name(Uuid);

        // Read from JSON, the prefixed form every response shows
        // (`tenant-<uuid>`) and the bare UUID are both taken, as `FromStr`
        // takes them: a client may hand back what it was given. What is
        // written stays the bare UUID the stores hold.
        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                if deserializer.is_human_readable() {
                    let s = String::deserialize(deserializer)?;
                    s.parse().map_err(serde::de::Error::custom)
                } else {
                    Uuid::deserialize(deserializer).map(Self)
                }
            }
        }

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

        // An id in the OpenAPI contract (docs/DEVELOPER_SURFACE.md §3,
        // phase 3): a string, read with or without its prefix.
        #[cfg(feature = "openapi")]
        impl utoipa::PartialSchema for $name {
            fn schema() -> utoipa::openapi::RefOr<utoipa::openapi::schema::Schema> {
                utoipa::openapi::ObjectBuilder::new()
                    .schema_type(utoipa::openapi::schema::Type::String)
                    .description(Some(concat!(
                        "A ",
                        stringify!($name),
                        ": `",
                        $prefix,
                        "-<uuid>`, or the bare UUID"
                    )))
                    .into()
            }
        }

        #[cfg(feature = "openapi")]
        impl utoipa::ToSchema for $name {}

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
define_id!(RecordingId, "rec");
define_id!(NodeId, "node");
define_id!(TenantId, "tenant");
define_id!(PhoneNumberId, "num");
define_id!(LineBindingId, "bind");
define_id!(ExtensionId, "ext");
define_id!(DeviceId, "dev");
define_id!(VoicemailBoxId, "vm");
define_id!(ServiceId, "svc");
define_id!(UserId, "user");
define_id!(MeetingId, "mtg");
define_id!(WebhookId, "whk");
define_id!(ApiTokenId, "token");
define_id!(LocationId, "loc");
define_id!(PromptId, "prompt");
define_id!(ConfigTemplateId, "tmpl");
define_id!(LegalHoldId, "hold");
define_id!(SubjectRequestId, "sreq");
define_id!(EmergencyNetworkId, "enet");
define_id!(SiteId, "site");
define_id!(SiteRequestId, "sitereq");
define_id!(ScheduleId, "sched");
define_id!(ClassOfServiceId, "cos");
define_id!(HotelingSessionId, "hotel");
define_id!(LoginSessionId, "login");
// A tenant's identity provider (docs/IDENTITY_AND_DEVICES.md phase 3).
define_id!(IdentityProviderId, "idp");

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
        let user = user.into();
        let host = host.into();
        debug_assert!(
            !contains_crlf(&user) && !contains_crlf(&host),
            "AOR components should not contain CR/LF"
        );
        Self { user, host }
    }

    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim();
        if contains_crlf(s) || s.len() > MAX_AOR_LENGTH {
            return None;
        }

        let s = s
            .strip_prefix("sip:")
            .or_else(|| s.strip_prefix("sips:"))
            .unwrap_or(s);
        let parts: Vec<&str> = s.split('@').collect();
        if parts.len() == 2
            && !parts[0].is_empty()
            && !parts[1].is_empty()
            && !contains_crlf(parts[0])
            && !contains_crlf(parts[1])
        {
            let candidate = Self {
                user: parts[0].to_string(),
                host: parts[1].to_string(),
            };
            if candidate.user.len() + candidate.host.len() < MAX_AOR_LENGTH {
                return Some(candidate);
            }
        }
        None
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

fn contains_crlf(value: &str) -> bool {
    value.bytes().any(|b| matches!(b, b'\r' | b'\n'))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A response shows `tenant-<uuid>`; a request may hand it back, or the
    /// bare UUID; what is written is the bare UUID.
    #[test]
    fn an_id_reads_both_its_forms_and_writes_the_bare_uuid() {
        let id = TenantId::new();
        let prefixed = serde_json::to_string(&id.to_string()).unwrap();
        let bare = serde_json::to_string(id.as_uuid()).unwrap();
        assert_eq!(serde_json::from_str::<TenantId>(&prefixed).unwrap(), id);
        assert_eq!(serde_json::from_str::<TenantId>(&bare).unwrap(), id);
        assert_eq!(serde_json::to_string(&id).unwrap(), bare);
        assert!(serde_json::from_str::<TenantId>("\"call-not-a-uuid\"").is_err());
        let call: CallId =
            serde_json::from_str(&serde_json::to_string(&CallId::nil().to_string()).unwrap())
                .unwrap();
        assert!(call.is_nil());
    }

    #[test]
    fn aor_parse_rejects_crlf_and_overlong_values() {
        assert!(Aor::parse("sip:alice@example.com").is_some());
        assert!(Aor::parse("sip:alice\r\n@example.com").is_none());
        let long_user = "a".repeat(MAX_AOR_LENGTH);
        let overlong = format!("sip:{}@example.com", long_user);
        assert!(Aor::parse(&overlong).is_none());
    }
}
