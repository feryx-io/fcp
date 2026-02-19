//! Multipart MIME builder for RFC 4662 resource list NOTIFY bodies.
//!
//! Builds `multipart/related` bodies containing an RLMI XML root document
//! and individual PIDF XML body parts for each resource list entry.

/// A single part in a multipart MIME body.
pub struct MimePart {
    /// Content-Type header (e.g., "application/pidf+xml").
    pub content_type: String,
    /// Content-ID header (e.g., "<alice@example.com>").
    pub content_id: String,
    /// The body of this part.
    pub body: String,
}

/// Builder for multipart/related MIME bodies.
pub struct MultipartBuilder {
    /// MIME boundary string.
    boundary: String,
    /// The parts in order.
    parts: Vec<MimePart>,
}

impl MultipartBuilder {
    /// Create a new multipart builder with a random boundary.
    pub fn new() -> Self {
        let boundary = format!("----=_Part_{}", uuid::Uuid::new_v4().simple());
        Self {
            boundary,
            parts: Vec::new(),
        }
    }

    /// Create with a specific boundary (useful for testing).
    pub fn with_boundary(boundary: &str) -> Self {
        Self {
            boundary: boundary.to_string(),
            parts: Vec::new(),
        }
    }

    /// Get the boundary string.
    pub fn boundary(&self) -> &str {
        &self.boundary
    }

    /// Get the Content-Type header value for the overall multipart body.
    ///
    /// Returns: `multipart/related;type="application/rlmi+xml";boundary=...`
    pub fn content_type(&self) -> String {
        format!(
            "multipart/related;type=\"application/rlmi+xml\";boundary={}",
            self.boundary
        )
    }

    /// Add the RLMI root part (must be first).
    pub fn add_rlmi(&mut self, rlmi_xml: String, content_id: &str) {
        self.parts.push(MimePart {
            content_type: "application/rlmi+xml".to_string(),
            content_id: format!("<{content_id}>"),
            body: rlmi_xml,
        });
    }

    /// Add a PIDF body part for a resource list entry.
    pub fn add_pidf(&mut self, pidf_xml: String, content_id: &str) {
        self.parts.push(MimePart {
            content_type: "application/pidf+xml".to_string(),
            content_id: format!("<{content_id}>"),
            body: pidf_xml,
        });
    }

    /// Build the complete multipart body.
    pub fn build(&self) -> String {
        let mut body = String::with_capacity(2048);

        for part in &self.parts {
            body.push_str("--");
            body.push_str(&self.boundary);
            body.push_str("\r\n");
            body.push_str(&format!("Content-Type: {}\r\n", part.content_type));
            body.push_str(&format!("Content-ID: {}\r\n", part.content_id));
            body.push_str("\r\n");
            body.push_str(&part.body);
            body.push_str("\r\n");
        }

        // Closing boundary
        body.push_str("--");
        body.push_str(&self.boundary);
        body.push_str("--\r\n");

        body
    }
}

impl Default for MultipartBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_multipart_build() {
        let mut builder = MultipartBuilder::with_boundary("test-boundary");

        builder.add_rlmi(
            "<list xmlns=\"urn:ietf:params:xml:ns:rlmi\" uri=\"sip:team@example.com\"/>".to_string(),
            "list@example.com",
        );

        builder.add_pidf(
            "<presence xmlns=\"urn:ietf:params:xml:ns:pidf\" entity=\"pres:alice@example.com\"/>".to_string(),
            "alice@example.com",
        );

        let body = builder.build();

        // Check boundary delimiters
        assert!(body.contains("--test-boundary\r\n"));
        assert!(body.contains("--test-boundary--\r\n"));

        // Check RLMI part
        assert!(body.contains("Content-Type: application/rlmi+xml\r\n"));
        assert!(body.contains("Content-ID: <list@example.com>\r\n"));
        assert!(body.contains("urn:ietf:params:xml:ns:rlmi"));

        // Check PIDF part
        assert!(body.contains("Content-Type: application/pidf+xml\r\n"));
        assert!(body.contains("Content-ID: <alice@example.com>\r\n"));
        assert!(body.contains("urn:ietf:params:xml:ns:pidf"));
    }

    #[test]
    fn test_content_type() {
        let builder = MultipartBuilder::with_boundary("abc123");
        assert_eq!(
            builder.content_type(),
            "multipart/related;type=\"application/rlmi+xml\";boundary=abc123"
        );
    }

    #[test]
    fn test_random_boundary() {
        let b1 = MultipartBuilder::new();
        let b2 = MultipartBuilder::new();
        assert_ne!(b1.boundary(), b2.boundary());
    }
}
