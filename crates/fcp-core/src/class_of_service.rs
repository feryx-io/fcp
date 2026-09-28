//! Class of service (`docs/ENTERPRISE_CALLING.md` §3.7): what kind of
//! destination a dialled number is, and what a user may call.
//!
//! A number is classified by the tenant's country — its international and
//! trunk prefixes, its toll-free and premium ranges — from a built-in table
//! of the countries the platform has trunked to, with the tenant's own
//! overrides winning. A class of service names the kinds a user may call,
//! how many calls they may be on at once, and how their caller ID leaves.
//! A user with no class, in a tenant with no default class, may call
//! anything: an upgrade refuses no call.

use crate::{ClassOfServiceId, TenantId};
use anyhow::Result as AnyhowResult;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::time::SystemTime;

/// What kind of destination a dialled number is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DestinationClass {
    /// An extension, a user, a feature code, a number of the tenant's own.
    Internal,
    /// An emergency number. Always allowed.
    Emergency,
    TollFree,
    National,
    International,
    Premium,
    /// A number that could not be read.
    Unknown,
}

impl DestinationClass {
    pub const ALL: [DestinationClass; 7] = [
        Self::Internal,
        Self::Emergency,
        Self::TollFree,
        Self::National,
        Self::International,
        Self::Premium,
        Self::Unknown,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Internal => "internal",
            Self::Emergency => "emergency",
            Self::TollFree => "toll_free",
            Self::National => "national",
            Self::International => "international",
            Self::Premium => "premium",
            Self::Unknown => "unknown",
        }
    }
}

impl std::fmt::Display for DestinationClass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A tenant's own word on a range of numbers: `prefix` is E.164 when it
/// starts with `+` (`+1900`), or the digits as dialled otherwise (`118`).
/// The longest matching override wins over the built-in table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DestinationOverride {
    pub prefix: String,
    pub class: DestinationClass,
}

impl DestinationOverride {
    pub fn validate(&self) -> Result<(), String> {
        let digits = self.prefix.strip_prefix('+').unwrap_or(&self.prefix);
        if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
            return Err(format!(
                "destination override `{}`: a prefix is digits, with a leading `+` for E.164",
                self.prefix
            ));
        }
        if self.class == DestinationClass::Emergency {
            return Err(
                "destination override: emergency numbers are the tenant's emergency settings"
                    .into(),
            );
        }
        Ok(())
    }
}

/// How a country dials, and its special ranges (E.164 prefixes).
struct Country {
    codes: &'static [&'static str],
    calling_code: &'static str,
    international_prefix: &'static str,
    /// The national trunk prefix (`0`), when the country has one.
    trunk_prefix: Option<&'static str>,
    /// Digits of a national number dialled with no trunk prefix (NANP's
    /// ten, Australia's `13`/`18`/`19` service numbers).
    bare_national: fn(&str) -> bool,
    emergency: &'static [&'static str],
    toll_free: &'static [&'static str],
    premium: &'static [&'static str],
}

fn nanp_bare(digits: &str) -> bool {
    digits.len() == 10 || (digits.len() == 11 && digits.starts_with('1'))
}

fn no_bare(_: &str) -> bool {
    false
}

fn au_bare(digits: &str) -> bool {
    digits.starts_with('1') && (6..=10).contains(&digits.len())
}

