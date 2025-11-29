//! Type adapters and conversions between fcp-core and siphon-rs.
//!
//! This module provides conversion traits and adapter types that bridge
//! the fcp-core abstractions with siphon-rs concrete implementations.

use fcp_core::{Error as FcpError, TransportType as FcpTransportType};
use sip_transport::TransportKind as SipTransportKind;

/// Convert siphon-rs transport kind to fcp-core transport type.
pub fn to_fcp_transport_type(sip_transport: SipTransportKind) -> FcpTransportType {
    match sip_transport {
        SipTransportKind::Udp => FcpTransportType::Udp,
        SipTransportKind::Tcp => FcpTransportType::Tcp,
        SipTransportKind::Tls => FcpTransportType::Tls,
        SipTransportKind::Ws => FcpTransportType::Ws,
        SipTransportKind::Wss => FcpTransportType::Wss,
        SipTransportKind::Sctp | SipTransportKind::TlsSctp => {
            // SCTP not supported in fcp-core, map to TCP
            FcpTransportType::Tcp
        }
    }
}

/// Convert fcp-core transport type to siphon-rs transport kind.
pub fn to_sip_transport_kind(fcp_transport: FcpTransportType) -> SipTransportKind {
    match fcp_transport {
        FcpTransportType::Udp => SipTransportKind::Udp,
        FcpTransportType::Tcp => SipTransportKind::Tcp,
        FcpTransportType::Tls => SipTransportKind::Tls,
        FcpTransportType::Ws => SipTransportKind::Ws,
        FcpTransportType::Wss => SipTransportKind::Wss,
    }
}

/// Convert siphon-rs errors to fcp-core errors.
///
/// This provides a bridge for error handling across the boundary.
pub fn convert_sip_error(err: anyhow::Error) -> FcpError {
    // Wrap as internal error
    // TODO: Add more specific error type conversions as needed
    FcpError::Internal(err.to_string())
}

/// Adapter for SIP Call-ID to FCP CallId mapping.
///
/// In FCP, we use UUIDs for internal call tracking, while SIP uses
/// string-based Call-ID headers. This adapter provides the mapping.
pub struct CallIdAdapter;

impl CallIdAdapter {
    /// Generate a new SIP Call-ID from an FCP CallId.
    pub fn to_sip_call_id(call_id: &fcp_core::CallId) -> fcp_core::SipCallId {
        // Use the UUID as the Call-ID value
        fcp_core::SipCallId::from_string(format!("{}@fcp.local", call_id.as_uuid()))
    }

    /// Extract or generate an FCP CallId from a SIP Call-ID.
    ///
    /// If the Call-ID follows our format (UUID@fcp.local), we parse it.
    /// Otherwise, we generate a new CallId.
    pub fn from_sip_call_id(sip_call_id: &str) -> fcp_core::CallId {
        // Try to parse UUID from the Call-ID
        if let Some(uuid_part) = sip_call_id.split('@').next() {
            if let Ok(uuid) = uuid::Uuid::parse_str(uuid_part) {
                return fcp_core::CallId::from_uuid(uuid);
            }
        }

        // If parsing fails, generate a new CallId
        fcp_core::CallId::new()
    }
}

/// Adapter for converting between fcp-core configuration and siphon-rs configuration.
pub struct ConfigAdapter;

impl ConfigAdapter {
    /// Convert FCP transport config to siphon-rs UDP transport bind address.
    pub fn udp_bind_addr(config: &fcp_core::TransportConfig) -> Option<std::net::SocketAddr> {
        config.udp_bind
    }

    /// Convert FCP transport config to siphon-rs TCP transport bind address.
    pub fn tcp_bind_addr(config: &fcp_core::TransportConfig) -> Option<std::net::SocketAddr> {
        config.tcp_bind
    }

    /// Convert FCP transport config to siphon-rs TLS transport configuration.
    pub fn tls_config(
        config: &fcp_core::TransportConfig,
    ) -> Option<(std::net::SocketAddr, std::path::PathBuf, std::path::PathBuf)> {
        match (config.tls_bind, &config.tls_cert, &config.tls_key) {
            (Some(bind), Some(cert), Some(key)) => Some((bind, cert.clone(), key.clone())),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_transport_type_conversion() {
        assert_eq!(
            to_fcp_transport_type(SipTransportKind::Udp),
            FcpTransportType::Udp
        );
        assert_eq!(
            to_fcp_transport_type(SipTransportKind::Tcp),
            FcpTransportType::Tcp
        );
        assert_eq!(
            to_fcp_transport_type(SipTransportKind::Tls),
            FcpTransportType::Tls
        );

        assert_eq!(
            to_sip_transport_kind(FcpTransportType::Udp),
            SipTransportKind::Udp
        );
        assert_eq!(
            to_sip_transport_kind(FcpTransportType::Tcp),
            SipTransportKind::Tcp
        );
    }

    #[test]
    fn test_call_id_adapter() {
        let fcp_call_id = fcp_core::CallId::new();
        let sip_call_id = CallIdAdapter::to_sip_call_id(&fcp_call_id);

        // Should contain the UUID
        assert!(sip_call_id.as_str().contains(&fcp_call_id.as_uuid().to_string()));

        // Round trip should preserve or generate valid CallId
        let recovered = CallIdAdapter::from_sip_call_id(sip_call_id.as_str());
        assert_eq!(recovered.as_uuid(), fcp_call_id.as_uuid());
    }

    #[test]
    fn test_call_id_adapter_non_fcp_format() {
        // Test with a non-FCP format Call-ID
        let recovered = CallIdAdapter::from_sip_call_id("random-call-id@example.com");

        // Should generate a new valid CallId
        assert!(!recovered.is_nil());
    }
}
