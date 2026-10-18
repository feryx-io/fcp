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
        "sbc" => ("fcp-sbc", "fcp-sbc"),
        "ivr" => ("fcp-ivr", "fcp-ivr"),
        "voicemail" => ("fcp-voicemail-server", "fcp-voicemail"),
        "presence" => ("fcp-presence-server", "fcp-presence"),
        "tts" => ("fcp-tts-server", "fcp-tts"),
        "link" => ("fcp-link", "fcp-link"),
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
        Self::load_at("deploy/docker-compose")
    }

    /// The site stack, `deploy/docker-compose/site` — a tenant's own
    /// stack with its link (`docs/SITE_SETUP.md`).
    pub fn load_site() -> Self {
        Self::load_at("deploy/docker-compose/site")
    }

    /// Read the `docker-compose.yml` and `.env` of the compose project at
    /// `rel` from the repository root.
    pub fn load_at(rel: &str) -> Self {
        let root = repo_root();
        let dir = root.join(rel);
        let text = std::fs::read_to_string(dir.join("docker-compose.yml")).expect("compose file");
        let doc: serde_yaml::Value = serde_yaml::from_str(&text).expect("compose yaml");
        let dotenv = crate::smoke::compose_env_at(rel);
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
                    src.ends_with(".toml")
                        .then(|| PathBuf::from(rel).join(src.trim_start_matches("./")))
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
                .find(|s| s.starts_with("http://") || s.starts_with("https://"))
                .map(|u| u.replace("localhost", "127.0.0.1"))
                .map(|u| on_the_host(&u, svc.get("ports")))
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
            .unwrap_or_else(|| panic!("{key} is not in the compose project's .env"))
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
            .replace(
                "/app/routing.yaml",
                &root
                    .join("deploy/docker-compose/configs/routing.yaml")
                    .to_string_lossy(),
            )
            .replace("/var/lib/fcp", &format!("{d}/var-lib-fcp"))
            .replace("/var/fcp", &format!("{d}/var-fcp"))
    }

    /// The service's environment as the test runs it: every
    /// `*_DATABASE_URL` the test database, the containers' paths under
    /// `dir` as [`ComposeStack::config_for`] puts them.
    pub fn env_for(&self, name: &str, database_url: &str, dir: &Path) -> Vec<(String, String)> {
        let d = dir.to_string_lossy().to_string();
        self.service(name)
            .env
            .iter()
            .map(|(k, v)| {
                if k.ends_with("_DATABASE_URL") {
                    (k.clone(), database_url.to_string())
                } else {
                    (
                        k.clone(),
                        v.replace("/var/lib/fcp", &format!("{d}/var-lib-fcp")),
                    )
                }
            })
            .collect()
    }
}

/// Every service the compose file runs by default, in its start order:
/// the events server, then the admin and the gateway, then every SIP and
/// media service and the job worker. Not the TTS server (its provider
/// needs the image's Python) and not the registrar profile.
pub const DEFAULT_SERVICES: [&str; 12] = [
    "fcp-events-server",
    "fcp-admin",
    "fcp-api-gateway",
    "fcp-call-manager",
    "fcp-conference",
    "fcp-ivr",
    "fcp-voicemail",
    "fcp-presence",
    "fcp-sbc",
    "fcp-media",
    "fcp-webrtc-gw",
    "fcp-jobs",
];

/// The compose stack's default services started from this checkout's
/// binaries, as [`ComposeStack::start_local`] brings them up.
pub struct LocalStack {
    /// Where the containers' paths went; the certificates are under
    /// `var-lib-fcp/certs`.
    pub dir: tempfile::TempDir,
    /// The CA the `certs` service would have made, trusted by this process.
    pub ca: crate::certs::ClusterCa,
    /// The running services, by compose name, in start order.
    pub services: Vec<(String, crate::Service)>,
}

impl LocalStack {
    /// Stop every service, print the tail of each one's log, and panic
    /// with `why`.
    pub async fn fail(self, why: &str) -> ! {
        for (name, service) in self.services.into_iter().rev() {
            let log = service.stop().await;
            let lines: Vec<&str> = log.lines().collect();
            let tail = &lines[lines.len().saturating_sub(60)..];
            eprintln!("---- last lines of {name}'s log ----\n{}", tail.join("\n"));
        }
        panic!("{why}");
    }
}

