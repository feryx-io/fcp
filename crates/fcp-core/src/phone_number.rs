//! Phone number management types for FCP.
//!
//! This module provides types for managing phone numbers as independent resources
//! that can be assigned to users and devices via line bindings.
//!
//! ## Design Philosophy
//!
//! Phone numbers are treated as inventory items that exist independently of users and devices.
//! This CUCM-style separation enables:
//! - Shared lines across multiple devices
//! - Number assignment to services (IVR, queues, etc.) without users
//! - Number portability and lifecycle management
//! - Flexible routing policies

use crate::identifiers::{LocationId, PhoneNumberId, SiteId, TenantId};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
use std::time::SystemTime;

/// A phone number in E.164 format.
///
/// PhoneNumber represents a routable endpoint in the PSTN. It includes
/// regulatory metadata (E911, CNAM), carrier information, and lifecycle state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhoneNumber {
    /// Internal unique identifier
    pub id: PhoneNumberId,

    /// E.164 formatted number (e.g., "+15183217034")
    pub e164: String,

    /// Country code (e.g., "1" for US/Canada)
    pub country_code: String,

    /// Numbering Plan Area / Area code (e.g., "518")
    pub npa: String,

    /// Central office / Exchange code (e.g., "321")
    pub nxx: String,

    /// Subscriber number / Line number (e.g., "7034")
    pub line: String,

    /// Current lifecycle state
    pub state: NumberState,

    /// Tenant this number belongs to
    pub tenant_id: TenantId,

    // ===== Regulatory & Compliance =====
    /// E911 emergency address
    pub e911_address: Option<E911Address>,

    /// E911 callback number (must be set before activation)
    pub e911_callback: Option<String>,

    /// Caller ID name (max 15 characters for CNAM)
    pub cnam: Option<String>,

    /// STIR/SHAKEN attestation level (0.0-1.0)
    pub spam_score: Option<f32>,

    // ===== Carrier Metadata =====
    /// Carrier/provider who owns this number
    pub carrier: Option<String>,

    /// Location Routing Number (for Local Number Portability)
    pub lrn: Option<String>,

    /// Operating Company Number
    pub ocn: Option<String>,

    /// Rate center
    pub rate_center: Option<String>,

    /// Local Access and Transport Area
    pub lata: Option<String>,

    // ===== Lifecycle =====
    /// When the number was acquired
    pub acquired_at: SystemTime,

    /// When the number was activated
    pub activated_at: Option<SystemTime>,

    /// When the number was ported in/out
    pub ported_at: Option<SystemTime>,

    /// Letter of Authorization date (for porting)
    pub loa_date: Option<SystemTime>,

    /// Firm Order Commitment date (for porting)
    pub foc_date: Option<SystemTime>,

    // ===== Feature Flags =====
    /// Enable call recording for this number
    pub recording_enabled: bool,

    /// Block calls from anonymous/private numbers
    pub block_anonymous: bool,

    /// Whitelist of allowed caller numbers
    pub allow_list: Vec<String>,

    /// Blacklist of blocked caller numbers
    pub block_list: Vec<String>,

    /// Tags for billing, departments, etc.
    pub tags: Vec<String>,

    // ===== Location =====
    /// Associated site location (for E911, regulatory, organizational purposes)
    pub location_id: Option<LocationId>,

    /// The site whose phones answer this number, in a hybrid deployment
    /// (`docs/HYBRID_CONTROL_PLANE.md` §3.6): the cloud sends a call for
    /// it down that site's link. `None`: answered here, as ever.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub site_id: Option<SiteId>,

    // ===== Audit =====
    /// Created timestamp
    pub created_at: SystemTime,

    /// Last modified timestamp
    pub updated_at: SystemTime,
}

