//! What a SIP message body holds (`docs/E911_BLOCK_2.md` §4).
//!
//! A body is not always SDP. A device that sends its own location puts the
//! SDP and a PIDF-LO in one `multipart/mixed` body (RFC 6442), and a
//! recording server gets SDP with its metadata the same way. Everything that
//! reads a request's SDP reads it through [`sdp_of`], which finds the SDP
//! part of a multipart body, and the other parts are there to be read by
//! type.

use sip_core::Headers;

/// One part of a multipart body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BodyPart {
    /// The part's `Content-Type`, without parameters, lower-cased;
    /// `text/plain` when the part names none (RFC 2046).
    pub content_type: String,
    /// The part's `Content-ID`, without its angle brackets.
    pub content_id: Option<String>,
    pub body: Vec<u8>,
}

/// A `Content-Type` value's media type, lower-cased, without parameters.
fn media_type(value: &str) -> String {
    value
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
}

/// The parts of a `multipart/*` body; `None` when the content type is not
/// multipart or names no usable boundary, or the body has no parts.
/// Parsed by `sip_core::MultipartBody` (RFC 2046, RFC 5621).
pub fn parse_multipart(content_type: &str, body: &[u8]) -> Option<Vec<BodyPart>> {
    let parsed = sip_core::MultipartBody::parse(content_type, body).ok()?;
    Some(
        parsed
            .parts()
            .iter()
            .map(|p| BodyPart {
                content_type: p.media_type(),
                content_id: p.content_id().map(str::to_string),
                body: p.body().to_vec(),
            })
            .collect(),
    )
}

/// The message's `Content-Type`, if any.
fn content_type_of(headers: &Headers) -> Option<String> {
    sip_parse::header(headers, "Content-Type")
        .or_else(|| sip_parse::header(headers, "c"))
        .map(|v| v.to_string())
}

/// The parts of a message's body: its parts when multipart, else the body
/// as one part of its content type (`application/sdp` when it names none,
/// as SIP bodies without a type have been SDP here all along).
pub fn parts_of(headers: &Headers, body: &[u8]) -> Vec<BodyPart> {
    if body.is_empty() {
        return Vec::new();
    }
    let content_type = content_type_of(headers);
    if let Some(parts) = content_type
        .as_deref()
        .and_then(|ct| parse_multipart(ct, body))
    {
        return parts;
    }
    vec![BodyPart {
        content_type: content_type
            .map(|ct| media_type(&ct))
            .unwrap_or_else(|| "application/sdp".to_string()),
        content_id: None,
        body: body.to_vec(),
    }]
}

/// The SDP a message carries, alone or as a part of a multipart body.
pub fn sdp_of(headers: &Headers, body: &[u8]) -> Option<String> {
    parts_of(headers, body)
        .into_iter()
        .find(|p| p.content_type == "application/sdp")
        .map(|p| String::from_utf8_lossy(&p.body).into_owned())
}

/// The SDP of a request, as [`sdp_of`].
pub fn request_sdp(request: &sip_core::Request) -> Option<String> {
    sdp_of(request.headers(), request.body())
}

/// The parts of a message that are not its SDP (a PIDF-LO, recording
/// metadata), for whoever reads them.
pub fn non_sdp_parts(headers: &Headers, body: &[u8]) -> Vec<BodyPart> {
    parts_of(headers, body)
        .into_iter()
        .filter(|p| p.content_type != "application/sdp")
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use smol_str::SmolStr;

    const SDP: &str = "v=0\r\no=- 1 1 IN IP4 10.0.0.5\r\ns=-\r\nc=IN IP4 10.0.0.5\r\nt=0 0\r\nm=audio 4000 RTP/AVP 0\r\n";

    fn headers(content_type: Option<&str>) -> Headers {
        let mut h = Headers::new();
        if let Some(ct) = content_type {
            h.push(SmolStr::new("Content-Type"), SmolStr::new(ct))
                .unwrap();
        }
        h
    }

    fn located_body() -> String {
        format!(
            "--boundary1\r\nContent-Type: application/sdp\r\n\r\n{SDP}\r\n\
             --boundary1\r\nContent-Type: application/pidf+xml\r\nContent-ID: <loc@pbx.example>\r\n\r\n\
             <presence entity=\"pres:alice@pbx.example\"/>\r\n\
             --boundary1--\r\n"
        )
    }

    #[test]
    fn plain_sdp_is_the_sdp() {
        assert_eq!(
            sdp_of(&headers(Some("application/sdp")), SDP.as_bytes()).as_deref(),
            Some(SDP)
        );
        assert_eq!(
            sdp_of(&headers(None), SDP.as_bytes()).as_deref(),
            Some(SDP),
            "an untyped body is SDP, as it always was"
        );
        assert_eq!(sdp_of(&headers(Some("application/sdp")), b""), None);
        assert_eq!(
            sdp_of(&headers(Some("application/pidf+xml")), b"<presence/>"),
            None
        );
    }

    #[test]
    fn a_located_invite_gives_its_sdp_and_its_location() {
        let h = headers(Some("multipart/mixed; boundary=\"boundary1\""));
        let body = located_body();
        assert_eq!(sdp_of(&h, body.as_bytes()).as_deref(), Some(SDP));
        let others = non_sdp_parts(&h, body.as_bytes());
        assert_eq!(others.len(), 1);
        assert_eq!(others[0].content_type, "application/pidf+xml");
        assert_eq!(others[0].content_id.as_deref(), Some("loc@pbx.example"));
        assert_eq!(
            others[0].body,
            b"<presence entity=\"pres:alice@pbx.example\"/>"
        );
    }

    #[test]
    fn malformed_multipart_bodies_give_what_they_can() {
        // No boundary parameter: not parsed as multipart.
        assert_eq!(
            parse_multipart("multipart/mixed", b"--x\r\n\r\nhi\r\n--x--"),
            None
        );
        // No close delimiter: the parts before the end are still read.
        let body = format!("--b\r\nContent-Type: application/sdp\r\n\r\n{SDP}\r\n--b\r\nContent-Type: text/plain\r\n\r\ncut off");
        let parts = parse_multipart("multipart/mixed;boundary=b", body.as_bytes()).unwrap();
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0].content_type, "application/sdp");
        // A preamble and LF-only line ends.
        let lf = "preamble\n--b\nContent-Type: application/sdp\n\nv=0\n--b--\n";
        let parts = parse_multipart("multipart/mixed; boundary=b", lf.as_bytes()).unwrap();
        assert_eq!(parts[0].body, b"v=0");
        // A part without headers is text/plain.
        let bare = "--b\r\n\r\nhello\r\n--b--";
        let parts = parse_multipart("multipart/mixed; boundary=b", bare.as_bytes()).unwrap();
        assert_eq!(parts[0].content_type, "text/plain");
        assert_eq!(parts[0].body, b"hello");
        assert_eq!(parse_multipart("application/sdp", SDP.as_bytes()), None);
    }
}
