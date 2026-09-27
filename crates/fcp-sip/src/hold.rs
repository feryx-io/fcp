//! Hold, as the SDP says it (`docs/ENTERPRISE_CALLING.md` §3.2): whether an
//! offer puts its far party on hold, and an SDP with its direction set.

/// Whether this SDP puts the far party on hold: its media direction is
/// `sendonly` or `inactive`, or its connection is the RFC 2543 `0.0.0.0`.
/// A direction on the first audio section wins over one at session level.
pub fn holds(sdp: &str) -> bool {
    matches!(direction(sdp), Some("sendonly") | Some("inactive"))
        || sdp
            .lines()
            .any(|l| l.trim_end().eq_ignore_ascii_case("c=IN IP4 0.0.0.0"))
}

/// The direction attribute that applies to the first audio section.
pub fn direction(sdp: &str) -> Option<&'static str> {
    let mut session_level = None;
    let mut in_audio = false;
    let mut seen_media = false;
    for line in sdp.lines().map(str::trim_end) {
        if let Some(media) = line.strip_prefix("m=") {
            if in_audio {
                break;
            }
            seen_media = true;
            in_audio = media.starts_with("audio");
            continue;
        }
        let dir = match line {
            "a=sendrecv" => "sendrecv",
            "a=sendonly" => "sendonly",
            "a=recvonly" => "recvonly",
            "a=inactive" => "inactive",
            _ => continue,
        };
        if in_audio {
            return Some(dir);
        }
        if !seen_media {
            session_level = Some(dir);
        }
    }
    session_level
}

/// The SDP with every direction attribute replaced by `dir`, and one added
/// to each media section that had none.
pub fn with_direction(sdp: &str, dir: &str) -> String {
    let newline = if sdp.contains("\r\n") { "\r\n" } else { "\n" };
    let is_direction =
        |l: &str| matches!(l, "a=sendrecv" | "a=sendonly" | "a=recvonly" | "a=inactive");
    let mut out: Vec<String> = Vec::new();
    let mut section_has_direction = true;
    for line in sdp.lines().map(str::trim_end) {
        if line.starts_with("m=") {
            if !section_has_direction {
                out.push(format!("a={dir}"));
            }
            section_has_direction = false;
            out.push(line.to_string());
            continue;
        }
        if is_direction(line) {
            if !out.iter().any(|l| l.starts_with("m=")) {
                // A session-level direction goes; the sections carry it.
                continue;
            }
            section_has_direction = true;
            out.push(format!("a={dir}"));
            continue;
        }
        if !line.is_empty() {
            out.push(line.to_string());
        }
    }
    if !section_has_direction {
        out.push(format!("a={dir}"));
    }
    let mut joined = out.join(newline);
    joined.push_str(newline);
    joined
}

#[cfg(test)]
mod tests {
    use super::*;

    const SDP: &str = "v=0\r\no=- 1 1 IN IP4 10.0.0.1\r\ns=-\r\nc=IN IP4 10.0.0.1\r\nt=0 0\r\nm=audio 4000 RTP/AVP 0\r\na=rtpmap:0 PCMU/8000\r\n";

    #[test]
    fn a_plain_offer_does_not_hold() {
        assert!(!holds(SDP));
        assert_eq!(direction(SDP), None);
    }

    #[test]
    fn sendonly_inactive_and_zero_address_hold() {
        assert!(holds(&format!("{SDP}a=sendonly\r\n")));
        assert!(holds(&format!("{SDP}a=inactive\r\n")));
        assert!(!holds(&format!("{SDP}a=recvonly\r\n")));
        assert!(!holds(&format!("{SDP}a=sendrecv\r\n")));
        assert!(holds(&SDP.replace("c=IN IP4 10.0.0.1", "c=IN IP4 0.0.0.0")));
    }

    #[test]
    fn the_audio_section_wins_over_the_session() {
        let sdp = SDP.replace("t=0 0\r\n", "t=0 0\r\na=sendonly\r\n");
        assert!(holds(&sdp));
        assert!(!holds(&format!("{sdp}a=sendrecv\r\n")));
    }

    #[test]
    fn a_direction_is_set_everywhere() {
        let held = format!("{SDP}a=sendonly\r\n");
        let resumed = with_direction(&held, "sendrecv");
        assert_eq!(direction(&resumed), Some("sendrecv"));
        assert!(!resumed.contains("sendonly"));
        let added = with_direction(SDP, "sendrecv");
        assert_eq!(direction(&added), Some("sendrecv"));
        assert!(added.ends_with("a=sendrecv\r\n"));
        let session = SDP.replace("t=0 0\r\n", "t=0 0\r\na=inactive\r\n");
        let fixed = with_direction(&session, "sendrecv");
        assert_eq!(fixed.matches("a=").filter(|_| true).count(), 2, "{fixed}");
        assert_eq!(direction(&fixed), Some("sendrecv"));
    }
}