impl ComposeStack {
    /// Start [`DEFAULT_SERVICES`] with the compose file's configurations
    /// and environment against `database_url` and `redis_url`, each
    /// healthy before the next starts. The certificates and master key
    /// the `certs` service would have made come from a CA of the
    /// caller's, which this process trusts. `edit` may change a
    /// service's configuration and environment before it starts.
    pub async fn start_local(
        &self,
        database_url: &str,
        redis_url: &str,
        edit: impl Fn(&str, &mut String, &mut Vec<(String, String)>),
    ) -> LocalStack {
        let dir = tempfile::tempdir().expect("tempdir");
        // Under the path the configurations name once `config_for` has
        // moved `/var/lib/fcp` under `dir`.
        let certs_dir = dir.path().join("var-lib-fcp").join("certs");
        std::fs::create_dir_all(&certs_dir).expect("certs dir");
        let ca = crate::certs::ClusterCa::new(&certs_dir);
        for service in [
            "fcp-call-manager",
            "fcp-registrar",
            "fcp-conference",
            "fcp-sbc",
            "fcp-media",
            "fcp-webrtc-gw",
            "fcp-proxy",
            "fcp-admin",
        ] {
            ca.node(service);
        }
        crate::smoke::trust_ca(&ca.ca_path);
        // The master key beside them: 32 random bytes, base64, as
        // `openssl rand -base64 32` writes it.
        let mut key = uuid::Uuid::new_v4().as_bytes().to_vec();
        key.extend_from_slice(uuid::Uuid::new_v4().as_bytes());
        std::fs::write(
            certs_dir.join("master.key"),
            format!(
                "{}\n",
                base64::Engine::encode(&base64::engine::general_purpose::STANDARD, key)
            ),
        )
        .expect("master key");

        let mut services = Vec::new();
        for name in DEFAULT_SERVICES {
            let svc = self.service(name);
            let mut config = self.config_for(name, dir.path(), redis_url);
            let mut env = self.env_for(name, database_url, dir.path());
            edit(name, &mut config, &mut env);
            let env_refs: Vec<(&str, &str)> =
                env.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
            let bin = crate::bins::path(svc.binary, svc.package);
            let service =
                crate::Service::start_with(name, bin, &config, &svc.health_url, &env_refs).await;
            services.push((name.to_string(), service));
        }
        LocalStack { dir, ca, services }
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
            "fcp-sbc",
            "fcp-ivr",
            "fcp-voicemail",
            "fcp-presence",
            "fcp-tts",
        ] {
            let svc = stack.service(name);
            assert!(
                repo_root().join(&svc.config).is_file(),
                "{name}: {}",
                svc.config.display()
            );
            assert!(
                svc.health_url.starts_with("http://127.0.0.1:")
                    || svc.health_url.starts_with("https://127.0.0.1:"),
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

/// A health check's URL as the host reaches it: the check runs inside the
/// container, so its port is the container's; a service that publishes
/// that port (`HOST:CONTAINER`, `IP:HOST:CONTAINER`) is reached on the
/// host's side of the mapping. One on the host's network, or that maps it
/// to the same number, keeps its URL.
fn on_the_host(url: &str, ports: Option<&serde_yaml::Value>) -> String {
    let Some((scheme_host, rest)) = url.rsplit_once(':') else {
        return url.to_string();
    };
    let (port, path) = rest.split_at(rest.find('/').unwrap_or(rest.len()));
    let published = ports
        .and_then(|p| p.as_sequence())
        .into_iter()
        .flatten()
        .filter_map(|p| p.as_str())
        .find_map(|mapping| {
            let mapping = mapping.split('/').next().unwrap_or(mapping);
            let parts: Vec<&str> = mapping.split(':').collect();
            match parts.as_slice() {
                [host, container] | [_, host, container] if *container == port => {
                    Some(host.to_string())
                }
                _ => None,
            }
        });
    match published {
        Some(host_port) => format!("{scheme_host}:{host_port}{path}"),
        None => url.to_string(),
    }
}

#[cfg(test)]
mod host_port_tests {
    use super::on_the_host;

    #[test]
    fn a_health_check_is_reached_on_the_published_port() {
        let ports: serde_yaml::Value =
            serde_yaml::from_str(r#"["18080:8080", "127.0.0.1:18096:8096", "5060:5060/udp"]"#)
                .unwrap();
        assert_eq!(
            on_the_host("http://127.0.0.1:8080/api/v1/health", Some(&ports)),
            "http://127.0.0.1:18080/api/v1/health"
        );
        assert_eq!(
            on_the_host("http://127.0.0.1:8096/health", Some(&ports)),
            "http://127.0.0.1:18096/health"
        );
        assert_eq!(
            on_the_host("http://127.0.0.1:18084/health", None),
            "http://127.0.0.1:18084/health",
            "on the host's network, as it is"
        );
    }
}
