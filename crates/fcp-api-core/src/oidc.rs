//! The OpenID Connect relying party (`docs/IDENTITY_AND_DEVICES.md` phase
//! 3): a provider's discovery document and keys, fetched through the same
//! guard as webhooks so a tenant's issuer cannot point the admin at its
//! own network; the authorization-code exchange with PKCE; and the ID
//! token verified — RS256 or ES256 against the issuer's JWKS, `iss`, `aud`
//! and `azp`, `exp` and `iat` with a minute's skew, and the `nonce` the
//! sign-in began with. Nothing else is accepted: no `none`, no HMAC.

use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use base64::Engine;
use rand::RngCore;
use serde::Deserialize;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// How long a discovery document and its keys are kept.
const CACHE_TTL: Duration = Duration::from_secs(3600);
/// The least time between fetches of keys for an unknown key id.
const JWKS_REFETCH: Duration = Duration::from_secs(60);
/// The clock skew allowed between us and the provider.
const SKEW_SECS: u64 = 60;
/// How old a back-channel logout token may be.
const LOGOUT_TOKEN_WINDOW_SECS: u64 = 300;
/// The event a logout token carries.
const LOGOUT_EVENT: &str = "http://schemas.openid.net/event/backchannel-logout";
/// The weakest RSA key accepted.
const MIN_RSA_BITS: usize = 2048;

/// Why a sign-in through a provider failed. The reason names what the
/// failed sign-in event and the login page say.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OidcError {
    /// The provider could not be reached or answered nonsense.
    Provider(String),
    /// The provider refused the sign-in (`error=` on the callback, or the
    /// token endpoint said no).
    Refused(String),
    /// The ID token did not verify.
    BadToken(String),
}

impl std::fmt::Display for OidcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Provider(e) => write!(f, "identity provider unavailable: {e}"),
            Self::Refused(e) => write!(f, "identity provider refused: {e}"),
            Self::BadToken(e) => write!(f, "ID token refused: {e}"),
        }
    }
}

impl OidcError {
    /// The short reason for events and the login page.
    pub fn reason(&self) -> &'static str {
        match self {
            Self::Provider(_) => "provider_unavailable",
            Self::Refused(_) => "provider_refused",
            Self::BadToken(_) => "bad_id_token",
        }
    }
}

/// What a provider's discovery document says, as much as is used.
#[derive(Debug, Clone, Deserialize)]
pub struct Discovery {
    pub issuer: String,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    pub jwks_uri: String,
    #[serde(default)]
    pub end_session_endpoint: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct Jwk {
    kty: String,
    #[serde(default)]
    kid: Option<String>,
    #[serde(default, rename = "use")]
    use_: Option<String>,
    #[serde(default)]
    n: Option<String>,
    #[serde(default)]
    e: Option<String>,
    #[serde(default)]
    crv: Option<String>,
    #[serde(default)]
    x: Option<String>,
    #[serde(default)]
    y: Option<String>,
}

#[derive(Debug, Deserialize)]
struct JwkSet {
    keys: Vec<Jwk>,
}

struct Cached {
    discovery: Discovery,
    keys: Vec<Jwk>,
    fetched: Instant,
    keys_fetched: Instant,
}

/// The relying party: an HTTP client that can reach only public addresses
/// (unless the deployment says otherwise), and what it has fetched.
pub struct OidcClient {
    http: reqwest::Client,
    allow_private: bool,
    cache: Mutex<HashMap<String, Cached>>,
}

/// A sign-in's secrets: the `state`, the `nonce` and the PKCE verifier.
pub struct Begun {
    pub state: String,
    pub nonce: String,
    pub code_verifier: String,
}

/// Thirty-two random bytes, base64url.
pub fn random_token() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

impl Begun {
    pub fn new() -> Self {
        Self {
            state: random_token(),
            nonce: random_token(),
            code_verifier: random_token(),
        }
    }

