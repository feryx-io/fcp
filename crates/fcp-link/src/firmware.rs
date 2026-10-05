//! A site's firmware (`docs/PROVISIONING.md` §11, Q11): the images its
//! tenant's targets name, fetched from the cloud over the link and kept in
//! `[site] firmware_dir`, which the site's admin serves to its phones as
//! its `[api.firmware] directory`. Checked against each image's SHA-256;
//! looked at whenever a target arrives and every few minutes.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use fcp_core::firmware::FirmwareImage;
use fcp_core::{DynStorage, TenantId};
use futures_util::StreamExt;
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;
use tokio::sync::Notify;
use tracing::{info, warn};

/// How often the cache is looked at without a nudge.
const EVERY: Duration = Duration::from_secs(300);

pub struct FirmwareCache {
    client: reqwest::Client,
    link_url: String,
    secret: String,
    tenant: TenantId,
    storage: Arc<DynStorage>,
    dir: PathBuf,
    nudge: Notify,
}

impl FirmwareCache {
    pub fn new(
        client: reqwest::Client,
        link_url: &str,
        secret: String,
        tenant: TenantId,
        storage: Arc<DynStorage>,
        dir: impl Into<PathBuf>,
    ) -> Self {
        Self {
            client,
            link_url: link_url.trim_end_matches('/').to_string(),
            secret,
            tenant,
            storage,
            dir: dir.into(),
            nudge: Notify::new(),
        }
    }

    /// Look again soon: a target arrived or went.
    pub fn nudge(&self) {
        self.nudge.notify_one();
    }

    /// Fill the cache, then keep it filled.
    pub async fn run(&self) {
        loop {
            if let Err(e) = self.reconcile().await {
                warn!(error = %format!("{e:#}"), "The site's firmware could not be brought up to date");
            }
            tokio::select! {
                _ = self.nudge.notified() => {}
                _ = tokio::time::sleep(EVERY) => {}
            }
        }
    }

    fn path(&self, image: &FirmwareImage) -> PathBuf {
        self.dir
            .join(image.id.simple().to_string())
            .join(&image.file_name)
    }

    /// Every image a target names, present and whole.
    pub async fn reconcile(&self) -> Result<()> {
        let Some(store) = self.storage.provisioning_store() else {
            return Ok(());
        };
        for target in store.list_firmware_targets(&self.tenant).await? {
            let Some(image) = store.get_firmware_image(&target.image_id).await? else {
                continue;
            };
            let path = self.path(&image);
            let present = tokio::fs::metadata(&path)
                .await
                .is_ok_and(|m| m.len() == image.size);
            if !present {
                self.fetch(&image)
                    .await
                    .with_context(|| format!("fetching image {}", image.id))?;
            }
        }
        Ok(())
    }

    /// The image from the cloud, written beside its place and moved there
    /// only once its hash is the image's.
    async fn fetch(&self, image: &FirmwareImage) -> Result<()> {
        let url = format!("{}/link/v1/firmware/{}", self.link_url, image.id);
        let response = self
            .client
            .get(&url)
            .bearer_auth(&self.secret)
            .send()
            .await
            .context("reaching the cloud")?;
        if !response.status().is_success() {
            anyhow::bail!("the cloud answered {}", response.status());
        }
        let path = self.path(image);
        let parent = path.parent().context("an image's path has a directory")?;
        tokio::fs::create_dir_all(parent).await?;
        let partial = parent.join(format!(".{}.partial", image.file_name));
        let mut out = tokio::fs::File::create(&partial).await?;
        let mut hash = Sha256::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.context("the download broke off")?;
            hash.update(&chunk);
            out.write_all(&chunk).await?;
        }
        out.flush().await?;
        drop(out);
        let got = format!("{:x}", hash.finalize());
        if got != image.sha256 {
            let _ = tokio::fs::remove_file(&partial).await;
            anyhow::bail!("its SHA-256 is {got}, not the image's {}", image.sha256);
        }
        tokio::fs::rename(&partial, &path).await?;
        info!(image_id = %image.id, file = %image.file_name, size = image.size, "Firmware image cached for the site");
        Ok(())
    }

    /// An image no target names any more: its file goes.
    pub async fn forget(&self, image: &FirmwareImage) {
        let dir = self.dir.join(image.id.simple().to_string());
        let _ = tokio::fs::remove_dir_all(dir).await;
    }
}