const COUNTRIES: &[Country] = &[
    Country {
        codes: &["US", "CA"],
        calling_code: "1",
        international_prefix: "011",
        trunk_prefix: None,
        bare_national: nanp_bare,
        emergency: &["911", "933"],
        toll_free: &[
            "+1800", "+1833", "+1844", "+1855", "+1866", "+1877", "+1888",
        ],
        premium: &["+1900", "+1976"],
    },
    Country {
        codes: &["GB"],
        calling_code: "44",
        international_prefix: "00",
        trunk_prefix: Some("0"),
        bare_national: no_bare,
        emergency: &["999", "112"],
        toll_free: &["+44800", "+44808"],
        premium: &["+4490", "+4491", "+4498"],
    },
    Country {
        codes: &["DE"],
        calling_code: "49",
        international_prefix: "00",
        trunk_prefix: Some("0"),
        bare_national: no_bare,
        emergency: &["112", "110"],
        toll_free: &["+49800"],
        premium: &["+49900", "+49137"],
    },
    Country {
        codes: &["FR"],
        calling_code: "33",
        international_prefix: "00",
        trunk_prefix: Some("0"),
        bare_national: no_bare,
        emergency: &["112", "15", "17", "18"],
        toll_free: &["+33800", "+33801", "+33802", "+33803", "+33804", "+33805"],
        premium: &["+3389"],
    },
    Country {
        codes: &["AU"],
        calling_code: "61",
        international_prefix: "0011",
        trunk_prefix: Some("0"),
        bare_national: au_bare,
        emergency: &["000", "112"],
        toll_free: &["+611800"],
        premium: &["+61190"],
    },
    Country {
        codes: &["NZ"],
        calling_code: "64",
        international_prefix: "00",
        trunk_prefix: Some("0"),
        bare_national: no_bare,
        emergency: &["111"],
        toll_free: &["+64800", "+64508"],
        premium: &["+64900"],
    },
];

/// The countries the built-in table knows (ISO 3166-1 alpha-2).
pub fn known_countries() -> Vec<&'static str> {
    COUNTRIES
        .iter()
        .flat_map(|c| c.codes.iter().copied())
        .collect()
}

fn country(code: Option<&str>) -> &'static Country {
    let code = code.unwrap_or("US");
    COUNTRIES
        .iter()
        .find(|c| c.codes.iter().any(|k| k.eq_ignore_ascii_case(code)))
        .unwrap_or(&COUNTRIES[0])
}

/// How a tenant dials.
#[derive(Debug, Clone, Copy)]
pub struct Dialing<'a> {
    /// ISO 3166-1 alpha-2; unset is `US`, the numbering the platform
    /// normalised by before it knew a tenant's country.
    pub country: Option<&'a str>,
    /// Digits up to this length, dialled bare, are extensions.
    pub max_extension_digits: usize,
    pub overrides: &'a [DestinationOverride],
}

/// A dialled number, classified.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Classified {
    pub class: DestinationClass,
    /// The number in E.164, when it is a number.
    pub e164: Option<String>,
}

/// Classify what was dialled: the user part of the Request-URI.
pub fn classify(dialed: &str, dialing: &Dialing<'_>) -> Classified {
    let home = country(dialing.country);
    let cleaned: String = dialed
        .trim()
        .chars()
        .filter(|c| !matches!(c, ' ' | '-' | '.' | '(' | ')'))
        .collect();
    let internal = Classified {
        class: DestinationClass::Internal,
        e164: None,
    };
    let unknown = Classified {
        class: DestinationClass::Unknown,
        e164: None,
    };
    if cleaned.is_empty() {
        return unknown;
    }
    // A feature code.
    if cleaned.starts_with('*') || cleaned.starts_with('#') {
        return internal;
    }
    let (plus, digits) = match cleaned.strip_prefix('+') {
        Some(rest) => (true, rest),
        None => (false, cleaned.as_str()),
    };
    if !digits.chars().all(|c| c.is_ascii_digit()) {
        // A name — a user, a room — reaches something of the platform's.
        return if plus { unknown } else { internal };
    }
    if !plus && home.emergency.contains(&digits) {
        return Classified {
            class: DestinationClass::Emergency,
            e164: None,
        };
    }
    // A tenant's override on the digits as dialled.
    let dialled_override = (!plus)
        .then(|| longest_override(dialing.overrides, digits, false))
        .flatten();

    let e164 = if plus {
        Some(format!("+{digits}"))
    } else if let Some(rest) = digits.strip_prefix(home.international_prefix) {
        Some(format!("+{rest}"))
    } else if let Some(rest) = home
        .trunk_prefix
        .and_then(|p| digits.strip_prefix(p))
        .filter(|rest| !rest.is_empty())
    {
        Some(format!("+{}{rest}", home.calling_code))
    } else if (home.bare_national)(digits) {
        if home.calling_code == "1" && digits.len() == 11 {
            Some(format!("+{digits}"))
        } else {
            Some(format!("+{}{digits}", home.calling_code))
        }
    } else {
        None
    };

    if let Some(class) = dialled_override {
        return Classified { class, e164 };
    }
    let Some(e164) = e164 else {
        return if digits.len() <= dialing.max_extension_digits {
            internal
        } else {
            unknown
        };
    };
    // E.164 needs a country code and a subscriber number.
    if e164.len() < 8 || e164.len() > 16 {
        return unknown;
    }
    let class = longest_override(dialing.overrides, &e164, true).unwrap_or_else(|| {
        if home.premium.iter().any(|p| e164.starts_with(p)) {
            DestinationClass::Premium
        } else if home.toll_free.iter().any(|p| e164.starts_with(p)) {
            DestinationClass::TollFree
        } else if e164[1..].starts_with(home.calling_code) {
            DestinationClass::National
        } else {
            DestinationClass::International
        }
    });
    Classified {
        class,
        e164: Some(e164),
    }
}