    /// The PKCE challenge (`S256`).
    pub fn challenge(verifier: &str) -> String {
        URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
    }
}

impl Default for Begun {
    fn default() -> Self {
        Self::new()
    }
}

impl OidcClient {
    /// `allow_private` lets an issuer be a private or loopback address
    /// over plain HTTP: a lab, or a test's stand-in provider.
    pub fn new(allow_private: bool) -> Self {
        let http = reqwest::Client::builder()
            .dns_resolver(std::sync::Arc::new(fcp_webhooks::target::GuardedResolver {
                allow_private,
            }))
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(10))
            .build()
            .expect("an HTTP client with a resolver builds");
        Self {
            http,
            allow_private,
            cache: Mutex::new(HashMap::new()),
        }
    }

    /// Whether `issuer` is a URL this client would fetch from.
    pub fn check_issuer(&self, issuer: &str) -> Result<(), String> {
        self.check_url(issuer).map(|_| ())
    }

    fn check_url(&self, raw: &str) -> Result<url::Url, String> {
        let url = fcp_webhooks::target::check_url(raw, self.allow_private)
            .map_err(|e| e.0.replace("a webhook", "an identity provider"))?;
        if !fcp_webhooks::target::literal_target_allowed(raw, self.allow_private) {
            return Err(format!("{raw} is not a public address"));
        }
        Ok(url)
    }

    /// A document at `url` as text, fetched through the guard: a SAML
    /// provider's metadata. At most a megabyte.
    pub async fn fetch_text(&self, url: &str) -> Result<String, OidcError> {
        self.check_url(url).map_err(OidcError::Provider)?;
        let response = self
            .http
            .get(url)
            .send()
            .await
            .map_err(|e| OidcError::Provider(format!("{url}: {e}")))?;
        if !response.status().is_success() {
            return Err(OidcError::Provider(format!("{url}: {}", response.status())));
        }
        let body = response
            .bytes()
            .await
            .map_err(|e| OidcError::Provider(format!("{url}: {e}")))?;
        if body.len() > 1 << 20 {
            return Err(OidcError::Provider(format!(
                "{url}: larger than a megabyte"
            )));
        }
        String::from_utf8(body.to_vec())
            .map_err(|_| OidcError::Provider(format!("{url}: not text")))
    }

    async fn get_json<T: serde::de::DeserializeOwned>(&self, url: &str) -> Result<T, OidcError> {
        self.check_url(url).map_err(OidcError::Provider)?;
        let response = self
            .http
            .get(url)
            .send()
            .await
            .map_err(|e| OidcError::Provider(format!("{url}: {e}")))?;
        if !response.status().is_success() {
            return Err(OidcError::Provider(format!("{url}: {}", response.status())));
        }
        response
            .json()
            .await
            .map_err(|e| OidcError::Provider(format!("{url}: {e}")))
    }

    async fn fetch_keys(&self, jwks_uri: &str) -> Result<Vec<Jwk>, OidcError> {
        Ok(self.get_json::<JwkSet>(jwks_uri).await?.keys)
    }

    /// The provider's discovery document, fetched or remembered.
    pub async fn discovery(&self, issuer: &str) -> Result<Discovery, OidcError> {
        let issuer = issuer.trim();
        if let Some(c) = self.cache.lock().unwrap().get(issuer) {
            if c.fetched.elapsed() < CACHE_TTL {
                return Ok(c.discovery.clone());
            }
        }
        let url = format!(
            "{}/.well-known/openid-configuration",
            issuer.trim_end_matches('/')
        );
        let discovery: Discovery = self.get_json(&url).await?;
        if discovery.issuer.trim_end_matches('/') != issuer.trim_end_matches('/') {
            return Err(OidcError::Provider(format!(
                "the discovery document names the issuer {}, not {issuer}",
                discovery.issuer
            )));
        }
        for endpoint in [
            &discovery.authorization_endpoint,
            &discovery.token_endpoint,
            &discovery.jwks_uri,
        ] {
            self.check_url(endpoint).map_err(OidcError::Provider)?;
        }
        let keys = self.fetch_keys(&discovery.jwks_uri).await?;
        let now = Instant::now();
        self.cache.lock().unwrap().insert(
            issuer.to_string(),
            Cached {
                discovery: discovery.clone(),
                keys,
                fetched: now,
                keys_fetched: now,
            },
        );
        Ok(discovery)
    }

