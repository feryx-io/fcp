//! The compose stack, as a test can run it (`docs/SHIP_AND_OPERATE.md`
//! phase 4). `deploy/docker-compose/docker-compose.yml` names, for each
//! service, the image's Dockerfile (hence the binary), the configuration
//! it mounts, the environment it sets — with `${VAR}` from `.env` — and
//! the health check it waits on. This module reads those, so a test
//! starts the very binaries with the very configurations and tokens the
//! compose file does, against the test's Postgres and Redis; a change to
//! the compose file is checked by the test, and a change the test needs
//! is made in the compose file.
//!
//! What the test changes, and only this: the compose network's names
//! (`fcp-admin`, `fcp-events-server`, `host.docker.internal`, …) become
//! the loopback, since there is no Docker network; the containers'
//! paths (`/app/data`, `/var/fcp`, `/var/lib/fcp`)
//! become the test's directory; every `*_DATABASE_URL` becomes the test
//! database and every Redis URL the test's Redis; and `HOST_IP` is the
//! loopback. Ports are the compose file's own, so the test needs them
//! free, and is the one test in its binary.

use crate::smoke::repo_root;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// One service of the compose file, as the test runs it.
#[derive(Debug, Clone)]
pub struct ComposeService {
    /// The compose service name (`fcp-call-manager`).
    pub name: String,
    /// The package and binary the Dockerfile builds.
    pub package: &'static str,
    pub binary: &'static str,
    /// The configuration the compose file mounts, repo-relative.
    pub config: PathBuf,
    /// The environment the compose file sets, interpolated.
    pub env: Vec<(String, String)>,
    /// The health URL the compose file polls, on the loopback.
    pub health_url: String,
}

/// The stack as the compose file defines it.
pub struct ComposeStack {
    services: BTreeMap<String, ComposeService>,
    dotenv: Vec<(String, String)>,
}

/// The binary each Dockerfile builds, by the Dockerfile's suffix.
fn binary_for(dockerfile: &str) -> Option<(&'static str, &'static str)> {
    let suffix = dockerfile.rsplit("Dockerfile.").next()?;
    Some(match suffix {
        "admin" => ("fcp-admin", "fcp-admin"),
        "gateway" => ("fcp-api-gateway", "fcp-api-gateway"),
        "events" => ("fcp-events-server", "fcp-events-server"),
        "call-manager" => ("fcp-call-manager", "fcp-call-manager"),
        "conference" | "conference.nvidia" => ("fcp-conference-server", "fcp-conference"),
        "webrtc-gw" => ("fcp-webrtc-gateway", "fcp-webrtc-gw"),
        "media" => ("fcp-media-server", "fcp-media"),
        "jobs" => ("fcp-jobs", "fcp-jobs"),
        "registrar" => ("fcp-registrar", "fcp-registrar"),
        _ => return None,
    })
}

/// `${VAR}` and `${VAR:-default}` from `vars`, in `s`.
fn interpolate(s: &str, vars: &BTreeMap<String, String>) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find("${") {
        out.push_str(&rest[..i]);
        let after = &rest[i + 2..];
        let Some(j) = after.find('}') else {
            out.push_str(&rest[i..]);
            return out;
        };
        let expr = &after[..j];
        let (name, default) = match expr.split_once(":-") {
            Some((n, d)) => (n, Some(d)),
            None => (expr, None),
        };
        match vars.get(name) {
            Some(v) => out.push_str(v),
            None => out.push_str(default.unwrap_or("")),
        }
        rest = &after[j + 1..];
    }
    out.push_str(rest);
    out
}

impl ComposeStack {
    /// Read `deploy/docker-compose/docker-compose.yml` and `.env`.
    pub fn load() -> Self {
        let root = repo_root();
        let dir = root.join("deploy/docker-compose");
        let text = std::fs::read_to_string(dir.join("docker-compose.yml")).expect("compose file");
        let doc: serde_yaml::Value = serde_yaml::from_str(&text).expect("compose yaml");
        let dotenv = crate::smoke::compose_env();
        let mut vars: BTreeMap<String, String> = dotenv.iter().cloned().collect();
        vars.insert("HOST_IP".into(), "127.0.0.1".into());

        let mut services = BTreeMap::new();
        let Some(map) = doc.get("services").and_then(|s| s.as_mapping()) else {
            panic!("no services in the compose file");
        };
        for (name, svc) in map {
            let name = name.as_str().unwrap_or_default().to_string();
            let Some(dockerfile) = svc
                .get("build")
                .and_then(|b| b.get("dockerfile"))
                .and_then(|d| d.as_str())
            else {
                continue; // an image (postgres, redis, prometheus, grafana)
            };
            let Some((package, binary)) = binary_for(dockerfile) else {
                panic!("{name}: no binary known for {dockerfile}");
            };
            // The mounted configuration: `./configs/x.toml:/app/x.toml:ro`.
            let config = svc
                .get("volumes")
                .and_then(|v| v.as_sequence())
                .into_iter()
                .flatten()
                .filter_map(|v| v.as_str())
                .find_map(|v| {
                    let src = v.split(':').next()?;
                    src.ends_with(".toml").then(|| {
                        PathBuf::from("deploy/docker-compose").join(src.trim_start_matches("./"))
                    })
                })
                .unwrap_or_else(|| panic!("{name}: no configuration mounted"));
            let mut env = Vec::new();
            if let Some(e) = svc.get("environment").and_then(|e| e.as_mapping()) {
                for (k, v) in e {
                    let k = k.as_str().unwrap_or_default().to_string();
                    let v = match v {
                        serde_yaml::Value::String(s) => s.clone(),
                        other => serde_yaml::to_string(other)
                            .unwrap_or_default()
                            .trim()
                            .to_string(),
                    };
                    env.push((k, interpolate(&v, &vars)));
                }
            }
            let health_url = svc
                .get("healthcheck")
                .and_then(|h| h.get("test"))
                .and_then(|t| t.as_sequence())
                .into_iter()
                .flatten()
                .filter_map(|v| v.as_str())
                .find(|s| s.starts_with("http://"))
                .map(|u| u.replace("localhost", "127.0.0.1"))
                .unwrap_or_else(|| panic!("{name}: no HTTP health check"));
            services.insert(
                name.clone(),
                ComposeService {
                    name,
                    package,
                    binary,
                    config,
                    env,
                    health_url,
                },
            );
        }
        Self { services, dotenv }
    }

