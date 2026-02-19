//! RFC 4662 RLMI (Resource List Meta-Information) XML builder.
//!
//! Generates `application/rlmi+xml` documents for resource list NOTIFY bodies.

/// An individual resource instance within an RLMI document.
pub struct RlmiInstance {
    /// Unique instance ID.
    pub id: String,
    /// State: "active", "pending", or "terminated".
    pub state: String,
    /// Content-ID reference to the corresponding PIDF body part.
    pub cid: String,
}

/// A resource entry within an RLMI document.
pub struct RlmiResource {
    /// The presentity URI.
    pub uri: String,
    /// Instances for this resource.
    pub instances: Vec<RlmiInstance>,
}

/// Builder for RLMI XML documents (RFC 4662 §4).
pub struct RlmiBuilder {
    /// The resource list URI.
    list_uri: String,
    /// Version number (monotonically increasing per subscription).
    version: u32,
    /// Whether this is a full-state notification.
    full_state: bool,
    /// Resources in the list.
    resources: Vec<RlmiResource>,
}

impl RlmiBuilder {
    /// Create a new RLMI builder.
    pub fn new(list_uri: &str, version: u32, full_state: bool) -> Self {
        Self {
            list_uri: list_uri.to_string(),
            version,
            full_state,
            resources: Vec::new(),
        }
    }

    /// Add a resource with instances to the RLMI document.
    pub fn add_resource(&mut self, resource: RlmiResource) {
        self.resources.push(resource);
    }

    /// Build the RLMI XML document.
    pub fn build(&self) -> String {
        let mut xml = String::with_capacity(1024);

        xml.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
        xml.push_str("<list xmlns=\"urn:ietf:params:xml:ns:rlmi\"\n");
        xml.push_str(&format!("      uri=\"{}\"", escape_xml(&self.list_uri)));
        xml.push_str(&format!(" version=\"{}\"", self.version));
        xml.push_str(&format!(
            " fullState=\"{}\">\n",
            if self.full_state { "true" } else { "false" }
        ));

        for resource in &self.resources {
            xml.push_str(&format!(
                "  <resource uri=\"{}\">\n",
                escape_xml(&resource.uri)
            ));
            for instance in &resource.instances {
                xml.push_str(&format!(
                    "    <instance id=\"{}\" state=\"{}\" cid=\"{}\"/>\n",
                    escape_xml(&instance.id),
                    escape_xml(&instance.state),
                    escape_xml(&instance.cid),
                ));
            }
            xml.push_str("  </resource>\n");
        }

        xml.push_str("</list>\n");
        xml
    }
}

fn escape_xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rlmi_full_state() {
        let mut builder = RlmiBuilder::new("sip:my-team@example.com", 1, true);
        builder.add_resource(RlmiResource {
            uri: "sip:alice@example.com".to_string(),
            instances: vec![RlmiInstance {
                id: "alice1".to_string(),
                state: "active".to_string(),
                cid: "alice@example.com".to_string(),
            }],
        });
        builder.add_resource(RlmiResource {
            uri: "sip:bob@example.com".to_string(),
            instances: vec![RlmiInstance {
                id: "bob1".to_string(),
                state: "active".to_string(),
                cid: "bob@example.com".to_string(),
            }],
        });

        let xml = builder.build();

        assert!(xml.contains("urn:ietf:params:xml:ns:rlmi"));
        assert!(xml.contains("uri=\"sip:my-team@example.com\""));
        assert!(xml.contains("version=\"1\""));
        assert!(xml.contains("fullState=\"true\""));
        assert!(xml.contains("sip:alice@example.com"));
        assert!(xml.contains("sip:bob@example.com"));
        assert!(xml.contains("id=\"alice1\""));
        assert!(xml.contains("state=\"active\""));
        assert!(xml.contains("cid=\"alice@example.com\""));
    }

    #[test]
    fn test_rlmi_partial_state() {
        let builder = RlmiBuilder::new("sip:list@example.com", 5, false);
        let xml = builder.build();
        assert!(xml.contains("fullState=\"false\""));
        assert!(xml.contains("version=\"5\""));
    }

    #[test]
    fn test_xml_escaping() {
        let mut builder = RlmiBuilder::new("sip:team&group@example.com", 0, true);
        builder.add_resource(RlmiResource {
            uri: "sip:user<1>@example.com".to_string(),
            instances: vec![],
        });
        let xml = builder.build();
        assert!(xml.contains("team&amp;group"));
        assert!(xml.contains("user&lt;1&gt;"));
    }
}