    /// Where the browser is sent to sign in.
    pub fn authorize_url(
        &self,
        discovery: &Discovery,
        settings: &fcp_core::identity::ProviderSettings,
        redirect_uri: &str,
        begun: &Begun,
    ) -> Result<String, OidcError> {
        let mut url = url::Url::parse(&discovery.authorization_endpoint)
            .map_err(|e| OidcError::Provider(e.to_string()))?;
        let mut scopes = vec!["openid".to_string()];
        scopes.extend(
            settings
                .scopes
                .iter()
                .filter(|s| s.as_str() != "openid")
                .cloned(),
        );
        url.query_pairs_mut()
            .append_pair("response_type", "code")
            .append_pair("client_id", &settings.client_id)
            .append_pair("redirect_uri", redirect_uri)
            .append_pair("scope", &scopes.join(" "))
            .append_pair("state", &begun.state)
            .append_pair("nonce", &begun.nonce)
            .append_pair("code_challenge", &Begun::challenge(&begun.code_verifier))
            .append_pair("code_challenge_method", "S256");
        Ok(url.into())
    }

    /// Trade the callback's code for the ID token, and verify it: its
    /// claims, and the token itself (for `id_token_hint` at sign-out).
    #[allow(clippy::too_many_arguments)]
    pub async fn sign_in(
        &self,
        settings: &fcp_core::identity::ProviderSettings,
        client_secret: Option<&str>,
        code: &str,
        code_verifier: &str,
        redirect_uri: &str,
        nonce: &str,
    ) -> Result<(Map<String, Value>, String), OidcError> {
        let discovery = self.discovery(&settings.issuer).await?;
        self.check_url(&discovery.token_endpoint)
            .map_err(OidcError::Provider)?;
        let mut form = vec![
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", redirect_uri),
            ("code_verifier", code_verifier),
        ];
        let mut request = self.http.post(&discovery.token_endpoint);
        match client_secret {
            // client_secret_basic, which every provider must take (RFC 6749
            // §2.3.1), its parts form-encoded.
            Some(secret) => {
                let credentials = format!(
                    "{}:{}",
                    urlencoding::encode(&settings.client_id),
                    urlencoding::encode(secret)
                );
                request = request.header(
                    reqwest::header::AUTHORIZATION,
                    format!("Basic {}", STANDARD.encode(credentials)),
                );
            }
            None => form.push(("client_id", &settings.client_id)),
        }
        let response = request
            .form(&form)
            .send()
            .await
            .map_err(|e| OidcError::Provider(format!("token endpoint: {e}")))?;
        let status = response.status();
        let body: Value = response
            .json()
            .await
            .map_err(|e| OidcError::Provider(format!("token endpoint: {e}")))?;
        if !status.is_success() {
            let error = body["error"].as_str().unwrap_or("error").to_string();
            return Err(OidcError::Refused(error));
        }
        let id_token = body["id_token"]
            .as_str()
            .ok_or_else(|| OidcError::Provider("the token endpoint gave no id_token".into()))?;
        let claims = self
            .verify(
                &discovery,
                &settings.client_id,
                id_token,
                nonce,
                SystemTime::now(),
            )
            .await?;
        Ok((claims, id_token.to_string()))
    }

