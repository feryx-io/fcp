//! Stand-ins for the speech vendors FCP's TTS providers call (#339):
//! ElevenLabs, Azure AI Speech and Amazon Polly, and the EC2 instance
//! metadata service a Polly provider with no keys reads the machine's role
//! from. Each serves the vendor's routes on a loopback port, checks what
//! the vendor would check — the key, the output format, Polly's SigV4
//! signature recomputed with the secret it was given — and answers a
//! second of a [`TONE_HZ`] tone in the vendor's format. A provider is
//! pointed at one with its `endpoint`.

use axum::{
    body::Bytes,
    extract::{Path, Query, State},
    http::{HeaderMap, Method, StatusCode, Uri},
    response::{IntoResponse, Response},
    routing::{get, post, put},
    Json, Router,
};
use chrono::{NaiveDateTime, Utc};
use fcp_core::sigv4;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// The tone every stand-in speaks.
pub const TONE_HZ: f32 = 440.0;

/// One request a stand-in was sent.
#[derive(Debug, Clone)]
pub struct VendorRequest {
    pub method: String,
    pub path: String,
    pub query: HashMap<String, String>,
    /// Header names in lower case.
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
}

impl VendorRequest {
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).to_string()
    }

    pub fn json(&self) -> serde_json::Value {
        serde_json::from_slice(&self.body).unwrap_or_default()
    }
}

#[derive(Clone, Copy)]
enum Vendor {
    ElevenLabs,
    Azure,
    Polly,
}

struct Shared {
    /// The key (ElevenLabs, Azure), or the access key id (Polly).
    key: String,
    /// Polly's secret access key.
    secret: String,
    region: String,
    requests: Mutex<Vec<VendorRequest>>,
}

/// A vendor's stand-in, serving until the process ends.
pub struct StandInTts {
    url: String,
    shared: Arc<Shared>,
}

impl StandInTts {
    /// ElevenLabs, taking `api_key` in `xi-api-key`. Its voices are
    /// Rachel (`21m00Tcm4TlvDq8ikWAM`) and Domi (`AZnzlk1XvdvUeBnXmlld`).
    pub async fn elevenlabs(api_key: &str) -> Self {
        Self::start(Vendor::ElevenLabs, api_key, "", "").await
    }

    /// Azure AI Speech, taking `key` in `Ocp-Apim-Subscription-Key`. Its
    /// voices are `en-US-AvaMultilingualNeural` and `en-GB-SoniaNeural`.
    pub async fn azure(key: &str) -> Self {
        Self::start(Vendor::Azure, key, "", "").await
    }

    /// Amazon Polly in `region`, taking requests signed by
    /// `access_key_id` and `secret`. Its voices are Joanna (en-US) and
    /// Amy (en-GB).
    pub async fn polly(region: &str, access_key_id: &str, secret: &str) -> Self {
        Self::start(Vendor::Polly, access_key_id, secret, region).await
    }

    async fn start(vendor: Vendor, key: &str, secret: &str, region: &str) -> Self {
        let shared = Arc::new(Shared {
            key: key.to_string(),
            secret: secret.to_string(),
            region: region.to_string(),
            requests: Mutex::new(Vec::new()),
        });
        let router = match vendor {
            Vendor::ElevenLabs => Router::new()
                .route("/v1/text-to-speech/:voice", post(elevenlabs_speech))
                .route("/v1/voices", get(elevenlabs_voices)),
            Vendor::Azure => Router::new()
                .route("/cognitiveservices/v1", post(azure_speech))
                .route("/cognitiveservices/voices/list", get(azure_voices)),
            Vendor::Polly => Router::new()
                .route("/v1/speech", post(polly_speech))
                .route("/v1/voices", get(polly_voices)),
        }
        .with_state(Arc::clone(&shared));
        Self {
            url: serve(router).await,
            shared,
        }
    }

    /// The base URL, for the provider's `endpoint`.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// The requests sent so far, refused ones included.
    pub fn requests(&self) -> Vec<VendorRequest> {
        self.shared.requests.lock().unwrap().clone()
    }

