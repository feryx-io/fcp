//! Amazon Polly text to speech (#339).
//!
//! - `POST /v1/speech` on `polly.<region>.amazonaws.com` (SynthesizeSpeech),
//!   `OutputFormat = pcm` at `SampleRate = 8000`: raw 16-bit PCM, returned
//!   as an 8 kHz WAV like every provider's audio.
//! - `GET /v1/voices` (DescribeVoices): the voices of a language and engine.
//!
//! Requests are signed with AWS Signature Version 4 (`fcp_core::sigv4`).
//! Credentials come from the configuration, else from
//! `AWS_ACCESS_KEY_ID`/`AWS_SECRET_ACCESS_KEY`/`AWS_SESSION_TOKEN`, else
//! from the machine's role: a container's credentials endpoint
//! (`AWS_CONTAINER_CREDENTIALS_RELATIVE_URI` or `_FULL_URI`, as ECS and
//! EKS set) or the EC2 instance metadata service over IMDSv2
//! (`AWS_EC2_METADATA_SERVICE_ENDPOINT` to move it). A role's credentials
//! are kept until five minutes before they expire.
//!
//! <https://docs.aws.amazon.com/polly/latest/dg/API_SynthesizeSpeech.html>

use super::super::{TtsProvider, VoiceInfo};
use super::{pcm_to_8k_wav, vendor_error};
use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use fcp_core::sigv4::{self, Credentials};
use serde::Deserialize;
use std::collections::HashMap;
use tokio::sync::Mutex;
use tracing::{debug, warn};

const RATE: u32 = 8000;
const ENGINES: &[&str] = &["neural", "standard", "long-form", "generative"];
/// When DescribeVoices has nothing for a language.
const FALLBACK_VOICE: &str = "Joanna";
const IMDS: &str = "http://169.254.169.254";
const CONTAINER_HOST: &str = "http://169.254.170.2";

pub struct AwsPollyTts {
    region: String,
    engine: String,
    base: reqwest::Url,
    client: reqwest::Client,
    source: CredentialSource,
    /// A role's credentials and when they stop being used.
    role: Mutex<Option<(Credentials, DateTime<Utc>)>>,
    /// The voice chosen for each language a request named none for.
    defaults: Mutex<HashMap<String, String>>,
}

enum CredentialSource {
    Static(Credentials),
    /// The environment's at each request, else the machine's role.
    Ambient,
}

impl AwsPollyTts {
    pub fn new(
        region: String,
        access_key_id: Option<String>,
        secret_access_key: Option<String>,
        engine: String,
        endpoint: Option<String>,
    ) -> Result<Self> {
        let region = region.trim().to_string();
        if region.is_empty()
            || !region
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-')
        {
            bail!("AWS Polly TTS provider requires a region such as us-east-1, not {region:?}");
        }
        if !ENGINES.contains(&engine.as_str()) {
            bail!("AWS Polly engine {engine:?} is not one of {ENGINES:?}");
        }
        let nonempty = |v: Option<String>| v.filter(|v| !v.trim().is_empty());
        let source = match (nonempty(access_key_id), nonempty(secret_access_key)) {
            (Some(access_key_id), Some(secret_access_key)) => {
                CredentialSource::Static(Credentials {
                    access_key_id,
                    secret_access_key,
                    session_token: None,
                })
            }
            (None, None) => CredentialSource::Ambient,
            _ => bail!("AWS Polly needs both access_key_id and secret_access_key, or neither"),
        };
        let base = endpoint.unwrap_or_else(|| format!("https://polly.{region}.amazonaws.com"));
        let base = reqwest::Url::parse(base.trim_end_matches('/'))
            .with_context(|| format!("AWS Polly endpoint {base:?}"))?;
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .context("Failed to create HTTP client")?;
        Ok(Self {
            region,
            engine,
            base,
            client,
            source,
            role: Mutex::new(None),
            defaults: Mutex::new(HashMap::new()),
        })
    }

    async fn credentials(&self) -> Result<Credentials> {
        if let CredentialSource::Static(credentials) = &self.source {
            return Ok(credentials.clone());
        }
        if let (Ok(access_key_id), Ok(secret_access_key)) = (
            std::env::var("AWS_ACCESS_KEY_ID"),
            std::env::var("AWS_SECRET_ACCESS_KEY"),
        ) {
            return Ok(Credentials {
                access_key_id,
                secret_access_key,
                session_token: std::env::var("AWS_SESSION_TOKEN").ok(),
            });
        }
        let mut role = self.role.lock().await;
        if let Some((credentials, until)) = role.as_ref() {
            if Utc::now() < *until {
                return Ok(credentials.clone());
            }
        }
        let fetched = self.role_credentials().await.context(
            "AWS Polly has no credentials: none configured, none in the environment, \
             and none from the machine's role",
        )?;
        let until = fetched.expiration - chrono::Duration::minutes(5);
        let credentials = Credentials {
            access_key_id: fetched.access_key_id,
            secret_access_key: fetched.secret_access_key,
            session_token: Some(fetched.token),
        };
        *role = Some((credentials.clone(), until));
        Ok(credentials)
    }