    /// Where to send the browser to sign out at the provider (OpenID
    /// Connect RP-Initiated Logout 1.0): its `end_session_endpoint` with
    /// the ID token as the hint, the client, and the page to come back to.
    /// `None` when the provider has no such endpoint.
    pub async fn end_session_url(
        &self,
        settings: &fcp_core::identity::ProviderSettings,
        id_token: Option<&str>,
        post_logout_redirect_uri: &str,
    ) -> Result<Option<String>, OidcError> {
        let discovery = self.discovery(&settings.issuer).await?;
        let Some(endpoint) = discovery.end_session_endpoint.as_deref() else {
            return Ok(None);
        };
        let mut url = url::Url::parse(endpoint).map_err(|e| OidcError::Provider(e.to_string()))?;
        {
            let mut query = url.query_pairs_mut();
            if let Some(token) = id_token {
                query.append_pair("id_token_hint", token);
            }
            query
                .append_pair("client_id", &settings.client_id)
                .append_pair("post_logout_redirect_uri", post_logout_redirect_uri);
        }
        Ok(Some(url.into()))
    }

    /// The claims of an ID token from `discovery`'s issuer for
    /// `client_id`, if it verifies.
    pub async fn verify(
        &self,
        discovery: &Discovery,
        client_id: &str,
        token: &str,
        nonce: &str,
        now: SystemTime,
    ) -> Result<Map<String, Value>, OidcError> {
        let bad = |e: &str| OidcError::BadToken(e.to_string());
        let claims = self.issued_to(discovery, client_id, token).await?;
        let t = now.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
        match claims.get("exp").and_then(Value::as_u64) {
            Some(exp) if exp + SKEW_SECS > t => {}
            _ => return Err(bad("expired")),
        }
        if claims
            .get("iat")
            .and_then(Value::as_u64)
            .is_some_and(|iat| iat > t + SKEW_SECS)
        {
            return Err(bad("issued in the future"));
        }
        match claims.get("nonce").and_then(Value::as_str) {
            Some(n) if fcp_core::secret::constant_time_eq(n, nonce) => {}
            _ => return Err(bad("not this sign-in's (nonce)")),
        }
        if claims
            .get("sub")
            .and_then(Value::as_str)
            .is_none_or(str::is_empty)
        {
            return Err(bad("no subject"));
        }
        Ok(claims)
    }

    /// A back-channel logout token's claims, if it verifies (OpenID
    /// Connect Back-Channel Logout 1.0 §2.6): signed like an ID token, for
    /// this client, issued in the last few minutes, the logout event, no
    /// `nonce`, and a `sub`, a `sid` or both.
    pub async fn verify_logout(
        &self,
        discovery: &Discovery,
        client_id: &str,
        token: &str,
        now: SystemTime,
    ) -> Result<Map<String, Value>, OidcError> {
        let bad = |e: &str| OidcError::BadToken(e.to_string());
        let claims = self.issued_to(discovery, client_id, token).await?;
        let t = now.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
        // Fresh: a captured token replayed later must not sign people out.
        match claims.get("iat").and_then(Value::as_u64) {
            Some(iat) if iat + LOGOUT_TOKEN_WINDOW_SECS > t && iat <= t + SKEW_SECS => {}
            _ => return Err(bad("not a fresh logout token")),
        }
        if claims
            .get("exp")
            .and_then(Value::as_u64)
            .is_some_and(|exp| exp + SKEW_SECS <= t)
        {
            return Err(bad("expired"));
        }
        let event = claims
            .get("events")
            .and_then(Value::as_object)
            .is_some_and(|e| e.contains_key(LOGOUT_EVENT));
        if !event {
            return Err(bad("not a logout event"));
        }
        if claims.contains_key("nonce") {
            return Err(bad("a logout token carries no nonce"));
        }
        let named = |c: &str| {
            claims
                .get(c)
                .and_then(Value::as_str)
                .is_some_and(|s| !s.is_empty())
        };
        if !named("sub") && !named("sid") {
            return Err(bad("a logout token names a sub or a sid"));
        }
        Ok(claims)
    }