    /// The requests sent to `path`.
    pub fn requests_to(&self, path: &str) -> Vec<VendorRequest> {
        self.requests()
            .into_iter()
            .filter(|r| r.path == path || r.path.starts_with(&format!("{path}/")))
            .collect()
    }
}

async fn serve(router: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind a vendor stand-in");
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    url
}

fn record(
    shared: &Shared,
    method: &Method,
    uri: &Uri,
    headers: &HeaderMap,
    body: &[u8],
) -> VendorRequest {
    let request = VendorRequest {
        method: method.to_string(),
        path: uri.path().to_string(),
        query: uri
            .query()
            .map(|q| {
                url::form_urlencoded::parse(q.as_bytes())
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect()
            })
            .unwrap_or_default(),
        headers: headers
            .iter()
            .map(|(k, v)| {
                (
                    k.as_str().to_ascii_lowercase(),
                    v.to_str().unwrap_or("").to_string(),
                )
            })
            .collect(),
        body: body.to_vec(),
    };
    shared.requests.lock().unwrap().push(request.clone());
    request
}

/// A second of [`TONE_HZ`] at `rate`.
fn tone(rate: u32) -> Vec<i16> {
    (0..rate)
        .map(|i| {
            (9000.0 * (2.0 * std::f32::consts::PI * TONE_HZ * i as f32 / rate as f32).sin()) as i16
        })
        .collect()
}

fn pcm_bytes(samples: &[i16]) -> Vec<u8> {
    samples.iter().flat_map(|s| s.to_le_bytes()).collect()
}

fn wav_bytes(samples: &[i16], rate: u32) -> Vec<u8> {
    let mut out = std::io::Cursor::new(Vec::new());
    let mut writer = hound::WavWriter::new(
        &mut out,
        hound::WavSpec {
            channels: 1,
            sample_rate: rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        },
    )
    .unwrap();
    for s in samples {
        writer.write_sample(*s).unwrap();
    }
    writer.finalize().unwrap();
    out.into_inner()
}

fn refuse(status: StatusCode, why: &str) -> Response {
    (status, Json(serde_json::json!({ "message": why }))).into_response()
}

// --- ElevenLabs -------------------------------------------------------

async fn elevenlabs_speech(
    State(shared): State<Arc<Shared>>,
    Path(voice): Path<String>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let request = record(&shared, &method, &uri, &headers, &body);
    if request.headers.get("xi-api-key") != Some(&shared.key) {
        return refuse(StatusCode::UNAUTHORIZED, "invalid_api_key");
    }
    if !["21m00Tcm4TlvDq8ikWAM", "AZnzlk1XvdvUeBnXmlld"].contains(&voice.as_str()) {
        return refuse(StatusCode::NOT_FOUND, "voice_not_found");
    }
    let Some(rate) = request
        .query
        .get("output_format")
        .and_then(|f| f.strip_prefix("pcm_"))
        .and_then(|r| r.parse::<u32>().ok())
    else {
        return refuse(StatusCode::BAD_REQUEST, "only pcm_<rate> is served here");
    };
    if request.json()["text"].as_str().unwrap_or("").is_empty() {
        return refuse(StatusCode::UNPROCESSABLE_ENTITY, "text is required");
    }
    pcm_bytes(&tone(rate)).into_response()
}

async fn elevenlabs_voices(
    State(shared): State<Arc<Shared>>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
) -> Response {
    let request = record(&shared, &method, &uri, &headers, &[]);
    if request.headers.get("xi-api-key") != Some(&shared.key) {
        return refuse(StatusCode::UNAUTHORIZED, "invalid_api_key");
    }
    Json(serde_json::json!({ "voices": [
        { "voice_id": "21m00Tcm4TlvDq8ikWAM", "name": "Rachel", "category": "premade",
          "labels": { "gender": "female", "accent": "american" } },
        { "voice_id": "AZnzlk1XvdvUeBnXmlld", "name": "Domi", "category": "premade",
          "labels": { "gender": "female", "accent": "american" } },
    ]}))
    .into_response()
}

// --- Azure ------------------------------------------------------------

