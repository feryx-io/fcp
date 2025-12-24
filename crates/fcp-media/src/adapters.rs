//! Type adapters between FCP and forge-media
//!
//! Maps FCP identifiers and types to forge equivalents

use fcp_core as fcp;
use forge_core as forge;

/// Convert FCP CallId to forge CallId
pub fn fcp_to_forge_call_id(fcp_id: &fcp::CallId) -> forge::CallId {
    // Use the full UUID string, not the display format (which is truncated)
    forge::CallId::new(fcp_id.as_uuid().to_string())
}

/// Convert forge CallId to FCP CallId
pub fn forge_to_fcp_call_id(forge_id: &forge::CallId) -> fcp::CallId {
    // Parse the forge CallId string as FCP CallId
    // forge CallId is just a String, so parse it
    std::str::FromStr::from_str(&forge_id.0).unwrap_or_else(|_| {
        // If parsing fails (shouldn't happen), create new ID
        fcp::CallId::new()
    })
}

/// Generate forge ParticipantId for a call leg
pub fn generate_participant_id(leg: &str) -> forge::ParticipantId {
    forge::ParticipantId::new(format!("{}-{}", leg, uuid::Uuid::new_v4()))
}

/// Create participant IDs for both legs of a call
pub fn create_participant_ids(call_id: &fcp::CallId) -> (forge::ParticipantId, forge::ParticipantId) {
    let call_id_str = call_id.to_string();
    (
        forge::ParticipantId::new(format!("{}-leg-a", call_id_str)),
        forge::ParticipantId::new(format!("{}-leg-b", call_id_str)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_call_id_conversion() {
        let fcp_id = fcp::CallId::new();
        let forge_id = fcp_to_forge_call_id(&fcp_id);
        let back_to_fcp = forge_to_fcp_call_id(&forge_id);

        // Compare the actual UUIDs, not the display strings (which are truncated)
        assert_eq!(fcp_id.as_uuid(), back_to_fcp.as_uuid());
    }

    #[test]
    fn test_participant_id_generation() {
        let call_id = fcp::CallId::new();
        let (part_a, part_b) = create_participant_ids(&call_id);

        assert!(part_a.0.contains("leg-a"));
        assert!(part_b.0.contains("leg-b"));
        assert_ne!(part_a, part_b);
    }
}
