//! restic, driven (`docs/SCHEDULED_BACKUPS.md` §2): a repository's
//! address and secrets in the environment, never on a command line.

use anyhow::{bail, Context, Result};
use std::path::PathBuf;
use std::process::Stdio;
use tokio::process::Command;

/// One restic repository, as reached from here.
pub struct Repo {
    pub bin: PathBuf,
    pub repository: String,
    pub password: String,
    /// Credentials restic reads from the environment (`AWS_*`).
    pub env: Vec<(String, String)>,
    /// `-o key=value` options (`sftp.command`).
    pub options: Vec<String>,
    pub cache_dir: PathBuf,
}

/// What `backup --json` said at the end.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Summary {
    pub snapshot_id: String,
    pub data_added: u64,
}

impl Repo {
    fn command(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new(&self.bin);
        cmd.env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env(
                "HOME",
                std::env::var_os("HOME").unwrap_or_else(|| "/tmp".into()),
            )
            .env("RESTIC_REPOSITORY", &self.repository)
            .env("RESTIC_PASSWORD", &self.password)
            .env("RESTIC_PROGRESS_FPS", "0.1");
        for (k, v) in &self.env {
            cmd.env(k, v);
        }
        cmd.arg("--cache-dir").arg(&self.cache_dir);
        for o in &self.options {
            cmd.arg("-o").arg(o);
        }
        cmd.args(args);
        cmd.kill_on_drop(true);
        cmd
    }

    /// Run restic to its end; its output, or its error with what it said.
    pub async fn run(&self, args: &[&str]) -> Result<String> {
        let out = self
            .command(args)
            .stdin(Stdio::null())
            .output()
            .await
            .with_context(|| format!("restic {}", args.first().unwrap_or(&"")))?;
        if !out.status.success() {
            bail!(
                "restic {} failed: {}",
                args.first().unwrap_or(&""),
                tail(&String::from_utf8_lossy(&out.stderr))
            );
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }

    /// Make the repository if there is none; whether it was made.
    pub async fn ensure_init(&self) -> Result<bool> {
        if self.run(&["cat", "config"]).await.is_ok() {
            return Ok(false);
        }
        self.run(&["init"]).await?;
        Ok(true)
    }

    /// How many snapshots the repository holds.
    pub async fn snapshot_count(&self) -> Result<usize> {
        let json = self.run(&["snapshots", "--json", "--no-lock"]).await?;
        let list: Vec<serde_json::Value> =
            serde_json::from_str(json.trim()).context("restic snapshots --json")?;
        Ok(list.len())
    }

    /// Back up what `source` writes to its stdout, as one file `name`.
    pub async fn backup_stdin(
        &self,
        name: &str,
        tags: &[String],
        host: &str,
        mut source: Command,
    ) -> Result<Summary> {
        let mut args = vec![
            "backup",
            "--json",
            "--stdin",
            "--stdin-filename",
            name,
            "--host",
            host,
        ];
        for t in tags {
            args.push("--tag");
            args.push(t);
        }
        let mut restic = self
            .command(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("starting restic backup")?;
        let mut child = source
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .context("starting the dump")?;
        let mut from = child.stdout.take().context("the dump's output")?;
        let mut into = restic.stdin.take().context("restic's input")?;
        let copied = tokio::io::copy(&mut from, &mut into).await;
        drop(into);
        let dumped = child.wait_with_output().await.context("the dump")?;
        let out = restic.wait_with_output().await.context("restic backup")?;
        if !dumped.status.success() {
            bail!(
                "the dump failed: {}",
                tail(&String::from_utf8_lossy(&dumped.stderr))
            );
        }
        copied.context("streaming the dump into restic")?;
        if !out.status.success() {
            bail!(
                "restic backup failed: {}",
                tail(&String::from_utf8_lossy(&out.stderr))
            );
        }
        summary(&String::from_utf8_lossy(&out.stdout))
    }

    /// Back up `paths`, each stored at its own path.
    pub async fn backup_paths(
        &self,
        paths: &[PathBuf],
        excludes: &[PathBuf],
        tags: &[String],
        host: &str,
    ) -> Result<Summary> {
        let mut args: Vec<String> = vec![
            "backup".into(),
            "--json".into(),
            "--host".into(),
            host.into(),
        ];
        for e in excludes {
            args.push("--exclude".into());
            args.push(e.display().to_string());
        }
        for t in tags {
            args.push("--tag".into());
            args.push(t.clone());
        }
        for p in paths {
            args.push(p.display().to_string());
        }
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let out = self.run(&refs).await?;
        summary(&out)
    }

    /// Keep what retention says for snapshots tagged `tag`, and prune.
    pub async fn forget(&self, tag: &str, keep: &fcp_core::backup::BackupRetention) -> Result<()> {
        let (d, w, m) = (
            keep.daily.to_string(),
            keep.weekly.to_string(),
            keep.monthly.to_string(),
        );
        let mut args = vec!["forget", "--tag", tag, "--group-by", "host", "--prune"];
        if keep.daily > 0 {
            args.extend(["--keep-daily", &d]);
        }
        if keep.weekly > 0 {
            args.extend(["--keep-weekly", &w]);
        }
        if keep.monthly > 0 {
            args.extend(["--keep-monthly", &m]);
        }
        self.run(&args).await.map(|_| ())
    }
}

/// One snapshot as `restic snapshots --json` lists it.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct Snapshot {
    pub id: String,
    pub time: String,
    #[serde(default)]
    pub hostname: String,
    #[serde(default)]
    pub tags: Vec<String>,
}

impl Snapshot {
    /// The value of a `<prefix>:<value>` tag.
    pub fn tag(&self, prefix: &str) -> Option<&str> {
        self.tags
            .iter()
            .find_map(|t| t.strip_prefix(prefix).and_then(|r| r.strip_prefix(':')))
    }

