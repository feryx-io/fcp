//! A run, carried out (`docs/SCHEDULED_BACKUPS.md` B1): a test reaches the
//! destination, pins its host key and makes the repository; a backup
//! streams the database into one snapshot and the files into another, then
//! keeps what retention says.

use anyhow::{bail, Context, Result};
use fcp_core::backup::{BackupDestination, BackupRun, BackupRunKind, BackupStore, BackupTarget};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::process::Command;
use tracing::info;

use crate::config::BackupConfig;
use crate::restic::Repo;

pub struct Ctx {
    pub config: BackupConfig,
    pub store: Arc<dyn BackupStore>,
}

/// What a run did, for its row and its event.
#[derive(Debug, Default)]
pub struct Outcome {
    pub snapshots: Vec<String>,
    pub bytes_added: u64,
    pub message: String,
}

/// Owner-only, for a key file.
#[cfg(unix)]
async fn private(path: &std::path::Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    tokio::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).await?;
    Ok(())
}

/// The repository a destination names, with its secrets in place. An SFTP
/// destination's host key is pinned by its test and required after.
async fn repo_for(ctx: &Ctx, dest: &mut BackupDestination, testing: bool) -> Result<Repo> {
    let work = &ctx.config.backup.work_dir;
    let id = dest.id.to_string();
    let password = dest
        .password
        .clone()
        .filter(|p| !p.is_empty())
        .context("the destination has no repository password")?;
    let mut env = Vec::new();
    let mut options = Vec::new();
    match &mut dest.target {
        BackupTarget::Sftp {
            host,
            port,
            user,
            private_key,
            host_key,
            ..
        } => {
            let keys = work.join("keys");
            tokio::fs::create_dir_all(&keys).await?;
            let key_file = keys.join(&id);
            let key = private_key
                .clone()
                .context("the destination has no private key")?;
            let key = if key.ends_with('\n') {
                key
            } else {
                format!("{key}\n")
            };
            tokio::fs::write(&key_file, key).await?;
            private(&key_file).await?;
            if host_key.is_none() {
                if !testing {
                    bail!("the server's host key is not pinned: test the destination first");
                }
                let scanned = Command::new(&ctx.config.backup.ssh_keyscan)
                    .args(["-p", &port.to_string(), "-T", "10", host.as_str()])
                    .output()
                    .await
                    .context("ssh-keyscan")?;
                let found: Vec<String> = String::from_utf8_lossy(&scanned.stdout)
                    .lines()
                    .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
                    .map(str::to_string)
                    .collect();
                if found.is_empty() {
                    bail!(
                        "the server at {host}:{port} offered no host key: {}",
                        String::from_utf8_lossy(&scanned.stderr).trim()
                    );
                }
                *host_key = Some(found.join("\n"));
                info!(destination = %dest.name, "Pinned the SFTP server's host key");
            }
            let known = work.join("known_hosts");
            tokio::fs::create_dir_all(&known).await?;
            let known_file = known.join(&id);
            tokio::fs::write(
                &known_file,
                format!("{}\n", host_key.clone().unwrap_or_default()),
            )
            .await?;
            options.push(format!(
                "sftp.command={} -i {} -p {} -o UserKnownHostsFile={} -o StrictHostKeyChecking=yes \
                 -o BatchMode=yes -o IdentitiesOnly=yes -o ConnectTimeout=20 {}@{} -s sftp",
                ctx.config.backup.ssh.display(),
                key_file.display(),
                port,
                known_file.display(),
                user,
                host
            ));
        }
        BackupTarget::S3 {
            region,
            access_key_id,
            secret_access_key,
            ..
        } => {
            env.push(("AWS_ACCESS_KEY_ID".into(), access_key_id.clone()));
            env.push((
                "AWS_SECRET_ACCESS_KEY".into(),
                secret_access_key
                    .clone()
                    .context("the destination has no secret access key")?,
            ));
            if let Some(r) = region {
                env.push(("AWS_DEFAULT_REGION".into(), r.clone()));
            }
        }
    }
    let cache_dir = work.join("cache");
    tokio::fs::create_dir_all(&cache_dir).await?;
    Ok(Repo {
        bin: ctx.config.backup.restic.clone(),
        repository: dest.target.repository(),
        password,
        env,
        options,
        cache_dir,
    })
}

/// Carry out `run` against `dest`.
pub async fn carry_out(ctx: &Ctx, dest: &BackupDestination, run: &BackupRun) -> Result<Outcome> {
    let mut dest = dest.clone();
    let testing = run.kind == BackupRunKind::Test;
    let pinned_before = match &dest.target {
        BackupTarget::Sftp { host_key, .. } => host_key.clone(),
        _ => None,
    };
    let repo = repo_for(ctx, &mut dest, testing).await?;
    if let BackupTarget::Sftp { host_key, .. } = &dest.target {
        if *host_key != pinned_before {
            ctx.store
                .put_backup_destination(&dest)
                .await
                .context("keeping the pinned host key")?;
        }
    }
    let made = repo.ensure_init().await?;
    if testing {
        let n = repo.snapshot_count().await?;
        return Ok(Outcome {
            message: if made {
                "Reached; the repository was made".into()
            } else {
                format!("Reached; the repository holds {n} snapshot(s)")
            },
            ..Default::default()
        });
    }
    let host = ctx.config.service.node_id.clone();
    let release = format!("release:{}", env!("CARGO_PKG_VERSION"));
    let run_tag = format!("run:{}", run.id);
    let mut out = Outcome::default();

    // The database, streamed: never on this service's disk.
    let mut dump = Command::new(&ctx.config.backup.pg_dump);
    dump.args(["--format=custom", "--no-password"])
        .arg(format!("--dbname={}", ctx.config.storage.database_url));
    let db = repo
        .backup_stdin(
            "fcp.dump",
            &[
                "fcp".into(),
                "part:db".into(),
                run_tag.clone(),
                release.clone(),
            ],
            &host,
            dump,
        )
        .await?;
    out.snapshots.push(db.snapshot_id.clone());
    out.bytes_added += db.data_added;

    // The files, read in place.
    let mut paths: Vec<PathBuf> = ctx
        .config
        .backup
        .paths
        .values()
        .filter(|p| p.exists())
        .cloned()
        .collect();
    if dest.include_key {
        match &ctx.config.backup.key_ring {
            Some(k) if k.exists() => paths.push(k.clone()),
            _ => bail!("the destination includes the key, and [backup] key_ring is not there"),
        }
    }
    if !paths.is_empty() {
        let files = repo
            .backup_paths(
                &paths,
                &["fcp".into(), "part:files".into(), run_tag, release],
                &host,
            )
            .await?;
        out.snapshots.push(files.snapshot_id.clone());
        out.bytes_added += files.data_added;
    }

    // Retention, each part on its own.
    for part in ["part:db", "part:files"] {
        repo.forget(part, &dest.keep).await?;
    }
    out.message = format!(
        "{} snapshot(s), {} added",
        out.snapshots.len(),
        human(out.bytes_added)
    );
    Ok(out)
}

fn human(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut v = bytes as f64;
    let mut u = 0;
    while v >= 1024.0 && u < UNITS.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    if u == 0 {
        format!("{bytes} B")
    } else {
        format!("{v:.1} {}", UNITS[u])
    }
}