    /// The service by its compose name.
    pub fn service(&self, name: &str) -> &ComposeService {
        self.services
            .get(name)
            .unwrap_or_else(|| panic!("no compose service {name}"))
    }

    /// A value from `.env`.
    pub fn dotenv(&self, key: &str) -> String {
        self.dotenv
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.clone())
            .unwrap_or_else(|| panic!("{key} is not in deploy/docker-compose/.env"))
    }

    /// The service's configuration as the test runs it: the compose
    /// network's names on the loopback, the containers' paths under
    /// `dir`, the compose stack's Redis replaced by `redis_url`.
    pub fn config_for(&self, name: &str, dir: &Path, redis_url: &str) -> String {
        let svc = self.service(name);
        let root = repo_root();
        let text = std::fs::read_to_string(root.join(&svc.config))
            .unwrap_or_else(|e| panic!("{}: {e}", svc.config.display()));
        let d = dir.to_string_lossy().to_string();
        text.replace("fcp-events-server:", "127.0.0.1:")
            .replace("fcp-admin:", "127.0.0.1:")
            .replace("fcp-jobs:", "127.0.0.1:")
            .replace("host.docker.internal:", "127.0.0.1:")
            .replace("@postgres:", "@127.0.0.1:")
            // With the closing quote, so a test Redis on a port that
            // starts with 6379 is not matched again by the second.
            .replace("redis://redis:6379\"", &format!("{redis_url}\""))
            .replace("redis://127.0.0.1:6379\"", &format!("{redis_url}\""))
            .replace("/app/data", &format!("{d}/app-data"))
            .replace("/var/lib/fcp", &format!("{d}/var-lib-fcp"))
            .replace("/var/fcp", &format!("{d}/var-fcp"))
    }

    /// The service's environment as the test runs it: every
    /// `*_DATABASE_URL` the test database.
    pub fn env_for(&self, name: &str, database_url: &str) -> Vec<(String, String)> {
        self.service(name)
            .env
            .iter()
            .map(|(k, v)| {
                if k.ends_with("_DATABASE_URL") {
                    (k.clone(), database_url.to_string())
                } else {
                    (k.clone(), v.clone())
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_compose_file_names_every_binary_its_configuration_and_its_health() {
        let stack = ComposeStack::load();
        for name in [
            "fcp-admin",
            "fcp-api-gateway",
            "fcp-events-server",
            "fcp-call-manager",
            "fcp-conference",
            "fcp-webrtc-gw",
            "fcp-media",
            "fcp-jobs",
            "fcp-registrar",
        ] {
            let svc = stack.service(name);
            assert!(
                repo_root().join(&svc.config).is_file(),
                "{name}: {}",
                svc.config.display()
            );
            assert!(
                svc.health_url.starts_with("http://127.0.0.1:"),
                "{name}: {}",
                svc.health_url
            );
        }
        // The shared tokens are handed on, interpolated from .env.
        let admin = stack.service("fcp-admin");
        let events_token = stack.dotenv("FCP_EVENTS_AUTH_TOKEN");
        assert!(admin
            .env
            .iter()
            .any(|(k, v)| k == "FCP_ADMIN_EVENTS_AUTH_TOKEN" && *v == events_token));
        let cm = stack.service("fcp-call-manager");
        assert!(cm
            .env
            .iter()
            .any(|(k, v)| k == "FCP_CALL_MANAGER_PUBLIC_IP" && v == "127.0.0.1"));
        assert!(cm
            .env
            .iter()
            .any(|(k, v)| k == "FCP_CALL_MANAGER_DATABASE_URL"
                && v.contains(&stack.dotenv("POSTGRES_PASSWORD"))));
    }

    #[test]
    fn interpolation_reads_env_and_defaults() {
        let mut vars = BTreeMap::new();
        vars.insert("A".to_string(), "x".to_string());
        assert_eq!(interpolate("pre-${A}-post", &vars), "pre-x-post");
        assert_eq!(interpolate("${B:-dflt}", &vars), "dflt");
        assert_eq!(interpolate("${B}", &vars), "");
        assert_eq!(interpolate("plain", &vars), "plain");
    }
}
