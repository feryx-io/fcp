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
//!
//! Each provider has a key of FCP's own (made when the provider is, kept
//! sealed as its secret, its certificate in FCP's metadata): it signs FCP's
//! requests and its answers to a logout, and a provider may encrypt
//! assertions to it. A logout the provider asks for (`…/{id}/slo`, either
//! binding) is taken only signed by the provider's certificate: over the
//! query string exactly as it came for HTTP-Redirect, the XML reduced to its
//! signed parts for HTTP-POST.

use fcp_core::identity::IdentityProvider;
use serde_json::{Map, Value};
use std::time::SystemTime;

/// Where a provider's service provider lives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoints {
    /// The SP's entity id, which is also where its metadata is served.
    pub entity_id: String,
    pub acs_url: String,
    /// Single logout, both bindings.
    pub slo_url: String,
}

impl Endpoints {
    pub fn new(public_url: &str, provider: &IdentityProvider) -> Self {
        let base = format!("{public_url}/api/v1/auth/saml/{}", provider.id);
        Self {
            acs_url: format!("{base}/acs"),
            slo_url: format!("{base}/slo"),
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
    /// Its HTTP-Redirect single logout address, if it has one.
    pub slo_url: Option<String>,
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
    /// The subject as FCP's own `LogoutRequest` must name it: its NameID
    /// with its format ([`name_id_hint`]).
    pub logout_hint: String,
}

/// A NameID and its format as one string, kept with a session so FCP can
/// ask the provider to end it: the format, a tab, the value.
pub fn name_id_hint(format: Option<&str>, value: &str) -> String {
    format!("{}\t{value}", format.unwrap_or_default())
}

/// The format and value [`name_id_hint`] kept.
pub fn split_name_id_hint(hint: &str) -> (Option<&str>, &str) {
    match hint.split_once('\t') {
        Some((format, value)) => (Some(format).filter(|f| !f.is_empty()), value),
        None => (None, hint),
    }
}

/// A logout the provider asks for: whom, which of its sessions, and what
/// the answer goes back with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogoutAsked {
    pub request_id: String,
    pub name_id: Option<String>,
    pub session_index: Option<String>,
}

/// FCP's key for a provider: the private key (base64 DER, the provider's
/// sealed secret) and its certificate (base64 DER, published).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpKey {
    pub private_key: String,
    pub certificate: String,
}

/// Whether this build speaks SAML.
pub const AVAILABLE: bool = cfg!(feature = "saml");

#[cfg(feature = "saml")]
mod imp {
    use super::*;
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine;
    use samael::crypto::AllowedSignatureAlgorithm as Alg;
    use samael::crypto::{CertificateDer, Crypto, CryptoProvider, ReduceMode};
    use samael::metadata::EntityDescriptor;
    use samael::schema::LogoutRequest;
    use samael::service_provider::{ServiceProvider, ServiceProviderBuilder};
    use samael::traits::ToXml;
    use std::io::{Read, Write};

    const REDIRECT_BINDING: &str = "urn:oasis:names:tc:SAML:2.0:bindings:HTTP-Redirect";
    const TRANSIENT: &str = "urn:oasis:names:tc:SAML:2.0:nameid-format:transient";
    /// How far a logout request's issue time may be from now.
    const LOGOUT_SKEW_SECS: i64 = 300;

    fn allowed() -> Vec<Alg> {
        vec![
            Alg::RsaSha256,
            Alg::RsaSha384,
            Alg::RsaSha512,
            Alg::EcdsaSha256,
            Alg::EcdsaSha384,
            Alg::EcdsaSha512,
        ]
    }

    fn metadata(xml: &str) -> Result<EntityDescriptor, SamlError> {
        xml.parse::<EntityDescriptor>()
            .map_err(|e| SamlError::Metadata(format!("not SAML metadata: {e}")))
    }

    fn private_key(sp_key: &str) -> Result<openssl::pkey::PKey<openssl::pkey::Private>, SamlError> {
        let der = STANDARD
            .decode(sp_key.trim())
            .map_err(|_| SamlError::Metadata("FCP's key for it is unreadable".to_string()))?;
        openssl::pkey::PKey::private_key_from_der(&der)
            .map_err(|_| SamlError::Metadata("FCP's key for it is unreadable".to_string()))
    }

