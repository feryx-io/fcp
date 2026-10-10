//! Stand-ins for the push services (`docs/PUSH.md` §3): an APNs-shaped
//! HTTP/2 endpoint that checks each request's provider token (ES256, the
//! team's key) and headers, and an FCM-shaped one with its token endpoint
//! (a service account's RS256 assertion traded for an access token) and its
//! send endpoint. Each keeps the pushes it took, tells a test of each as it
//! arrives, and answers "unregistered" for a token it was told is gone.

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::post;
use axum::{Form, Json, Router};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use fcp_core::push::{ApnsConfig, FcmConfig, PushAddress};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

/// The APNs stand-in's key: a throwaway made for these tests, never a
/// secret of anything.
const APNS_TEST_KEY: &str = "
-----BEGIN PRIVATE KEY-----
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgbFZhohIpop57GZvW
c79dIy9xrrEyQMfRhU4gDAL31KihRANCAASVuDq9ObGWOb/tbxYCSvqRmEKSN9E0
CXjY2ZLmjAkbNa6OWWX+2MdJSx2PeBSuMTSsFvSFHeILR0Ft2B86lM0l
-----END PRIVATE KEY-----
";

pub const APNS_TEAM: &str = "TEAMTEST01";
pub const APNS_KEY_ID: &str = "KEYTEST001";
pub const APNS_TOPIC: &str = "com.example.anvil.voip";
pub const FCM_PROJECT: &str = "fcp-test-project";
const FCM_EMAIL: &str = "push@fcp-test-project.iam.gserviceaccount.com";

/// A push a stand-in took.
#[derive(Debug, Clone)]
pub struct Pushed {
    /// `apns` or `fcm`.
    pub provider: &'static str,
    pub token: String,
    /// APNs's `apns-topic`.
    pub topic: Option<String>,
    /// The request's headers, lower-cased.
    pub headers: HashMap<String, String>,
    /// What the app is told: APNs's body, FCM's `data`.
    pub payload: Value,
    /// FCM's `android` block.
    pub android: Option<Value>,
}

impl Pushed {
    /// The payload's `type`: `call`, `cancel` or `test`.
    pub fn kind(&self) -> &str {
        self.payload["type"].as_str().unwrap_or_default()
    }
}

#[derive(Default)]
struct Shared {
    pushes: Mutex<Vec<Pushed>>,
    gone: Mutex<HashSet<String>>,
    refused: Mutex<Vec<String>>,
    access: Mutex<HashSet<String>>,
    tx: Option<tokio::sync::broadcast::Sender<Pushed>>,
}

impl Shared {
    fn new() -> Arc<Self> {
        let (tx, _) = tokio::sync::broadcast::channel(64);
        Arc::new(Self {
            tx: Some(tx),
            ..Default::default()
        })
    }

    fn took(&self, pushed: Pushed) {
        self.pushes.lock().unwrap().push(pushed.clone());
        if let Some(tx) = &self.tx {
            let _ = tx.send(pushed);
        }
    }

    fn refuse(&self, why: impl Into<String>) {
        self.refused.lock().unwrap().push(why.into());
    }
}

fn lowered(headers: &HeaderMap) -> HashMap<String, String> {
    headers
        .iter()
        .filter_map(|(n, v)| Some((n.as_str().to_lowercase(), v.to_str().ok()?.to_string())))
        .collect()
}

/// `header.claims` decoded and the signing input and signature, of a JWT.
fn jwt_parts(token: &str) -> Option<(Value, Value, String, Vec<u8>)> {
    let mut it = token.split('.');
    let (h, c, s) = (it.next()?, it.next()?, it.next()?);
    let header = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(h).ok()?).ok()?;
    let claims = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(c).ok()?).ok()?;
    Some((
        header,
        claims,
        format!("{h}.{c}"),
        URL_SAFE_NO_PAD.decode(s).ok()?,
    ))
}

/// Methods both stand-ins have.
macro_rules! recorded {
    () => {
        /// Every push taken, in order.
        pub fn pushes(&self) -> Vec<Pushed> {
            self.shared.pushes.lock().unwrap().clone()
        }

        /// Told of each push as it is taken.
        pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<Pushed> {
            self.shared.tx.as_ref().expect("a channel").subscribe()
        }

        /// The service no longer knows `token`: pushes to it are refused as
        /// unregistered.
        pub fn forget(&self, token: &str) {
            self.shared.gone.lock().unwrap().insert(token.to_string());
        }

        /// Requests refused, and why: a test that expects none asserts it.
        pub fn refused(&self) -> Vec<String> {
            self.shared.refused.lock().unwrap().clone()
        }

        /// The next push taken, within `secs`.
        pub async fn next_push(
            rx: &mut tokio::sync::broadcast::Receiver<Pushed>,
            secs: u64,
        ) -> Option<Pushed> {
            tokio::time::timeout(std::time::Duration::from_secs(secs), rx.recv())
                .await
                .ok()?
                .ok()
        }
    };
}