    /// A JWS's claims if a key of the issuer's signed it, the issuer is
    /// `discovery`'s, and it is for `client_id`.
    async fn issued_to(
        &self,
        discovery: &Discovery,
        client_id: &str,
        token: &str,
    ) -> Result<Map<String, Value>, OidcError> {
        let bad = |e: &str| OidcError::BadToken(e.to_string());
        let mut parts = token.split('.');
        let (Some(h), Some(p), Some(s), None) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            return Err(bad("not a JWS"));
        };
        let header: Value =
            serde_json::from_slice(&URL_SAFE_NO_PAD.decode(h).map_err(|_| bad("header"))?)
                .map_err(|_| bad("header"))?;
        let alg = header["alg"].as_str().unwrap_or_default().to_string();
        if alg != "RS256" && alg != "ES256" {
            return Err(OidcError::BadToken(format!("alg {alg} is not accepted")));
        }
        let kid = header["kid"].as_str().map(str::to_string);
        let signature = URL_SAFE_NO_PAD.decode(s).map_err(|_| bad("signature"))?;
        let input = format!("{h}.{p}");

        let key = match self.key_for(discovery, &alg, kid.as_deref(), false) {
            Some(key) => key,
            None => {
                // A key the provider has rotated in since: fetch once more.
                self.refresh_keys(discovery).await?;
                self.key_for(discovery, &alg, kid.as_deref(), true)
                    .ok_or_else(|| bad("no key of the provider's signed it"))?
            }
        };
        verify_signature(&alg, &key, input.as_bytes(), &signature)?;

        let claims: Map<String, Value> =
            serde_json::from_slice(&URL_SAFE_NO_PAD.decode(p).map_err(|_| bad("payload"))?)
                .map_err(|_| bad("payload"))?;
        if claims.get("iss").and_then(Value::as_str) != Some(discovery.issuer.as_str()) {
            return Err(bad("another issuer's token"));
        }
        let audiences: Vec<&str> = match claims.get("aud") {
            Some(Value::String(a)) => vec![a.as_str()],
            Some(Value::Array(list)) => list.iter().filter_map(Value::as_str).collect(),
            _ => Vec::new(),
        };
        if !audiences.contains(&client_id) {
            return Err(bad("issued to another client"));
        }
        if audiences.len() > 1 && claims.get("azp").and_then(Value::as_str) != Some(client_id) {
            return Err(bad("issued to another party"));
        }
        Ok(claims)
    }

    fn key_for(
        &self,
        discovery: &Discovery,
        alg: &str,
        kid: Option<&str>,
        last_try: bool,
    ) -> Option<Jwk> {
        let cache = self.cache.lock().unwrap();
        let keys = &cache
            .get(discovery.issuer.trim_end_matches('/'))
            .or_else(|| cache.get(&discovery.issuer))?
            .keys;
        let kty = if alg == "RS256" { "RSA" } else { "EC" };
        let fits = |k: &&Jwk| k.kty == kty && k.use_.as_deref().is_none_or(|u| u == "sig");
        match kid {
            Some(kid) => keys
                .iter()
                .filter(fits)
                .find(|k| k.kid.as_deref() == Some(kid))
                .cloned(),
            // No key id: only a provider with one key of the kind may omit it.
            None => {
                let mut candidates = keys.iter().filter(fits);
                let only = candidates.next().cloned();
                (candidates.next().is_none() || last_try)
                    .then_some(only)
                    .flatten()
            }
        }
    }

    async fn refresh_keys(&self, discovery: &Discovery) -> Result<(), OidcError> {
        let key = discovery.issuer.trim_end_matches('/').to_string();
        let due = self
            .cache
            .lock()
            .unwrap()
            .get(&key)
            .is_none_or(|c| c.keys_fetched.elapsed() >= JWKS_REFETCH);
        if !due {
            return Ok(());
        }
        let keys = self.fetch_keys(&discovery.jwks_uri).await?;
        let now = Instant::now();
        let mut cache = self.cache.lock().unwrap();
        match cache.get_mut(&key) {
            Some(c) => {
                c.keys = keys;
                c.keys_fetched = now;
            }
            None => {
                cache.insert(
                    key,
                    Cached {
                        discovery: discovery.clone(),
                        keys,
                        fetched: now,
                        keys_fetched: now,
                    },
                );
            }
        }
        Ok(())
    }
}

