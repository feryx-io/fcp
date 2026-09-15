//! An S3-compatible bucket as the archive's target: AWS S3, or any store
//! that speaks its API with SigV4 (MinIO, Garage, Ceph RGW, Cloudflare R2,
//! Backblaze B2). An object up to `multipart_threshold` goes in one PUT
//! streamed from the file; a larger one in parts. Every body's SHA-256 is
//! signed, so the store refuses a body that is not the one sent, and the
//! object carries it as `x-amz-meta-sha256`, which a HEAD confirms.

use std::path::Path;
use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use reqwest::{Method, StatusCode, Url};
use serde::{Deserialize, Serialize};
use tokio::io::AsyncReadExt;
use tracing::{debug, warn};

use super::sigv4::{self, Credentials, EMPTY_SHA256};
use super::{ArchiveError, ArchiveTarget, ObjectInfo};

/// How to reach the bucket.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct S3Config {
    pub bucket: String,
    #[serde(default = "default_region")]
    pub region: String,
    /// The service's base URL; AWS's regional endpoint when unset.
    #[serde(default)]
    pub endpoint: Option<String>,
    /// Address the bucket in the path (`endpoint/bucket/key`) rather than
    /// the host name (`bucket.endpoint/key`). Most self-hosted stores want
    /// this.
    #[serde(default)]
    pub path_style: bool,
    /// Credentials; `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY` and
    /// `AWS_SESSION_TOKEN` when unset.
    #[serde(default)]
    pub access_key_id: Option<String>,
    #[serde(default)]
    pub secret_access_key: Option<String>,
    #[serde(default)]
    pub session_token: Option<String>,
    /// `x-amz-storage-class` for the objects (`STANDARD_IA`, `GLACIER_IR`).
    #[serde(default)]
    pub storage_class: Option<String>,
    /// Objects larger than this go in parts, of `part_size` bytes each
    /// (S3's minimum part is 5 MiB).
    #[serde(default = "default_multipart_threshold")]
    pub multipart_threshold: u64,
    #[serde(default = "default_part_size")]
    pub part_size: u64,
}

fn default_region() -> String {
    "us-east-1".to_string()
}
fn default_multipart_threshold() -> u64 {
    256 * 1024 * 1024
}
fn default_part_size() -> u64 {
    64 * 1024 * 1024
}

impl S3Config {
    pub fn new(bucket: impl Into<String>) -> Self {
        Self {
            bucket: bucket.into(),
            region: default_region(),
            endpoint: None,
            path_style: false,
            access_key_id: None,
            secret_access_key: None,
            session_token: None,
            storage_class: None,
            multipart_threshold: default_multipart_threshold(),
            part_size: default_part_size(),
        }
    }
}

const MIN_PART: u64 = 5 * 1024 * 1024;
const ATTEMPTS: u32 = 3;

/// How long a request may take: a minute, and a second more per MiB it
/// carries, so a stalled connection is given up on and a large body that
/// is still moving is not.
fn deadline(bytes: u64) -> Duration {
    Duration::from_secs(60 + bytes / (1024 * 1024))
}

/// The bucket.
pub struct S3Target {
    config: S3Config,
    credentials: Credentials,
    base: Url,
    client: reqwest::Client,
}

impl std::fmt::Debug for S3Target {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("S3Target")
            .field("bucket", &self.config.bucket)
            .field("base", &self.base.as_str())
            .finish()
    }
}

/// A failed request: whether trying again could help, and what the store
/// said.
struct Failure {
    retry: bool,
    message: String,
}

impl From<reqwest::Error> for Failure {
    fn from(e: reqwest::Error) -> Self {
        Failure {
            retry: true,
            message: e.to_string(),
        }
    }
}

impl From<std::io::Error> for Failure {
    fn from(e: std::io::Error) -> Self {
        Failure {
            retry: false,
            message: e.to_string(),
        }
    }
}

