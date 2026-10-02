//! The SAML 2.0 service provider (`docs/IDENTITY_AND_DEVICES.md` phase 4),
//! over `samael` and the `xmlsec` C library behind the `saml` feature.
//! A build without it keeps every route and answers that SAML is not here.
//!
//! Each of a tenant's SAML providers is its own service provider: entity id
//! `<public_url>/api/v1/auth/saml/{id}`, assertions posted to
//! `…/{id}/acs`. A response is taken only when:
//! - it is signed by a certificate in the provider's metadata (`samael`
//!   reduces it to its signed parts, so nothing unsigned rides along), with
//!   SHA-256 or stronger;
//! - it answers this sign-in's request (`InResponseTo`), not an
//!   unsolicited one;
//! - it is for this service provider: its destination, its bearer
//!   confirmation's recipient, and an audience naming us, which samael
//!   only checks when there is one and this module requires;
//! - it is in its time window;
//! - its assertion has not been seen before (the caller remembers ids);
//! - it names a subject that is not transient.

use fcp_core::identity::IdentityProvider;
use serde_json::{Map, Value};
use std::time::SystemTime;

/// Where a provider's service provider lives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoints {
    /// The SP's entity id, which is also where its metadata is served.
    pub entity_id: String,
    pub acs_url: String,
}

impl Endpoints {
    pub fn new(public_url: &str, provider: &IdentityProvider) -> Self {
        let base = format!("{public_url}/api/v1/auth/saml/{}", provider.id);
        Self {
            acs_url: format!("{base}/acs"),
            entity_id: base,
        }
    }
}

/// Why SAML failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SamlError {
    /// This build has no SAML.
    Unavailable,
    /// The provider's metadata is unusable.
    Metadata(String),
    /// The response is not one to accept.
    Refused(String),
}

impl std::fmt::Display for SamlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable => {
                write!(f, "this server was built without SAML (the `saml` feature)")
            }
            Self::Metadata(e) => write!(f, "the identity provider's metadata: {e}"),
            Self::Refused(e) => write!(f, "SAML response refused: {e}"),
        }
    }
}

impl SamlError {
    /// The short reason for events and the login page.
    pub fn reason(&self) -> &'static str {
        match self {
            Self::Unavailable | Self::Metadata(_) => "provider_unavailable",
            Self::Refused(_) => "bad_assertion",
        }
    }
}

/// What a provider's metadata says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdpSummary {
    pub entity_id: String,
    pub sso_url: String,
    pub signing_certificates: usize,
}

/// A response taken: its claims for the user mapping (`sub` the NameID,
/// `sid` the session index, each attribute by name), and what the replay
/// check needs.
#[derive(Debug, Clone)]
pub struct Accepted {
    pub claims: Map<String, Value>,
    pub assertion_id: String,
    pub expires: SystemTime,
}

/// Whether this build speaks SAML.
pub const AVAILABLE: bool = cfg!(feature = "saml");

#[cfg(feature = "saml")]
mod imp {
    use super::*;
    use samael::crypto::AllowedSignatureAlgorithm as Alg;
    use samael::metadata::EntityDescriptor;
    use samael::service_provider::{ServiceProvider, ServiceProviderBuilder};
    use samael::traits::ToXml;

    const REDIRECT_BINDING: &str = "urn:oasis:names:tc:SAML:2.0:bindings:HTTP-Redirect";
    const TRANSIENT: &str = "urn:oasis:names:tc:SAML:2.0:nameid-format:transient";

    fn metadata(xml: &str) -> Result<EntityDescriptor, SamlError> {
        xml.parse::<EntityDescriptor>()
            .map_err(|e| SamlError::Metadata(format!("not SAML metadata: {e}")))
    }