fn verify_signature(alg: &str, key: &Jwk, input: &[u8], signature: &[u8]) -> Result<(), OidcError> {
    let bad = |e: &str| OidcError::BadToken(e.to_string());
    let field = |v: &Option<String>| {
        v.as_deref()
            .and_then(|s| URL_SAFE_NO_PAD.decode(s).ok())
            .ok_or_else(|| bad("a malformed key"))
    };
    match alg {
        "RS256" => {
            use rsa::signature::Verifier;
            use rsa::traits::PublicKeyParts;
            let public = rsa::RsaPublicKey::new(
                rsa::BigUint::from_bytes_be(&field(&key.n)?),
                rsa::BigUint::from_bytes_be(&field(&key.e)?),
            )
            .map_err(|_| bad("a malformed RSA key"))?;
            if public.size() * 8 < MIN_RSA_BITS {
                return Err(bad("an RSA key under 2048 bits"));
            }
            let signature =
                rsa::pkcs1v15::Signature::try_from(signature).map_err(|_| bad("signature"))?;
            rsa::pkcs1v15::VerifyingKey::<Sha256>::new(public)
                .verify(input, &signature)
                .map_err(|_| bad("the signature does not verify"))
        }
        "ES256" => {
            use p256::ecdsa::signature::Verifier;
            if key.crv.as_deref() != Some("P-256") {
                return Err(bad("an EC key not on P-256"));
            }
            let (x, y) = (field(&key.x)?, field(&key.y)?);
            if x.len() != 32 || y.len() != 32 {
                return Err(bad("a malformed EC key"));
            }
            // An uncompressed SEC1 point: 0x04, then x and y.
            let point = [&[0x04u8][..], &x, &y].concat();
            let public = p256::ecdsa::VerifyingKey::from_sec1_bytes(&point)
                .map_err(|_| bad("a malformed EC key"))?;
            let signature =
                p256::ecdsa::Signature::from_slice(signature).map_err(|_| bad("signature"))?;
            public
                .verify(input, &signature)
                .map_err(|_| bad("the signature does not verify"))
        }
        _ => Err(bad("alg")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fcp_testkit::oidc::StandInIdp;
    use serde_json::json;

    async fn provider() -> (StandInIdp, OidcClient, Discovery) {
        let idp = StandInIdp::start("fcp", Some("s3cret")).await.unwrap();
        let client = OidcClient::new(true);
        let discovery = client.discovery(&idp.issuer).await.unwrap();
        (idp, client, discovery)
    }

    fn claims(idp: &StandInIdp, extra: Value) -> Value {
        let t = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let mut c = json!({"iss": idp.issuer, "aud": "fcp", "sub": "u1", "iat": t, "exp": t + 300, "nonce": "n1"});
        c.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        c
    }

    #[tokio::test]
    async fn a_good_token_verifies_and_every_wrong_one_is_refused() {
        let (idp, client, d) = provider().await;
        let verify = |token: String| {
            let (client, d) = (&client, &d);
            async move {
                client
                    .verify(d, "fcp", &token, "n1", SystemTime::now())
                    .await
            }
        };
        assert_eq!(
            verify(idp.sign(&claims(&idp, json!({})))).await.unwrap()["sub"],
            "u1"
        );

        let t = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        for (why, extra) in [
            ("issuer", json!({"iss": "https://elsewhere"})),
            ("audience", json!({"aud": "another"})),
            ("azp", json!({"aud": ["fcp", "other"]})),
            ("expired", json!({"exp": t - 3600})),
            ("future", json!({"iat": t + 3600})),
            ("nonce", json!({"nonce": "n2"})),
            ("subject", json!({"sub": ""})),
        ] {
            let refused = verify(idp.sign(&claims(&idp, extra))).await;
            assert!(
                matches!(refused, Err(OidcError::BadToken(_))),
                "{why}: {refused:?}"
            );
        }
        assert!(verify(idp.sign(&claims(
            &idp,
            json!({"aud": ["fcp", "other"], "azp": "fcp"})
        )))
        .await
        .is_ok());

        // A signature over other claims, and an unsigned token.
        let good = idp.sign(&claims(&idp, json!({})));
        let forged_payload =
            URL_SAFE_NO_PAD.encode(claims(&idp, json!({"sub": "admin"})).to_string());
        let parts: Vec<&str> = good.split('.').collect();
        assert!(
            verify(format!("{}.{forged_payload}.{}", parts[0], parts[2]))
                .await
                .is_err()
        );
        let none = URL_SAFE_NO_PAD.encode(r#"{"alg":"none"}"#);
        assert!(verify(format!("{none}.{}.", parts[1])).await.is_err());
        let hs = URL_SAFE_NO_PAD.encode(r#"{"alg":"HS256","kid":"fcp-test-1"}"#);
        assert!(verify(format!("{hs}.{}.{}", parts[1], parts[2]))
            .await
            .is_err());
    }

    #[tokio::test]
    async fn a_logout_token_is_fresh_names_someone_and_is_no_id_token() {
        let (idp, client, d) = provider().await;
        let verify = |token: String| {
            let (client, d) = (&client, &d);
            async move {
                client
                    .verify_logout(d, "fcp", &token, SystemTime::now())
                    .await
            }
        };
        assert_eq!(
            verify(idp.logout_token(Some("u1"), None)).await.unwrap()["sub"],
            "u1"
        );
        assert!(verify(idp.logout_token(None, Some("s1"))).await.is_ok());
        assert!(verify(idp.logout_token(None, None)).await.is_err());

        let t = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let event = json!({"http://schemas.openid.net/event/backchannel-logout": {}});
        // An ID token (no event, a nonce) is not a logout token.
        assert!(verify(idp.sign(&claims(&idp, json!({})))).await.is_err());
        let stale =
            json!({"iss": idp.issuer, "aud": "fcp", "sub": "u1", "iat": t - 3600, "events": event});
        assert!(verify(idp.sign(&stale)).await.is_err());
        let with_nonce = json!({"iss": idp.issuer, "aud": "fcp", "sub": "u1", "iat": t, "events": event, "nonce": "n"});
        assert!(verify(idp.sign(&with_nonce)).await.is_err());
        let other =
            json!({"iss": idp.issuer, "aud": "other", "sub": "u1", "iat": t, "events": event});
        assert!(verify(idp.sign(&other)).await.is_err());
    }

    #[tokio::test]
    async fn an_issuer_must_be_public_unless_the_deployment_allows_it() {
        let strict = OidcClient::new(false);
        assert!(strict.check_issuer("http://127.0.0.1:8080").is_err());
        assert!(strict.check_issuer("https://10.0.0.1").is_err());
        assert!(strict.check_issuer("http://login.example.com").is_err());
        assert!(strict
            .check_issuer("https://login.example.com/tenant/v2.0")
            .is_ok());
        assert!(OidcClient::new(true)
            .check_issuer("http://127.0.0.1:8080")
            .is_ok());
    }

    #[test]
    fn the_challenge_is_rfc_7636s() {
        // RFC 7636 appendix B.
        assert_eq!(
            Begun::challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }
}
