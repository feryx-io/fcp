//! A directory as the archive's target: a mounted volume, an NFS export,
//! or a path another tool syncs to a bucket. Objects are written beside
//! their key under a temporary name, synced, and renamed into place, so a
//! reader never sees half an object.

use std::path::{Component, Path, PathBuf};

use async_trait::async_trait;

use super::{sha256_file, ArchiveError, ArchiveTarget, ObjectInfo};

#[derive(Debug, Clone)]
pub struct DirectoryTarget {
    root: PathBuf,
}

impl DirectoryTarget {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// The path for `key`, which must stay under the root.
    fn path(&self, key: &str) -> Result<PathBuf, ArchiveError> {
        let relative = Path::new(key);
        if key.is_empty()
            || relative
                .components()
                .any(|c| !matches!(c, Component::Normal(_)))
        {
            return Err(ArchiveError::Target(format!(
                "object key {key:?} is not a relative path under the archive"
            )));
        }
        Ok(self.root.join(relative))
    }
}

#[async_trait]
impl ArchiveTarget for DirectoryTarget {
    fn location(&self) -> String {
        format!("file://{}", self.root.display())
    }

    async fn put(&self, key: &str, file: &Path, _object: &ObjectInfo) -> Result<(), ArchiveError> {
        let dest = self.path(key)?;
        let parent = dest
            .parent()
            .ok_or_else(|| ArchiveError::Target(format!("object key {key:?} has no directory")))?
            .to_path_buf();
        let source = file.to_path_buf();
        tokio::task::spawn_blocking(move || -> std::io::Result<()> {
            std::fs::create_dir_all(&parent)?;
            let name = dest
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let temp = parent.join(format!(".{name}.{}.tmp", uuid::Uuid::new_v4().simple()));
            let result = (|| {
                std::fs::copy(&source, &temp)?;
                std::fs::File::open(&temp)?.sync_all()?;
                std::fs::rename(&temp, &dest)?;
                std::fs::File::open(&parent)?.sync_all()
            })();
            if result.is_err() {
                let _ = std::fs::remove_file(&temp);
            }
            result
        })
        .await
        .map_err(|e| ArchiveError::Target(e.to_string()))??;
        Ok(())
    }

    async fn confirm(&self, key: &str, object: &ObjectInfo) -> Result<bool, ArchiveError> {
        let path = self.path(key)?;
        match tokio::fs::metadata(&path).await {
            Ok(meta) if meta.len() == object.bytes => {}
            Ok(_) => return Ok(false),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(e) => return Err(e.into()),
        }
        Ok(sha256_file(&path).await? == object.sha256)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn puts_confirms_and_replaces_objects_under_the_root() {
        let root = tempfile::tempdir().unwrap();
        let work = tempfile::tempdir().unwrap();
        let target = DirectoryTarget::new(root.path());
        let file = work.path().join("object");

        std::fs::write(&file, b"first").unwrap();
        let first = ObjectInfo {
            bytes: 5,
            sha256: sha256_file(&file).await.unwrap(),
            records: 1,
        };
        assert!(!target
            .confirm("t/2026/09/13.jsonl.gz", &first)
            .await
            .unwrap());
        target
            .put("t/2026/09/13.jsonl.gz", &file, &first)
            .await
            .unwrap();
        assert!(target
            .confirm("t/2026/09/13.jsonl.gz", &first)
            .await
            .unwrap());
        assert_eq!(
            std::fs::read(root.path().join("t/2026/09/13.jsonl.gz")).unwrap(),
            b"first"
        );

        std::fs::write(&file, b"again").unwrap();
        let second = ObjectInfo {
            bytes: 5,
            sha256: sha256_file(&file).await.unwrap(),
            records: 2,
        };
        assert!(
            !target
                .confirm("t/2026/09/13.jsonl.gz", &second)
                .await
                .unwrap(),
            "same size, different content"
        );
        target
            .put("t/2026/09/13.jsonl.gz", &file, &second)
            .await
            .unwrap();
        assert!(target
            .confirm("t/2026/09/13.jsonl.gz", &second)
            .await
            .unwrap());
        let leftovers: Vec<_> = std::fs::read_dir(root.path().join("t/2026/09"))
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(leftovers.len(), 1, "no temporary files left: {leftovers:?}");

        for bad in ["", "../escape", "/abs/key", "a/../../b"] {
            assert!(target.put(bad, &file, &second).await.is_err(), "{bad:?}");
        }
    }
}