impl S3Target {
    pub fn new(config: S3Config) -> Result<Self, ArchiveError> {
        let env = |name: &str| std::env::var(name).ok().filter(|v| !v.is_empty());
        let access_key_id = config
            .access_key_id
            .clone()
            .or_else(|| env("AWS_ACCESS_KEY_ID"))
            .ok_or_else(|| ArchiveError::Config("no S3 access key id".into()))?;
        let secret_access_key = config
            .secret_access_key
            .clone()
            .or_else(|| env("AWS_SECRET_ACCESS_KEY"))
            .ok_or_else(|| ArchiveError::Config("no S3 secret access key".into()))?;
        let session_token = config
            .session_token
            .clone()
            .or_else(|| env("AWS_SESSION_TOKEN"));
        if config.bucket.is_empty() {
            return Err(ArchiveError::Config("no S3 bucket".into()));
        }
        if config.part_size < MIN_PART {
            return Err(ArchiveError::Config(format!(
                "S3 part_size must be at least {MIN_PART} bytes"
            )));
        }
        let endpoint = config
            .endpoint
            .clone()
            .unwrap_or_else(|| format!("https://s3.{}.amazonaws.com", config.region));
        let mut base = Url::parse(&endpoint)
            .map_err(|e| ArchiveError::Config(format!("S3 endpoint {endpoint}: {e}")))?;
        if config.path_style {
            let path = format!(
                "{}/{}/",
                base.path().trim_end_matches('/'),
                sigv4::uri_encode(&config.bucket, false)
            );
            base.set_path(&path);
        } else {
            let host = base
                .host_str()
                .ok_or_else(|| ArchiveError::Config(format!("S3 endpoint {endpoint} has no host")))?
                .to_string();
            base.set_host(Some(&format!("{}.{host}", config.bucket)))
                .map_err(|e| ArchiveError::Config(format!("S3 bucket host: {e}")))?;
            let path = format!("{}/", base.path().trim_end_matches('/'));
            base.set_path(&path);
        }
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .build()
            .map_err(|e| ArchiveError::Config(format!("HTTP client: {e}")))?;
        Ok(Self {
            config,
            credentials: Credentials {
                access_key_id,
                secret_access_key,
                session_token,
            },
            base,
            client,
        })
    }

    fn url(&self, key: &str, query: &[(String, String)]) -> Result<Url, Failure> {
        let mut url = self.base.clone();
        let path = format!("{}{}", url.path(), sigv4::uri_encode(key, true));
        url.set_path(&path);
        if !query.is_empty() {
            let text = query
                .iter()
                .map(|(k, v)| {
                    if v.is_empty() {
                        sigv4::uri_encode(k, false)
                    } else {
                        format!(
                            "{}={}",
                            sigv4::uri_encode(k, false),
                            sigv4::uri_encode(v, false)
                        )
                    }
                })
                .collect::<Vec<_>>()
                .join("&");
            url.set_query(Some(&text));
        }
        Ok(url)
    }

    /// A signed request. `headers` are signed along with the host, date,
    /// payload hash and session token.
    fn request(
        &self,
        method: Method,
        key: &str,
        query: &[(String, String)],
        mut headers: Vec<(String, String)>,
        payload_sha256: &str,
    ) -> Result<reqwest::RequestBuilder, Failure> {
        let url = self.url(key, query)?;
        let host = match url.port() {
            Some(port) => format!("{}:{port}", url.host_str().unwrap_or_default()),
            None => url.host_str().unwrap_or_default().to_string(),
        };
        let now: DateTime<Utc> = SystemTime::now().into();
        headers.push(("host".into(), host));
        headers.push((
            "x-amz-date".into(),
            now.format("%Y%m%dT%H%M%SZ").to_string(),
        ));
        headers.push(("x-amz-content-sha256".into(), payload_sha256.to_string()));
        if let Some(token) = &self.credentials.session_token {
            headers.push(("x-amz-security-token".into(), token.clone()));
        }
        let authorization = sigv4::authorization(
            &sigv4::Request {
                method: method.as_str(),
                path: url.path(),
                query,
                headers: &headers,
                payload_sha256,
            },
            &self.credentials,
            &self.config.region,
            "s3",
            now,
        );
        let mut builder = self.client.request(method, url).timeout(deadline(0));
        for (name, value) in headers {
            if name != "host" {
                builder = builder.header(name, value);
            }
        }
        Ok(builder.header("authorization", authorization))
    }

    async fn check(response: reqwest::Response) -> Result<reqwest::Response, Failure> {
        let status = response.status();
        if status.is_success() {
            return Ok(response);
        }
        let body = response.text().await.unwrap_or_default();
        Err(Failure {
            retry: status.is_server_error() || status == StatusCode::TOO_MANY_REQUESTS,
            message: format!("{status}: {}", error_code(&body).unwrap_or(&body)),
        })
    }