    /// The machine's role: a container's credentials endpoint, else the
    /// instance's (IMDSv2).
    async fn role_credentials(&self) -> Result<RoleCredentials> {
        let short = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(2))
            .build()?;
        if let Ok(path) = std::env::var("AWS_CONTAINER_CREDENTIALS_RELATIVE_URI") {
            let response = short.get(format!("{CONTAINER_HOST}{path}")).send().await?;
            return Ok(response.error_for_status()?.json().await?);
        }
        if let Ok(url) = std::env::var("AWS_CONTAINER_CREDENTIALS_FULL_URI") {
            let mut request = short.get(url);
            if let Ok(token) = std::env::var("AWS_CONTAINER_AUTHORIZATION_TOKEN") {
                request = request.header("Authorization", token);
            }
            return Ok(request.send().await?.error_for_status()?.json().await?);
        }
        let imds =
            std::env::var("AWS_EC2_METADATA_SERVICE_ENDPOINT").unwrap_or_else(|_| IMDS.to_string());
        let imds = imds.trim_end_matches('/');
        let token = short
            .put(format!("{imds}/latest/api/token"))
            .header("X-aws-ec2-metadata-token-ttl-seconds", "21600")
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;
        let roles = format!("{imds}/latest/meta-data/iam/security-credentials/");
        let name = short
            .get(&roles)
            .header("X-aws-ec2-metadata-token", &token)
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;
        let name = name.lines().next().unwrap_or("").trim();
        if name.is_empty() {
            bail!("the instance has no role");
        }
        Ok(short
            .get(format!("{roles}{name}"))
            .header("X-aws-ec2-metadata-token", &token)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?)
    }

    /// Send a signed request to Polly.
    async fn send(
        &self,
        method: reqwest::Method,
        path: &str,
        query: &[(String, String)],
        body: Option<Vec<u8>>,
    ) -> Result<reqwest::Response> {
        let credentials = self.credentials().await?;
        let mut url = self.base.clone();
        url.set_path(path);
        if !query.is_empty() {
            url.query_pairs_mut().extend_pairs(query);
        }
        let host = match url.port() {
            Some(port) => format!("{}:{port}", url.host_str().unwrap_or("")),
            None => url.host_str().unwrap_or("").to_string(),
        };
        let now = Utc::now();
        let amz_date = now.format("%Y%m%dT%H%M%SZ").to_string();
        let payload = body.as_deref().unwrap_or(&[]);
        let payload_sha256 = sigv4::sha256_hex(payload);
        let mut headers = vec![
            ("host".to_string(), host),
            ("x-amz-date".to_string(), amz_date),
        ];
        if body.is_some() {
            headers.push(("content-type".to_string(), "application/json".to_string()));
        }
        if let Some(token) = &credentials.session_token {
            headers.push(("x-amz-security-token".to_string(), token.clone()));
        }
        let authorization = sigv4::authorization(
            &sigv4::Request {
                method: method.as_str(),
                path: &sigv4::uri_encode(path, true),
                query,
                headers: &headers,
                payload_sha256: &payload_sha256,
            },
            &credentials,
            &self.region,
            "polly",
            now,
        );
        let mut request = self
            .client
            .request(method, url)
            .header("Authorization", authorization);
        for (name, value) in headers.iter().filter(|(n, _)| n != "host") {
            request = request.header(name, value);
        }
        if let Some(body) = body {
            request = request.body(body);
        }
        request.send().await.context("Failed to reach AWS Polly")
    }

    async fn voices(&self, language: &str) -> Result<Vec<Voice>> {
        let mut query = vec![("Engine".to_string(), self.engine.clone())];
        if !language.is_empty() {
            query.push(("LanguageCode".to_string(), language.to_string()));
        }
        let response = self
            .send(reqwest::Method::GET, "/v1/voices", &query, None)
            .await?;
        if !response.status().is_success() {
            return Err(vendor_error("AWS Polly", response).await);
        }
        let list: VoiceList = response.json().await.context("Polly's voice list")?;
        Ok(list.voices)
    }

    /// The voice for `language` when a request names none.
    async fn default_voice(&self, language: &str) -> String {
        if let Some(voice) = self.defaults.lock().await.get(language) {
            return voice.clone();
        }
        let voice = match self.voices(language).await {
            Ok(voices) if !voices.is_empty() => voices[0].id.clone(),
            Ok(_) => {
                warn!(language, engine = %self.engine, "No Polly voice for the language; using {FALLBACK_VOICE}");
                FALLBACK_VOICE.to_string()
            }
            Err(e) => {
                warn!(language, error = %e, "Could not list Polly voices");
                return FALLBACK_VOICE.to_string();
            }
        };
        self.defaults
            .lock()
            .await
            .insert(language.to_string(), voice.clone());
        voice
    }
}