/// Extract country code from E.164 digits (without the leading +).
///
/// Returns (country_code, remaining_digits).
/// This function uses a lookup table for known 3-digit country codes,
/// then falls back to heuristics for 1 and 2-digit codes.
fn extract_country_code(digits: &str) -> (String, &str) {
    // Try 1-digit country code (only valid is '1' for NANP)
    if let Some(rest) = digits.strip_prefix('1') {
        return ("1".to_string(), rest);
    }

    // Check for known 3-digit country codes using exact prefix matching
    // This avoids ambiguity with 2-digit codes (e.g., 33 France vs 353 Ireland)
    if digits.len() >= 3 {
        let three_digit = &digits[0..3];
        // List of common 3-digit country codes
        // Source: ITU-T E.164 country calling codes
        const THREE_DIGIT_CODES: &[&str] = &[
            "290", // Saint Helena
            "291", // Eritrea
            "297", // Aruba
            "298", // Faroe Islands
            "299", // Greenland
            "350", // Gibraltar
            "351", // Portugal
            "352", // Luxembourg
            "353", // Ireland
            "354", // Iceland
            "355", // Albania
            "356", // Malta
            "357", // Cyprus
            "358", // Finland
            "359", // Bulgaria
            "370", // Lithuania
            "371", // Latvia
            "372", // Estonia
            "373", // Moldova
            "374", // Armenia
            "375", // Belarus
            "376", // Andorra
            "377", // Monaco
            "378", // San Marino
            "379", // Vatican City (reserved)
            "380", // Ukraine
            "381", // Serbia
            "382", // Montenegro
            "383", // Kosovo
            "385", // Croatia
            "386", // Slovenia
            "387", // Bosnia and Herzegovina
            "389", // North Macedonia
            "420", // Czech Republic
            "421", // Slovakia
            "423", // Liechtenstein
            "500", // Falkland Islands
            "501", // Belize
            "502", // Guatemala
            "503", // El Salvador
            "504", // Honduras
            "505", // Nicaragua
            "506", // Costa Rica
            "507", // Panama
            "508", // Saint Pierre and Miquelon
            "509", // Haiti
            "590", // Guadeloupe
            "591", // Bolivia
            "592", // Guyana
            "593", // Ecuador
            "594", // French Guiana
            "595", // Paraguay
            "596", // Martinique
            "597", // Suriname
            "598", // Uruguay
            "599", // Curaçao / Caribbean Netherlands
            "670", // East Timor
            "672", // Australian Antarctic Territory / Norfolk Island
            "673", // Brunei
            "674", // Nauru
            "675", // Papua New Guinea
            "676", // Tonga
            "677", // Solomon Islands
            "678", // Vanuatu
            "679", // Fiji
            "680", // Palau
            "681", // Wallis and Futuna
            "682", // Cook Islands
            "683", // Niue
            "685", // Samoa
            "686", // Kiribati
            "687", // New Caledonia
            "688", // Tuvalu
            "689", // French Polynesia
            "690", // Tokelau
            "850", // North Korea
            "852", // Hong Kong
            "853", // Macau
            "855", // Cambodia
            "856", // Laos
            "880", // Bangladesh
            "886", // Taiwan
            "960", // Maldives
            "961", // Lebanon
            "962", // Jordan
            "963", // Syria
            "964", // Iraq
            "965", // Kuwait
            "966", // Saudi Arabia
            "967", // Yemen
            "968", // Oman
            "970", // Palestine
            "971", // United Arab Emirates
            "972", // Israel
            "973", // Bahrain
            "974", // Qatar
            "975", // Bhutan
            "976", // Mongolia
            "977", // Nepal
            "992", // Tajikistan
            "993", // Turkmenistan
            "994", // Azerbaijan
            "995", // Georgia
            "996", // Kyrgyzstan
            "998", // Uzbekistan
        ];

        if THREE_DIGIT_CODES.contains(&three_digit) {
            return (three_digit.to_string(), &digits[3..]);
        }
    }

    // Try 2-digit country codes (most common case)
    // Common 2-digit codes include: 20-99
    // Notable: 44 (UK), 49 (Germany), 33 (France), 39 (Italy), 34 (Spain), etc.
    if digits.len() >= 2 {
        let two_digit = &digits[0..2];
        if let Ok(code) = two_digit.parse::<u16>() {
            if (20..=99).contains(&code) {
                return (two_digit.to_string(), &digits[2..]);
            }
        }
    }

    // Final fallback: assume 2 digits (most common case)
    if digits.len() >= 2 {
        (digits[0..2].to_string(), &digits[2..])
    } else {
        // Very short number, just use what we have
        (digits.to_string(), "")
    }
}

