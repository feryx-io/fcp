//! Message handlers for processing SIP requests and responses.
//!
//! NOTE: This module contains placeholder implementations.
//! The actual siphon-rs API structure needs to be studied more carefully
//! before implementing complete request/response handling.

use anyhow::Result;
use tracing::{debug, info};

/// Handle an incoming SIP request (placeholder).
///
/// TODO: Implement actual request handling once siphon-rs API is better understood.
/// The Request/Response structures in siphon-rs may have different field names
/// than initially assumed.
pub async fn handle_request(_request_data: &[u8]) -> Result<Vec<u8>> {
    info!("Received SIP request (handler not yet implemented)");

    // Return a placeholder response
    Ok(Vec::new())
}

/// Handle OPTIONS request (placeholder).
pub async fn handle_options() -> Result<()> {
    debug!("Handling OPTIONS request (placeholder)");
    Ok(())
}

/// Handle REGISTER request (placeholder).
pub async fn handle_register() -> Result<()> {
    debug!("Handling REGISTER request (placeholder)");
    Ok(())
}

/// Handle INVITE request (placeholder).
pub async fn handle_invite() -> Result<()> {
    debug!("Handling INVITE request (placeholder)");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_handle_request_placeholder() {
        let request_data = b"OPTIONS sip:test@example.com SIP/2.0\r\n\r\n";
        let result = handle_request(request_data).await;

        assert!(result.is_ok());
    }
}
