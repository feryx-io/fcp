//! baresip, a softphone that is not ours, with real media
//! (`docs/DEEP_TESTING.md` D2). SIPp speaks SIP but carries no audio a test
//! can listen to; the testkit's [`Phone`](crate::Phone) carries audio but is
//! built on siphon-rs and forge-media, the stack the node runs. A baresip
//! phone plays a tone from a WAV file as its microphone and writes what it
//! hears to another, so a test asks whether a call carried sound — through
//! codecs, transcoding, SRTP and DTMF — with nothing of ours at the ends.
//!
//! baresip is found as `FCP_BARESIP` (the binary) when set, else `baresip`
//! on the `PATH`. Its modules are looked for beside it
//! (`<prefix>/lib/baresip/modules`) unless `FCP_BARESIP_MODULES` names
//! them, and `FCP_BARESIP_LIBRARY_PATH`, when set, is its
//! `LD_LIBRARY_PATH` (for a copy unpacked from packages). A test without
//! it is skipped, unless `FCP_BARESIP_TESTS=required` (as CI sets it).
//!
//! A phone is driven over baresip's `ctrl_tcp` module: commands as
//! netstring-framed JSON, call events pushed back the same way.

use anyhow::{anyhow, bail, Context, Result};
use std::collections::VecDeque;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::{oneshot, Mutex, Notify};

/// A baresip installation.
#[derive(Clone, Debug)]
pub struct Baresip {
    bin: PathBuf,
    modules: PathBuf,
    library_path: Option<String>,
}

/// How one phone is set up.
#[derive(Clone, Debug)]
pub struct PhoneSetup {
    /// The SIP user, its password and realm; the phone registers at
    /// `registrar` with them.
    pub user: String,
    pub password: String,
    pub realm: String,
    pub registrar: SocketAddr,
    /// The tone the phone sends as its microphone.
    pub tone_hz: f32,
    /// The codecs it offers and accepts, in order, as baresip names them
    /// (`PCMU`, `PCMA`, `G722`, `opus/48000/2`). Empty: all it has.
    pub codecs: Vec<String>,
    /// `srtp-mand` (SDES SRTP required), or `None` for plain RTP.
    pub media_encryption: Option<String>,
    /// `rtpevent` (RFC 4733, the default) or `info` (SIP INFO).
    pub dtmf_mode: Option<String>,
    /// Answer incoming calls at once.
    pub auto_answer: bool,
    /// Register over TLS (the registrar then being the node's TLS
    /// listener), trusting this CA.
    pub tls_ca: Option<PathBuf>,
    /// Register at `registrar`; off, the phone only sends its requests
    /// there (a conference server, which takes calls unregistered).
    pub register: bool,
}

impl PhoneSetup {
    pub fn new(
        user: &str,
        password: &str,
        realm: &str,
        registrar: SocketAddr,
        tone_hz: f32,
    ) -> Self {
        Self {
            user: user.into(),
            password: password.into(),
            realm: realm.into(),
            registrar,
            tone_hz,
            codecs: Vec::new(),
            media_encryption: None,
            dtmf_mode: None,
            auto_answer: false,
            tls_ca: None,
            register: true,
        }
    }
}

/// A running baresip phone.
pub struct BaresipPhone {
    child: tokio::process::Child,
    dir: tempfile::TempDir,
    writer: Mutex<tokio::net::tcp::OwnedWriteHalf>,
    waiting: Arc<Mutex<std::collections::HashMap<String, oneshot::Sender<serde_json::Value>>>>,
    events: Arc<Mutex<VecDeque<serde_json::Value>>>,
    event_arrived: Arc<Notify>,
    next_token: std::sync::atomic::AtomicU64,
    /// The phone's SIP address.
    pub sip: SocketAddr,
    pub user: String,
}