impl PhoneNumber {
    /// Create a new phone number from E.164 string.
    ///
    /// This will parse the E.164 number and extract country code, NPA, NXX, and line.
    /// For North American numbers (+1XXXXXXXXXX), this follows NANP rules.
    pub fn new(e164: String, tenant_id: TenantId) -> Result<Self, ParseError> {
        // Validate E.164 format
        if !e164.starts_with('+') {
            return Err(ParseError::InvalidE164("Must start with +".to_string()));
        }

        if e164.len() < 8 || e164.len() > 16 {
            return Err(ParseError::InvalidE164(
                "Length must be between 8 and 16 characters".to_string(),
            ));
        }

        // Extract digits only (remove '+')
        let digits = &e164[1..];
        if !digits.chars().all(|c| c.is_ascii_digit()) {
            return Err(ParseError::InvalidE164(
                "Must contain only digits after +".to_string(),
            ));
        }

        // Parse NANP numbers (North America)
        let (country_code, npa, nxx, line) = if digits.starts_with('1') && digits.len() == 11 {
            // NANP: +1 NPA NXX XXXX
            let country_code = "1".to_string();
            let npa = digits[1..4].to_string();
            let nxx = digits[4..7].to_string();
            let line = digits[7..11].to_string();
            (country_code, npa, nxx, line)
        } else {
            // For non-NANP numbers, extract country code properly
            let (country_code, remaining) = extract_country_code(digits);

            // For the remaining digits, use a reasonable split
            // We don't try to enforce NANP-style NPA/NXX structure on international numbers
            // Instead, we store the remaining digits more naturally
            if remaining.len() >= 6 {
                // Split remaining as: area code (variable), exchange (3 digits), line (rest)
                let split_point = remaining.len() - 4;
                let npa = if split_point >= 3 {
                    remaining[..3].to_string()
                } else {
                    remaining[..split_point].to_string()
                };
                let nxx = if split_point >= 3 && remaining.len() >= 7 {
                    remaining[3..split_point].to_string()
                } else {
                    String::new()
                };
                let line = remaining[split_point..].to_string();
                (country_code, npa, nxx, line)
            } else {
                // Short number, store everything as line
                (
                    country_code,
                    String::new(),
                    String::new(),
                    remaining.to_string(),
                )
            }
        };

        let now = SystemTime::now();

        Ok(Self {
            id: PhoneNumberId::new(),
            e164,
            country_code,
            npa,
            nxx,
            line,
            state: NumberState::Unassigned,
            tenant_id,
            e911_address: None,
            e911_callback: None,
            cnam: None,
            spam_score: None,
            carrier: None,
            lrn: None,
            ocn: None,
            rate_center: None,
            lata: None,
            acquired_at: now,
            activated_at: None,
            ported_at: None,
            loa_date: None,
            foc_date: None,
            recording_enabled: false,
            block_anonymous: false,
            allow_list: Vec::new(),
            block_list: Vec::new(),
            tags: Vec::new(),
            location_id: None,
            site_id: None,
            created_at: now,
            updated_at: now,
        })
    }

    /// Returns true if this is a system pool number (not assigned to any tenant).
    pub fn is_pool_number(&self) -> bool {
        self.tenant_id.is_nil()
    }

    /// Check if the number can be activated.
    ///
    /// A number can only be activated if it has an E911 address configured.
    pub fn can_activate(&self) -> Result<(), String> {
        if self.e911_address.is_none() {
            return Err("E911 address is required before activation".to_string());
        }
        if self.e911_callback.is_none() {
            return Err("E911 callback number is required before activation".to_string());
        }
        Ok(())
    }

    /// Activate the number.
    ///
    /// This checks E911 requirements and transitions from Unassigned/Reserved to Active.
    pub fn activate(&mut self) -> Result<(), String> {
        self.can_activate()?;

        match self.state {
            NumberState::Unassigned | NumberState::Assigned | NumberState::Reserved => {
                self.state = NumberState::Active;
                self.activated_at = Some(SystemTime::now());
                self.updated_at = SystemTime::now();
                Ok(())
            }
            NumberState::Active => Err("Number is already active".to_string()),
            NumberState::Available => {
                Err("Cannot activate a pool number — purchase it first".to_string())
            }
            NumberState::Quarantine => {
                Err("Cannot activate number in quarantine state".to_string())
            }
            NumberState::Ported => Err("Cannot activate ported-out number".to_string()),
        }
    }

