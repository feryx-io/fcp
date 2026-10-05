//! Zero-touch: vendors' redirection services (`docs/PROVISIONING.md` §10,
//! `docs/IDENTITY_AND_DEVICES.md` phase 8).
//!
//! A phone out of its box asks its vendor where its provisioning server
//! is. A tenant gives FCP its account with the vendor's service; FCP then
//! registers each of the tenant's phones with it — its MAC, the tenant's
//! provisioning URL, and the phone's own credentials where the service
//! carries them — when the device is made or claimed, and removes it when
//! the device goes.
//!
//! - **Yealink RPS**: a JSON API signed with HMAC-SHA256 ("Yealink Json API
//!   for RPS Management Platform"); each device gets its own URL
//!   (`uniqueServerUrl`) and credentials.
//! - **Poly ZTP**: an XML API with the account's user id and password in
//!   each request ("Polycom Zero Touch Provisioning API Developer Guide");
//!   each device is associated with a profile made in the ZTP console,
//!   which names the URL. It carries no per-device credentials.

use std::time::Duration;

use async_trait::async_trait;
use base64::Engine;
use fcp_core::provisioning::{RedirectionAccount, RedirectionService};
use hmac::{Hmac, Mac};
use sha2::Sha256;

/// The most a vendor's API is waited for.
const TIMEOUT: Duration = Duration::from_secs(15);

/// One phone, as a service registers it.
#[derive(Debug, Clone)]
pub struct PhoneRegistration {
    /// 12 hex digits, lower case.
    pub mac: String,
    /// The tenant's provisioning URL (`https://…/provision/<key>/`).
    pub url: String,
    /// HTTP credentials the service hands the phone, where it carries them.
    pub username: String,
    pub password: Option<String>,
    /// A note the vendor's console shows: the device's name.
    pub remark: String,
}

/// Why a service refused, or could not be asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RedirectionError(pub String);

impl std::fmt::Display for RedirectionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// A vendor's redirection service.
#[async_trait]
pub trait RedirectionConnector: Send + Sync {
    /// Point the phone at its URL: the service's id for it, where it has
    /// one. A phone already registered with this account is re-pointed.
    async fn register(&self, phone: &PhoneRegistration)
        -> Result<Option<String>, RedirectionError>;
    /// Take the phone off the service; one it does not have is no error.
    async fn unregister(&self, mac: &str, vendor_ref: Option<&str>)
        -> Result<(), RedirectionError>;
    /// Whether the account's credentials work.
    async fn check(&self) -> Result<(), RedirectionError>;
    /// Whether it carries per-device credentials to the phone.
    fn carries_credentials(&self) -> bool;
}

/// The connector for a tenant's account.
pub fn connector(
    config: &fcp_core::ApiRedirectionConfig,
    account: &RedirectionAccount,
) -> Box<dyn RedirectionConnector> {
    let http = reqwest::Client::builder()
        .timeout(TIMEOUT)
        .build()
        .unwrap_or_default();
    match account.service {
        RedirectionService::YealinkRps => Box::new(YealinkRps {
            http,
            base: config.yealink_rps_url.trim_end_matches('/').to_string(),
            key_id: account.key_id.clone(),
            secret: account.secret.clone(),
        }),
        RedirectionService::PolyZtp => Box::new(PolyZtp {
            http,
            url: config.poly_ztp_url.clone(),
            user: account.key_id.clone(),
            password: account.secret.clone(),
            account_id: account.account_id.clone().unwrap_or_default(),
            profile: account.profile.clone().unwrap_or_default(),
        }),
    }
}

// ---- Yealink RPS ----

/// The longest password RPS keeps for a device.
pub const RPS_MAX_PASSWORD: usize = 32;

pub struct YealinkRps {
    http: reqwest::Client,
    base: String,
    key_id: String,
    secret: String,
}

