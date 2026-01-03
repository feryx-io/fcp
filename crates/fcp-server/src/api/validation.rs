//! Common input validation helpers for API endpoints
//!
//! This module provides reusable validation functions to ensure API inputs
//! meet security and correctness requirements.

use crate::api::error::ApiError;

/// Maximum allowed username length
const MAX_USERNAME_LENGTH: usize = 255;

/// Rejects strings containing CR or LF characters to prevent injection attacks.
///
/// # Security
/// CRLF injection can allow attackers to:
/// - Inject headers in HTTP responses
/// - Manipulate log files
/// - Break protocol parsing
///
/// # Arguments
/// * `field` - Name of the field being validated (for error messages)
/// * `value` - String value to validate
///
/// # Errors
/// Returns `ApiError::BadRequest` if the value contains CR or LF characters.
pub fn reject_crlf(field: &str, value: &str) -> Result<(), ApiError> {
    if value.bytes().any(|b| matches!(b, b'\r' | b'\n')) {
        Err(ApiError::BadRequest(format!(
            "{} must not contain CR or LF characters",
            field
        )))
    } else {
        Ok(())
    }
}

/// Validates username format and content.
///
/// # Validation Rules
/// - Must not be empty
/// - Must not exceed 255 characters
/// - Must not contain CR or LF characters
/// - Must only contain alphanumeric characters, @, ., -, _
///
/// # Arguments
/// * `username` - Username string to validate
///
/// # Errors
/// Returns `ApiError::BadRequest` if validation fails.
pub fn validate_username(username: &str) -> Result<(), ApiError> {
    if username.is_empty() {
        return Err(ApiError::BadRequest(
            "username must not be empty".to_string(),
        ));
    }

    if username.len() > MAX_USERNAME_LENGTH {
        return Err(ApiError::BadRequest(format!(
            "username must not exceed {} characters",
            MAX_USERNAME_LENGTH
        )));
    }

    reject_crlf("username", username)?;

    // Validate username format (alphanumeric + common SIP URI characters)
    if !username
        .chars()
        .all(|c| c.is_alphanumeric() || matches!(c, '@' | '.' | '-' | '_'))
    {
        return Err(ApiError::BadRequest(
            "username contains invalid characters (allowed: alphanumeric, @, ., -, _)".to_string(),
        ));
    }

    Ok(())
}

/// Validates display name content.
///
/// # Validation Rules
/// - Must not contain CR or LF characters if present
///
/// # Arguments
/// * `name` - Optional display name to validate
///
/// # Errors
/// Returns `ApiError::BadRequest` if validation fails.
pub fn validate_display_name(name: &Option<String>) -> Result<(), ApiError> {
    if let Some(name) = name {
        reject_crlf("display_name", name)?;
    }
    Ok(())
}

/// Validates SIP Address of Record (AoR) format.
///
/// # Validation Rules
/// - Must not be empty
/// - Must not contain CR or LF characters
///
/// # Arguments
/// * `aor` - AoR string to validate (e.g., "alice@example.com" or "sip:alice@example.com")
///
/// # Errors
/// Returns `ApiError::BadRequest` if validation fails.
pub fn validate_aor(aor: &str) -> Result<(), ApiError> {
    if aor.is_empty() {
        return Err(ApiError::BadRequest("aor must not be empty".to_string()));
    }

    reject_crlf("aor", aor)?;

    Ok(())
}

/// Validates SIP contact URI.
///
/// # Validation Rules
/// - Must not be empty
/// - Must not contain CR or LF characters
///
/// # Arguments
/// * `contact` - Contact URI to validate
///
/// # Errors
/// Returns `ApiError::BadRequest` if validation fails.
pub fn validate_contact(contact: &str) -> Result<(), ApiError> {
    if contact.is_empty() {
        return Err(ApiError::BadRequest(
            "contact must not be empty".to_string(),
        ));
    }

    reject_crlf("contact", contact)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reject_crlf_valid() {
        assert!(reject_crlf("field", "valid-value").is_ok());
        assert!(reject_crlf("field", "alice@example.com").is_ok());
    }

    #[test]
    fn test_reject_crlf_with_cr() {
        assert!(reject_crlf("field", "invalid\rvalue").is_err());
    }

    #[test]
    fn test_reject_crlf_with_lf() {
        assert!(reject_crlf("field", "invalid\nvalue").is_err());
    }

    #[test]
    fn test_reject_crlf_with_crlf() {
        assert!(reject_crlf("field", "invalid\r\nvalue").is_err());
    }

    #[test]
    fn test_validate_username_valid() {
        assert!(validate_username("alice").is_ok());
        assert!(validate_username("alice@example.com").is_ok());
        assert!(validate_username("user-name_123").is_ok());
        assert!(validate_username("user.name@domain.com").is_ok());
    }

    #[test]
    fn test_validate_username_empty() {
        assert!(validate_username("").is_err());
    }

    #[test]
    fn test_validate_username_too_long() {
        let long_name = "a".repeat(256);
        assert!(validate_username(&long_name).is_err());
    }

    #[test]
    fn test_validate_username_with_crlf() {
        assert!(validate_username("user\r\n").is_err());
        assert!(validate_username("user\n").is_err());
    }

    #[test]
    fn test_validate_username_invalid_chars() {
        assert!(validate_username("user name").is_err()); // space
        assert!(validate_username("user!name").is_err()); // !
        assert!(validate_username("user#name").is_err()); // #
        assert!(validate_username("user$name").is_err()); // $
    }

    #[test]
    fn test_validate_display_name_none() {
        assert!(validate_display_name(&None).is_ok());
    }

    #[test]
    fn test_validate_display_name_valid() {
        assert!(validate_display_name(&Some("Alice Johnson".to_string())).is_ok());
    }

    #[test]
    fn test_validate_display_name_with_crlf() {
        assert!(validate_display_name(&Some("Alice\nJohnson".to_string())).is_err());
    }

    #[test]
    fn test_validate_aor_valid() {
        assert!(validate_aor("alice@example.com").is_ok());
        assert!(validate_aor("sip:alice@example.com").is_ok());
    }

    #[test]
    fn test_validate_aor_empty() {
        assert!(validate_aor("").is_err());
    }

    #[test]
    fn test_validate_aor_with_crlf() {
        assert!(validate_aor("alice@example.com\r\n").is_err());
    }

    #[test]
    fn test_validate_contact_valid() {
        assert!(validate_contact("sip:alice@192.168.1.100:5060").is_ok());
    }

    #[test]
    fn test_validate_contact_empty() {
        assert!(validate_contact("").is_err());
    }

    #[test]
    fn test_validate_contact_with_crlf() {
        assert!(validate_contact("sip:alice@host\n").is_err());
    }
}