#[derive(Deserialize)]
struct RoleCredentials {
    #[serde(rename = "AccessKeyId")]
    access_key_id: String,
    #[serde(rename = "SecretAccessKey")]
    secret_access_key: String,
    #[serde(rename = "Token")]
    token: String,
    #[serde(rename = "Expiration")]
    expiration: DateTime<Utc>,
}

#[derive(Deserialize)]
struct VoiceList {
    #[serde(rename = "Voices")]
    voices: Vec<Voice>,
}

#[derive(Deserialize)]
struct Voice {
    #[serde(rename = "Id")]
    id: String,
    #[serde(rename = "Name", default)]
    name: Option<String>,
    #[serde(rename = "Gender", default)]
    gender: Option<String>,
    #[serde(rename = "LanguageCode", default)]
    language_code: Option<String>,
}

#[async_trait]
impl TtsProvider for AwsPollyTts {
    fn name(&self) -> String {
        "aws_polly".to_string()
    }

    async fn synthesize(&self, text: &str, language: &str, voice: Option<&str>) -> Result<Vec<u8>> {
        let voice = match voice.filter(|v| !v.trim().is_empty()) {
            Some(v) => v.to_string(),
            None => self.default_voice(language).await,
        };
        debug!(text_len = text.len(), language, voice = %voice, engine = %self.engine, "Synthesizing with AWS Polly");
        let body = serde_json::to_vec(&serde_json::json!({
            "Engine": self.engine,
            "OutputFormat": "pcm",
            "SampleRate": RATE.to_string(),
            "Text": text,
            "TextType": "text",
            "VoiceId": voice,
        }))?;
        let response = self
            .send(reqwest::Method::POST, "/v1/speech", &[], Some(body))
            .await?;
        if !response.status().is_success() {
            return Err(vendor_error("AWS Polly", response).await);
        }
        let pcm = response
            .bytes()
            .await
            .context("Failed to read Polly's audio")?;
        pcm_to_8k_wav(&pcm, RATE)
    }

    async fn list_voices(&self, language: &str) -> Result<Vec<VoiceInfo>> {
        Ok(self
            .voices(language)
            .await?
            .into_iter()
            .map(|v| VoiceInfo {
                display_name: v.name.unwrap_or_else(|| v.id.clone()),
                name: v.id,
                language: v.language_code.unwrap_or_else(|| language.to_string()),
                gender: v.gender.map(|g| g.to_ascii_lowercase()),
                metadata: HashMap::from([
                    ("provider".to_string(), "aws_polly".to_string()),
                    ("engine".to_string(), self.engine.clone()),
                ]),
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn polly(key: Option<&str>, secret: Option<&str>, engine: &str) -> Result<AwsPollyTts> {
        AwsPollyTts::new(
            "us-east-1".into(),
            key.map(String::from),
            secret.map(String::from),
            engine.into(),
            None,
        )
    }

    #[test]
    fn keys_come_in_pairs_and_the_engine_is_one_polly_has() {
        assert!(polly(Some("AKID"), Some("secret"), "neural").is_ok());
        assert!(polly(None, None, "standard").is_ok());
        assert!(polly(Some("AKID"), None, "neural").is_err());
        assert!(polly(None, None, "turbo").is_err());
        assert!(AwsPollyTts::new("us east".into(), None, None, "neural".into(), None).is_err());
    }

    /// Against the real service when `FCP_TEST_POLLY_REGION` is set, with
    /// whatever credentials the environment or the machine has.
    #[tokio::test]
    async fn polly_speaks_when_a_region_is_set() {
        let Ok(region) = std::env::var("FCP_TEST_POLLY_REGION") else {
            eprintln!("FCP_TEST_POLLY_REGION not set; skipping");
            return;
        };
        let tts = AwsPollyTts::new(region, None, None, "neural".into(), None).unwrap();
        let wav = tts
            .synthesize("Hello from FCP.", "en-US", None)
            .await
            .unwrap();
        let reader = hound::WavReader::new(std::io::Cursor::new(wav)).unwrap();
        assert_eq!(reader.spec().sample_rate, 8000);
        assert!(reader.len() > 4000, "{} samples", reader.len());
        assert!(!tts.list_voices("en-US").await.unwrap().is_empty());
    }
}