fn longest_override(
    overrides: &[DestinationOverride],
    number: &str,
    e164: bool,
) -> Option<DestinationClass> {
    overrides
        .iter()
        .filter(|o| o.prefix.starts_with('+') == e164 && number.starts_with(o.prefix.as_str()))
        .max_by_key(|o| o.prefix.len())
        .map(|o| o.class)
}

/// How a user's caller ID leaves on a trunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CallerIdPolicy {
    /// The number of the user's line (its `outbound_caller_id`), as the
    /// trunk allows.
    #[default]
    Line,
    /// An anonymous From with `Privacy: id` (RFC 3323/3325); the carrier
    /// still sees the identity in `P-Asserted-Identity`.
    Withheld,
}

/// The calls a user may be on at once, unless a class says otherwise.
pub const DEFAULT_MAX_CALLS: u32 = 2;

fn default_max_calls() -> Option<u32> {
    Some(DEFAULT_MAX_CALLS)
}

/// A tenant's class of service.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClassOfService {
    pub id: ClassOfServiceId,
    pub tenant_id: TenantId,
    pub name: String,
    /// The destinations its users may call. Emergency is always allowed,
    /// listed or not.
    pub allowed: Vec<DestinationClass>,
    /// The calls a user may be on at once, placed and received; `None`
    /// for no limit.
    #[serde(default = "default_max_calls")]
    pub max_calls: Option<u32>,
    #[serde(default)]
    pub caller_id: CallerIdPolicy,
    /// The class of a user who has none of their own.
    #[serde(default)]
    pub is_default: bool,
    #[serde(default = "SystemTime::now")]
    pub created_at: SystemTime,
    #[serde(default = "SystemTime::now")]
    pub updated_at: SystemTime,
}

impl ClassOfService {
    /// A class that allows everything, as a tenant's first default does.
    pub fn new(tenant_id: TenantId, name: impl Into<String>) -> Self {
        let now = SystemTime::now();
        Self {
            id: ClassOfServiceId::new(),
            tenant_id,
            name: name.into(),
            allowed: DestinationClass::ALL.to_vec(),
            max_calls: Some(DEFAULT_MAX_CALLS),
            caller_id: CallerIdPolicy::Line,
            is_default: false,
            created_at: now,
            updated_at: now,
        }
    }

    pub fn allows(&self, class: DestinationClass) -> bool {
        class == DestinationClass::Emergency || self.allowed.contains(&class)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty() {
            return Err("a class of service needs a name".into());
        }
        if self.max_calls == Some(0) {
            return Err("max_calls: at least one call, or none for no limit".into());
        }
        Ok(())
    }
}

/// The store of classes of service.
#[async_trait]
pub trait ClassOfServiceStore: Send + Sync {
    async fn get_class_of_service(
        &self,
        id: &ClassOfServiceId,
    ) -> AnyhowResult<Option<ClassOfService>>;
    async fn list_classes_of_service(
        &self,
        tenant_id: &TenantId,
    ) -> AnyhowResult<Vec<ClassOfService>>;
    async fn put_class_of_service(&self, class: ClassOfService) -> AnyhowResult<()>;
    async fn delete_class_of_service(&self, id: &ClassOfServiceId) -> AnyhowResult<()>;
}