async fn serve(router: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("a port");
    let addr = listener.local_addr().expect("an address");
    // HTTP/2 without TLS as well as HTTP/1.1: APNs is HTTP/2 only.
    tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    format!("http://{addr}")
}

// ===== APNs =====

/// An APNs-shaped service for team `APNS_TEAM`, topic `APNS_TOPIC`.
pub struct StandInApns {
    url: String,
    shared: Arc<Shared>,
}

async fn apns_push(
    State(shared): State<Arc<Shared>>,
    Path(token): Path<String>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> (StatusCode, Json<Value>) {
    use p256::ecdsa::signature::Verifier;
    use p256::pkcs8::DecodePrivateKey;
    let refuse = |code: StatusCode, reason: &str| {
        shared.refuse(reason);
        (code, Json(json!({"reason": reason})))
    };
    let h = lowered(&headers);
    let Some(bearer) = h.get("authorization").and_then(|a| {
        a.strip_prefix("bearer ")
            .or_else(|| a.strip_prefix("Bearer "))
    }) else {
        return refuse(StatusCode::FORBIDDEN, "MissingProviderToken");
    };
    let Some((header, claims, input, signature)) = jwt_parts(bearer) else {
        return refuse(StatusCode::FORBIDDEN, "InvalidProviderToken");
    };
    let key = p256::ecdsa::SigningKey::from_pkcs8_pem(APNS_TEST_KEY.trim()).expect("the test key");
    let verified = p256::ecdsa::Signature::from_slice(&signature)
        .ok()
        .is_some_and(|s| key.verifying_key().verify(input.as_bytes(), &s).is_ok());
    if !verified
        || header["alg"] != "ES256"
        || header["kid"] != APNS_KEY_ID
        || claims["iss"] != APNS_TEAM
        || !claims["iat"].is_u64()
    {
        return refuse(StatusCode::FORBIDDEN, "InvalidProviderToken");
    }
    if h.get("apns-push-type").map(String::as_str) != Some("voip") {
        return refuse(StatusCode::BAD_REQUEST, "InvalidPushType");
    }
    let topic = h.get("apns-topic").cloned();
    if topic.as_deref() != Some(APNS_TOPIC) {
        return refuse(StatusCode::BAD_REQUEST, "TopicDisallowed");
    }
    if shared.gone.lock().unwrap().contains(&token) {
        return (StatusCode::GONE, Json(json!({"reason": "Unregistered"})));
    }
    let Ok(payload) = serde_json::from_slice::<Value>(&body) else {
        return refuse(StatusCode::BAD_REQUEST, "PayloadEmpty");
    };
    shared.took(Pushed {
        provider: "apns",
        token,
        topic,
        headers: h,
        payload,
        android: None,
    });
    (StatusCode::OK, Json(json!({})))
}

impl StandInApns {
    pub async fn start() -> Self {
        let shared = Shared::new();
        let router = Router::new()
            .route("/3/device/:token", post(apns_push))
            .with_state(Arc::clone(&shared));
        Self {
            url: serve(router).await,
            shared,
        }
    }

    /// `[[push.apns]]` for this stand-in.
    pub fn config(&self) -> ApnsConfig {
        ApnsConfig {
            team_id: APNS_TEAM.into(),
            key_id: APNS_KEY_ID.into(),
            key: Some(APNS_TEST_KEY.trim().into()),
            key_file: None,
            topics: vec![APNS_TOPIC.into()],
            sandbox: false,
            endpoint: Some(self.url.clone()),
        }
    }

    /// A device's push address at this service: `pn-param` the team and
    /// the topic.
    pub fn address(token: &str) -> PushAddress {
        PushAddress {
            provider: "apns".into(),
            param: format!("{APNS_TEAM}.{APNS_TOPIC}"),
            prid: token.into(),
        }
    }

    recorded!();
}

// ===== FCM =====

/// An FCM-shaped service for project `FCM_PROJECT`, with the service
/// account's token endpoint.
pub struct StandInFcm {
    url: String,
    shared: Arc<Shared>,
}

async fn fcm_token(
    State(shared): State<Arc<Shared>>,
    Form(form): Form<HashMap<String, String>>,
) -> (StatusCode, Json<Value>) {
    use rsa::pkcs8::DecodePrivateKey;
    use rsa::signature::Verifier;
    let refuse = |why: &str| {
        shared.refuse(why);
        (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "invalid_grant"})),
        )
    };
    if form.get("grant_type").map(String::as_str)
        != Some("urn:ietf:params:oauth:grant-type:jwt-bearer")
    {
        return refuse("grant_type");
    }
    let Some((header, claims, input, signature)) = form.get("assertion").and_then(|a| jwt_parts(a))
    else {
        return refuse("assertion");
    };
    let key =
        rsa::RsaPrivateKey::from_pkcs8_pem(crate::oidc::TEST_KEY.trim()).expect("the test key");
    let verifier = rsa::pkcs1v15::VerifyingKey::<sha2::Sha256>::new(key.to_public_key());
    let verified = rsa::pkcs1v15::Signature::try_from(signature.as_slice())
        .ok()
        .is_some_and(|s| verifier.verify(input.as_bytes(), &s).is_ok());
    if !verified
        || header["alg"] != "RS256"
        || claims["iss"] != FCM_EMAIL
        || claims["scope"] != "https://www.googleapis.com/auth/firebase.messaging"
        || !claims["aud"]
            .as_str()
            .is_some_and(|a| a.ends_with("/token"))
    {
        return refuse("the assertion");
    }
    let token = format!("ya29.test-{}", uuid::Uuid::new_v4().simple());
    shared.access.lock().unwrap().insert(token.clone());
    (
        StatusCode::OK,
        Json(json!({"access_token": token, "expires_in": 3599, "token_type": "Bearer"})),
    )
}