    /// Run `op` until it succeeds, fails for good, or runs out of attempts.
    async fn retrying<T, F, Fut>(&self, what: &str, mut op: F) -> Result<T, ArchiveError>
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = Result<T, Failure>>,
    {
        let mut attempt = 1;
        loop {
            match op().await {
                Ok(v) => return Ok(v),
                Err(f) if f.retry && attempt < ATTEMPTS => {
                    warn!(operation = what, attempt, error = %f.message, "S3 request failed; trying again");
                    tokio::time::sleep(Duration::from_millis(500 * 2u64.pow(attempt))).await;
                    attempt += 1;
                }
                Err(f) => return Err(ArchiveError::Target(format!("S3 {what}: {}", f.message))),
            }
        }
    }

    fn object_headers(&self, object: &ObjectInfo) -> Vec<(String, String)> {
        let mut headers = vec![
            ("content-type".into(), "application/gzip".into()),
            ("x-amz-meta-sha256".into(), object.sha256.clone()),
            ("x-amz-meta-records".into(), object.records.to_string()),
        ];
        if let Some(class) = &self.config.storage_class {
            headers.push(("x-amz-storage-class".into(), class.clone()));
        }
        headers
    }

    async fn put_whole(
        &self,
        key: &str,
        file: &Path,
        object: &ObjectInfo,
    ) -> Result<(), ArchiveError> {
        self.retrying("PUT", || async {
            let handle = tokio::fs::File::open(file).await?;
            let body = reqwest::Body::wrap_stream(tokio_util::io::ReaderStream::new(handle));
            let response = self
                .request(
                    Method::PUT,
                    key,
                    &[],
                    self.object_headers(object),
                    &object.sha256,
                )?
                .header("content-length", object.bytes)
                .timeout(deadline(object.bytes))
                .body(body)
                .send()
                .await?;
            Self::check(response).await.map(|_| ())
        })
        .await
    }

    async fn put_parts(
        &self,
        key: &str,
        file: &Path,
        object: &ObjectInfo,
    ) -> Result<(), ArchiveError> {
        let upload_id = self
            .retrying("CreateMultipartUpload", || async {
                let response = self
                    .request(
                        Method::POST,
                        key,
                        &[("uploads".into(), String::new())],
                        self.object_headers(object),
                        EMPTY_SHA256,
                    )?
                    .send()
                    .await?;
                let body = Self::check(response).await?.text().await?;
                xml_value(&body, "UploadId")
                    .map(str::to_string)
                    .ok_or(Failure {
                        retry: false,
                        message: format!("no UploadId in {body}"),
                    })
            })
            .await?;
        match self.upload_parts(key, file, &upload_id).await {
            Ok(()) => Ok(()),
            Err(e) => {
                let abort = self
                    .request(
                        Method::DELETE,
                        key,
                        &[("uploadId".into(), upload_id.clone())],
                        Vec::new(),
                        EMPTY_SHA256,
                    )
                    .map(|r| r.send());
                if let Ok(send) = abort {
                    if let Err(abort_error) = send.await {
                        warn!(key, error = %abort_error, "Could not abort a multipart upload");
                    }
                }
                Err(e)
            }
        }
    }

    async fn upload_parts(
        &self,
        key: &str,
        file: &Path,
        upload_id: &str,
    ) -> Result<(), ArchiveError> {
        let mut handle = tokio::fs::File::open(file).await?;
        let mut etags = Vec::new();
        let mut number = 0u32;
        loop {
            let mut part = Vec::with_capacity(self.config.part_size as usize);
            (&mut handle)
                .take(self.config.part_size)
                .read_to_end(&mut part)
                .await?;
            if part.is_empty() && number > 0 {
                break;
            }
            number += 1;
            let hash = sigv4::sha256_hex(&part);
            let query = vec![
                ("partNumber".to_string(), number.to_string()),
                ("uploadId".to_string(), upload_id.to_string()),
            ];
            let size = part.len();
            let part = bytes::Bytes::from(part);
            let etag = self
                .retrying("UploadPart", || async {
                    let response = self
                        .request(Method::PUT, key, &query, Vec::new(), &hash)?
                        .timeout(deadline(size as u64))
                        .body(part.clone())
                        .send()
                        .await?;
                    let response = Self::check(response).await?;
                    response
                        .headers()
                        .get("etag")
                        .and_then(|v| v.to_str().ok())
                        .map(str::to_string)
                        .ok_or(Failure {
                            retry: false,
                            message: "no ETag for a part".into(),
                        })
                })
                .await?;
            debug!(key, part = number, bytes = size, "Uploaded an archive part");
            etags.push((number, etag));
            if (size as u64) < self.config.part_size {
                break;
            }
        }
        let body: String = format!(
            "<CompleteMultipartUpload>{}</CompleteMultipartUpload>",
            etags
                .iter()
                .map(|(n, etag)| format!(
                    "<Part><PartNumber>{n}</PartNumber><ETag>{}</ETag></Part>",
                    xml_escape(etag)
                ))
                .collect::<String>()
        );
        let hash = sigv4::sha256_hex(body.as_bytes());
        let query = vec![("uploadId".to_string(), upload_id.to_string())];
        self.retrying("CompleteMultipartUpload", || async {
            let response = self
                .request(
                    Method::POST,
                    key,
                    &query,
                    vec![("content-type".into(), "application/xml".into())],
                    &hash,
                )?
                .body(body.clone())
                .send()
                .await?;
            // S3 can answer 200 and put the error in the body.
            let text = Self::check(response).await?.text().await?;
            match error_code(&text) {
                Some(code) if text.contains("<Error>") => Err(Failure {
                    retry: code == "InternalError" || code == "SlowDown",
                    message: code.to_string(),
                }),
                _ => Ok(()),
            }
        })
        .await
    }
}

