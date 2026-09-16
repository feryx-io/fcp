//! Places a test can have messages delivered to: an SMTP server and an
//! HTTP endpoint that keep what they were sent.
//!
//! The emergency notifier (`docs/E911_BLOCK_2.md` §8) delivers by email,
//! by an HTTP SMS provider, by asking a call manager to ring someone and
//! by webhook. A test points it at these and reads back what arrived —
//! including what it was told to say, which is what Kari's Law is about.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};

/// One message an [`SmtpSink`] received.
#[derive(Debug, Clone)]
pub struct Email {
    pub from: String,
    pub to: Vec<String>,
    /// The whole message: headers, a blank line, then the body.
    pub data: String,
}

impl Email {
    /// The `Subject:` header, unfolded enough for a test.
    pub fn subject(&self) -> String {
        self.header("Subject").unwrap_or_default()
    }

    pub fn header(&self, name: &str) -> Option<String> {
        let prefix = format!("{}:", name.to_ascii_lowercase());
        self.data
            .lines()
            .take_while(|line| !line.is_empty())
            .find(|line| line.to_ascii_lowercase().starts_with(&prefix))
            .map(|line| line[prefix.len()..].trim().to_string())
    }

    /// Everything after the blank line.
    pub fn body(&self) -> String {
        match self.data.split_once("\r\n\r\n") {
            Some((_, body)) => body.replace("\r\n", "\n"),
            None => self.data.clone(),
        }
    }
}

/// A plain SMTP server that accepts anything and keeps it. No TLS and no
/// authentication: a test's mail server, on loopback.
pub struct SmtpSink {
    addr: SocketAddr,
    received: Arc<Mutex<Vec<Email>>>,
}