    /// Validate CNAM (Caller ID Name).
    ///
    /// CNAM is limited to 15 characters by carrier requirements.
    /// An empty one clears it.
    pub fn set_cnam(&mut self, cnam: String) -> Result<(), String> {
        if cnam.len() > 15 {
            return Err("CNAM must be 15 characters or less".to_string());
        }
        self.cnam = Some(cnam).filter(|c| !c.trim().is_empty());
        self.updated_at = SystemTime::now();
        Ok(())
    }
}

/// Lifecycle state of a phone number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NumberState {
    /// In the system pool, available for purchase/assignment to a tenant
    Available,

    /// In inventory, not assigned to anyone
    Unassigned,

    /// Assigned to a tenant (purchased from pool) but not yet activated
    Assigned,

    /// Held for future use (e.g., port-in pending)
    Reserved,

    /// In use, routing calls
    Active,

    /// Recently released (90-day cool-off for spam reputation)
    Quarantine,

    /// Ported out to another carrier
    Ported,
}

impl fmt::Display for NumberState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Available => write!(f, "available"),
            Self::Unassigned => write!(f, "unassigned"),
            Self::Assigned => write!(f, "assigned"),
            Self::Reserved => write!(f, "reserved"),
            Self::Active => write!(f, "active"),
            Self::Quarantine => write!(f, "quarantine"),
            Self::Ported => write!(f, "ported"),
        }
    }
}

impl FromStr for NumberState {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "available" => Ok(Self::Available),
            "unassigned" => Ok(Self::Unassigned),
            "assigned" => Ok(Self::Assigned),
            "reserved" => Ok(Self::Reserved),
            "active" => Ok(Self::Active),
            "quarantine" => Ok(Self::Quarantine),
            "ported" => Ok(Self::Ported),
            _ => Err(ParseError::InvalidNumberState(s.to_string())),
        }
    }
}

/// E911 emergency address.
///
/// This address is sent to Public Safety Answering Points (PSAPs) when
/// emergency services (911 in US) are dialed from this number.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct E911Address {
    /// Street address line 1
    pub address1: String,

    /// Street address line 2 (optional)
    pub address2: Option<String>,

    /// City
    pub city: String,

    /// State/Province
    pub state: String,

    /// Postal/ZIP code
    pub postal_code: String,

    /// Country
    pub country: String,

    /// Validated flag (set by E911 validation service)
    #[serde(default)]
    pub validated: bool,

    /// Validation timestamp
    #[serde(default)]
    pub validated_at: Option<SystemTime>,

    /// The validator that last checked this address (`basic`, `http`, …),
    /// set with `validated` (`docs/E911_BLOCK_2.md` §7).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validated_by: Option<String>,

    /// The reference a validation provider gave for its answer, for
    /// support calls about a rejected address.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_reference: Option<String>,

    /// The address's RFC 5139 civic elements, when something structured it
    /// (`docs/E911_BLOCK_2.md` §5.1). Kept alongside the line fields, which
    /// stay what people read and edit; flattened into the same JSON object.
    #[serde(flatten, default)]
    pub civic: CivicElements,
}

/// The RFC 5139 civic address elements beyond city, state, postal code and
/// country, which [`E911Address`] carries as fields of its own.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CivicElements {
    /// `HNO`: the numeric part of the house number (`1455`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub house_number: Option<String>,
    /// `HNS`: a house number suffix (`A`, `1/2`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub house_number_suffix: Option<String>,
    /// `PRD`: a leading street direction (`N`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub street_predirection: Option<String>,
    /// `RD`: the street name (`Market`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub street_name: Option<String>,
    /// `STS`: the street suffix (`St`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub street_suffix: Option<String>,
    /// `POD`: a trailing street direction (`SW`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub street_postdirection: Option<String>,
    /// `LOC`: additional location information (`Room 4B`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub additional: Option<String>,
    /// `FLR`: the floor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub floor: Option<String>,
    /// `UNIT`: a unit or suite.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    /// `ROOM`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub room: Option<String>,
    /// `BLD`: the building.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub building: Option<String>,
    /// `NAM`: the name of the place (`Example HQ`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