async fn azure_speech(
    State(shared): State<Arc<Shared>>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let request = record(&shared, &method, &uri, &headers, &body);
    if request.headers.get("ocp-apim-subscription-key") != Some(&shared.key) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if request.headers.get("content-type").map(String::as_str) != Some("application/ssml+xml") {
        return StatusCode::UNSUPPORTED_MEDIA_TYPE.into_response();
    }
    let rate = match request
        .headers
        .get("x-microsoft-outputformat")
        .map(String::as_str)
    {
        Some("riff-8khz-16bit-mono-pcm") => 8000,
        Some("riff-16khz-16bit-mono-pcm") => 16_000,
        _ => return StatusCode::BAD_REQUEST.into_response(),
    };
    let ssml = request.text();
    if !ssml.starts_with("<speak") || !ssml.contains("<voice name=\"") {
        return StatusCode::BAD_REQUEST.into_response();
    }
    (
        [("content-type", "audio/x-wav")],
        wav_bytes(&tone(rate), rate),
    )
        .into_response()
}

async fn azure_voices(
    State(shared): State<Arc<Shared>>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
) -> Response {
    let request = record(&shared, &method, &uri, &headers, &[]);
    if request.headers.get("ocp-apim-subscription-key") != Some(&shared.key) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    Json(serde_json::json!([
        { "ShortName": "en-US-AvaMultilingualNeural", "DisplayName": "Ava Multilingual",
          "Locale": "en-US", "Gender": "Female", "VoiceType": "Neural" },
        { "ShortName": "en-GB-SoniaNeural", "DisplayName": "Sonia",
          "Locale": "en-GB", "Gender": "Female", "VoiceType": "Neural" },
    ]))
    .into_response()
}

// --- Polly ------------------------------------------------------------

/// Whether `request` is signed by the stand-in's credentials: the
/// signature recomputed over the headers it names.
fn polly_signed(shared: &Shared, request: &VendorRequest) -> Result<(), String> {
    let auth = request
        .headers
        .get("authorization")
        .ok_or("no Authorization")?;
    let rest = auth
        .strip_prefix("AWS4-HMAC-SHA256 Credential=")
        .ok_or("not AWS4-HMAC-SHA256")?;
    let credential = rest.split(',').next().unwrap_or("");
    let parts: Vec<&str> = credential.split('/').collect();
    if parts.len() != 5
        || parts[0] != shared.key
        || parts[2] != shared.region
        || parts[3] != "polly"
        || parts[4] != "aws4_request"
    {
        return Err(format!("credential scope {credential:?}"));
    }
    let signed: Vec<&str> = rest
        .split("SignedHeaders=")
        .nth(1)
        .and_then(|s| s.split(',').next())
        .ok_or("no SignedHeaders")?
        .split(';')
        .collect();
    if !signed.contains(&"host") || !signed.contains(&"x-amz-date") {
        return Err("host and x-amz-date must be signed".into());
    }
    let amz_date = request.headers.get("x-amz-date").ok_or("no x-amz-date")?;
    let at = NaiveDateTime::parse_from_str(amz_date, "%Y%m%dT%H%M%SZ")
        .map_err(|e| e.to_string())?
        .and_utc();
    if (Utc::now() - at).num_minutes().abs() > 5 {
        return Err("x-amz-date is not now".into());
    }
    let headers: Vec<(String, String)> = signed
        .iter()
        .map(|name| {
            (
                name.to_string(),
                request.headers.get(*name).cloned().unwrap_or_default(),
            )
        })
        .collect();
    let query: Vec<(String, String)> = request
        .query
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let expected = sigv4::authorization(
        &sigv4::Request {
            method: &request.method,
            path: &sigv4::uri_encode(&request.path, true),
            query: &query,
            headers: &headers,
            payload_sha256: &sigv4::sha256_hex(&request.body),
        },
        &sigv4::Credentials {
            access_key_id: shared.key.clone(),
            secret_access_key: shared.secret.clone(),
            session_token: None,
        },
        &shared.region,
        "polly",
        at,
    );
    if &expected != auth {
        return Err("the signature does not match".into());
    }
    Ok(())
}

