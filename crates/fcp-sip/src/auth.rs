//! Custom SIP authenticator that uses pre-computed HA1 hashes.
//!
//! This authenticator accepts pre-computed HA1 hashes instead of plaintext passwords,
//! providing better security by never storing or handling plaintext SIP passwords.

use anyhow::Result;
use async_trait::async_trait;
use sip_auth::{DigestAlgorithm, NonceManager, Qop};
use sip_core::{Headers, Method, Request, Response, StatusLine};
use sip_parse::parse_authorization_header;
use smol_str::SmolStr;
use std::time::Duration;
use tracing::{debug, info, warn};

/// How long a nonce this node issued stays usable.
///
/// Long enough that a device's INVITE can reuse the nonce from its
/// REGISTER, short enough that a captured `Authorization` header is worth
/// little. A device that comes back later is challenged again with
/// `stale=true` and retries without prompting anyone.
const DEFAULT_NONCE_TTL: Duration = Duration::from_secs(300);

/// What a request's credentials amount to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthOutcome {
    /// Proven: the digest matched, on a nonce this node issued, and the
    /// request is not a replay of another one.
    Authenticated { username: String },
    /// The credentials are right but the nonce is not usable — this node
    /// never issued it, it has expired, or it was already spent on a
    /// different request. The client may retry silently against a fresh
    /// challenge (RFC 7616 §3.5, `stale=true`).
    Stale,
    /// No credentials, or the wrong ones.
    Failed,
    /// The right password, signed for a `uri` that is not the
    /// Request-URI (RFC 3261 §22.4). A fresh challenge would be signed
    /// the same way, so it is answered `400` (RFC 2617 §3.2.2.5), never
    /// challenged (#307).
    WrongUri { username: String, signed: String },
}

/// A user's HA1 as a node kept it, by username (#326).
pub type RememberedHa1 = dyn Fn(&str) -> Option<String> + Send + Sync;

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
    /// The nonces this node has issued. A digest is only proof against a
    /// nonce from here (`docs/EDGE_SECURITY.md` §2.7).
    nonces: NonceManager,
    /// Whether a digest signed for another Request-URI is refused
    /// ([`AuthOutcome::WrongUri`]) or only logged (`[registrar]
    /// digest_uri_check`).
    check_uri: bool,
}

impl<S: Ha1CredentialStore> Ha1Authenticator<S> {
    pub fn new(realm: String, store: S) -> Self {
        Self::with_nonce_ttl(realm, store, DEFAULT_NONCE_TTL)
    }

    pub fn with_nonce_ttl(realm: String, store: S, nonce_ttl: Duration) -> Self {
        Self {
            realm,
            store,
            algorithm: DigestAlgorithm::Md5,
            qop: Qop::Auth,
            // A live nonce is usable for its whole life: the nonce-count
            // and the request identity below are what separate a
            // retransmission from a replay, not a short reuse window.
            nonces: NonceManager::new(nonce_ttl).with_max_request_age(nonce_ttl),
            check_uri: true,
        }
    }