impl CivicElements {
    /// Why these elements cannot be stored: an element longer than 128
    /// characters or holding control characters (they are written into
    /// XML and headers). Empty when fine.
    pub fn problems(&self) -> Vec<String> {
        let named = [
            ("house_number", &self.house_number),
            ("house_number_suffix", &self.house_number_suffix),
            ("street_predirection", &self.street_predirection),
            ("street_name", &self.street_name),
            ("street_suffix", &self.street_suffix),
            ("street_postdirection", &self.street_postdirection),
            ("additional", &self.additional),
            ("floor", &self.floor),
            ("unit", &self.unit),
            ("room", &self.room),
            ("building", &self.building),
            ("name", &self.name),
        ];
        named
            .iter()
            .filter_map(|(name, value)| {
                let value = value.as_deref()?;
                if value.chars().count() > 128 {
                    Some(format!("{name} is longer than 128 characters"))
                } else if value.chars().any(char::is_control) {
                    Some(format!("{name} contains control characters"))
                } else {
                    None
                }
            })
            .collect()
    }

    /// Whether the street address is in elements: a street name and a
    /// house number. An address that is not is sent with its street line
    /// as the street name and flagged unstructured.
    pub fn is_structured(&self) -> bool {
        let present = |v: &Option<String>| v.as_deref().is_some_and(|s| !s.trim().is_empty());
        present(&self.street_name) && present(&self.house_number)
    }
}

impl E911Address {
    /// Create a new E911 address.
    pub fn new(
        address1: String,
        city: String,
        state: String,
        postal_code: String,
        country: String,
    ) -> Self {
        Self {
            address1,
            address2: None,
            city,
            state,
            postal_code,
            country,
            validated: false,
            validated_at: None,
            validated_by: None,
            provider_reference: None,
            civic: CivicElements::default(),
        }
    }

    /// The parts a person edits: everything but the validation marks.
    /// Two addresses that agree here are the same address.
    pub fn same_address(&self, other: &Self) -> bool {
        self.address1 == other.address1
            && self.address2 == other.address2
            && self.city == other.city
            && self.state == other.state
            && self.postal_code == other.postal_code
            && self.country == other.country
            && self.civic == other.civic
    }

    /// Forget that this address was validated: editing an address clears
    /// its validation (`docs/E911_BLOCK_2.md` §7).
    pub fn clear_validation(&mut self) {
        self.validated = false;
        self.validated_at = None;
        self.validated_by = None;
        self.provider_reference = None;
    }

    /// Keep the validation of `previous` when this is the same address,
    /// and drop it when anything about the address changed.
    pub fn carry_validation(&mut self, previous: Option<&Self>) {
        match previous.filter(|p| p.same_address(self)) {
            Some(previous) => {
                self.validated = previous.validated;
                self.validated_at = previous.validated_at;
                self.validated_by = previous.validated_by.clone();
                self.provider_reference = previous.provider_reference.clone();
            }
            None => self.clear_validation(),
        }
    }

    /// Format address as a single line.
    pub fn to_single_line(&self) -> String {
        let mut parts = vec![self.address1.as_str()];
        if let Some(ref addr2) = self.address2 {
            parts.push(addr2.as_str());
        }
        parts.push(self.city.as_str());
        parts.push(self.state.as_str());
        parts.push(self.postal_code.as_str());
        parts.push(self.country.as_str());
        parts.join(", ")
    }
}

impl fmt::Display for E911Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_single_line())
    }
}

/// Parse errors for phone number operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    /// Invalid E.164 format
    InvalidE164(String),

    /// Invalid number state
    InvalidNumberState(String),
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidE164(msg) => write!(f, "Invalid E.164 format: {}", msg),
            Self::InvalidNumberState(state) => write!(f, "Invalid number state: {}", state),
        }
    }
}

impl std::error::Error for ParseError {}

/// What a number's screening says of a caller (`docs/ENTERPRISE_CALLING.md`
/// §3.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screening {
    Allowed,
    /// The caller withheld their number and the number blocks that.
    Anonymous,
    /// The number has an allow list and the caller is not on it.
    NotAllowed,
    /// The caller is on the number's block list.
    Blocked,
}

