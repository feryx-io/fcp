//! Firmware images in an S3-compatible bucket (`[api.firmware.s3]`,
//! `docs/PROVISIONING.md` §11), through the CDR archive's S3 client: every
//! admin of a deployment serves the same images.

use async_trait::async_trait;
use axum::body::Body;
use fcp_api_core::firmware_files::FirmwareFiles;
use fcp_cdr::archive::{ArchiveTarget, ObjectInfo, S3Config, S3Target};
use std::path::Path;

pub struct S3Files {
    target: S3Target,
    bucket: String,
    prefix: String,
}

impl S3Files {
    pub fn new(config: &fcp_core::FirmwareS3Config) -> anyhow::Result<Self> {
        let target = S3Target::new(S3Config {
            region: config.region.clone(),
            endpoint: config.endpoint.clone(),
            path_style: config.path_style,
            access_key_id: config.access_key_id.clone(),
            secret_access_key: config.secret_access_key.clone(),
            session_token: config.session_token.clone(),
            ..S3Config::new(config.bucket.clone())
        })
        .map_err(|e| anyhow::anyhow!("api.firmware.s3: {e}"))?;
        Ok(Self {
            target,
            bucket: config.bucket.clone(),
            prefix: config.prefix.clone(),
        })
    }

    fn key(&self, key: &str) -> String {
        format!("{}{key}", self.prefix)
    }
}

#[async_trait]
impl FirmwareFiles for S3Files {
    fn location(&self) -> String {
        format!("s3://{}/{}", self.bucket, self.prefix)
    }

    async fn put(&self, key: &str, file: &Path, size: u64, sha256: &str) -> Result<(), String> {
        let object = ObjectInfo {
            bytes: size,
            sha256: sha256.to_string(),
            records: 0,
        };
        let key = self.key(key);
        self.target
            .put(&key, file, &object)
            .await
            .map_err(|e| e.to_string())?;
        // What is there is what was sent.
        match self.target.confirm(&key, &object).await {
            Ok(true) => Ok(()),
            Ok(false) => Err("the bucket holds another object than the one sent".to_string()),
            Err(e) => Err(e.to_string()),
        }
    }

    async fn open(&self, key: &str) -> Result<Option<(u64, Body)>, String> {
        let Some(response) = self
            .target
            .get(&self.key(key))
            .await
            .map_err(|e| e.to_string())?
        else {
            return Ok(None);
        };
        let size = response.content_length().unwrap_or(0);
        Ok(Some((size, Body::from_stream(response.bytes_stream()))))
    }

    async fn delete(&self, key: &str) -> Result<(), String> {
        self.target
            .delete(&self.key(key))
            .await
            .map_err(|e| e.to_string())
    }
}
