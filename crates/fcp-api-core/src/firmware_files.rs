//! Where firmware images are kept (`docs/PROVISIONING.md` §11): a
//! directory on the admin, or an S3-compatible bucket, which the admin
//! wires in (`fcp-admin`'s `firmware_s3`).

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use axum::body::Body;

/// The images' store: keys are `<image>/<file>`.
#[async_trait]
pub trait FirmwareFiles: Send + Sync {
    /// Where, for logs and the operator: `file:///…`, `s3://bucket/prefix`.
    fn location(&self) -> String;
    /// Store a finished upload (`file`, staged on local disk) as `key`. The
    /// staged file is the caller's to remove afterwards, if it is still
    /// there.
    async fn put(&self, key: &str, file: &Path, size: u64, sha256: &str) -> Result<(), String>;
    /// The object's length and bytes, or `None` when there is none.
    async fn open(&self, key: &str) -> Result<Option<(u64, Body)>, String>;
    /// Remove the object; one that is not there is no error.
    async fn delete(&self, key: &str) -> Result<(), String>;
}

/// Images in a directory: `<root>/<image>/<file>`.
pub struct DirectoryFiles {
    root: PathBuf,
}

impl DirectoryFiles {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    fn path(&self, key: &str) -> Result<PathBuf, String> {
        // Keys are made by the caller from an id and a checked file name;
        // never anything that climbs out.
        if key
            .split('/')
            .any(|p| p.is_empty() || p == "." || p == "..")
        {
            return Err(format!("`{key}` is not an image's key"));
        }
        Ok(self.root.join(key))
    }
}

#[async_trait]
impl FirmwareFiles for DirectoryFiles {
    fn location(&self) -> String {
        format!("file://{}", self.root.display())
    }

    async fn put(&self, key: &str, file: &Path, _size: u64, _sha256: &str) -> Result<(), String> {
        let path = self.path(key)?;
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| format!("cannot write images: {e}"))?;
        }
        if tokio::fs::rename(file, &path).await.is_err() {
            // Another file system: copy, and leave the staged file to the
            // caller.
            tokio::fs::copy(file, &path)
                .await
                .map_err(|e| format!("cannot write the image: {e}"))?;
        }
        Ok(())
    }

    async fn open(&self, key: &str) -> Result<Option<(u64, Body)>, String> {
        let path = self.path(key)?;
        let file = match tokio::fs::File::open(&path).await {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(format!("cannot read the image: {e}")),
        };
        let size = file
            .metadata()
            .await
            .map_err(|e| format!("cannot read the image: {e}"))?
            .len();
        Ok(Some((
            size,
            Body::from_stream(tokio_util::io::ReaderStream::new(file)),
        )))
    }

    async fn delete(&self, key: &str) -> Result<(), String> {
        let path = self.path(key)?;
        match tokio::fs::remove_file(&path).await {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("cannot remove the image: {e}")),
        }
        // The image's own directory, now empty.
        if let Some(parent) = path.parent() {
            let _ = tokio::fs::remove_dir(parent).await;
        }
        Ok(())
    }
}