    /// The volumes a files snapshot holds, `vol:<name>:<path>`.
    pub fn volumes(&self) -> Vec<(String, String)> {
        self.tags
            .iter()
            .filter_map(|t| t.strip_prefix("vol:"))
            .filter_map(|r| r.split_once(':'))
            .map(|(n, p)| (n.to_string(), p.to_string()))
            .collect()
    }
}

impl Repo {
    /// FCP's snapshots, oldest first.
    pub async fn snapshots(&self) -> Result<Vec<Snapshot>> {
        let json = self
            .run(&["snapshots", "--json", "--no-lock", "--tag", "fcp"])
            .await?;
        serde_json::from_str(json.trim()).context("restic snapshots --json")
    }

    /// `snapshot`'s file `path`, written to `out`.
    pub async fn dump_to(&self, snapshot: &str, path: &str, out: &std::path::Path) -> Result<()> {
        let file = std::fs::File::create(out).with_context(|| format!("{}", out.display()))?;
        // Spawned, not `output()`, which would take the stdout given here.
        let child = self
            .command(&["dump", snapshot, path])
            .stdin(Stdio::null())
            .stdout(Stdio::from(file))
            .stderr(Stdio::piped())
            .spawn()
            .context("restic dump")?;
        let done = child.wait_with_output().await.context("restic dump")?;
        if !done.status.success() {
            bail!(
                "restic dump failed: {}",
                tail(&String::from_utf8_lossy(&done.stderr))
            );
        }
        Ok(())
    }

    /// Restore `snapshot`'s `includes` under `target`, never overwriting a
    /// file already there.
    pub async fn restore(
        &self,
        snapshot: &str,
        target: &std::path::Path,
        includes: &[String],
        overwrite: bool,
    ) -> Result<()> {
        let mut args: Vec<String> = vec![
            "restore".into(),
            snapshot.into(),
            "--target".into(),
            target.display().to_string(),
            "--overwrite".into(),
            if overwrite {
                "always".into()
            } else {
                "never".into()
            },
        ];
        for i in includes {
            args.push("--include".into());
            args.push(i.clone());
        }
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        self.run(&refs).await.map(|_| ())
    }
}

/// The last lines of what a tool said, for an error.
fn tail(text: &str) -> String {
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    lines[lines.len().saturating_sub(4)..].join(" | ")
}

/// The `summary` message of `backup --json`.
pub fn summary(json_lines: &str) -> Result<Summary> {
    for line in json_lines.lines().rev() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if v["message_type"] == "summary" {
            return Ok(Summary {
                snapshot_id: v["snapshot_id"].as_str().unwrap_or_default().to_string(),
                data_added: v["data_added"].as_u64().unwrap_or(0),
            });
        }
    }
    bail!("restic backup said nothing of what it did")
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_summary_is_the_last_word() {
        let out = r#"{"message_type":"status","percent_done":0.5}
{"message_type":"summary","files_new":3,"data_added":1234,"snapshot_id":"ab12cd34"}
"#;
        let s = super::summary(out).unwrap();
        assert_eq!((s.snapshot_id.as_str(), s.data_added), ("ab12cd34", 1234));
        assert!(super::summary("{}").is_err());
    }
}