/// The body of a `PutObjectLegalHold` request.
fn legal_hold_body(on: bool) -> String {
    format!(
        "<LegalHold xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\"><Status>{}</Status></LegalHold>",
        if on { "ON" } else { "OFF" }
    )
}

/// The `Content-MD5` of a body: base64 of its MD5.
fn content_md5(body: &[u8]) -> String {
    use base64::Engine;
    use md5::{Digest, Md5};
    base64::engine::general_purpose::STANDARD.encode(Md5::digest(body))
}

/// The text of the first `<name>` element.
fn xml_value<'a>(xml: &'a str, name: &str) -> Option<&'a str> {
    let open = format!("<{name}>");
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&format!("</{name}>"))? + start;
    Some(&xml[start..end])
}

fn error_code(xml: &str) -> Option<&str> {
    xml_value(xml, "Code")
}

fn xml_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[async_trait]
impl ArchiveTarget for S3Target {
    fn location(&self) -> String {
        format!("s3://{}", self.config.bucket)
    }

    async fn put(&self, key: &str, file: &Path, object: &ObjectInfo) -> Result<(), ArchiveError> {
        if object.bytes > self.config.multipart_threshold {
            self.put_parts(key, file, object).await
        } else {
            self.put_whole(key, file, object).await
        }
    }

    async fn set_legal_hold(&self, key: &str, on: bool) -> Result<bool, ArchiveError> {
        // S3 Object Lock's legal hold: the bucket must have Object Lock
        // enabled, and the request must carry the body's MD5.
        let body = legal_hold_body(on);
        let hash = sigv4::sha256_hex(body.as_bytes());
        let md5 = content_md5(body.as_bytes());
        let query = vec![("legal-hold".to_string(), String::new())];
        self.retrying("PutObjectLegalHold", || async {
            let response = self
                .request(
                    Method::PUT,
                    key,
                    &query,
                    vec![
                        ("content-md5".into(), md5.clone()),
                        ("content-type".into(), "application/xml".into()),
                    ],
                    &hash,
                )?
                .body(body.clone())
                .send()
                .await?;
            Self::check(response).await.map(|_| true)
        })
        .await
    }

