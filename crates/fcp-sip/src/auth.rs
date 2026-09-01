//! Custom SIP authenticator that uses pre-computed HA1 hashes.
//!
//! This authenticator accepts pre-computed HA1 hashes instead of plaintext passwords,
//! providing better security by never storing or handling plaintext SIP passwords.

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use sip_auth::{DigestAlgorithm, Qop};
use sip_core::{Headers, Method, Request, Response, StatusLine};
use sip_parse::parse_authorization_header;
use smol_str::SmolStr;

#[async_trait]
pub trait Ha1CredentialStore: Send + Sync {
    async fn fetch(
        &self,
        username: &str,
        realm: &str,
        request: &Request,
    ) -> Option<sip_auth::Credentials>;
}

/// Custom authenticator that uses pre-computed HA1 hashes.
///
/// Unlike the standard siphon-rs authenticator which expects plaintext passwords
/// and computes HA1, this authenticator accepts pre-computed HA1 hashes directly.
/// This is more secure because we never store plaintext passwords in the database.
pub struct Ha1Authenticator<S: Ha1CredentialStore> {
    realm: String,
    store: S,
    algorithm: DigestAlgorithm,
    qop: Qop,
}

impl<S: Ha1CredentialStore> Ha1Authenticator<S> {
    pub fn new(realm: String, store: S) -> Self {
        Self {
            realm,
            store,
            algorithm: DigestAlgorithm::Md5,
            qop: Qop::Auth,
        }
    }

    /// Compute HA2 for authentication
    fn compute_ha2(&self, method: &Method, uri: &str, body: &[u8]) -> String {
        let ha2_input = match self.qop {
            Qop::Auth => format!("{}:{}", method.as_str(), uri),
            Qop::AuthInt => {
                let body_hash = format!("{:x}", md5::compute(body));
                format!("{}:{}:{}", method.as_str(), uri, body_hash)
            }
        };
        format!("{:x}", md5::compute(ha2_input.as_bytes()))
    }

    /// Compute the expected response using pre-computed HA1
    #[allow(clippy::too_many_arguments)]
    fn compute_response_from_ha1(
        &self,
        ha1: &str,
        method: &Method,
        uri: &str,
        nonce: &str,
        nc: Option<&str>,
        cnonce: Option<&str>,
        qop: Option<Qop>,
        body: &[u8],
    ) -> String {
        let ha2 = self.compute_ha2(method, uri, body);

        let final_input = if let (Some(qop), Some(nc), Some(cnonce)) = (qop, nc, cnonce) {
            format!(
                "{}:{}:{}:{}:{}:{}",
                ha1,
                nonce,
                nc,
                cnonce,
                qop.as_str(),
                ha2
            )
        } else {
            format!("{}:{}:{}", ha1, nonce, ha2)
        };

        format!("{:x}", md5::compute(final_input.as_bytes()))
    }

    /// Generate a 401 Unauthorized challenge
    pub fn challenge(&self, request: &Request) -> Result<Response> {
        let nonce = self.generate_nonce();

        let mut headers = Headers::new();

        // Copy required headers from request
        if let Some(via) = request.headers().get("Via") {
            headers.push(SmolStr::new("Via"), via)?;
        }
        if let Some(from) = request.headers().get("From") {
            headers.push(SmolStr::new("From"), from)?;
        }
        if let Some(to) = request.headers().get("To") {
            let to_with_tag = if !to.contains(";tag=") {
                format!("{};tag={}", to, self.generate_tag())
            } else {
                to.to_string()
            };
            headers.push(SmolStr::new("To"), SmolStr::new(to_with_tag))?;
        }
        if let Some(call_id) = request.headers().get("Call-ID") {
            headers.push(SmolStr::new("Call-ID"), call_id)?;
        }
        if let Some(cseq) = request.headers().get("CSeq") {
            headers.push(SmolStr::new("CSeq"), cseq)?;
        }

        // Add authentication challenge
        let challenge = format!(
            "Digest realm=\"{}\", nonce=\"{}\", algorithm={}, qop=\"{}\"",
            self.realm,
            nonce,
            self.algorithm.as_str(),
            self.qop.as_str()
        );
        headers.push(SmolStr::new("WWW-Authenticate"), SmolStr::new(challenge))?;
        headers.push(SmolStr::new("Content-Length"), SmolStr::new("0"))?;

        Ok(Response::new(
            StatusLine::new(401, "Unauthorized")?,
            headers,
            bytes::Bytes::new(),
        )?)
    }

    /// Verify authentication using pre-computed HA1
    pub async fn verify(&self, request: &Request) -> Result<bool> {
        // Parse Authorization header
        let auth_header = request
            .headers()
            .get_smol("Authorization")
            .or_else(|| request.headers().get_smol("Proxy-Authorization"))
            .ok_or_else(|| anyhow!("No authorization header"))?;

        let auth = parse_authorization_header(auth_header)
            .ok_or_else(|| anyhow!("Failed to parse authorization header"))?;

        // Extract required fields
        // Normalize username by trimming trailing @ (some buggy clients like sipsak add it)
        let username = auth
            .param("username")
            .ok_or_else(|| anyhow!("Missing username"))?;
        let username_str = username.as_str().trim_end_matches('@');
        let realm = auth
            .param("realm")
            .ok_or_else(|| anyhow!("Missing realm"))?;
        let nonce = auth
            .param("nonce")
            .ok_or_else(|| anyhow!("Missing nonce"))?;
        let uri = auth.param("uri").ok_or_else(|| anyhow!("Missing URI"))?;
        let response = auth
            .param("response")
            .ok_or_else(|| anyhow!("Missing response"))?;

        // Verify realm matches
        if realm.as_str() != self.realm.as_str() {
            return Ok(false);
        }

        // Fetch pre-computed HA1 from store
        let creds = self
            .store
            .fetch(username_str, realm.as_str(), request)
            .await
            .ok_or_else(|| anyhow!("User not found: {}", username_str))?;

        // The "password" in credentials is actually the pre-computed HA1 hash
        let ha1 = creds.password();

        // Parse qop if present
        let qop = auth.param("qop").and_then(|q| match q.as_str() {
            "auth" => Some(Qop::Auth),
            "auth-int" => Some(Qop::AuthInt),
            _ => None,
        });

        // Compute expected response
        let expected_response = self.compute_response_from_ha1(
            ha1,
            request.method(),
            uri.as_str(),
            nonce.as_str(),
            auth.param("nc").map(|s| s.as_str()),
            auth.param("cnonce").map(|s| s.as_str()),
            qop,
            request.body().as_ref(),
        );

        // Constant-time comparison
        Ok(constant_time_eq(
            expected_response.as_bytes(),
            response.as_bytes(),
        ))
    }

    fn generate_nonce(&self) -> String {
        use rand::{distributions::Alphanumeric, thread_rng, Rng};
        thread_rng()
            .sample_iter(&Alphanumeric)
            .take(32)
            .map(char::from)
            .collect()
    }

    fn generate_tag(&self) -> String {
        use rand::{distributions::Alphanumeric, thread_rng, Rng};
        thread_rng()
            .sample_iter(&Alphanumeric)
            .take(10)
            .map(char::from)
            .collect()
    }
}

/// Constant-time string comparison to prevent timing attacks
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }

    let mut result = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        result |= x ^ y;
    }
    result == 0
}