impl SmtpSink {
    /// Listen on a free loopback port and serve until the process ends.
    pub async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind SMTP sink");
        let addr = listener.local_addr().expect("SMTP sink address");
        let received = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&received);
        tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    return;
                };
                let sink = Arc::clone(&sink);
                tokio::spawn(async move {
                    let _ = serve_smtp(stream, sink).await;
                });
            }
        });
        Self { addr, received }
    }

    pub fn host(&self) -> String {
        self.addr.ip().to_string()
    }

    pub fn port(&self) -> u16 {
        self.addr.port()
    }

    /// Every message received so far.
    pub fn received(&self) -> Vec<Email> {
        self.received.lock().expect("smtp sink").clone()
    }

    /// Wait until at least `count` messages have arrived, or panic.
    pub async fn wait_for(&self, count: usize, within: Duration) -> Vec<Email> {
        let deadline = Instant::now() + within;
        loop {
            let seen = self.received();
            if seen.len() >= count {
                return seen;
            }
            if Instant::now() >= deadline {
                panic!(
                    "SMTP sink: expected {count} message(s) within {within:?}, got {}",
                    seen.len()
                );
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }
}

async fn serve_smtp(stream: TcpStream, received: Arc<Mutex<Vec<Email>>>) -> std::io::Result<()> {
    let (read, mut write) = stream.into_split();
    let mut reader = BufReader::new(read);
    let mut line = String::new();
    write.write_all(b"220 fcp-testkit ESMTP\r\n").await?;
    let mut from = String::new();
    let mut to: Vec<String> = Vec::new();
    loop {
        line.clear();
        if reader.read_line(&mut line).await? == 0 {
            return Ok(());
        }
        let command = line.trim_end().to_string();
        let upper = command.to_ascii_uppercase();
        if upper.starts_with("EHLO") || upper.starts_with("HELO") {
            // No STARTTLS and no AUTH: a test's server offers nothing.
            write
                .write_all(b"250-fcp-testkit\r\n250 SIZE 10485760\r\n")
                .await?;
        } else if upper.starts_with("MAIL FROM") {
            from = between_angles(&command).unwrap_or_default();
            write.write_all(b"250 OK\r\n").await?;
        } else if upper.starts_with("RCPT TO") {
            if let Some(address) = between_angles(&command) {
                to.push(address);
            }
            write.write_all(b"250 OK\r\n").await?;
        } else if upper.starts_with("DATA") {
            write
                .write_all(b"354 End data with <CR><LF>.<CR><LF>\r\n")
                .await?;
            let mut data = String::new();
            loop {
                line.clear();
                if reader.read_line(&mut line).await? == 0 {
                    return Ok(());
                }
                if line.trim_end() == "." {
                    break;
                }
                data.push_str(&line);
            }
            received.lock().expect("smtp sink").push(Email {
                from: std::mem::take(&mut from),
                to: std::mem::take(&mut to),
                data,
            });
            write.write_all(b"250 OK: queued\r\n").await?;
        } else if upper.starts_with("QUIT") {
            write.write_all(b"221 Bye\r\n").await?;
            return Ok(());
        } else if upper.starts_with("RSET") || upper.starts_with("NOOP") {
            write.write_all(b"250 OK\r\n").await?;
        } else {
            write.write_all(b"502 Command not implemented\r\n").await?;
        }
    }
}

fn between_angles(command: &str) -> Option<String> {
    let start = command.find('<')? + 1;
    let end = command[start..].find('>')? + start;
    Some(command[start..end].to_string())
}

/// An HTTP endpoint that keeps the JSON bodies posted to it and answers
/// what it was told to: the SMS provider, the call manager's notification
/// call, or a webhook endpoint, depending on the test.
pub struct HttpSink {
    addr: SocketAddr,
    received: Arc<Mutex<Vec<serde_json::Value>>>,
    reply: Arc<Mutex<(u16, serde_json::Value)>>,
}

impl HttpSink {
    /// Listen on a free loopback port, answering `200` with `{}`.
    pub async fn start() -> Self {
        Self::start_with(200, serde_json::json!({})).await
    }

    /// Listen answering `status` with `body`.
    pub async fn start_with(status: u16, body: serde_json::Value) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind HTTP sink");
        let addr = listener.local_addr().expect("HTTP sink address");
        let received = Arc::new(Mutex::new(Vec::new()));
        let reply = Arc::new(Mutex::new((status, body)));
        let (sink, answer) = (Arc::clone(&received), Arc::clone(&reply));
        tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    return;
                };
                let (sink, answer) = (Arc::clone(&sink), Arc::clone(&answer));
                tokio::spawn(async move {
                    let _ = serve_http(stream, sink, answer).await;
                });
            }
        });
        Self {
            addr,
            received,
            reply,
        }
    }

    pub fn url(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// Answer differently from now on.
    pub fn answer(&self, status: u16, body: serde_json::Value) {
        *self.reply.lock().expect("http sink") = (status, body);
    }

    pub fn received(&self) -> Vec<serde_json::Value> {
        self.received.lock().expect("http sink").clone()
    }

    /// Wait until at least `count` requests have arrived, or panic.
    pub async fn wait_for(&self, count: usize, within: Duration) -> Vec<serde_json::Value> {
        let deadline = Instant::now() + within;
        loop {
            let seen = self.received();
            if seen.len() >= count {
                return seen;
            }
            if Instant::now() >= deadline {
                panic!(
                    "HTTP sink: expected {count} request(s) within {within:?}, got {}",
                    seen.len()
                );
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }
}

async fn serve_http(
    stream: TcpStream,
    received: Arc<Mutex<Vec<serde_json::Value>>>,
    reply: Arc<Mutex<(u16, serde_json::Value)>>,
) -> std::io::Result<()> {
    let (read, mut write) = stream.into_split();
    let mut reader = BufReader::new(read);
    let mut line = String::new();
    let mut length = 0usize;
    // Request line and headers.
    loop {
        line.clear();
        if reader.read_line(&mut line).await? == 0 {
            return Ok(());
        }
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            break;
        }
        if let Some(value) = trimmed
            .to_ascii_lowercase()
            .strip_prefix("content-length:")
            .map(str::trim)
            .and_then(|v| v.parse::<usize>().ok())
        {
            length = value;
        }
    }
    let mut body = vec![0u8; length];
    if length > 0 {
        tokio::io::AsyncReadExt::read_exact(&mut reader, &mut body).await?;
    }
    let parsed = serde_json::from_slice(&body).unwrap_or(serde_json::Value::Null);
    received.lock().expect("http sink").push(parsed);

    let (status, answer) = reply.lock().expect("http sink").clone();
    let answer = answer.to_string();
    let response = format!(
        "HTTP/1.1 {status} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{answer}",
        answer.len()
    );
    write.write_all(response.as_bytes()).await?;
    write.flush().await
}