/// The `X-Ca-Signature` of a request: Base64 of HMAC-SHA256, keyed by the
/// AccessKey Secret, over
///
/// ```text
/// <METHOD>\n
/// [Content-MD5:<md5>\n]X-Ca-Key:<key>\nX-Ca-Nonce:<nonce>\nX-Ca-Timestamp:<ms>\n
/// <path without its leading '/'>[\n<query, keys sorted, k=v joined by &>]
/// ```
///
/// (the guide's §1.3.3: Content-MD5 only with a body, the query only
/// without one).
pub fn rps_signature(
    secret: &str,
    method: &str,
    content_md5: Option<&str>,
    key_id: &str,
    nonce: &str,
    timestamp: &str,
    path: &str,
    query: &[(&str, &str)],
) -> String {
    let mut to_sign = format!("{method}\n");
    if let Some(md5) = content_md5 {
        to_sign.push_str(&format!("Content-MD5:{md5}\n"));
    }
    to_sign.push_str(&format!(
        "X-Ca-Key:{key_id}\nX-Ca-Nonce:{nonce}\nX-Ca-Timestamp:{timestamp}\n"
    ));
    to_sign.push_str(path.trim_start_matches('/'));
    if content_md5.is_none() && !query.is_empty() {
        let mut q: Vec<&(&str, &str)> = query.iter().collect();
        q.sort_by(|a, b| a.0.cmp(b.0));
        let formatted: Vec<String> = q
            .iter()
            .map(|(k, v)| {
                if v.trim().is_empty() {
                    k.to_string()
                } else {
                    format!("{k}={v}")
                }
            })
            .collect();
        to_sign.push('\n');
        to_sign.push_str(&formatted.join("&"));
    }
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("HMAC takes any key");
    mac.update(to_sign.as_bytes());
    base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes())
}

/// `Content-MD5`: Base64 of the body's MD5.
pub fn content_md5(body: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(md5::compute(body).0)
}

impl YealinkRps {
    async fn call(
        &self,
        method: reqwest::Method,
        path: &str,
        query: &[(&str, &str)],
        body: Option<serde_json::Value>,
    ) -> Result<serde_json::Value, RedirectionError> {
        let nonce = uuid::Uuid::new_v4().simple().to_string();
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            .to_string();
        let body = body.map(|b| b.to_string());
        let md5 = body.as_deref().map(|b| content_md5(b.as_bytes()));
        let signature = rps_signature(
            &self.secret,
            method.as_str(),
            md5.as_deref(),
            &self.key_id,
            &nonce,
            &timestamp,
            path,
            query,
        );
        let mut request = self
            .http
            .request(method, format!("{}{path}", self.base))
            .query(query)
            .header("X-Ca-Key", &self.key_id)
            .header("X-Ca-Nonce", &nonce)
            .header("X-Ca-Timestamp", &timestamp)
            .header("X-Ca-Signature", signature);
        if let (Some(body), Some(md5)) = (body, md5) {
            request = request
                .header("Content-MD5", md5)
                .header("Content-Type", "application/json;charset=UTF-8")
                .body(body);
        }
        let response = request
            .send()
            .await
            .map_err(|e| RedirectionError(format!("Yealink RPS did not answer: {e}")))?;
        let status = response.status();
        let answer: serde_json::Value = response
            .json()
            .await
            .map_err(|_| RedirectionError(format!("Yealink RPS answered {status} without JSON")))?;
        if answer["ret"].as_i64().is_some_and(|ret| ret >= 0) {
            return Ok(answer["data"].clone());
        }
        // The guide's envelope says `errors`, its examples `error`.
        let error = if answer["error"].is_object() {
            &answer["error"]
        } else {
            &answer["errors"]
        };
        let field = error["fieldErrors"]
            .as_array()
            .and_then(|f| f.first())
            .and_then(|f| f["msg"].as_str())
            .unwrap_or("");
        let msg = error["msg"]
            .as_str()
            .filter(|m| !m.is_empty())
            .unwrap_or(field);
        Err(RedirectionError(format!(
            "Yealink RPS refused ({}): {}",
            error["errorCode"]
                .as_i64()
                .unwrap_or(status.as_u16() as i64),
            if msg.is_empty() {
                "no reason given"
            } else {
                msg
            }
        )))
    }

    /// RPS's id for a MAC this account has.
    async fn find(&self, mac: &str) -> Result<Option<String>, RedirectionError> {
        let data = self
            .call(
                reqwest::Method::POST,
                "/api/open/v1/device/list",
                &[],
                Some(serde_json::json!({"key": mac, "skip": 0, "limit": 10})),
            )
            .await?;
        Ok(data["data"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|d| {
                d["mac"]
                    .as_str()
                    .is_some_and(|m| m.eq_ignore_ascii_case(mac))
            })
            .and_then(|d| d["id"].as_str())
            .map(str::to_string))
    }
}

#[async_trait]
impl RedirectionConnector for YealinkRps {
    async fn register(
        &self,
        phone: &PhoneRegistration,
    ) -> Result<Option<String>, RedirectionError> {
        let mut fields = serde_json::json!({
            "uniqueServerUrl": phone.url,
            "remark": phone.remark.chars().take(256).collect::<String>(),
        });
        if let Some(password) = &phone.password {
            fields["authName"] = serde_json::json!(phone.username);
            fields["password"] = serde_json::json!(password);
        }
        let mut add = fields.clone();
        add["macs"] = serde_json::json!([phone.mac]);
        match self
            .call(
                reqwest::Method::POST,
                "/api/open/v1/device/add",
                &[],
                Some(add),
            )
            .await
        {
            Ok(data) => Ok(data
                .as_array()
                .and_then(|d| d.first())
                .and_then(|d| d["id"].as_str())
                .map(str::to_string)),
            // Already this account's: re-point it.
            Err(refused) => match self.find(&phone.mac).await? {
                Some(id) => {
                    let mut edit = fields;
                    edit["id"] = serde_json::json!(id);
                    self.call(
                        reqwest::Method::POST,
                        "/api/open/v1/device/edit",
                        &[],
                        Some(edit),
                    )
                    .await?;
                    Ok(Some(id))
                }
                None => Err(refused),
            },
        }
    }

