//! A stand-in OpenID Connect provider for tests
//! (`docs/IDENTITY_AND_DEVICES.md` phase 3): discovery, a JWKS, an
//! authorization endpoint that signs in whoever the test says, and a token
//! endpoint that checks the client and the PKCE verifier and answers an ID
//! token signed RS256 with a fixed test key — what Entra ID, Okta and
//! Google sign with.

use axum::extract::{Form, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use base64::Engine;
use rsa::pkcs1v15::SigningKey;
use rsa::pkcs8::DecodePrivateKey;
use rsa::signature::{SignatureEncoding, Signer};
use rsa::traits::PublicKeyParts;
use rsa::RsaPrivateKey;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

/// The provider's signing key: a throwaway made for these tests, never a
/// secret of anything.
const TEST_KEY: &str = "
-----BEGIN PRIVATE KEY-----
MIIEvgIBADANBgkqhkiG9w0BAQEFAASCBKgwggSkAgEAAoIBAQDL6aoFM9IcPMws
CYmvnB0pO5+LCCJa5ngIBsBBwy7W3nFbesNipeOte+3UVFy4GThPuUmjwf8zu/Si
GO8vm7iMXZenAQStXQOdZ4hHnWvMb6W3Lj8WeMqA2A20VHQVK93ZNkvzGzxrsbzJ
1njpyoVin4Dy8bQZiLK71GXg/KIuuoHsV5xe9E5jkoycRw45FOFj81nux3wSk7vu
wVA4166F7Ge8r92P+5fvwIZsIRL2MpiOp0IeRThHxLh29aSf6lbLOgqw1qB/6e2z
Cf+pNhcnzAKH9BIdyl+Vzd8C88qh7SKqYFJh4IgBJJKUOsm60NTKSinjPi5f6DuQ
utj3SOpnAgMBAAECggEALWjXOnpcT3qAk0v6c/eFjPGgqWMB15FcaJksRljTfPkj
cFZJ3ZWIutEO2QqCbWL8ZdCi5AtnLBuuskxwM39vnR6Yj5HL+1lr3Sk5sY0OfQPd
+Qmc9gGnQwMaJi7qfO58ZItRZY2Kqob1vlL7MUEu7BQOMMv/qfIFr4twtqY77cCo
AOnClJsP6F6NBYopTg/hXLZsExY07qk06ZkJkDOVnZsT3ZCyWqFJuvCRy7O13vYz
cYqfy5ORhR4z1pN7F/ubxfLMF51C0YWznFiLCewan6LR6BJUeTVTlCz5sQlDRnQb
FgSPYfNYchqHzWCjmXPdSM+1Q54uOWzGjJw2si4lsQKBgQD0cxyl9iDzD7VEkF3R
JgbBvMSolrQX6kqnqKPwl6ZzRv1dmYqIGefcANI9EM9AVbJ0YzRzDyBO3u1rcq6+
Dm4ltlLG2rJm5W+P/yUeHucu2GaM1Pf3HBmu1+6AmA5hYn3qFR8l8oTxAQvRTTXK
SezvC97R1lDcwE+Onj3mQbA5twKBgQDVjDazQbT+SIihlQ0RiLvHiPIcQWG4Nyhd
PQ5G3dZUQ7gRKy2vSD/F24IAsp7O7D2eh8zFHCcy+zurnlCtPJV2rsm+NjlfJ0cw
MS+Jc2E8ojLsfxqDA7GAQjwgjz3YvhBgfr0DhVCeryVlE1Yf6pAgfmvwYxjyGlRq
cW03o1SU0QKBgQDqVYNMEVNqzk8dqCtXeNWXWYdIehewqVfhu0dVN3tIHPB7jtn/
OJ5NB7iC859J9Bz7d7Jq7kAxe4ErtKeQ6DJtEoftuu60txMofhWt9z/1Z1t/fBUt
HRH9y2AGhD2c25cLFC5k6UTJ6A/VF+gNwKpSShXTIFkqFWzusgFyvtvZpwKBgBU9
CrutT0jzAdPANTqHNNXVgu7YedWOds8/IUpfCc/7pDTg7HxAXUdaW4WiPjUqPb6k
lvMLp8JkRjz9U9zG576UB7CJSA90opniam3rMcKMBzCKO6LIHUE6VSyP0oDZ9lsC
f933xlrh5IA2+hvWglsY97dCw9pgUAD+IvSjo+fhAoGBAN03nOLnmXIMnsMmyilE
hWeAaHiyQkKIr0+LFr9mJYVBlCxMnBMMld1xuYqJiM3Y9brqsB2wsUhW/7PT5995
LeLp+GCKK767U2Sbm/HqMXAFpaneWcSCplkmMrPx76T2wajuxT/3OBPKsZeyl8qf
cuoSKQw7hPUpypWUIszPlIDq
-----END PRIVATE KEY-----
";

/// The key id the JWKS names it by.
pub const KID: &str = "fcp-test-1";

struct Pending {
    claims: Value,
    nonce: Option<String>,
    challenge: Option<String>,
    redirect_uri: String,
}

struct Inner {
    issuer: String,
    client_id: String,
    client_secret: Option<String>,
    key: RsaPrivateKey,
    next: Mutex<Option<Value>>,
    codes: Mutex<HashMap<String, Pending>>,
    overrides: Mutex<serde_json::Map<String, Value>>,
}

/// A provider listening on loopback.
pub struct StandInIdp {
    /// The issuer: `http://127.0.0.1:<port>`.
    pub issuer: String,
    inner: Arc<Inner>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for StandInIdp {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl StandInIdp {
    /// A provider with one client registered.
    pub async fn start(client_id: &str, client_secret: Option<&str>) -> anyhow::Result<Self> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let issuer = format!("http://{}", listener.local_addr()?);
        let inner = Arc::new(Inner {
            issuer: issuer.clone(),
            client_id: client_id.to_string(),
            client_secret: client_secret.map(str::to_string),
            key: RsaPrivateKey::from_pkcs8_pem(TEST_KEY.trim())?,
            next: Mutex::new(None),
            codes: Mutex::new(HashMap::new()),
            overrides: Mutex::new(serde_json::Map::new()),
        });
        let app = Router::new()
            .route("/.well-known/openid-configuration", get(discovery))
            .route("/jwks", get(jwks))
            .route("/authorize", get(authorize))
            .route("/token", post(token))
            .with_state(Arc::clone(&inner));
        let task = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        Ok(Self {
            issuer,
            inner,
            task,
        })
    }

    /// Whom the next authorization signs in: the ID token's claims (`sub`
    /// at least). Unset, the provider answers `access_denied`.
    pub fn sign_in_as(&self, claims: Value) {
        *self.inner.next.lock().unwrap() = Some(claims);
    }

    /// Claims every ID token is given over its usual ones — a wrong `aud`,
    /// an `exp` in the past — to see a client refuse them.
    pub fn override_claims(&self, claims: Value) {
        let mut overrides = self.inner.overrides.lock().unwrap();
        overrides.clear();
        if let Value::Object(map) = claims {
            overrides.extend(map);
        }
    }

    /// A JWS of `claims` signed with the provider's key (`alg` RS256).
    pub fn sign(&self, claims: &Value) -> String {
        sign(&self.inner.key, claims)
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn sign(key: &RsaPrivateKey, claims: &Value) -> String {
    let header =
        URL_SAFE_NO_PAD.encode(json!({"alg": "RS256", "typ": "JWT", "kid": KID}).to_string());
    let payload = URL_SAFE_NO_PAD.encode(claims.to_string());
    let input = format!("{header}.{payload}");
    let signature = SigningKey::<Sha256>::new(key.clone()).sign(input.as_bytes());
    format!("{input}.{}", URL_SAFE_NO_PAD.encode(signature.to_bytes()))
}

async fn discovery(State(idp): State<Arc<Inner>>) -> Json<Value> {
    let base = &idp.issuer;
    Json(json!({
        "issuer": base,
        "authorization_endpoint": format!("{base}/authorize"),
        "token_endpoint": format!("{base}/token"),
        "jwks_uri": format!("{base}/jwks"),
        "response_types_supported": ["code"],
        "subject_types_supported": ["public"],
        "id_token_signing_alg_values_supported": ["RS256"],
        "code_challenge_methods_supported": ["S256"],
    }))
}

async fn jwks(State(idp): State<Arc<Inner>>) -> Json<Value> {
    let public = idp.key.to_public_key();
    Json(json!({"keys": [{
        "kty": "RSA",
        "use": "sig",
        "alg": "RS256",
        "kid": KID,
        "n": URL_SAFE_NO_PAD.encode(public.n().to_bytes_be()),
        "e": URL_SAFE_NO_PAD.encode(public.e().to_bytes_be()),
    }]}))
}

async fn authorize(
    State(idp): State<Arc<Inner>>,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let Some(redirect_uri) = q.get("redirect_uri").cloned() else {
        return (StatusCode::BAD_REQUEST, "redirect_uri missing").into_response();
    };
    let state = q.get("state").cloned().unwrap_or_default();
    let back = |params: &[(&str, &str)]| {
        let mut url = url_with(&redirect_uri, params);
        url.push_str(&format!("&state={}", urlencode(&state)));
        Redirect::to(&url).into_response()
    };
    if q.get("client_id") != Some(&idp.client_id)
        || q.get("response_type").map(String::as_str) != Some("code")
    {
        return back(&[("error", "unauthorized_client")]);
    }
    let Some(claims) = idp.next.lock().unwrap().clone() else {
        return back(&[("error", "access_denied")]);
    };
    let code = uuid::Uuid::new_v4().simple().to_string();
    idp.codes.lock().unwrap().insert(
        code.clone(),
        Pending {
            claims,
            nonce: q.get("nonce").cloned(),
            challenge: (q.get("code_challenge_method").map(String::as_str) == Some("S256"))
                .then(|| q.get("code_challenge").cloned())
                .flatten(),
            redirect_uri: redirect_uri.clone(),
        },
    );
    back(&[("code", &code)])
}

async fn token(
    State(idp): State<Arc<Inner>>,
    headers: HeaderMap,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    let refuse =
        |error: &str| (StatusCode::BAD_REQUEST, Json(json!({"error": error}))).into_response();
    if form.get("grant_type").map(String::as_str) != Some("authorization_code") {
        return refuse("unsupported_grant_type");
    }
    // The client: HTTP Basic, or its id (and secret) in the form.
    let basic = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Basic "))
        .and_then(|b| STANDARD.decode(b).ok())
        .and_then(|b| String::from_utf8(b).ok())
        .and_then(|s| s.split_once(':').map(|(i, p)| (urldecode(i), urldecode(p))));
    let (client_id, secret) = match basic {
        Some((id, secret)) => (id, Some(secret)),
        None => (
            form.get("client_id").cloned().unwrap_or_default(),
            form.get("client_secret").cloned(),
        ),
    };
    if client_id != idp.client_id || (idp.client_secret.is_some() && secret != idp.client_secret) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": "invalid_client"})),
        )
            .into_response();
    }
    let Some(code) = form.get("code") else {
        return refuse("invalid_request");
    };
    let Some(pending) = idp.codes.lock().unwrap().remove(code) else {
        return refuse("invalid_grant");
    };
    if form.get("redirect_uri") != Some(&pending.redirect_uri) {
        return refuse("invalid_grant");
    }
    if let Some(challenge) = &pending.challenge {
        let verifier = form.get("code_verifier").cloned().unwrap_or_default();
        if URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes())) != *challenge {
            return refuse("invalid_grant");
        }
    }
    let t = now();
    let mut claims = json!({
        "iss": idp.issuer,
        "aud": idp.client_id,
        "iat": t,
        "exp": t + 300,
    });
    if let Some(nonce) = pending.nonce {
        claims["nonce"] = json!(nonce);
    }
    if let (Value::Object(base), Value::Object(user)) = (&mut claims, pending.claims) {
        base.extend(user);
        base.extend(idp.overrides.lock().unwrap().clone());
    }
    Json(json!({
        "access_token": uuid::Uuid::new_v4().simple().to_string(),
        "token_type": "Bearer",
        "expires_in": 300,
        "id_token": sign(&idp.key, &claims),
    }))
    .into_response()
}

fn urlencode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn urldecode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => match u8::from_str_radix(&s[i + 1..i + 3], 16) {
                Ok(b) => {
                    out.push(b);
                    i += 3;
                    continue;
                }
                Err(_) => out.push(b'%'),
            },
            b'+' => out.push(b' '),
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn url_with(base: &str, params: &[(&str, &str)]) -> String {
    let sep = if base.contains('?') { '&' } else { '?' };
    let query: Vec<String> = params
        .iter()
        .map(|(k, v)| format!("{k}={}", urlencode(v)))
        .collect();
    format!("{base}{sep}{}", query.join("&"))
}
