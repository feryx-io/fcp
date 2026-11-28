//! Files put back from a snapshot (`docs/SCHEDULED_BACKUPS.md` B3b): a
//! path a service wrote into a row mapped to where it is mounted here,
//! whether it is there, which snapshots hold it, and a restore that never
//! overwrites what is there.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use crate::config::BackupSection;
use crate::restic::Repo;

/// What restores this service may do.
#[derive(Debug, Clone, Serialize)]
pub struct Capabilities {
    /// Every volume writable: files can be put back.
    pub file_restores: bool,
    /// The Docker socket mounted: the whole system can be restored (B3c).
    pub full_restore: bool,
    /// The volumes still read-only.
    pub read_only: Vec<String>,
}

pub fn capabilities(config: &BackupSection) -> Capabilities {
    let mut read_only = Vec::new();
    for (name, path) in &config.paths {
        if path.exists() && !writable(path) {
            read_only.push(name.clone());
        }
    }
    Capabilities {
        file_restores: read_only.is_empty() && !config.paths.is_empty(),
        full_restore: config.docker_socket.exists(),
        read_only,
    }
}

/// Whether a file can be made in `dir`: a read-only mount says not.
fn writable(dir: &Path) -> bool {
    let probe = dir.join(format!(".fcp-backup-probe-{}", std::process::id()));
    match std::fs::File::create(&probe) {
        Ok(_) => {
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

/// A plain absolute path: no `..`, nothing relative.
fn plain(path: &Path) -> bool {
    path.is_absolute() && !path.components().any(|c| matches!(c, Component::ParentDir))
}

/// Where a service's path is here, by the longest `service_paths` prefix,
/// and only inside a backed-up volume.
pub fn map(config: &BackupSection, service_path: &str) -> Option<PathBuf> {
    let p = Path::new(service_path);
    if !plain(p) {
        return None;
    }
    let (prefix, here) = config
        .service_paths
        .iter()
        .filter(|(prefix, _)| p.starts_with(prefix.as_str()))
        .max_by_key(|(prefix, _)| prefix.len())?;
    let rest = p.strip_prefix(prefix.as_str()).ok()?;
    let mapped = here.join(rest);
    config
        .paths
        .values()
        .any(|v| mapped.starts_with(v))
        .then_some(mapped)
}

/// A volume's path and a relative path within it, checked.
pub fn in_volume(config: &BackupSection, volume: &str, rel: &str) -> Result<PathBuf> {
    let root = config
        .paths
        .get(volume)
        .with_context(|| format!("no volume {volume}"))?;
    let rel = Path::new(rel.trim_start_matches('/'));
    if rel.components().any(|c| !matches!(c, Component::Normal(_))) && !rel.as_os_str().is_empty() {
        bail!("{} is not a plain path", rel.display());
    }
    Ok(root.join(rel))
}

#[derive(Debug, Clone, Serialize)]
pub struct Checked {
    pub path: String,
    /// Whether this service can reach it at all.
    pub held_here: bool,
    pub present: bool,
}

pub fn check(config: &BackupSection, paths: &[String]) -> Vec<Checked> {
    paths
        .iter()
        .map(|p| match map(config, p) {
            Some(here) => Checked {
                path: p.clone(),
                held_here: true,
                present: here.exists(),
            },
            None => Checked {
                path: p.clone(),
                held_here: false,
                present: false,
            },
        })
        .collect()
}

#[derive(Debug, Deserialize)]
struct FindHit {
    snapshot: String,
    #[serde(default)]
    matches: Vec<FindMatch>,
}

#[derive(Debug, Deserialize)]
struct FindMatch {
    path: String,
}

/// A snapshot holding a file.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Held {
    pub snapshot: String,
    pub time: String,
}

/// For each service path, the snapshots holding it, newest first.
pub async fn find(
    repo: &Repo,
    config: &BackupSection,
    paths: &[String],
) -> Result<BTreeMap<String, Vec<Held>>> {
    let mut wanted: BTreeMap<String, String> = BTreeMap::new();
    for p in paths {
        if let Some(here) = map(config, p) {
            wanted.insert(here.display().to_string(), p.clone());
        }
    }
    let mut out: BTreeMap<String, Vec<Held>> = BTreeMap::new();
    if wanted.is_empty() {
        return Ok(out);
    }
    let snapshots = repo.snapshots().await?;
    let mut args: Vec<String> = vec![
        "find".into(),
        "--json".into(),
        "--no-lock".into(),
        "--tag".into(),
        "part:files".into(),
    ];
    args.extend(wanted.keys().cloned());
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let json = repo.run(&refs).await?;
    let hits: Vec<FindHit> = serde_json::from_str(json.trim()).unwrap_or_default();
    let time = |id: &str| {
        snapshots
            .iter()
            .find(|s| s.id == id)
            .map(|s| s.time.clone())
            .unwrap_or_default()
    };
    for hit in hits {
        for m in hit.matches {
            if let Some(service) = wanted.get(&m.path) {
                out.entry(service.clone()).or_default().push(Held {
                    time: time(&hit.snapshot),
                    snapshot: hit.snapshot.clone(),
                });
            }
        }
    }
    // Newest first.
    for list in out.values_mut() {
        list.sort_by(|a, b| b.time.cmp(&a.time));
        list.dedup();
    }
    Ok(out)
}

/// Put `paths` (here) back from `snapshot`, never overwriting a file
/// already there; which of them are there after.
pub async fn restore(
    repo: &Repo,
    snapshot: &str,
    paths: &[PathBuf],
) -> Result<Vec<(PathBuf, bool)>> {
    if paths.is_empty() {
        return Ok(Vec::new());
    }
    let includes: Vec<String> = paths.iter().map(|p| p.display().to_string()).collect();
    repo.restore(snapshot, Path::new("/"), &includes, false)
        .await?;
    Ok(paths.iter().map(|p| (p.clone(), p.exists())).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> BackupSection {
        let mut c = BackupSection::default();
        c.paths
            .insert("call-manager-data".into(), "/data/call-manager".into());
        c.paths
            .insert("voicemail-data".into(), "/data/voicemail".into());
        c.service_paths.insert(
            "/app/data/recordings".into(),
            "/data/call-manager/recordings".into(),
        );
        c.service_paths
            .insert("/var/lib/fcp/voicemail".into(), "/data/voicemail".into());
        c.service_paths.insert("/etc".into(), "/etc".into());
        c
    }

    #[test]
    fn a_service_path_is_mapped_into_a_volume_or_not_at_all() {
        let c = config();
        assert_eq!(
            map(&c, "/app/data/recordings/t1/call.wav"),
            Some(PathBuf::from("/data/call-manager/recordings/t1/call.wav"))
        );
        assert_eq!(
            map(&c, "/var/lib/fcp/voicemail/recordings/m.wav"),
            Some(PathBuf::from("/data/voicemail/recordings/m.wav"))
        );
        // Out of a volume, climbing out, relative, or unknown: none.
        assert_eq!(map(&c, "/etc/passwd"), None);
        assert_eq!(map(&c, "/var/lib/fcp/voicemail/../../../etc/passwd"), None);
        assert_eq!(map(&c, "recordings/x.wav"), None);
        assert_eq!(map(&c, "/somewhere/else.wav"), None);
        // A prefix by path components, not by characters.
        assert_eq!(map(&c, "/app/data/recordings-old/x.wav"), None);
    }

    #[test]
    fn a_volume_path_stays_in_its_volume() {
        let c = config();
        assert_eq!(
            in_volume(&c, "voicemail-data", "recordings/m.wav").unwrap(),
            PathBuf::from("/data/voicemail/recordings/m.wav")
        );
        assert_eq!(
            in_volume(&c, "voicemail-data", "").unwrap(),
            PathBuf::from("/data/voicemail")
        );
        assert!(in_volume(&c, "voicemail-data", "../call-manager").is_err());
        assert!(in_volume(&c, "nope", "x").is_err());
    }
}