    pub(super) fn service_provider(
        settings: &fcp_core::identity::SamlSettings,
        endpoints: &Endpoints,
    ) -> Result<ServiceProvider, SamlError> {
        let sp = ServiceProviderBuilder::default()
            .entity_id(endpoints.entity_id.clone())
            .metadata_url(endpoints.entity_id.clone())
            .acs_url(endpoints.acs_url.clone())
            .idp_metadata(metadata(&settings.idp_metadata_xml)?)
            .allowed_signature_algorithms(vec![
                Alg::RsaSha256,
                Alg::RsaSha384,
                Alg::RsaSha512,
                Alg::EcdsaSha256,
                Alg::EcdsaSha384,
                Alg::EcdsaSha512,
            ])
            .build()
            .map_err(|e| SamlError::Metadata(e.to_string()))?;
        // samael verifies nothing when the metadata names no signing
        // certificate: such a provider is refused here, every time.
        match sp.idp_signing_certs() {
            Ok(Some(certs)) if !certs.is_empty() => Ok(sp),
            Ok(_) => Err(SamlError::Metadata(
                "it names no signing certificate".to_string(),
            )),
            Err(e) => Err(SamlError::Metadata(format!("its certificates: {e}"))),
        }
    }

    pub(super) fn summarize(xml: &str) -> Result<IdpSummary, SamlError> {
        let md = metadata(xml)?;
        let entity_id = md
            .entity_id
            .clone()
            .ok_or_else(|| SamlError::Metadata("it names no entityID".to_string()))?;
        let endpoints = Endpoints {
            entity_id: "urn:fcp:check".to_string(),
            acs_url: "urn:fcp:check".to_string(),
        };
        let settings = fcp_core::identity::SamlSettings {
            idp_metadata_xml: xml.to_string(),
            trust_email: false,
        };
        let sp = service_provider(&settings, &endpoints)?;
        let sso_url = sp.sso_binding_location(REDIRECT_BINDING).ok_or_else(|| {
            SamlError::Metadata("it offers no HTTP-Redirect single sign-on".to_string())
        })?;
        let signing_certificates = sp.idp_signing_certs().ok().flatten().map_or(0, |c| c.len());
        Ok(IdpSummary {
            entity_id,
            sso_url,
            signing_certificates,
        })
    }

    pub(super) fn begin(
        settings: &fcp_core::identity::SamlSettings,
        endpoints: &Endpoints,
        relay_state: &str,
    ) -> Result<(String, String), SamlError> {
        let sp = service_provider(settings, endpoints)?;
        let sso = sp.sso_binding_location(REDIRECT_BINDING).ok_or_else(|| {
            SamlError::Metadata("it offers no HTTP-Redirect single sign-on".to_string())
        })?;
        let request = sp
            .make_authentication_request(&sso)
            .map_err(|e| SamlError::Metadata(e.to_string()))?;
        let url = request
            .redirect(relay_state)
            .map_err(|e| SamlError::Metadata(e.to_string()))?
            .ok_or_else(|| SamlError::Metadata("no redirect address".to_string()))?;
        Ok((url.into(), request.id))
    }

    pub(super) fn sp_metadata(
        settings: &fcp_core::identity::SamlSettings,
        endpoints: &Endpoints,
    ) -> Result<String, SamlError> {
        let sp = service_provider(settings, endpoints)?;
        sp.metadata()
            .map_err(|e| SamlError::Metadata(e.to_string()))?
            .to_string()
            .map_err(|e| SamlError::Metadata(e.to_string()))
    }