impl Baresip {
    /// baresip, or `None` (skip) when it is not installed and not required.
    pub fn find() -> Option<Self> {
        let required = std::env::var("FCP_BARESIP_TESTS").is_ok_and(|v| v == "required");
        let bin = std::env::var("FCP_BARESIP")
            .ok()
            .map(PathBuf::from)
            .or_else(|| which("baresip"));
        let Some(bin) = bin.filter(|b| b.exists()) else {
            assert!(
                !required,
                "FCP_BARESIP_TESTS=required but baresip is not installed (FCP_BARESIP or `baresip` on the PATH)"
            );
            eprintln!("baresip not found (FCP_BARESIP or `baresip` on the PATH); skipping");
            return None;
        };
        let modules = std::env::var("FCP_BARESIP_MODULES")
            .ok()
            .map(PathBuf::from)
            .or_else(|| {
                let prefix = bin.parent()?.parent()?;
                [
                    prefix.join("lib/baresip/modules"),
                    prefix.join("lib/x86_64-linux-gnu/baresip/modules"),
                ]
                .into_iter()
                .find(|p| p.is_dir())
            })
            .unwrap_or_else(|| PathBuf::from("/usr/lib/baresip/modules"));
        assert!(
            modules.join("g711.so").exists(),
            "baresip modules not found at {} (FCP_BARESIP_MODULES)",
            modules.display()
        );
        Some(Self {
            bin,
            modules,
            library_path: std::env::var("FCP_BARESIP_LIBRARY_PATH").ok(),
        })
    }