async fn fcm_send(
    State(shared): State<Arc<Shared>>,
    Path(project): Path<String>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    let h = lowered(&headers);
    let bearer = h
        .get("authorization")
        .and_then(|a| {
            a.strip_prefix("Bearer ")
                .or_else(|| a.strip_prefix("bearer "))
        })
        .unwrap_or_default();
    if !shared.access.lock().unwrap().contains(bearer) {
        shared.refuse("access token");
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": {"code": 401, "status": "UNAUTHENTICATED"}})),
        );
    }
    // The route is `/v1/projects/{project}/messages:send`; axum matches the
    // segment whole.
    let project = project.trim_end_matches("/messages:send").to_string();
    if project != FCM_PROJECT {
        shared.refuse(format!("project {project}"));
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error": {"code": 404, "status": "NOT_FOUND"}})),
        );
    }
    let message = &body["message"];
    let token = message["token"].as_str().unwrap_or_default().to_string();
    if shared.gone.lock().unwrap().contains(&token) {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error": {
                "code": 404,
                "status": "NOT_FOUND",
                "details": [{
                    "@type": "type.googleapis.com/google.firebase.fcm.v1.FcmError",
                    "errorCode": "UNREGISTERED"
                }]
            }})),
        );
    }
    shared.took(Pushed {
        provider: "fcm",
        token,
        topic: None,
        headers: h,
        payload: message["data"].clone(),
        android: Some(message["android"].clone()),
    });
    (
        StatusCode::OK,
        Json(json!({"name": format!("projects/{FCM_PROJECT}/messages/1")})),
    )
}

impl StandInFcm {
    pub async fn start() -> Self {
        let shared = Shared::new();
        let router = Router::new()
            .route("/token", post(fcm_token))
            .route("/v1/projects/*rest", post(fcm_send))
            .with_state(Arc::clone(&shared));
        Self {
            url: serve(router).await,
            shared,
        }
    }

    /// `[[push.fcm]]` for this stand-in: its service account's JSON, whose
    /// `token_uri` is the stand-in's.
    pub fn config(&self) -> FcmConfig {
        let account = json!({
            "type": "service_account",
            "project_id": FCM_PROJECT,
            "client_email": FCM_EMAIL,
            "private_key": crate::oidc::TEST_KEY.trim(),
            "token_uri": format!("{}/token", self.url),
        });
        FcmConfig {
            project_id: FCM_PROJECT.into(),
            service_account: Some(account.to_string()),
            service_account_file: None,
            endpoint: Some(self.url.clone()),
        }
    }

    /// A device's push address at this service: `pn-param` the project.
    pub fn address(token: &str) -> PushAddress {
        PushAddress {
            provider: "fcm".into(),
            param: FCM_PROJECT.into(),
            prid: token.into(),
        }
    }

    recorded!();
}