    async fn confirm(&self, key: &str, object: &ObjectInfo) -> Result<bool, ArchiveError> {
        self.retrying("HEAD", || async {
            let response = self
                .request(Method::HEAD, key, &[], Vec::new(), EMPTY_SHA256)?
                .send()
                .await?;
            if response.status() == StatusCode::NOT_FOUND {
                return Ok(false);
            }
            let response = Self::check(response).await?;
            let headers = response.headers();
            let length = headers
                .get("content-length")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<u64>().ok());
            let sha256 = headers
                .get("x-amz-meta-sha256")
                .and_then(|v| v.to_str().ok());
            Ok(length == Some(object.bytes) && sha256 == Some(object.sha256.as_str()))
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(config: S3Config) -> S3Target {
        S3Target::new(S3Config {
            access_key_id: Some("AKID".into()),
            secret_access_key: Some("secret".into()),
            ..config
        })
        .unwrap()
    }

    #[test]
    fn addresses_the_bucket_by_host_or_by_path() {
        let aws = target(S3Config {
            region: "eu-west-1".into(),
            ..S3Config::new("records")
        });
        assert_eq!(
            aws.url("cdrs/tenant-1/2026/09/13.jsonl.gz", &[])
                .ok()
                .unwrap()
                .as_str(),
            "https://records.s3.eu-west-1.amazonaws.com/cdrs/tenant-1/2026/09/13.jsonl.gz"
        );
        let local = target(S3Config {
            endpoint: Some("http://127.0.0.1:3900".into()),
            path_style: true,
            ..S3Config::new("records")
        });
        assert_eq!(
            local
                .url(
                    "a b/c",
                    &[
                        ("partNumber".into(), "2".into()),
                        ("uploadId".into(), "x/y".into())
                    ]
                )
                .ok()
                .unwrap()
                .as_str(),
            "http://127.0.0.1:3900/records/a%20b/c?partNumber=2&uploadId=x%2Fy"
        );
        assert_eq!(
            local
                .url("k", &[("uploads".into(), String::new())])
                .ok()
                .unwrap()
                .as_str(),
            "http://127.0.0.1:3900/records/k?uploads"
        );
        assert_eq!(local.location(), "s3://records");
    }

    #[test]
    fn refuses_what_cannot_work() {
        let no_keys = S3Target::new(S3Config {
            access_key_id: Some("AKID".into()),
            secret_access_key: None,
            ..S3Config::new("records")
        });
        if std::env::var("AWS_SECRET_ACCESS_KEY").is_err() {
            assert!(no_keys.is_err());
        }
        assert!(S3Target::new(S3Config {
            access_key_id: Some("AKID".into()),
            secret_access_key: Some("secret".into()),
            part_size: 1024,
            ..S3Config::new("records")
        })
        .is_err());
    }

    #[tokio::test]
    async fn a_legal_hold_is_a_signed_put_with_the_bodys_md5() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut seen = Vec::new();
            let mut buf = [0u8; 4096];
            loop {
                let n = socket.read(&mut buf).await.unwrap();
                seen.extend_from_slice(&buf[..n]);
                let text = String::from_utf8_lossy(&seen);
                if let Some(end) = text.find("\r\n\r\n") {
                    let length = text
                        .lines()
                        .find_map(|l| {
                            l.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(|v| v.trim().parse::<usize>().unwrap())
                        })
                        .unwrap_or(0);
                    if seen.len() >= end + 4 + length {
                        break;
                    }
                }
            }
            socket
                .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 0\r\n\r\n")
                .await
                .unwrap();
            String::from_utf8(seen).unwrap()
        });
        let target = target(S3Config {
            endpoint: Some(format!("http://{addr}")),
            path_style: true,
            ..S3Config::new("records")
        });
        assert!(target
            .set_legal_hold("t/2026/09/13.jsonl.gz", true)
            .await
            .unwrap());
        let request = server.await.unwrap();
        let lower = request.to_ascii_lowercase();
        assert!(
            request.starts_with("PUT /records/t/2026/09/13.jsonl.gz?legal-hold HTTP/1.1"),
            "{request}"
        );
        let body = legal_hold_body(true);
        assert!(request.ends_with(&body), "{request}");
        assert!(lower.contains(&format!(
            "content-md5: {}",
            content_md5(body.as_bytes()).to_ascii_lowercase()
        )));
        assert!(lower.contains(&format!(
            "x-amz-content-sha256: {}",
            sigv4::sha256_hex(body.as_bytes())
        )));
        assert!(
            lower.contains(
                "signedheaders=content-md5;content-type;host;x-amz-content-sha256;x-amz-date"
            ),
            "{request}"
        );
        assert!(body.contains("<Status>ON</Status>"));
        assert!(legal_hold_body(false).contains("<Status>OFF</Status>"));
        // The MD5 of the empty body, as S3's reference gives it.
        assert_eq!(content_md5(b""), "1B2M2Y8AsgTpgAmY7PhCfg==");
    }

    #[test]
    fn reads_the_answers_it_needs() {
        let created = "<?xml version=\"1.0\"?><InitiateMultipartUploadResult><Bucket>b</Bucket><Key>k</Key><UploadId>abc-123</UploadId></InitiateMultipartUploadResult>";
        assert_eq!(xml_value(created, "UploadId"), Some("abc-123"));
        let error = "<Error><Code>SlowDown</Code><Message>Reduce your rate</Message></Error>";
        assert_eq!(error_code(error), Some("SlowDown"));
        assert_eq!(xml_escape("\"etag\""), "&quot;etag&quot;");
    }
}