    pub(super) fn accept(
        settings: &fcp_core::identity::SamlSettings,
        endpoints: &Endpoints,
        saml_response: &str,
        request_id: &str,
    ) -> Result<Accepted, SamlError> {
        let refused = |e: &str| SamlError::Refused(e.to_string());
        let sp = service_provider(settings, endpoints)?;
        let assertion = sp
            .parse_base64_response(saml_response.trim(), Some(&[request_id]))
            .map_err(|e| SamlError::Refused(e.to_string()))?;

        // An audience naming us — samael checks one only where there is one.
        let ours = assertion
            .conditions
            .as_ref()
            .and_then(|c| c.audience_restrictions.as_ref())
            .is_some_and(|restrictions| {
                !restrictions.is_empty()
                    && restrictions
                        .iter()
                        .all(|r| r.audience.iter().any(|a| a == &endpoints.entity_id))
            });
        if !ours {
            return Err(refused(
                "the assertion is not restricted to this service provider",
            ));
        }

        let name_id = assertion
            .subject
            .as_ref()
            .and_then(|s| s.name_id.as_ref())
            .filter(|n| !n.value.trim().is_empty())
            .ok_or_else(|| refused("the assertion names no subject"))?;
        if name_id.format.as_deref() == Some(TRANSIENT) {
            return Err(refused(
                "a transient NameID names nobody twice: ask the provider for a persistent one",
            ));
        }

        let mut claims = Map::new();
        for statement in assertion.attribute_statements.iter().flatten() {
            for attribute in &statement.attributes {
                let values: Vec<Value> = attribute
                    .values
                    .iter()
                    .filter_map(|v| v.value.clone())
                    .map(Value::String)
                    .collect();
                let value = match values.len() {
                    1 => values.into_iter().next().unwrap_or(Value::Null),
                    _ => Value::Array(values),
                };
                for name in [&attribute.name, &attribute.friendly_name]
                    .into_iter()
                    .flatten()
                {
                    claims.entry(name.clone()).or_insert_with(|| value.clone());
                }
            }
        }
        // The subject and the session are the assertion's, whatever an
        // attribute of the same name says.
        claims.insert("sub".to_string(), Value::String(name_id.value.clone()));
        claims.insert(
            fcp_core::identity::SAML_NAME_ID.to_string(),
            Value::String(name_id.value.clone()),
        );
        claims.remove("sid");
        if let Some(index) = assertion
            .authn_statements
            .iter()
            .flatten()
            .find_map(|s| s.session_index.clone())
        {
            claims.insert("sid".to_string(), Value::String(index));
        }

        // Remembered until the latest moment it could still be taken.
        let until = [
            assertion.conditions.as_ref().and_then(|c| c.not_on_or_after),
            assertion
                .subject
                .as_ref()
                .and_then(|s| s.subject_confirmations.as_ref())
                .into_iter()
                .flatten()
                .filter_map(|c| c.subject_confirmation_data.as_ref())
                .filter_map(|d| d.not_on_or_after)
                .max(),
        ]
        .into_iter()
        .flatten()
        .max()
        .unwrap_or_else(|| chrono::Utc::now() + chrono::Duration::minutes(10))
            // The clock skew samael allows past it.
            + chrono::Duration::minutes(5);
        Ok(Accepted {
            claims,
            assertion_id: assertion.id.clone(),
            expires: until.into(),
        })
    }
}

fn settings(provider: &IdentityProvider) -> Result<&fcp_core::identity::SamlSettings, SamlError> {
    provider
        .settings
        .saml
        .as_ref()
        .ok_or_else(|| SamlError::Metadata("the provider has no SAML settings".to_string()))
}

/// What a provider's metadata XML says, or why it is unusable.
pub fn summarize(xml: &str) -> Result<IdpSummary, SamlError> {
    #[cfg(feature = "saml")]
    return imp::summarize(xml);
    #[cfg(not(feature = "saml"))]
    {
        let _ = xml;
        Err(SamlError::Unavailable)
    }
}

/// Begin a sign-in: where to send the browser (the request carries
/// `relay_state`), and the request's id the response must answer.
pub fn begin(
    provider: &IdentityProvider,
    endpoints: &Endpoints,
    relay_state: &str,
) -> Result<(String, String), SamlError> {
    let settings = settings(provider)?;
    #[cfg(feature = "saml")]
    return imp::begin(settings, endpoints, relay_state);
    #[cfg(not(feature = "saml"))]
    {
        let _ = (settings, endpoints, relay_state);
        Err(SamlError::Unavailable)
    }
}

/// The service provider's metadata, for the identity provider's admin.
pub fn sp_metadata(
    provider: &IdentityProvider,
    endpoints: &Endpoints,
) -> Result<String, SamlError> {
    let settings = settings(provider)?;
    #[cfg(feature = "saml")]
    return imp::sp_metadata(settings, endpoints);
    #[cfg(not(feature = "saml"))]
    {
        let _ = (settings, endpoints);
        Err(SamlError::Unavailable)
    }
}

/// Take a posted `SAMLResponse` answering `request_id`, if it is one to
/// take.
pub fn accept(
    provider: &IdentityProvider,
    endpoints: &Endpoints,
    saml_response: &str,
    request_id: &str,
) -> Result<Accepted, SamlError> {
    let settings = settings(provider)?;
    #[cfg(feature = "saml")]
    return imp::accept(settings, endpoints, saml_response, request_id);
    #[cfg(not(feature = "saml"))]
    {
        let _ = (settings, endpoints, saml_response, request_id);
        Err(SamlError::Unavailable)
    }
}