    /// Start a phone, registered, ready for commands.
    pub async fn phone(&self, setup: PhoneSetup) -> Result<BaresipPhone> {
        let dir = tempfile::Builder::new().prefix("fcp-baresip-").tempdir()?;
        let sip_port = free_sip_port();
        let ctrl_port = crate::smoke::free_port();
        crate::wav::write_tone(&dir.path().join("tone.wav"), setup.tone_hz, 120_000)?;

        let mut modules = vec!["g711.so", "aufile.so", "menu.so", "account.so"];
        for (name, module) in [("G722", "g722.so"), ("opus", "opus.so")] {
            if setup.codecs.is_empty() || setup.codecs.iter().any(|c| c.starts_with(name)) {
                modules.push(module);
            }
        }
        if setup.media_encryption.is_some() {
            modules.push("srtp.so");
        }
        let path = |name: &str| dir.path().join(name).display().to_string();
        let mut config = format!(
            "poll_method\tepoll\n\
             sip_listen\t127.0.0.1:{sip_port}\n\
             net_interface\t127.0.0.1\n\
             call_local_timeout\t120\n\
             audio_player\taufile,{heard}\n\
             audio_source\taufile,{tone}\n\
             audio_alert\taufile,{alert}\n\
             ausrc_srate\t8000\n\
             auplay_srate\t8000\n\
             ausrc_channels\t1\n\
             auplay_channels\t1\n\
             module_path\t{modules}\n\
             ctrl_tcp_listen\t127.0.0.1:{ctrl_port}\n",
            heard = path("heard.wav"),
            tone = path("tone.wav"),
            alert = path("alert.wav"),
            modules = self.modules.display(),
        );
        for module in &modules {
            let kind = if *module == "account.so" {
                "module_tmp"
            } else {
                "module"
            };
            config.push_str(&format!("{kind}\t{module}\n"));
        }
        config.push_str("module_app\tctrl_tcp.so\n");
        if let Some(ca) = &setup.tls_ca {
            config.push_str(&format!("sip_cafile\t{}\n", ca.display()));
        }
        std::fs::write(dir.path().join("config"), config)?;

        let transport = if setup.tls_ca.is_some() { "tls" } else { "udp" };
        let mut account = format!(
            "<sip:{user}@{realm};transport={transport}>;auth_user={user};auth_pass={password};\
             outbound=\"sip:{registrar};transport={transport}\";regint={regint}",
            user = setup.user,
            realm = setup.realm,
            password = setup.password,
            registrar = setup.registrar,
            regint = if setup.register { 300 } else { 0 },
        );
        if !setup.codecs.is_empty() {
            account.push_str(&format!(";audio_codecs={}", setup.codecs.join(",")));
        }
        if let Some(enc) = &setup.media_encryption {
            account.push_str(&format!(";mediaenc={enc}"));
        }
        if let Some(mode) = &setup.dtmf_mode {
            account.push_str(&format!(";dtmfmode={mode}"));
        }
        if setup.auto_answer {
            account.push_str(";answermode=auto");
        }
        std::fs::write(dir.path().join("accounts"), format!("{account}\n"))?;
        std::fs::write(dir.path().join("contacts"), "")?;

        let mut cmd = tokio::process::Command::new(&self.bin);
        cmd.arg("-f")
            .arg(dir.path())
            .current_dir(dir.path())
            .stdin(Stdio::null())
            .stdout(std::fs::File::create(dir.path().join("out.log"))?)
            .stderr(std::fs::File::create(dir.path().join("err.log"))?)
            .kill_on_drop(true);
        if let Some(lib) = &self.library_path {
            cmd.env("LD_LIBRARY_PATH", lib);
        }
        let child = cmd
            .spawn()
            .with_context(|| format!("start baresip {}", self.bin.display()))?;

        // The control port opens once baresip has loaded its modules.
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        let stream = loop {
            match TcpStream::connect(("127.0.0.1", ctrl_port)).await {
                Ok(s) => break s,
                Err(e) if tokio::time::Instant::now() >= deadline => {
                    let log =
                        std::fs::read_to_string(dir.path().join("out.log")).unwrap_or_default();
                    bail!("baresip's control port never opened: {e}\n{log}");
                }
                Err(_) => tokio::time::sleep(Duration::from_millis(100)).await,
            }
        };
        let (mut reader, writer) = stream.into_split();
        let waiting: Arc<
            Mutex<std::collections::HashMap<String, oneshot::Sender<serde_json::Value>>>,
        > = Arc::default();
        let events: Arc<Mutex<VecDeque<serde_json::Value>>> = Arc::default();
        let event_arrived = Arc::new(Notify::new());
        {
            let (waiting, events, event_arrived) = (
                Arc::clone(&waiting),
                Arc::clone(&events),
                Arc::clone(&event_arrived),
            );
            tokio::spawn(async move {
                let mut buf = Vec::new();
                let mut chunk = [0u8; 4096];
                loop {
                    let n = match reader.read(&mut chunk).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => n,
                    };
                    buf.extend_from_slice(&chunk[..n]);
                    while let Some((message, used)) = netstring(&buf) {
                        buf.drain(..used);
                        let Ok(value) = serde_json::from_slice::<serde_json::Value>(&message)
                        else {
                            continue;
                        };
                        if value["response"].as_bool() == Some(true) {
                            let token = value["token"].as_str().unwrap_or_default().to_string();
                            if let Some(tx) = waiting.lock().await.remove(&token) {
                                let _ = tx.send(value);
                            }
                        } else if value["event"].as_bool() == Some(true) {
                            events.lock().await.push_back(value);
                            event_arrived.notify_waiters();
                        }
                    }
                }
            });
        }
        let phone = BaresipPhone {
            child,
            dir,
            writer: Mutex::new(writer),
            waiting,
            events,
            event_arrived,
            next_token: 0.into(),
            sip: SocketAddr::from(([127, 0, 0, 1], sip_port)),
            user: setup.user,
        };
        // Registered: `REGISTER_OK` may have gone out before this
        // connection, so ask.
        if setup.register {
            let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
            loop {
                let info = phone.command("reginfo", "").await.unwrap_or_default();
                if info.contains("OK") {
                    break;
                }
                if tokio::time::Instant::now() >= deadline {
                    bail!("{} did not register: {info}\n{}", phone.user, phone.log());
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
        Ok(phone)
    }
}

impl BaresipPhone {
    /// Run a command (`dial`, `accept`, `hangup`, `sndcode`, …) and return
    /// its answer's data.
    pub async fn command(&self, command: &str, params: &str) -> Result<String> {
        let token = self
            .next_token
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            .to_string();
        let (tx, rx) = oneshot::channel();
        self.waiting.lock().await.insert(token.clone(), tx);
        let body =
            serde_json::json!({ "command": command, "params": params, "token": token }).to_string();
        let frame = format!("{}:{},", body.len(), body);
        self.writer
            .lock()
            .await
            .write_all(frame.as_bytes())
            .await
            .context("baresip control")?;
        let answer = tokio::time::timeout(Duration::from_secs(5), rx)
            .await
            .map_err(|_| anyhow!("baresip did not answer `{command} {params}`"))??;
        if answer["ok"].as_bool() != Some(true) {
            bail!("baresip refused `{command} {params}`: {answer}");
        }
        Ok(answer["data"].as_str().unwrap_or_default().to_string())
    }

    /// Dial `uri` (`sip:1002@realm`).
    pub async fn dial(&self, uri: &str) -> Result<()> {
        self.command("dial", uri).await.map(|_| ())
    }

    /// Answer the ringing call.
    pub async fn accept(&self) -> Result<()> {
        self.command("accept", "").await.map(|_| ())
    }

    /// Hang up the current call.
    pub async fn hangup(&self) -> Result<()> {
        self.command("hangup", "").await.map(|_| ())
    }

    /// Send DTMF digits, as the account's `dtmfmode` says.
    pub async fn send_digits(&self, digits: &str) -> Result<()> {
        for digit in digits.chars() {
            self.command("sndcode", &digit.to_string()).await?;
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        Ok(())
    }

    /// The next event of one of `types` (`CALL_ESTABLISHED`,
    /// `CALL_CLOSED`, `CALL_INCOMING`, `CALL_DTMF_START`, …), taking it
    /// and every event before it.
    pub async fn event(&self, types: &[&str], within: Duration) -> Result<serde_json::Value> {
        let deadline = tokio::time::Instant::now() + within;
        loop {
            let notified = self.event_arrived.notified();
            {
                let mut events = self.events.lock().await;
                while let Some(event) = events.pop_front() {
                    if types.iter().any(|t| event["type"] == *t) {
                        return Ok(event);
                    }
                }
            }
            if tokio::time::timeout_at(deadline, notified).await.is_err() {
                bail!("no {types:?} from {} within {within:?}", self.user);
            }
        }
    }

    /// The DTMF digits received so far, from `CALL_DTMF_START` events
    /// (taking every event up to now).
    pub async fn digits_received(&self) -> String {
        let mut events = self.events.lock().await;
        events
            .drain(..)
            .filter(|e| e["type"] == "CALL_DTMF_START")
            .filter_map(|e| e["param"].as_str().map(str::to_string))
            .collect()
    }

    /// The 8 kHz samples the phone has heard, from its player's file.
    pub fn heard(&self) -> Vec<i16> {
        crate::wav::read_pcm(&self.dir.path().join("heard.wav"))
            .map(|(_, samples)| samples)
            .unwrap_or_default()
    }

    /// Whether the phone heard `hz` in the last `ms` it played.
    pub fn heard_tone(&self, hz: u32, ms: usize) -> bool {
        let samples = self.heard();
        let tail = samples.len().saturating_sub(ms * 8);
        crate::wav::contains_tone(&samples[tail..], hz)
    }

    /// baresip's own output, for a failure.
    pub fn log(&self) -> String {
        std::fs::read_to_string(self.dir.path().join("out.log")).unwrap_or_default()
    }

    /// The directory the phone runs in.
    pub fn dir(&self) -> &Path {
        self.dir.path()
    }

    /// Stop the phone.
    pub async fn quit(mut self) {
        let _ = self.command("quit", "").await;
        if tokio::time::timeout(Duration::from_secs(3), self.child.wait())
            .await
            .is_err()
        {
            let _ = self.child.kill().await;
        }
    }
}

/// A port baresip can listen on: free for UDP and TCP, and the next one
/// for TCP too (where it puts TLS).
fn free_sip_port() -> u16 {
    loop {
        let port = crate::sipp::free_udp_port();
        let free = std::net::TcpListener::bind(("127.0.0.1", port)).is_ok()
            && port < u16::MAX
            && std::net::TcpListener::bind(("127.0.0.1", port + 1)).is_ok();
        if free {
            return port;
        }
    }
}

/// One netstring (`<len>:<data>,`) at the front of `buf`: its data and the
/// bytes it took.
fn netstring(buf: &[u8]) -> Option<(Vec<u8>, usize)> {
    let colon = buf.iter().position(|&b| b == b':')?;
    let len: usize = std::str::from_utf8(&buf[..colon]).ok()?.parse().ok()?;
    let end = colon + 1 + len;
    if buf.len() <= end {
        return None;
    }
    Some((buf[colon + 1..end].to_vec(), end + 1))
}

fn which(name: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|p| p.join(name))
            .find(|p| p.is_file())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn netstrings_are_split_whole() {
        let buf = b"5:hello,3:abc,2:x";
        let (a, used) = netstring(buf).unwrap();
        assert_eq!((a.as_slice(), used), (&b"hello"[..], 8));
        let (b, used2) = netstring(&buf[used..]).unwrap();
        assert_eq!((b.as_slice(), used2), (&b"abc"[..], 6));
        assert!(netstring(&buf[used + used2..]).is_none());
    }
}