    async fn unregister(
        &self,
        mac: &str,
        vendor_ref: Option<&str>,
    ) -> Result<(), RedirectionError> {
        let id = match vendor_ref {
            Some(id) => Some(id.to_string()),
            None => self.find(mac).await?,
        };
        let Some(id) = id else { return Ok(()) };
        self.call(
            reqwest::Method::POST,
            "/api/open/v1/device/delete",
            &[],
            Some(serde_json::json!({"ids": [id]})),
        )
        .await
        .map(|_| ())
    }

    async fn check(&self) -> Result<(), RedirectionError> {
        self.call(
            reqwest::Method::GET,
            "/api/open/v1/device/checkMac",
            &[("mac", "000000000000")],
            None,
        )
        .await
        .map(|_| ())
    }

    fn carries_credentials(&self) -> bool {
        true
    }
}

// ---- Poly ZTP ----

pub struct PolyZtp {
    http: reqwest::Client,
    url: String,
    user: String,
    password: String,
    account_id: String,
    profile: String,
}

/// A value safe in an XML attribute or text.
fn xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// The ZTP request envelope around one command.
pub fn ztp_request(user: &str, password: &str, command: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <request xmlns=\"http://schemas.alopa.com/Inbound\" userid=\"{}\" password=\"{}\" message-id=\"{}\">\n\
         {command}\n</request>\n",
        xml(user),
        xml(password),
        uuid::Uuid::new_v4().simple()
    )
}

/// An attribute of the first element named `element` in a response.
pub fn xml_attribute(document: &str, element: &str, attribute: &str) -> Option<String> {
    let start = document.find(&format!("<{element}"))?;
    let tag = &document[start..start + document[start..].find('>')?];
    let at = tag.find(&format!("{attribute}="))? + attribute.len() + 1;
    let rest = &tag[at..];
    let quote = rest.chars().next()?;
    let close = match quote {
        '\'' | '"' => quote,
        '\u{2018}' => '\u{2019}',
        '\u{201C}' => '\u{201D}',
        _ => return None,
    };
    let value = &rest[quote.len_utf8()..];
    let end = value.find(close)?;
    Some(
        value[..end]
            .replace("&apos;", "'")
            .replace("&quot;", "\"")
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&amp;", "&"),
    )
}

impl PolyZtp {
    async fn call(&self, command: String) -> Result<(), RedirectionError> {
        let body = ztp_request(&self.user, &self.password, &command);
        let response = self
            .http
            .post(&self.url)
            .header("Content-Type", "text/xml; charset=UTF-8")
            .body(body)
            .send()
            .await
            .map_err(|e| RedirectionError(format!("Poly ZTP did not answer: {e}")))?;
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        match xml_attribute(&text, "status", "ErrorCode") {
            Some(code) if code == "0000" => Ok(()),
            Some(code) => Err(RedirectionError(format!(
                "Poly ZTP refused ({code}): {}",
                xml_attribute(&text, "status", "ErrorMessage").unwrap_or_default()
            ))),
            None => Err(RedirectionError(format!(
                "Poly ZTP answered {status} without a status"
            ))),
        }
    }

    fn ready(&self) -> Result<(), RedirectionError> {
        if self.account_id.trim().is_empty() || self.profile.trim().is_empty() {
            return Err(RedirectionError(
                "the account needs its Reference ID and profile".to_string(),
            ));
        }
        Ok(())
    }

    fn add(&self, mac: &str) -> String {
        format!(
            "<add-sip-device account-id=\"{account}\"><device-params><deviceId>{mac}</deviceId>\
             <serialNo>{mac}</serialNo><vendor>Polycom</vendor>\
             <vendorModel>Polycom_UCS_Device</vendorModel></device-params>\
             <sip-device-common-params><templateCriteria>{profile}</templateCriteria>\
             </sip-device-common-params><package-data><base-package-name>default</base-package-name>\
             </package-data></add-sip-device>",
            account = xml(&self.account_id),
            mac = xml(mac),
            profile = xml(&self.profile),
        )
    }

