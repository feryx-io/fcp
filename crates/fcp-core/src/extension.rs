//! Extension mapping for short-code dialing.
//!
//! Extensions provide short dialing codes (e.g., "1234", "5000") that map to
//! users, phone numbers, devices, or services. This enables internal dialing
//! without using full E.164 numbers.

use crate::identifiers::{
    DeviceId, ExtensionId, LocationId, PhoneNumberId, ServiceId, TenantId, UserId,
};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::time::SystemTime;

/// An extension mapping for short-code dialing.
///
/// Extensions map short numeric codes to targets (users, phone numbers, devices, services).
/// They are tenant-scoped and must be unique within a tenant.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Extension {
    /// Unique identifier for this extension
    pub id: ExtensionId,

    /// Extension number (e.g., "1234", "5000")
    /// Typically 3-6 digits
    pub number: String,

    /// What this extension points to
    pub target: ExtensionTarget,

    /// Tenant this extension belongs to
    pub tenant_id: TenantId,

    /// Description for this extension
    pub description: Option<String>,

    /// Reference to a SiteLocation entity
    pub location_id: Option<LocationId>,

    /// Whether this extension is enabled
    pub enabled: bool,

    /// Created timestamp
    pub created_at: SystemTime,

    /// Last modified timestamp
    pub updated_at: SystemTime,
}

impl Extension {
    /// Create a new extension.
    pub fn new(
        number: String,
        target: ExtensionTarget,
        tenant_id: TenantId,
    ) -> Result<Self, ValidationError> {
        // Validate extension number
        Self::validate_extension_number(&number)?;

        let now = SystemTime::now();

        Ok(Self {
            id: ExtensionId::new(),
            number,
            target,
            tenant_id,
            description: None,
            location_id: None,
            enabled: true,
            created_at: now,
            updated_at: now,
        })
    }

    /// Validate extension number format.
    ///
    /// Requirements:
    /// - Must be numeric (digits only)
    /// - Must be 2-10 characters long
    /// - Cannot start with 0 or 1 (reserved for trunk access/operator)
    pub fn validate_extension_number(number: &str) -> Result<(), ValidationError> {
        if number.is_empty() {
            return Err(ValidationError::EmptyExtension);
        }

        if number.len() < 2 || number.len() > 10 {
            return Err(ValidationError::InvalidLength {
                length: number.len(),
            });
        }

        if !number.chars().all(|c| c.is_ascii_digit()) {
            return Err(ValidationError::NonNumeric);
        }

        // Check reserved prefixes
        if number.starts_with('0') {
            return Err(ValidationError::ReservedPrefix("0 (operator)".to_string()));
        }

        if number.starts_with("1") && number.len() >= 10 {
            return Err(ValidationError::ReservedPrefix(
                "1 (long distance/toll)".to_string(),
            ));
        }

        // Check for common reserved extensions
        match number {
            "911" | "933" => {
                return Err(ValidationError::ReservedPrefix(
                    "emergency services".to_string(),
                ))
            }
            "411" | "611" | "711" | "811" => {
                return Err(ValidationError::ReservedPrefix("service codes".to_string()))
            }
            _ => {}
        }

        Ok(())
    }

    /// Set description for this extension.
    pub fn set_description(&mut self, description: String) {
        self.description = Some(description);
        self.updated_at = SystemTime::now();
    }

    /// Enable or disable this extension.
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        self.updated_at = SystemTime::now();
    }
}

/// Target of an extension.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "id", rename_all = "snake_case")]
pub enum ExtensionTarget {
    /// Extension dials a user's primary line
    User(UserId),

    /// Extension maps to a specific phone number
    PhoneNumber(PhoneNumberId),

    /// Extension dials directly to a device
    Device(DeviceId),

    /// Extension routes to a service (IVR, queue, etc.)
    Service(ServiceId),
}

impl fmt::Display for ExtensionTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::User(id) => write!(f, "user:{}", id),
            Self::PhoneNumber(id) => write!(f, "phone_number:{}", id),
            Self::Device(id) => write!(f, "device:{}", id),
            Self::Service(id) => write!(f, "service:{}", id),
        }
    }
}

