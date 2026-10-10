//! Google's push service: a high-priority data message through FCM's
//! HTTP v1 API, authorized by an OAuth 2.0 access token the project's
//! service account gets for itself (an RS256 JWT traded at its
//! `token_uri`, kept until a few minutes before it lapses).

use std::time::{Duration, Instant};

use fcp_core::push::FcmConfig;
use serde_json::json;

use crate::jwt::{now, Rs256};
use crate::{PushMessage, PushOutcome};

const ENDPOINT: &str = "https://fcm.googleapis.com";
const TOKEN_URI: &str = "https://oauth2.googleapis.com/token";
const SCOPE: &str = "https://www.googleapis.com/auth/firebase.messaging";

/// The parts of a service account's JSON that push uses.
#[derive(serde::Deserialize)]
struct ServiceAccount {
    client_email: String,
    private_key: String,
    #[serde(default)]
    token_uri: Option<String>,
}

/// One project and its service account.
pub struct FcmProject {
    project_id: String,
    client_email: String,
    signer: Rs256,
    token_uri: String,
    base: String,
    http: reqwest::Client,
    access: tokio::sync::Mutex<Option<(String, Instant)>>,
}

impl FcmProject {
    pub(crate) fn new(
        config: &FcmConfig,
        account: &str,
        http: &reqwest::Client,
        _h2c: &reqwest::Client,
    ) -> anyhow::Result<Self> {
        let account: ServiceAccount = serde_json::from_str(account).map_err(|e| {
            anyhow::anyhow!(
                "[[push.fcm]] project {}: not a service account's JSON: {e}",
                config.project_id
            )
        })?;
        let signer = Rs256::from_pem(&account.private_key)
            .map_err(|e| anyhow::anyhow!("[[push.fcm]] project {}: {e}", config.project_id))?;
        Ok(Self {
            project_id: config.project_id.clone(),
            client_email: account.client_email,
            signer,
            token_uri: account.token_uri.unwrap_or_else(|| TOKEN_URI.to_string()),
            base: config
                .endpoint
                .clone()
                .unwrap_or_else(|| ENDPOINT.to_string())
                .trim_end_matches('/')
                .to_string(),
            http: http.clone(),
            access: tokio::sync::Mutex::new(None),
        })
    }

    pub(crate) fn project_id(&self) -> &str {
        &self.project_id
    }

    /// An access token: the one held until five minutes before it lapses,
    /// else a new one. One exchange at a time.
    async fn access_token(&self) -> Result<String, String> {
        let mut held = self.access.lock().await;
        if let Some((token, until)) = held.as_ref() {
            if Instant::now() < *until {
                return Ok(token.clone());
            }
        }
        let iat = now();
        let assertion = self
            .signer
            .sign(
                &json!({"alg": "RS256", "typ": "JWT"}),
                &json!({
                    "iss": self.client_email,
                    "scope": SCOPE,
                    "aud": self.token_uri,
                    "iat": iat,
                    "exp": iat + 3600,
                }),
            )
            .map_err(|e| format!("signing the service account's JWT: {e}"))?;
        let response = self
            .http
            .post(&self.token_uri)
            .form(&[
                ("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"),
                ("assertion", assertion.as_str()),
            ])
            .send()
            .await
            .map_err(|e| format!("the token endpoint did not answer: {e}"))?;
        if !response.status().is_success() {
            return Err(format!(
                "the token endpoint refused the service account: {}",
                response.status()
            ));
        }
        let body: serde_json::Value = response
            .json()
            .await
            .map_err(|e| format!("the token endpoint's answer: {e}"))?;
        let token = body["access_token"]
            .as_str()
            .ok_or("the token endpoint gave no access token")?
            .to_string();
        let life = body["expires_in"].as_u64().unwrap_or(3600);
        let until = Instant::now() + Duration::from_secs(life.saturating_sub(300));
        *held = Some((token.clone(), until));
        Ok(token)
    }

    pub(crate) async fn send(&self, token: &str, message: &PushMessage) -> PushOutcome {
        let access = match self.access_token().await {
            Ok(a) => a,
            Err(e) => return PushOutcome::Failed(e),
        };
        let body = json!({
            "message": {
                "token": token,
                "data": message.fields(),
                // Delivered now, waking the app from Doze, or not at all
                // once the call is long gone.
                "android": {"priority": "high", "ttl": "30s"},
            }
        });
        let sent = self
            .http
            .post(format!(
                "{}/v1/projects/{}/messages:send",
                self.base, self.project_id
            ))
            .bearer_auth(access)
            .json(&body)
            .send()
            .await;
        let response = match sent {
            Ok(r) => r,
            Err(e) => return PushOutcome::Failed(format!("FCM did not answer: {e}")),
        };
        let status = response.status().as_u16();
        if (200..300).contains(&status) {
            return PushOutcome::Sent;
        }
        let error = response
            .json::<serde_json::Value>()
            .await
            .unwrap_or_default();
        let state = error["error"]["status"].as_str().unwrap_or_default();
        let code = error["error"]["details"]
            .as_array()
            .and_then(|details| {
                details
                    .iter()
                    .find_map(|d| d["errorCode"].as_str().map(str::to_string))
            })
            .unwrap_or_default();
        if code == "UNREGISTERED" || state == "NOT_FOUND" {
            return PushOutcome::Gone(format!("FCM {status} {state} {code}").trim().to_string());
        }
        if status == 401 {
            // Asked for again next time.
            *self.access.lock().await = None;
        }
        PushOutcome::Failed(format!("FCM {status} {state} {code}").trim().to_string())
    }
}