impl Screening {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Allowed => "allowed",
            Self::Anonymous => "anonymous",
            Self::NotAllowed => "not_allowed",
            Self::Blocked => "blocked",
        }
    }
}

/// Whether a list entry names a caller: the digits equal, one a national
/// form of the other (the shorter at least seven digits), or the entry a
/// prefix ending in `*`.
fn list_matches(entry: &str, caller_digits: &str) -> bool {
    let entry = entry.trim();
    if let Some(prefix) = entry.strip_suffix('*') {
        let prefix: String = prefix.chars().filter(char::is_ascii_digit).collect();
        return !prefix.is_empty() && caller_digits.starts_with(&prefix);
    }
    let entry: String = entry.chars().filter(char::is_ascii_digit).collect();
    if entry.is_empty() || caller_digits.is_empty() {
        return false;
    }
    if entry == caller_digits {
        return true;
    }
    let (short, long) = if entry.len() < caller_digits.len() {
        (entry.as_str(), caller_digits)
    } else {
        (caller_digits, entry.as_str())
    };
    short.len() >= 7 && long.ends_with(short)
}

impl PhoneNumber {
    /// Screen a caller: `caller` is the calling number (the From user),
    /// `None` when withheld. The block list is read first, then the
    /// allow list; an anonymous caller is blocked only by
    /// `block_anonymous`, and an allow list lets no anonymous caller in.
    pub fn screen(&self, caller: Option<&str>) -> Screening {
        let Some(caller) = caller else {
            if self.block_anonymous || !self.allow_list.is_empty() {
                return Screening::Anonymous;
            }
            return Screening::Allowed;
        };
        let digits: String = caller.chars().filter(char::is_ascii_digit).collect();
        if self.block_list.iter().any(|e| list_matches(e, &digits)) {
            return Screening::Blocked;
        }
        if !self.allow_list.is_empty() && !self.allow_list.iter().any(|e| list_matches(e, &digits))
        {
            return Screening::NotAllowed;
        }
        Screening::Allowed
    }
}

#[cfg(test)]
mod screening_tests {
    use super::*;

    fn number() -> PhoneNumber {
        PhoneNumber::new("+15185550100".to_string(), crate::TenantId::new()).expect("a number")
    }