/// Extension validation errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationError {
    /// Extension number is empty
    EmptyExtension,

    /// Extension number has invalid length
    InvalidLength { length: usize },

    /// Extension contains non-numeric characters
    NonNumeric,

    /// Extension uses a reserved prefix
    ReservedPrefix(String),
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyExtension => write!(f, "Extension number cannot be empty"),
            Self::InvalidLength { length } => {
                write!(
                    f,
                    "Extension length {} is invalid (must be 2-10 digits)",
                    length
                )
            }
            Self::NonNumeric => write!(f, "Extension must contain only digits"),
            Self::ReservedPrefix(prefix) => {
                write!(f, "Extension uses reserved prefix: {}", prefix)
            }
        }
    }
}

impl std::error::Error for ValidationError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_extensions() {
        // Valid extensions
        assert!(Extension::validate_extension_number("100").is_ok());
        assert!(Extension::validate_extension_number("1234").is_ok());
        assert!(Extension::validate_extension_number("5000").is_ok());
        assert!(Extension::validate_extension_number("9999").is_ok());
        assert!(Extension::validate_extension_number("123456").is_ok());
    }

    #[test]
    fn test_invalid_extensions() {
        // Too short
        assert!(Extension::validate_extension_number("1").is_err());

        // Too long
        assert!(Extension::validate_extension_number("12345678901").is_err());

        // Non-numeric
        assert!(Extension::validate_extension_number("123A").is_err());
        assert!(Extension::validate_extension_number("12-34").is_err());

        // Reserved prefixes
        assert!(Extension::validate_extension_number("0").is_err()); // Operator
        assert!(Extension::validate_extension_number("911").is_err()); // Emergency
        assert!(Extension::validate_extension_number("411").is_err()); // Information
    }

    #[test]
    fn test_extension_creation() {
        let tenant_id = TenantId::new();
        let user_id = UserId::new();

        let extension = Extension::new(
            "1234".to_string(),
            ExtensionTarget::User(user_id),
            tenant_id,
        )
        .unwrap();

        assert_eq!(extension.number, "1234");
        assert_eq!(extension.target, ExtensionTarget::User(user_id));
        assert_eq!(extension.tenant_id, tenant_id);
        assert!(extension.enabled);
    }

    #[test]
    fn test_extension_description() {
        let tenant_id = TenantId::new();
        let user_id = UserId::new();

        let mut extension = Extension::new(
            "1234".to_string(),
            ExtensionTarget::User(user_id),
            tenant_id,
        )
        .unwrap();

        assert!(extension.description.is_none());

        extension.set_description("Alice's extension".to_string());
        assert_eq!(extension.description, Some("Alice's extension".to_string()));
    }

    #[test]
    fn test_extension_enable_disable() {
        let tenant_id = TenantId::new();
        let user_id = UserId::new();

        let mut extension = Extension::new(
            "1234".to_string(),
            ExtensionTarget::User(user_id),
            tenant_id,
        )
        .unwrap();

        assert!(extension.enabled);

        extension.set_enabled(false);
        assert!(!extension.enabled);

        extension.set_enabled(true);
        assert!(extension.enabled);
    }

    #[test]
    fn test_extension_target_display() {
        let user_id = UserId::new();
        let phone_id = PhoneNumberId::new();
        let device_id = DeviceId::new();
        let service_id = ServiceId::new();

        assert!(ExtensionTarget::User(user_id)
            .to_string()
            .starts_with("user:"));
        assert!(ExtensionTarget::PhoneNumber(phone_id)
            .to_string()
            .starts_with("phone_number:"));
        assert!(ExtensionTarget::Device(device_id)
            .to_string()
            .starts_with("device:"));
        assert!(ExtensionTarget::Service(service_id)
            .to_string()
            .starts_with("service:"));
    }

    #[test]
    fn test_reserved_extensions() {
        // Emergency services
        assert!(matches!(
            Extension::validate_extension_number("911"),
            Err(ValidationError::ReservedPrefix(_))
        ));

        // Service codes
        assert!(matches!(
            Extension::validate_extension_number("411"),
            Err(ValidationError::ReservedPrefix(_))
        ));
        assert!(matches!(
            Extension::validate_extension_number("611"),
            Err(ValidationError::ReservedPrefix(_))
        ));
        assert!(matches!(
            Extension::validate_extension_number("711"),
            Err(ValidationError::ReservedPrefix(_))
        ));
        assert!(matches!(
            Extension::validate_extension_number("811"),
            Err(ValidationError::ReservedPrefix(_))
        ));
    }
}
