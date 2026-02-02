//! Codec utilities

use std::collections::HashMap;

/// Codec priority for negotiation
pub const CODEC_PRIORITY: &[&str] = &[
    "opus", // Highest quality, best for VoIP
    "G722", // Wideband, good quality
    "PCMU", // G.711 µ-law, universal compatibility
    "PCMA", // G.711 A-law, universal compatibility
    "G729", // Low bandwidth
];

/// Check if codec requires transcoding
pub fn requires_transcoding(codec_a: &str, codec_b: &str) -> bool {
    !codec_a.eq_ignore_ascii_case(codec_b)
}

/// Get codec name in canonical form
pub fn canonical_codec_name(codec: &str) -> String {
    match codec.to_uppercase().as_str() {
        "PCMU" | "G711U" | "ULAW" => "PCMU".to_string(),
        "PCMA" | "G711A" | "ALAW" => "PCMA".to_string(),
        "G722" => "G722".to_string(),
        "G729" | "G729A" => "G729".to_string(),
        "OPUS" => "opus".to_string(),
        other => other.to_string(),
    }
}

/// Select best codec from available list based on priority
pub fn select_best_codec(available: &[String]) -> Option<String> {
    for preferred in CODEC_PRIORITY {
        if available.iter().any(|c| c.eq_ignore_ascii_case(preferred)) {
            return Some((*preferred).to_string());
        }
    }

    available.first().cloned()
}

/// Registry of codecs FCP+forge-engine can handle, with standard payload types and clock rates.
///
/// Returns a map from canonical codec name to `(payload_type, clock_rate)`.
pub fn supported_codec_registry() -> HashMap<String, (u8, u32)> {
    let mut m = HashMap::new();
    m.insert("PCMU".into(), (0, 8000));
    m.insert("PCMA".into(), (8, 8000));
    m.insert("G722".into(), (9, 16000));
    m.insert("opus".into(), (111, 48000));
    m
}

/// Find a common codec between two codec lists
/// Returns the first common codec based on priority
pub fn find_common_codec<'a>(a_leg: &'a [String], b_leg: &'a [String]) -> Option<&'a String> {
    // Normalize codec names for comparison
    for a_codec in a_leg {
        let a_canonical = canonical_codec_name(a_codec);
        for b_codec in b_leg {
            let b_canonical = canonical_codec_name(b_codec);
            if a_canonical == b_canonical {
                return Some(a_codec);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_requires_transcoding() {
        assert!(!requires_transcoding("PCMU", "PCMU"));
        assert!(!requires_transcoding("pcmu", "PCMU"));
        assert!(requires_transcoding("PCMU", "opus"));
    }

    #[test]
    fn test_canonical_codec_name() {
        assert_eq!(canonical_codec_name("pcmu"), "PCMU");
        assert_eq!(canonical_codec_name("g711u"), "PCMU");
        assert_eq!(canonical_codec_name("ulaw"), "PCMU");
        assert_eq!(canonical_codec_name("opus"), "opus");
    }

    #[test]
    fn test_select_best_codec() {
        let codecs = vec!["PCMU".to_string(), "PCMA".to_string(), "opus".to_string()];

        let best = select_best_codec(&codecs);
        assert_eq!(best, Some("opus".to_string())); // opus has highest priority

        let limited = vec!["PCMU".to_string(), "PCMA".to_string()];
        let best = select_best_codec(&limited);
        assert_eq!(best, Some("PCMU".to_string()));
    }
}