async fn polly_speech(
    State(shared): State<Arc<Shared>>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let request = record(&shared, &method, &uri, &headers, &body);
    if let Err(why) = polly_signed(&shared, &request) {
        return refuse(StatusCode::FORBIDDEN, &why);
    }
    let speech = request.json();
    if speech["OutputFormat"] != "pcm" || speech["Text"].as_str().unwrap_or("").is_empty() {
        return refuse(StatusCode::BAD_REQUEST, "pcm and a text are served here");
    }
    if !["Joanna", "Amy"].contains(&speech["VoiceId"].as_str().unwrap_or("")) {
        return refuse(StatusCode::BAD_REQUEST, "InvalidVoiceId");
    }
    let rate: u32 = speech["SampleRate"]
        .as_str()
        .and_then(|r| r.parse().ok())
        .unwrap_or(16_000);
    ([("content-type", "audio/pcm")], pcm_bytes(&tone(rate))).into_response()
}

async fn polly_voices(
    State(shared): State<Arc<Shared>>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    let request = record(&shared, &method, &uri, &headers, &[]);
    if let Err(why) = polly_signed(&shared, &request) {
        return refuse(StatusCode::FORBIDDEN, &why);
    }
    let voices = [("Joanna", "en-US"), ("Amy", "en-GB")]
        .into_iter()
        .filter(|(_, language)| {
            query
                .get("LanguageCode")
                .is_none_or(|wanted| wanted == language)
        })
        .map(|(id, language)| {
            serde_json::json!({ "Id": id, "Name": id, "Gender": "Female",
                                "LanguageCode": language, "SupportedEngines": ["neural", "standard"] })
        })
        .collect::<Vec<_>>();
    Json(serde_json::json!({ "Voices": voices })).into_response()
}

// --- EC2 instance metadata --------------------------------------------

/// The EC2 instance metadata service (IMDSv2) for a machine whose role is
/// `fcp-role`, handing out the credentials given. Point a process at it
/// with `AWS_EC2_METADATA_SERVICE_ENDPOINT` = [`StandInImds::url`].
pub struct StandInImds {
    url: String,
}

struct Imds {
    access_key_id: String,
    secret: String,
    token: String,
}

const IMDS_TOKEN: &str = "imds-session-token";

impl StandInImds {
    pub async fn start(access_key_id: &str, secret: &str, session_token: &str) -> Self {
        let state = Arc::new(Imds {
            access_key_id: access_key_id.to_string(),
            secret: secret.to_string(),
            token: session_token.to_string(),
        });
        let router = Router::new()
            .route("/latest/api/token", put(imds_token))
            .route(
                "/latest/meta-data/iam/security-credentials/",
                get(imds_roles),
            )
            .route(
                "/latest/meta-data/iam/security-credentials/:role",
                get(imds_role),
            )
            .with_state(state);
        Self {
            url: serve(router).await,
        }
    }

    pub fn url(&self) -> &str {
        &self.url
    }
}

async fn imds_token(headers: HeaderMap) -> Response {
    if headers
        .get("x-aws-ec2-metadata-token-ttl-seconds")
        .is_none()
    {
        return StatusCode::BAD_REQUEST.into_response();
    }
    IMDS_TOKEN.into_response()
}

fn imds_allowed(headers: &HeaderMap) -> bool {
    headers
        .get("x-aws-ec2-metadata-token")
        .and_then(|v| v.to_str().ok())
        == Some(IMDS_TOKEN)
}

async fn imds_roles(headers: HeaderMap) -> Response {
    if !imds_allowed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    "fcp-role".into_response()
}

async fn imds_role(
    State(state): State<Arc<Imds>>,
    Path(role): Path<String>,
    headers: HeaderMap,
) -> Response {
    if !imds_allowed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if role != "fcp-role" {
        return StatusCode::NOT_FOUND.into_response();
    }
    let expiration = (Utc::now() + chrono::Duration::hours(1))
        .format("%Y-%m-%dT%H:%M:%SZ")
        .to_string();
    Json(serde_json::json!({
        "Code": "Success",
        "Type": "AWS-HMAC",
        "AccessKeyId": state.access_key_id,
        "SecretAccessKey": state.secret,
        "Token": state.token,
        "Expiration": expiration,
    }))
    .into_response()
}