    fn delete(&self, mac: &str) -> String {
        format!(
            "<delete-sip-device accountid=\"{account}\"><device-params><deviceId>{mac}</deviceId>\
             </device-params></delete-sip-device>",
            account = xml(&self.account_id),
            mac = xml(mac),
        )
    }
}

#[async_trait]
impl RedirectionConnector for PolyZtp {
    async fn register(
        &self,
        phone: &PhoneRegistration,
    ) -> Result<Option<String>, RedirectionError> {
        self.ready()?;
        if let Err(refused) = self.call(self.add(&phone.mac)).await {
            // Already associated: associate it again, with this profile.
            if self.call(self.delete(&phone.mac)).await.is_err() {
                return Err(refused);
            }
            self.call(self.add(&phone.mac)).await?;
        }
        Ok(None)
    }

    async fn unregister(
        &self,
        mac: &str,
        _vendor_ref: Option<&str>,
    ) -> Result<(), RedirectionError> {
        self.ready()?;
        // A device ZTP does not have answers an error; nothing is left either way.
        if let Err(e) = self.call(self.delete(mac)).await {
            tracing::debug!(mac, error = %e, "Poly ZTP did not delete a device");
        }
        Ok(())
    }

    async fn check(&self) -> Result<(), RedirectionError> {
        self.ready()?;
        self.call(format!(
            "<query-sip-device accountid=\"{}\"><device-params></device-params></query-sip-device>",
            xml(&self.account_id)
        ))
        .await
    }

    fn carries_credentials(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_rps_signature_covers_what_the_guide_says() {
        // The guide's query example (§1.3.4.2): GET checkMac, no body.
        let with_query = rps_signature(
            "d4a4be460a8d43609d8e8a5e7d0d4ad1",
            "GET",
            None,
            "2df23f2d9c255e7138dc603b3847b58a",
            "9e730a223b48433785494801fb016d39",
            "1544094691000",
            "/api/open/v1/device/checkMac",
            &[("mac", "001565123123")],
        );
        let mut mac = Hmac::<Sha256>::new_from_slice(b"d4a4be460a8d43609d8e8a5e7d0d4ad1").unwrap();
        mac.update(
            b"GET\nX-Ca-Key:2df23f2d9c255e7138dc603b3847b58a\nX-Ca-Nonce:9e730a223b48433785494801fb016d39\nX-Ca-Timestamp:1544094691000\napi/open/v1/device/checkMac\nmac=001565123123",
        );
        assert_eq!(
            with_query,
            base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes())
        );
        // With a body: Content-MD5 among the headers, no query line.
        let with_body = rps_signature(
            "s",
            "POST",
            Some("md5=="),
            "k",
            "n",
            "1",
            "/api/open/v1/device/add",
            &[("x", "y")],
        );
        let mut mac = Hmac::<Sha256>::new_from_slice(b"s").unwrap();
        mac.update(b"POST\nContent-MD5:md5==\nX-Ca-Key:k\nX-Ca-Nonce:n\nX-Ca-Timestamp:1\napi/open/v1/device/add");
        assert_eq!(
            with_body,
            base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes())
        );
        // Query keys are sorted, an empty value keeps only its key.
        let sorted = rps_signature(
            "s",
            "GET",
            None,
            "k",
            "n",
            "1",
            "p",
            &[("b", "2"), ("a", " ")],
        );
        let mut mac = Hmac::<Sha256>::new_from_slice(b"s").unwrap();
        mac.update(b"GET\nX-Ca-Key:k\nX-Ca-Nonce:n\nX-Ca-Timestamp:1\np\na&b=2");
        assert_eq!(
            sorted,
            base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes())
        );
    }

    #[test]
    fn content_md5_is_base64_of_the_digest() {
        assert_eq!(content_md5(b""), "1B2M2Y8AsgTpgAmY7PhCfg==");
    }

    #[test]
    fn a_ztp_answer_is_read_whatever_its_quotes() {
        let ok = "<?xml version='1.0'?><response message-id='1'><status ErrorCode='0000' ErrorMessage='Successful.'/></response>";
        assert_eq!(
            xml_attribute(ok, "status", "ErrorCode").as_deref(),
            Some("0000")
        );
        let curly = "<response><status ErrorCode=\u{2018}1002\u{2019} ErrorMessage=\u{2018}Device exists\u{2019}/></response>";
        assert_eq!(
            xml_attribute(curly, "status", "ErrorMessage").as_deref(),
            Some("Device exists")
        );
        assert_eq!(xml_attribute("<x/>", "status", "ErrorCode"), None);
        let request = ztp_request("u&1", "p\"w", "<cmd/>");
        assert!(
            request.contains("userid=\"u&amp;1\" password=\"p&quot;w\""),
            "{request}"
        );
    }
}