    #[test]
    fn the_block_list_then_the_allow_list_then_anonymity() {
        let mut n = number();
        assert_eq!(n.screen(None), Screening::Allowed);
        assert_eq!(n.screen(Some("+12125550199")), Screening::Allowed);
        n.block_anonymous = true;
        assert_eq!(n.screen(None), Screening::Anonymous);
        n.block_list = vec!["+1 212 555 0199".into(), "1900*".into()];
        assert_eq!(n.screen(Some("2125550199")), Screening::Blocked);
        assert_eq!(n.screen(Some("+19005550100")), Screening::Blocked);
        n.allow_list = vec!["+13125550100".into()];
        assert_eq!(n.screen(Some("+13125550100")), Screening::Allowed);
        assert_eq!(n.screen(Some("+13125550101")), Screening::NotAllowed);
        n.block_anonymous = false;
        assert_eq!(n.screen(None), Screening::Anonymous);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_nanp_number() {
        let tenant_id = TenantId::new();
        let number = PhoneNumber::new("+15183217034".to_string(), tenant_id).unwrap();

        assert_eq!(number.e164, "+15183217034");
        assert_eq!(number.country_code, "1");
        assert_eq!(number.npa, "518");
        assert_eq!(number.nxx, "321");
        assert_eq!(number.line, "7034");
        assert_eq!(number.state, NumberState::Unassigned);
    }

    #[test]
    fn test_invalid_e164() {
        let tenant_id = TenantId::new();

        // Missing +
        assert!(PhoneNumber::new("15183217034".to_string(), tenant_id).is_err());

        // Too short
        assert!(PhoneNumber::new("+1234".to_string(), tenant_id).is_err());

        // Contains non-digits
        assert!(PhoneNumber::new("+1518abc7034".to_string(), tenant_id).is_err());
    }

    #[test]
    fn test_number_activation() {
        let tenant_id = TenantId::new();
        let mut number = PhoneNumber::new("+15183217034".to_string(), tenant_id).unwrap();

        // Cannot activate without E911
        assert!(number.activate().is_err());

        // Set E911 address
        number.e911_address = Some(E911Address::new(
            "123 Main St".to_string(),
            "Albany".to_string(),
            "NY".to_string(),
            "12201".to_string(),
            "US".to_string(),
        ));
        number.e911_callback = Some("+15183217034".to_string());

        // Now activation should work
        assert!(number.activate().is_ok());
        assert_eq!(number.state, NumberState::Active);
        assert!(number.activated_at.is_some());

        // Cannot activate twice
        assert!(number.activate().is_err());
    }

    #[test]
    fn test_cnam_validation() {
        let tenant_id = TenantId::new();
        let mut number = PhoneNumber::new("+15183217034".to_string(), tenant_id).unwrap();

        // Valid CNAM (15 chars)
        assert!(number.set_cnam("John Smith".to_string()).is_ok());
        assert_eq!(number.cnam, Some("John Smith".to_string()));

        // Too long (16 chars)
        assert!(number.set_cnam("This is too long".to_string()).is_err());
    }

    #[test]
    fn test_e911_address_display() {
        let addr = E911Address::new(
            "123 Main St".to_string(),
            "Albany".to_string(),
            "NY".to_string(),
            "12201".to_string(),
            "US".to_string(),
        );

        let display = addr.to_string();
        assert!(display.contains("123 Main St"));
        assert!(display.contains("Albany"));
        assert!(display.contains("NY"));
        assert!(display.contains("12201"));
    }

    #[test]
    fn test_number_state_parsing() {
        assert_eq!(
            "active".parse::<NumberState>().unwrap(),
            NumberState::Active
        );
        assert_eq!(
            "unassigned".parse::<NumberState>().unwrap(),
            NumberState::Unassigned
        );
        assert_eq!(
            "reserved".parse::<NumberState>().unwrap(),
            NumberState::Reserved
        );
        assert_eq!(
            "quarantine".parse::<NumberState>().unwrap(),
            NumberState::Quarantine
        );
        assert_eq!(
            "ported".parse::<NumberState>().unwrap(),
            NumberState::Ported
        );

        assert!("invalid".parse::<NumberState>().is_err());
    }

    #[test]
    fn test_parse_uk_number() {
        // UK number: +44 20 7946 0958
        let tenant_id = TenantId::new();
        let number = PhoneNumber::new("+442079460958".to_string(), tenant_id).unwrap();

        assert_eq!(number.e164, "+442079460958");
        assert_eq!(number.country_code, "44"); // Not "442"!
        assert_eq!(number.npa, "207"); // London area code
        assert!(!number.nxx.is_empty() || !number.line.is_empty()); // Remaining digits stored
    }

    #[test]
    fn test_parse_german_number() {
        // German number: +49 30 12345678
        let tenant_id = TenantId::new();
        let number = PhoneNumber::new("+493012345678".to_string(), tenant_id).unwrap();

        assert_eq!(number.e164, "+493012345678");
        assert_eq!(number.country_code, "49"); // Not "493"!
        assert_eq!(number.npa, "301"); // Berlin area code
    }

    #[test]
    fn test_parse_irish_number() {
        // Irish number: +353 1 234 5678 (3-digit country code)
        let tenant_id = TenantId::new();
        let number = PhoneNumber::new("+35312345678".to_string(), tenant_id).unwrap();

        assert_eq!(number.e164, "+35312345678");
        assert_eq!(number.country_code, "353"); // 3-digit country code
        assert_eq!(number.npa, "123"); // Dublin area
    }

    #[test]
    fn test_parse_french_number() {
        // French number: +33 1 42 86 82 00
        let tenant_id = TenantId::new();
        let number = PhoneNumber::new("+33142868200".to_string(), tenant_id).unwrap();

        assert_eq!(number.e164, "+33142868200");
        assert_eq!(number.country_code, "33"); // Not "331"!
        assert_eq!(number.npa, "142");
    }

    #[test]
    fn test_parse_japanese_number() {
        // Japanese number: +81 3 1234 5678
        let tenant_id = TenantId::new();
        let number = PhoneNumber::new("+81312345678".to_string(), tenant_id).unwrap();

        assert_eq!(number.e164, "+81312345678");
        assert_eq!(number.country_code, "81"); // Not "813"!
        assert_eq!(number.npa, "312");
    }
}