    pub(super) fn generate_sp_key() -> Result<SpKey, SamlError> {
        let fail =
            |e: &dyn std::fmt::Display| SamlError::Metadata(format!("making FCP's key: {e}"));
        let maker = samael::idp::IdentityProvider::generate_new(samael::idp::KeyType::Rsa(
            samael::idp::Rsa::Rsa2048,
        ))
        .map_err(|e| fail(&e))?;
        let certificate = maker
            .create_certificate(&samael::idp::CertificateParams {
                common_name: "FCP SAML service provider",
                issuer_name: "FCP SAML service provider",
                days_until_expiration: 3650,
            })
            .map_err(|e| fail(&e))?;
        Ok(SpKey {
            private_key: STANDARD.encode(maker.export_private_key_der().map_err(|e| fail(&e))?),
            certificate: STANDARD.encode(certificate.der_data()),
        })
    }

    pub(super) fn service_provider(
        settings: &fcp_core::identity::SamlSettings,
        endpoints: &Endpoints,
        sp_key: Option<&str>,
    ) -> Result<ServiceProvider, SamlError> {
        let mut builder = ServiceProviderBuilder::default();
        builder
            .entity_id(endpoints.entity_id.clone())
            .metadata_url(endpoints.entity_id.clone())
            .acs_url(endpoints.acs_url.clone())
            .slo_url(endpoints.slo_url.clone())
            .idp_metadata(metadata(&settings.idp_metadata_xml)?)
            .allowed_signature_algorithms(allowed());
        if let (Some(key), Some(cert)) = (sp_key, settings.sp_certificate.as_deref()) {
            let der = STANDARD.decode(cert.trim()).map_err(|_| {
                SamlError::Metadata("FCP's certificate for it is unreadable".to_string())
            })?;
            builder
                .key(private_key(key)?)
                .certificate(CertificateDer::from(der));
        }
        let sp = builder
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
            slo_url: "urn:fcp:check".to_string(),
        };
        let settings = fcp_core::identity::SamlSettings {
            idp_metadata_xml: xml.to_string(),
            idp_metadata_url: None,
            sp_certificate: None,
            trust_email: false,
        };
        let sp = service_provider(&settings, &endpoints, None)?;
        let sso_url = sp.sso_binding_location(REDIRECT_BINDING).ok_or_else(|| {
            SamlError::Metadata("it offers no HTTP-Redirect single sign-on".to_string())
        })?;
        let signing_certificates = sp.idp_signing_certs().ok().flatten().map_or(0, |c| c.len());
        Ok(IdpSummary {
            entity_id,
            sso_url,
            slo_url: sp.slo_binding_location(REDIRECT_BINDING),
            signing_certificates,
        })
    }

    pub(super) fn begin(
        settings: &fcp_core::identity::SamlSettings,
        endpoints: &Endpoints,
        sp_key: Option<&str>,
        relay_state: &str,
    ) -> Result<(String, String), SamlError> {
        let sp = service_provider(settings, endpoints, sp_key)?;
        let sso = sp.sso_binding_location(REDIRECT_BINDING).ok_or_else(|| {
            SamlError::Metadata("it offers no HTTP-Redirect single sign-on".to_string())
        })?;
        let request = sp
            .make_authentication_request(&sso)
            .map_err(|e| SamlError::Metadata(e.to_string()))?;
        // Signed with FCP's key, where it has one.
        let url = match &sp.key {
            Some(key) => request.signed_redirect(relay_state, key),
            None => request.redirect(relay_state),
        }
        .map_err(|e| SamlError::Metadata(e.to_string()))?
        .ok_or_else(|| SamlError::Metadata("no redirect address".to_string()))?;
        Ok((url.into(), request.id))
    }

    pub(super) fn sp_metadata(
        settings: &fcp_core::identity::SamlSettings,
        endpoints: &Endpoints,
        sp_key: Option<&str>,
    ) -> Result<String, SamlError> {
        let sp = service_provider(settings, endpoints, sp_key)?;
        let mut metadata = sp
            .metadata()
            .map_err(|e| SamlError::Metadata(e.to_string()))?;
        // FCP signs its requests where it has a key.
        if sp.key.is_some() {
            for descriptor in metadata.sp_sso_descriptors.iter_mut().flatten() {
                descriptor.authn_requests_signed = Some(true);
            }
        }
        metadata
            .to_string()
            .map_err(|e| SamlError::Metadata(e.to_string()))
    }

    pub(super) fn accept(
        settings: &fcp_core::identity::SamlSettings,
        endpoints: &Endpoints,
        sp_key: Option<&str>,
        saml_response: &str,
        request_id: &str,
    ) -> Result<Accepted, SamlError> {
        let refused = |e: &str| SamlError::Refused(e.to_string());
        let sp = service_provider(settings, endpoints, sp_key)?;
        // An encrypted assertion is decrypted with FCP's key; samael takes
        // one only inside a signed response.
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
            logout_hint: name_id_hint(name_id.format.as_deref(), &name_id.value),
        })
    }

    /// A query string's pairs, each value as it came (still encoded).
    fn raw_pairs(query: &str) -> Vec<(&str, &str)> {
        query
            .split('&')
            .filter_map(|pair| pair.split_once('='))
            .collect()
    }

    fn url_decode(raw: &str) -> Result<String, SamlError> {
        urlencoding::decode(&raw.replace('+', " "))
            .map(|s| s.into_owned())
            .map_err(|_| SamlError::Refused("a malformed query".to_string()))
    }

    /// The logout request, checked against what it must say.
    fn checked_logout(
        sp: &ServiceProvider,
        endpoints: &Endpoints,
        xml: &str,
    ) -> Result<LogoutAsked, SamlError> {
        let refused = |e: &str| SamlError::Refused(e.to_string());
        let request: LogoutRequest = xml.parse().map_err(|_| refused("not a LogoutRequest"))?;
        let idp = sp.idp_metadata.entity_id.clone().unwrap_or_default();
        if request.issuer.as_ref().and_then(|i| i.value.as_deref()) != Some(idp.as_str()) {
            return Err(refused("another provider's logout request"));
        }
        if request
            .destination
            .as_deref()
            .is_some_and(|d| d != endpoints.slo_url)
        {
            return Err(refused("a logout request for another service provider"));
        }
        let fresh = request
            .issue_instant
            .is_some_and(|at| (chrono::Utc::now() - at).num_seconds().abs() <= LOGOUT_SKEW_SECS);
        if !fresh {
            return Err(refused("a logout request not issued just now"));
        }
        // samael reads `SessionIndex` as an attribute, where SAML makes it
        // an element: both are read here, from the request's own children.
        let (name_id, session_index) = logout_subject(xml);
        if name_id.is_none() && session_index.is_none() {
            return Err(refused("a logout request names nobody"));
        }
        Ok(LogoutAsked {
            request_id: request
                .id
                .clone()
                .filter(|id| !id.is_empty())
                .ok_or_else(|| refused("a logout request without an ID"))?,
            name_id,
            session_index,
        })
    }

    /// A LogoutRequest's `NameID` and first `SessionIndex`: elements
    /// directly under the request, whatever their prefix.
    fn logout_subject(xml: &str) -> (Option<String>, Option<String>) {
        use quick_xml::events::Event;
        let mut reader = quick_xml::Reader::from_str(xml);
        reader.config_mut().trim_text(true);
        let (mut depth, mut reading) = (0usize, None::<&'static str>);
        let (mut name_id, mut session_index) = (None, None);
        loop {
            match reader.read_event() {
                Ok(Event::Start(e)) => {
                    depth += 1;
                    if depth == 2 {
                        reading = match e.local_name().as_ref() {
                            b"NameID" => Some("NameID"),
                            b"SessionIndex" => Some("SessionIndex"),
                            _ => None,
                        };
                    }
                }
                Ok(Event::Text(t)) if depth == 2 => {
                    let text = t
                        .decode()
                        .ok()
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty());
                    match reading {
                        Some("NameID") if name_id.is_none() => name_id = text,
                        Some("SessionIndex") if session_index.is_none() => session_index = text,
                        _ => {}
                    }
                }
                Ok(Event::End(_)) => {
                    depth = depth.saturating_sub(1);
                    reading = None;
                }
                Ok(Event::Eof) | Err(_) => break,
                _ => {}
            }
        }
        (name_id, session_index)
    }

    pub(super) fn logout_redirect(
        settings: &fcp_core::identity::SamlSettings,
        endpoints: &Endpoints,
        sp_key: Option<&str>,
        raw_query: &str,
    ) -> Result<LogoutAsked, SamlError> {
        let refused = |e: &str| SamlError::Refused(e.to_string());
        let sp = service_provider(settings, endpoints, sp_key)?;
        let pairs = raw_pairs(raw_query);
        let raw = |name: &str| pairs.iter().find(|(k, _)| *k == name).map(|(_, v)| *v);
        let (Some(request), Some(sig_alg), Some(signature)) =
            (raw("SAMLRequest"), raw("SigAlg"), raw("Signature"))
        else {
            return Err(refused("an unsigned logout request"));
        };
        // Signed is the query as it came: SAMLRequest, RelayState, SigAlg.
        let mut signed = format!("SAMLRequest={request}");
        if let Some(relay) = raw("RelayState") {
            signed.push_str(&format!("&RelayState={relay}"));
        }
        signed.push_str(&format!("&SigAlg={sig_alg}"));
        let digest = match url_decode(sig_alg)?.as_str() {
            "http://www.w3.org/2001/04/xmldsig-more#rsa-sha256" => {
                openssl::hash::MessageDigest::sha256()
            }
            "http://www.w3.org/2001/04/xmldsig-more#rsa-sha384" => {
                openssl::hash::MessageDigest::sha384()
            }
            "http://www.w3.org/2001/04/xmldsig-more#rsa-sha512" => {
                openssl::hash::MessageDigest::sha512()
            }
            other => {
                return Err(SamlError::Refused(format!(
                    "the signature algorithm {other} is not accepted"
                )))
            }
        };
        let signature = STANDARD
            .decode(url_decode(signature)?.replace(' ', "+"))
            .map_err(|_| refused("a malformed signature"))?;
        let certs = sp.idp_signing_certs().ok().flatten().unwrap_or_default();
        let verified = certs.iter().any(|cert| {
            openssl::x509::X509::from_der(cert.der_data())
                .and_then(|c| c.public_key())
                .and_then(|key| {
                    let mut verifier = openssl::sign::Verifier::new(digest, &key)?;
                    verifier.update(signed.as_bytes())?;
                    verifier.verify(&signature)
                })
                .unwrap_or(false)
        });
        if !verified {
            return Err(refused("the logout request's signature does not verify"));
        }
        let deflated = STANDARD
            .decode(url_decode(request)?.replace(' ', "+"))
            .map_err(|_| refused("a malformed logout request"))?;
        let mut xml = String::new();
        flate2::read::DeflateDecoder::new(&deflated[..])
            .take(1 << 20)
            .read_to_string(&mut xml)
            .map_err(|_| refused("a malformed logout request"))?;
        checked_logout(&sp, endpoints, &xml)
    }

    pub(super) fn logout_post(
        settings: &fcp_core::identity::SamlSettings,
        endpoints: &Endpoints,
        sp_key: Option<&str>,
        saml_request: &str,
    ) -> Result<LogoutAsked, SamlError> {
        let refused = |e: &str| SamlError::Refused(e.to_string());
        let sp = service_provider(settings, endpoints, sp_key)?;
        let xml = STANDARD
            .decode(saml_request.trim())
            .ok()
            .and_then(|b| String::from_utf8(b).ok())
            .ok_or_else(|| refused("a malformed logout request"))?;
        let certs = sp.idp_signing_certs().ok().flatten().unwrap_or_default();
        // Only what the provider signed is read.
        let reduced = Crypto::reduce_xml_to_signed_with_allowed_algorithms(
            &xml,
            &certs,
            ReduceMode::default(),
            Some(&allowed()),
        )
        .map_err(|_| refused("the logout request's signature does not verify"))?;
        checked_logout(&sp, endpoints, &reduced)
    }

    pub(super) fn logout_response(
        settings: &fcp_core::identity::SamlSettings,
        endpoints: &Endpoints,
        sp_key: Option<&str>,
        in_response_to: &str,
        relay_state: Option<&str>,
    ) -> Result<Option<String>, SamlError> {
        let sp = service_provider(settings, endpoints, sp_key)?;
        let Some(destination) = sp.slo_binding_location(REDIRECT_BINDING) else {
            return Ok(None);
        };
        let xml = format!(
            r#"<samlp:LogoutResponse xmlns:samlp="urn:oasis:names:tc:SAML:2.0:protocol" xmlns:saml="urn:oasis:names:tc:SAML:2.0:assertion" ID="_{id}" Version="2.0" IssueInstant="{at}" Destination="{dest}" InResponseTo="{irt}"><saml:Issuer>{issuer}</saml:Issuer><samlp:Status><samlp:StatusCode Value="urn:oasis:names:tc:SAML:2.0:status:Success"/></samlp:Status></samlp:LogoutResponse>"#,
            id = uuid::Uuid::new_v4().simple(),
            at = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ"),
            dest = escape(&destination),
            irt = escape(in_response_to),
            issuer = escape(&endpoints.entity_id),
        );
        redirect_with(&sp, &destination, "SAMLResponse", &xml, relay_state).map(Some)
    }

    fn escape(s: &str) -> String {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
    }

    pub(super) fn logout_request(
        settings: &fcp_core::identity::SamlSettings,
        endpoints: &Endpoints,
        sp_key: Option<&str>,
        hint: &str,
        session_index: Option<&str>,
    ) -> Result<Option<String>, SamlError> {
        let sp = service_provider(settings, endpoints, sp_key)?;
        let Some(destination) = sp.slo_binding_location(REDIRECT_BINDING) else {
            return Ok(None);
        };
        let (format, name_id) = split_name_id_hint(hint);
        let format = format
            .map(|f| format!(r#" Format="{}""#, escape(f)))
            .unwrap_or_default();
        let index = session_index
            .map(|i| format!("<samlp:SessionIndex>{}</samlp:SessionIndex>", escape(i)))
            .unwrap_or_default();
        let xml = format!(
            r#"<samlp:LogoutRequest xmlns:samlp="urn:oasis:names:tc:SAML:2.0:protocol" xmlns:saml="urn:oasis:names:tc:SAML:2.0:assertion" ID="_{id}" Version="2.0" IssueInstant="{at}" Destination="{dest}"><saml:Issuer>{issuer}</saml:Issuer><saml:NameID{format}>{name_id}</saml:NameID>{index}</samlp:LogoutRequest>"#,
            id = uuid::Uuid::new_v4().simple(),
            at = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ"),
            dest = escape(&destination),
            issuer = escape(&endpoints.entity_id),
            name_id = escape(name_id),
        );
        redirect_with(&sp, &destination, "SAMLRequest", &xml, None).map(Some)
    }

    /// `destination` with `xml` deflated under `param` (HTTP-Redirect),
    /// signed with FCP's key where it has one.
    fn redirect_with(
        sp: &ServiceProvider,
        destination: &str,
        param: &str,
        xml: &str,
        relay_state: Option<&str>,
    ) -> Result<String, SamlError> {
        let mut deflated = Vec::new();
        {
            let mut encoder =
                flate2::write::DeflateEncoder::new(&mut deflated, flate2::Compression::default());
            encoder
                .write_all(xml.as_bytes())
                .map_err(|e| SamlError::Metadata(e.to_string()))?;
        }
        let mut url: url::Url = destination
            .parse()
            .map_err(|_| SamlError::Metadata("its logout address is not a URL".to_string()))?;
        url.query_pairs_mut()
            .append_pair(param, &STANDARD.encode(&deflated));
        if let Some(relay) = relay_state.filter(|r| !r.is_empty()) {
            url.query_pairs_mut().append_pair("RelayState", relay);
        }
        let url = match &sp.key {
            Some(key) => samael::crypto::sign_url(url, key)
                .map_err(|e| SamlError::Metadata(e.to_string()))?,
            None => url,
        };
        Ok(url.into())
    }
}

fn settings(provider: &IdentityProvider) -> Result<&fcp_core::identity::SamlSettings, SamlError> {
    provider
        .settings
        .saml
        .as_ref()
        .ok_or_else(|| SamlError::Metadata("the provider has no SAML settings".to_string()))
}

macro_rules! with_saml {
    ($call:expr, $($unused:expr),*) => {{
        #[cfg(feature = "saml")]
        return $call;
        #[cfg(not(feature = "saml"))]
        {
            let _ = ($($unused),*);
            Err(SamlError::Unavailable)
        }
    }};
}

/// FCP's key and certificate for a new SAML provider.
pub fn generate_sp_key() -> Result<SpKey, SamlError> {
    #[cfg(feature = "saml")]
    return imp::generate_sp_key();
    #[cfg(not(feature = "saml"))]
    Err(SamlError::Unavailable)
}

/// What a provider's metadata XML says, or why it is unusable.
pub fn summarize(xml: &str) -> Result<IdpSummary, SamlError> {
    with_saml!(imp::summarize(xml), xml)
}

/// Begin a sign-in: where to send the browser (the request carries
/// `relay_state`, signed where FCP has a key), and the request's id the
/// response must answer.
pub fn begin(
    provider: &IdentityProvider,
    endpoints: &Endpoints,
    relay_state: &str,
) -> Result<(String, String), SamlError> {
    let settings = settings(provider)?;
    let key = provider.client_secret.as_deref();
    with_saml!(
        imp::begin(settings, endpoints, key, relay_state),
        settings,
        endpoints,
        key,
        relay_state
    )
}

/// The service provider's metadata, for the identity provider's admin.
pub fn sp_metadata(
    provider: &IdentityProvider,
    endpoints: &Endpoints,
) -> Result<String, SamlError> {
    let settings = settings(provider)?;
    let key = provider.client_secret.as_deref();
    with_saml!(
        imp::sp_metadata(settings, endpoints, key),
        settings,
        endpoints,
        key
    )
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
    let key = provider.client_secret.as_deref();
    with_saml!(
        imp::accept(settings, endpoints, key, saml_response, request_id),
        settings,
        endpoints,
        key,
        saml_response,
        request_id
    )
}

/// A logout the provider asks for by HTTP-Redirect, from the query string
/// exactly as it came, if its signature verifies.
pub fn logout_redirect(
    provider: &IdentityProvider,
    endpoints: &Endpoints,
    raw_query: &str,
) -> Result<LogoutAsked, SamlError> {
    let settings = settings(provider)?;
    let key = provider.client_secret.as_deref();
    with_saml!(
        imp::logout_redirect(settings, endpoints, key, raw_query),
        settings,
        endpoints,
        key,
        raw_query
    )
}

/// A logout the provider asks for by HTTP-POST, if signed.
pub fn logout_post(
    provider: &IdentityProvider,
    endpoints: &Endpoints,
    saml_request: &str,
) -> Result<LogoutAsked, SamlError> {
    let settings = settings(provider)?;
    let key = provider.client_secret.as_deref();
    with_saml!(
        imp::logout_post(settings, endpoints, key, saml_request),
        settings,
        endpoints,
        key,
        saml_request
    )
}

/// Where to send the browser with FCP's answer to a logout: the
/// provider's logout address with a signed `LogoutResponse`, or `None`
/// where its metadata names none.
pub fn logout_response(
    provider: &IdentityProvider,
    endpoints: &Endpoints,
    in_response_to: &str,
    relay_state: Option<&str>,
) -> Result<Option<String>, SamlError> {
    let settings = settings(provider)?;
    let key = provider.client_secret.as_deref();
    with_saml!(
        imp::logout_response(settings, endpoints, key, in_response_to, relay_state),
        settings,
        endpoints,
        key,
        in_response_to,
        relay_state
    )
}

/// Where to send the browser to sign out at the provider: its logout
/// address with FCP's signed `LogoutRequest` for the subject `hint` names
/// ([`name_id_hint`]) and its session, or `None` where its metadata names
/// no logout address.
pub fn logout_request(
    provider: &IdentityProvider,
    endpoints: &Endpoints,
    hint: &str,
    session_index: Option<&str>,
) -> Result<Option<String>, SamlError> {
    let settings = settings(provider)?;
    let key = provider.client_secret.as_deref();
    with_saml!(
        imp::logout_request(settings, endpoints, key, hint, session_index),
        settings,
        endpoints,
        key,
        hint,
        session_index
    )
}