    /// Whether a digest's `uri` must name the Request-URI (on by
    /// default). Off, a mismatch is logged and accepted, for a client or
    /// a proxy on the way that signs another form of the address.
    pub fn with_uri_check(mut self, check: bool) -> Self {
        self.check_uri = check;
        self
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

    /// Generate a 401 Unauthorized challenge.
    pub fn challenge(&self, request: &Request) -> Result<Response> {
        self.challenge_inner(request, false)
    }

    /// A challenge that tells the client its credentials were right and only
    /// the nonce was not, so it retries without prompting anyone
    /// (RFC 7616 §3.5).
    pub fn challenge_stale(&self, request: &Request) -> Result<Response> {
        self.challenge_inner(request, true)
    }

    fn challenge_inner(&self, request: &Request, stale: bool) -> Result<Response> {
        // The nonce is issued from this node's manager, which is what makes
        // the response to it proof of anything.
        let nonce = self.nonces.generate();
        let nonce = nonce.value();

        let mut headers = Headers::new();

        // Copy required headers from request: every Via, so a challenge
        // to a request that came through a proxy goes back through it.
        for via in request.headers().get_all("Via") {
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
            "Digest realm=\"{}\", nonce=\"{}\", algorithm={}, qop=\"{}\"{}",
            self.realm,
            nonce,
            self.algorithm.as_str(),
            self.qop.as_str(),
            if stale { ", stale=true" } else { "" }
        );
        headers.push(SmolStr::new("WWW-Authenticate"), SmolStr::new(challenge))?;
        headers.push(SmolStr::new("Content-Length"), SmolStr::new("0"))?;

        Ok(Response::new(
            StatusLine::new(401, "Unauthorized")?,
            headers,
            bytes::Bytes::new(),
        )?)
    }

    /// What a request's credentials amount to.
    ///
    /// The digest is checked first, against the nonce the client used: a
    /// client that knows the password is told its nonce was stale and
    /// retries silently, while a client that does not is simply refused,
    /// so a wrong password cannot loop. Only then is the nonce itself
    /// examined — this node must have issued it, it must still be live,
    /// and its nonce-count must not walk backwards
    /// (`docs/EDGE_SECURITY.md` §2.7).
    pub async fn authenticate(&self, request: &Request) -> AuthOutcome {
        self.authenticate_from(request, None).await
    }

    /// As [`authenticate`](Self::authenticate), with the HA1 of the user
    /// named taken from `remembered` rather than the store: what this node
    /// kept while the store answered, for an emergency call placed while it
    /// is away (#326). The nonce and its count are checked as ever.
    pub async fn authenticate_remembered(
        &self,
        request: &Request,
        remembered: &RememberedHa1,
    ) -> AuthOutcome {
        self.authenticate_from(request, Some(remembered)).await
    }

    async fn authenticate_from(
        &self,
        request: &Request,
        remembered: Option<&RememberedHa1>,
    ) -> AuthOutcome {
        let Some(auth_header) = request
            .headers()
            .get_smol("Authorization")
            .or_else(|| request.headers().get_smol("Proxy-Authorization"))
        else {
            return AuthOutcome::Failed;
        };

        let Some(auth) = parse_authorization_header(auth_header) else {
            debug!("Refusing credentials: the Authorization header does not parse");
            return AuthOutcome::Failed;
        };

        // Normalize username by trimming trailing @ (some buggy clients like sipsak add it)
        let (Some(username), Some(realm), Some(nonce), Some(uri), Some(response)) = (
            auth.param("username"),
            auth.param("realm"),
            auth.param("nonce"),
            auth.param("uri"),
            auth.param("response"),
        ) else {
            debug!("Refusing credentials: the Authorization header is missing a required field");
            return AuthOutcome::Failed;
        };
        let username = username.as_str().trim_end_matches('@');

        if realm.as_str() != self.realm.as_str() {
            debug!(realm = %realm, "Refusing credentials for another realm");
            return AuthOutcome::Failed;
        }

        // The "password" in credentials is actually the pre-computed HA1 hash.
        let creds = match remembered {
            Some(remembered) => remembered(username)
                .map(|ha1| sip_auth::Credentials::new(username, ha1, realm.as_str())),
            None => self.store.fetch(username, realm.as_str(), request).await,
        };
        let Some(creds) = creds else {
            debug!(username = %username, "Refusing credentials: no such user");
            return AuthOutcome::Failed;
        };

        let qop = auth.param("qop").and_then(|q| match q.as_str() {
            "auth" => Some(Qop::Auth),
            "auth-int" => Some(Qop::AuthInt),
            _ => None,
        });

        let expected_response = self.compute_response_from_ha1(
            creds.password(),
            request.method(),
            uri.as_str(),
            nonce.as_str(),
            auth.param("nc").map(|s| s.as_str()),
            auth.param("cnonce").map(|s| s.as_str()),
            qop,
            request.body().as_ref(),
        );

        if !fcp_core::secret::constant_time_eq(expected_response.as_bytes(), response.as_bytes()) {
            debug!(username = %username, "Refusing credentials: the digest does not match");
            return AuthOutcome::Failed;
        }

        // The password is proven. Now: is the nonce this node's, still
        // live, and not already spent on a different request?
        let nc = auth
            .param("nc")
            .and_then(|nc| u32::from_str_radix(nc.as_str().trim(), 16).ok())
            .unwrap_or(0);
        if self.nonces.verify_with_nc(
            nonce.as_str(),
            nc,
            request.method(),
            uri.as_str(),
            &request_identity(request),
        ) {
            // RFC 3261 §22.4: the `uri` signed is the Request-URI. Phones
            // were measured signing exactly that (#307); anything else is
            // a digest for some other request.
            if !digest_uri_matches(uri.as_str(), request.uri()) {
                let user_agent = request
                    .headers()
                    .get_smol("User-Agent")
                    .map(|v| v.as_str())
                    .unwrap_or("");
                if self.check_uri {
                    warn!(
                        username = %username,
                        method = %request.method().as_str(),
                        signed = %uri,
                        request_uri = %request.uri().as_str(),
                        user_agent = %user_agent,
                        "Refusing credentials: the digest was signed for another Request-URI"
                    );
                    return AuthOutcome::WrongUri {
                        username: username.to_string(),
                        signed: uri.to_string(),
                    };
                }
                info!(
                    username = %username,
                    method = %request.method().as_str(),
                    signed = %uri,
                    request_uri = %request.uri().as_str(),
                    user_agent = %user_agent,
                    "A digest signed for another Request-URI: accepted, digest_uri_check is off"
                );
            }
            AuthOutcome::Authenticated {
                username: username.to_string(),
            }
        } else {
            let known = self.nonces.is_stale(nonce.as_str());
            debug!(
                username = %username,
                nc = nc,
                issued_here = known,
                "Right credentials on a nonce that is not usable; challenging again as stale"
            );
            AuthOutcome::Stale
        }
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

/// Whether the `uri` a digest was computed over names the request's
/// Request-URI, as RFC 3261 §19.1.4 compares SIP URIs: scheme, user and
/// port exactly, the host without case, `user`, `ttl`, `method`, `maddr`
/// and `transport` equal when either has them, any other parameter equal
/// when both have it, and the same headers. A URI that is not SIP must be
/// the same text.
pub fn digest_uri_matches(signed: &str, request_uri: &sip_core::Uri) -> bool {
    if signed == request_uri.as_str() {
        return true;
    }
    let (Ok(a), Some(b)) = (sip_core::SipUri::parse(signed), request_uri.as_sip()) else {
        return false;
    };
    if a.is_sips() != b.is_sips()
        || a.user() != b.user()
        || !a.host().eq_ignore_ascii_case(b.host())
        || a.port() != b.port()
    {
        return false;
    }
    let param = |uri: &sip_core::SipUri, name: &str| {
        uri.params()
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_deref().unwrap_or("").to_ascii_lowercase())
    };
    const MUST_MATCH: [&str; 5] = ["user", "ttl", "method", "maddr", "transport"];
    for name in MUST_MATCH {
        if param(&a, name) != param(b, name) {
            return false;
        }
    }
    for name in a.params().keys() {
        if let Some(theirs) = param(b, name) {
            if param(&a, name) != Some(theirs) {
                return false;
            }
        }
    }
    a.headers() == b.headers()
}

/// What makes this request *this* request, for telling a retransmission
/// from a replay.
///
/// A digest response covers only the method, the request URI and (under
/// `qop=auth-int`) the body — never the headers that decide what the
/// request does. A captured `Authorization` replayed on a REGISTER that
/// names the attacker's own `Contact` is byte-identical where the digest
/// looks, so the nonce's retransmission check is given the fields that
/// actually differ.
fn request_identity(request: &Request) -> Vec<u8> {
    let mut identity = request.body().to_vec();
    for header in ["Contact", "CSeq", "Call-ID", "From", "To"] {
        identity.push(0);
        identity.extend_from_slice(header.as_bytes());
        identity.push(b':');
        if let Some(value) = request.headers().get_smol(header) {
            identity.extend_from_slice(value.as_bytes());
        }
    }
    identity
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request_uri(uri: &str) -> sip_core::Uri {
        sip_core::Uri::parse(uri).unwrap()
    }

    /// RFC 3261 §19.1.4's comparison for the digest's `uri` (#307).
    #[test]
    fn a_digest_uri_is_compared_as_rfc_3261_compares_uris() {
        let same = |a: &str, b: &str| digest_uri_matches(a, &request_uri(b));
        assert!(same("sip:e2e.local", "sip:e2e.local"));
        assert!(
            same("sip:E2E.Local", "sip:e2e.local"),
            "the host without case"
        );
        assert!(
            same("sip:alice@e2e.local;lr", "sip:alice@e2e.local"),
            "a parameter one side lacks"
        );
        assert!(same(
            "sip:e2e.local;TRANSPORT=TCP",
            "sip:e2e.local;transport=tcp"
        ));
        assert!(
            !same("sip:127.0.0.1:5070", "sip:e2e.local"),
            "SIPp's: another host"
        );
        assert!(
            !same("sip:e2e.local:5060", "sip:e2e.local"),
            "an explicit port is not none"
        );
        assert!(
            !same("sip:Alice@e2e.local", "sip:alice@e2e.local"),
            "the user with case"
        );
        assert!(!same("sips:e2e.local", "sip:e2e.local"));
        assert!(
            !same("sip:e2e.local;transport=tcp", "sip:e2e.local"),
            "transport on one side"
        );
        assert!(!same("sip:e2e.local;foo=1", "sip:e2e.local;foo=2"));
        assert!(!same("not a uri", "sip:e2e.local"));
    }

    struct OneUser {
        username: String,
        ha1: String,
    }

    #[async_trait]
    impl Ha1CredentialStore for OneUser {
        async fn fetch(
            &self,
            username: &str,
            realm: &str,
            _request: &Request,
        ) -> Option<sip_auth::Credentials> {
            (username == self.username)
                .then(|| sip_auth::Credentials::new(username, self.ha1.clone(), realm))
        }
    }

    fn ha1(user: &str, realm: &str, password: &str) -> String {
        format!("{:x}", md5::compute(format!("{user}:{realm}:{password}")))
    }

    fn authenticator() -> Ha1Authenticator<OneUser> {
        Ha1Authenticator::new(
            "fcp.local".to_string(),
            OneUser {
                username: "alice".to_string(),
                ha1: ha1("alice", "fcp.local", "secret"),
            },
        )
    }

    /// A REGISTER carrying a digest response computed over `nonce`.
    fn register(nonce: &str, nc: Option<&str>, contact: &str) -> Request {
        register_signing(nonce, nc, contact, "sip:fcp.local")
    }

    /// A `REGISTER sip:fcp.local` whose digest is computed over `uri`.
    fn register_signing(nonce: &str, nc: Option<&str>, contact: &str, uri: &str) -> Request {
        let ha1 = ha1("alice", "fcp.local", "secret");
        let ha2 = format!("{:x}", md5::compute(format!("REGISTER:{uri}")));
        let response = match nc {
            Some(nc) => format!(
                "{:x}",
                md5::compute(format!("{ha1}:{nonce}:{nc}:abc123:auth:{ha2}"))
            ),
            None => format!("{:x}", md5::compute(format!("{ha1}:{nonce}:{ha2}"))),
        };
        let qop = match nc {
            Some(nc) => format!(", qop=auth, nc={nc}, cnonce=\"abc123\""),
            None => String::new(),
        };
        let authorization = format!(
            "Digest username=\"alice\", realm=\"fcp.local\", nonce=\"{nonce}\", \
             uri=\"{uri}\", response=\"{response}\"{qop}"
        );
        let raw = format!(
            "REGISTER sip:fcp.local SIP/2.0\r\n\
             Via: SIP/2.0/UDP 10.0.0.9:5060;branch=z9hG4bK1\r\n\
             From: <sip:alice@fcp.local>;tag=1\r\n\
             To: <sip:alice@fcp.local>\r\n\
             Call-ID: call-1\r\n\
             CSeq: 2 REGISTER\r\n\
             Contact: <{contact}>\r\n\
             Authorization: {authorization}\r\n\
             Content-Length: 0\r\n\r\n"
        );
        sip_parse::parse_request(&bytes::Bytes::from(raw)).expect("test request parses")
    }

    /// The nonce from a challenge this authenticator issued.
    fn issued_nonce<S: Ha1CredentialStore>(
        auth: &Ha1Authenticator<S>,
        request: &Request,
    ) -> String {
        let challenge = auth.challenge(request).expect("challenge");
        let header = challenge
            .headers()
            .get_smol("WWW-Authenticate")
            .expect("challenge carries WWW-Authenticate");
        let (_, rest) = header.as_str().split_once("nonce=\"").expect("a nonce");
        rest.split('"').next().expect("a closed nonce").to_string()
    }

    #[tokio::test]
    async fn a_nonce_this_node_never_issued_is_not_proof() {
        // The credentials are genuine; the nonce is the client's invention.
        // Accepting it makes every captured Authorization header valid for
        // ever (`docs/EDGE_SECURITY.md` §2.7).
        let auth = authenticator();
        let request = register(
            "a-nonce-nobody-issued",
            Some("00000001"),
            "sip:alice@10.0.0.9",
        );
        assert!(matches!(
            auth.authenticate(&request).await,
            AuthOutcome::Stale
        ));
    }

    #[tokio::test]
    async fn a_nonce_this_node_issued_is_proof() {
        let auth = authenticator();
        let seed = register("unused", Some("00000001"), "sip:alice@10.0.0.9");
        let nonce = issued_nonce(&auth, &seed);
        let request = register(&nonce, Some("00000001"), "sip:alice@10.0.0.9");
        assert_eq!(
            auth.authenticate(&request).await,
            AuthOutcome::Authenticated {
                username: "alice".to_string()
            }
        );
    }

    /// A digest signed for another URI is still proof until what phones
    /// sign has been measured (#307); the mismatch is logged.
    #[tokio::test]
    async fn a_digest_for_another_uri_is_refused_unless_the_check_is_off() {
        let signed_elsewhere = |auth: &Ha1Authenticator<OneUser>| {
            let seed = register("unused", Some("00000001"), "sip:alice@10.0.0.9");
            let nonce = issued_nonce(auth, &seed);
            register_signing(
                &nonce,
                Some("00000001"),
                "sip:alice@10.0.0.9",
                "sip:127.0.0.1:5070",
            )
        };

        let auth = authenticator();
        let request = signed_elsewhere(&auth);
        assert!(!digest_uri_matches("sip:127.0.0.1:5070", request.uri()));
        assert_eq!(
            auth.authenticate(&request).await,
            AuthOutcome::WrongUri {
                username: "alice".into(),
                signed: "sip:127.0.0.1:5070".into(),
            }
        );

        let auth = authenticator().with_uri_check(false);
        let request = signed_elsewhere(&auth);
        assert!(matches!(
            auth.authenticate(&request).await,
            AuthOutcome::Authenticated { .. }
        ));
    }

    #[tokio::test]
    async fn a_captured_authorization_cannot_rebind_the_aor() {
        // The digest covers the method and the request URI, never the
        // Contact — so the attacker's REGISTER is byte-identical where the
        // digest looks, and only the nonce's own bookkeeping refuses it.
        let auth = authenticator();
        let seed = register("unused", Some("00000001"), "sip:alice@10.0.0.9");
        let nonce = issued_nonce(&auth, &seed);

        let device = register(&nonce, Some("00000001"), "sip:alice@10.0.0.9");
        assert!(matches!(
            auth.authenticate(&device).await,
            AuthOutcome::Authenticated { .. }
        ));

        let stolen = register(&nonce, Some("00000001"), "sip:alice@203.0.113.9");
        assert_eq!(
            stolen.headers().get_smol("Authorization"),
            device.headers().get_smol("Authorization")
        );
        assert_eq!(auth.authenticate(&stolen).await, AuthOutcome::Stale);
    }

    #[tokio::test]
    async fn a_retransmission_of_the_same_request_is_still_proof() {
        // UDP repeats itself; the same request on the same nonce-count is
        // the same request, not a replay.
        let auth = authenticator();
        let seed = register("unused", Some("00000001"), "sip:alice@10.0.0.9");
        let nonce = issued_nonce(&auth, &seed);
        let request = register(&nonce, Some("00000001"), "sip:alice@10.0.0.9");
        assert!(matches!(
            auth.authenticate(&request).await,
            AuthOutcome::Authenticated { .. }
        ));
        assert!(matches!(
            auth.authenticate(&request).await,
            AuthOutcome::Authenticated { .. }
        ));
    }

    #[tokio::test]
    async fn the_nonce_count_cannot_walk_backwards() {
        let auth = authenticator();
        let seed = register("unused", Some("00000002"), "sip:alice@10.0.0.9");
        let nonce = issued_nonce(&auth, &seed);
        let second = register(&nonce, Some("00000002"), "sip:alice@10.0.0.9");
        assert!(matches!(
            auth.authenticate(&second).await,
            AuthOutcome::Authenticated { .. }
        ));
        let first = register(&nonce, Some("00000001"), "sip:alice@10.0.0.9");
        assert_eq!(auth.authenticate(&first).await, AuthOutcome::Stale);
    }

    #[tokio::test]
    async fn an_expired_nonce_is_stale_not_a_refusal() {
        let auth = Ha1Authenticator::with_nonce_ttl(
            "fcp.local".to_string(),
            OneUser {
                username: "alice".to_string(),
                ha1: ha1("alice", "fcp.local", "secret"),
            },
            Duration::from_secs(0),
        );
        let seed = register("unused", Some("00000001"), "sip:alice@10.0.0.9");
        let nonce = issued_nonce(&auth, &seed);
        let request = register(&nonce, Some("00000001"), "sip:alice@10.0.0.9");
        assert_eq!(auth.authenticate(&request).await, AuthOutcome::Stale);
    }

    #[tokio::test]
    async fn a_wrong_password_is_refused_not_told_to_retry() {
        // Answering `stale=true` to a client that does not know the
        // password would loop it for ever without ever prompting anyone.
        let auth = Ha1Authenticator::new(
            "fcp.local".to_string(),
            OneUser {
                username: "alice".to_string(),
                ha1: ha1("alice", "fcp.local", "something-else"),
            },
        );
        let seed = register("unused", Some("00000001"), "sip:alice@10.0.0.9");
        let nonce = issued_nonce(&auth, &seed);
        let request = register(&nonce, Some("00000001"), "sip:alice@10.0.0.9");
        assert_eq!(auth.authenticate(&request).await, AuthOutcome::Failed);
    }

    #[tokio::test]
    async fn an_unknown_user_is_refused() {
        let auth = Ha1Authenticator::new(
            "fcp.local".to_string(),
            OneUser {
                username: "bob".to_string(),
                ha1: ha1("bob", "fcp.local", "secret"),
            },
        );
        let seed = register("unused", Some("00000001"), "sip:alice@10.0.0.9");
        let nonce = issued_nonce(&auth, &seed);
        let request = register(&nonce, Some("00000001"), "sip:alice@10.0.0.9");
        assert_eq!(auth.authenticate(&request).await, AuthOutcome::Failed);
    }

    #[tokio::test]
    async fn a_request_without_credentials_is_refused() {
        let auth = authenticator();
        let raw = "REGISTER sip:fcp.local SIP/2.0\r\n\
             Via: SIP/2.0/UDP 10.0.0.9:5060;branch=z9hG4bK1\r\n\
             From: <sip:alice@fcp.local>;tag=1\r\n\
             To: <sip:alice@fcp.local>\r\n\
             Call-ID: call-1\r\n\
             CSeq: 1 REGISTER\r\n\
             Content-Length: 0\r\n\r\n";
        let request =
            sip_parse::parse_request(&bytes::Bytes::from_static(raw.as_bytes())).expect("parses");
        assert_eq!(auth.authenticate(&request).await, AuthOutcome::Failed);
    }

    #[tokio::test]
    async fn a_stale_challenge_says_so() {
        let auth = authenticator();
        let request = register("unused", Some("00000001"), "sip:alice@10.0.0.9");
        let fresh = auth.challenge(&request).expect("challenge");
        let stale = auth.challenge_stale(&request).expect("challenge");
        let header = |r: &Response| {
            r.headers()
                .get_smol("WWW-Authenticate")
                .expect("WWW-Authenticate")
                .to_string()
        };
        assert!(!header(&fresh).contains("stale"));
        assert!(header(&stale).contains("stale=true"));
    }

    #[tokio::test]
    async fn a_client_that_sends_no_nonce_count_still_authenticates_once() {
        // RFC 2069-style clients send no qop, so every request carries the
        // same implicit nonce-count; the request identity is what keeps
        // one nonce from serving two different requests.
        let auth = authenticator();
        let seed = register("unused", None, "sip:alice@10.0.0.9");
        let nonce = issued_nonce(&auth, &seed);
        let request = register(&nonce, None, "sip:alice@10.0.0.9");
        assert!(matches!(
            auth.authenticate(&request).await,
            AuthOutcome::Authenticated { .. }
        ));
        let other = register(&nonce, None, "sip:alice@203.0.113.9");
        assert_eq!(auth.authenticate(&other).await, AuthOutcome::Stale);
    }
}