/// The class a user calls under: their own, else the tenant's default,
/// else none (anything goes).
pub fn effective_class(
    classes: &[ClassOfService],
    assigned: Option<&ClassOfServiceId>,
) -> Option<ClassOfService> {
    assigned
        .and_then(|id| classes.iter().find(|c| c.id == *id))
        .or_else(|| classes.iter().find(|c| c.is_default))
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn us() -> Dialing<'static> {
        Dialing {
            country: Some("US"),
            max_extension_digits: 6,
            overrides: &[],
        }
    }

    fn class(dialed: &str, dialing: &Dialing) -> DestinationClass {
        classify(dialed, dialing).class
    }

    #[test]
    fn nanp_numbers_classify_by_range() {
        let d = us();
        assert_eq!(class("1001", &d), DestinationClass::Internal);
        assert_eq!(class("alice", &d), DestinationClass::Internal);
        assert_eq!(class("*72", &d), DestinationClass::Internal);
        assert_eq!(class("911", &d), DestinationClass::Emergency);
        assert_eq!(class("5185550100", &d), DestinationClass::National);
        assert_eq!(class("15185550100", &d), DestinationClass::National);
        assert_eq!(class("+1 (518) 555-0100", &d), DestinationClass::National);
        assert_eq!(class("18005550100", &d), DestinationClass::TollFree);
        assert_eq!(class("+18885550100", &d), DestinationClass::TollFree);
        assert_eq!(class("19005550100", &d), DestinationClass::Premium);
        assert_eq!(class("01144207946000", &d), DestinationClass::International);
        assert_eq!(class("+442079460000", &d), DestinationClass::International);
        assert_eq!(class("1234567", &d), DestinationClass::Unknown);
        assert_eq!(
            classify("5185550100", &d).e164.as_deref(),
            Some("+15185550100")
        );
    }

    #[test]
    fn a_trunk_prefix_country_reads_its_own_numbers() {
        let gb = Dialing {
            country: Some("gb"),
            max_extension_digits: 4,
            overrides: &[],
        };
        assert_eq!(class("02079460000", &gb), DestinationClass::National);
        assert_eq!(
            classify("02079460000", &gb).e164.as_deref(),
            Some("+442079460000")
        );
        assert_eq!(class("08001234567", &gb), DestinationClass::TollFree);
        assert_eq!(class("09011234567", &gb), DestinationClass::Premium);
        assert_eq!(class("0015185550100", &gb), DestinationClass::International);
        assert_eq!(class("999", &gb), DestinationClass::Emergency);
        let au = Dialing {
            country: Some("AU"),
            max_extension_digits: 4,
            overrides: &[],
        };
        assert_eq!(class("1800123456", &au), DestinationClass::TollFree);
        assert_eq!(class("1900123456", &au), DestinationClass::Premium);
        assert_eq!(
            class("0011442079460000", &au),
            DestinationClass::International
        );
    }

    #[test]
    fn a_tenant_override_wins_the_longest_match() {
        let overrides = [
            DestinationOverride {
                prefix: "+1".into(),
                class: DestinationClass::International,
            },
            DestinationOverride {
                prefix: "+1518".into(),
                class: DestinationClass::National,
            },
            DestinationOverride {
                prefix: "118".into(),
                class: DestinationClass::Premium,
            },
        ];
        let d = Dialing {
            country: Some("US"),
            max_extension_digits: 6,
            overrides: &overrides,
        };
        assert_eq!(class("5185550100", &d), DestinationClass::National);
        assert_eq!(class("2125550100", &d), DestinationClass::International);
        assert_eq!(class("118118", &d), DestinationClass::Premium);
    }

    #[test]
    fn emergency_is_always_allowed_and_the_default_class_applies() {
        let tenant = TenantId::new();
        let mut internal_only = ClassOfService::new(tenant, "internal");
        internal_only.allowed = vec![DestinationClass::Internal];
        assert!(internal_only.allows(DestinationClass::Emergency));
        assert!(!internal_only.allows(DestinationClass::National));
        let mut open = ClassOfService::new(tenant, "open");
        open.is_default = true;
        let classes = vec![internal_only.clone(), open.clone()];
        assert_eq!(
            effective_class(&classes, Some(&internal_only.id)).map(|c| c.id),
            Some(internal_only.id)
        );
        assert_eq!(effective_class(&classes, None).map(|c| c.id), Some(open.id));
        assert_eq!(effective_class(&[internal_only], None), None);
    }
}
