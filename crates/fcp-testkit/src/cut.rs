//! A link a test can cut: a TCP proxy standing in for the network between
//! two stacks. `cut` drops every connection through it and refuses new
//! ones; `restore` lets them through again, on the same port. What
//! `fcp-cluster/tests/partition.rs` used for a node's link to its store,
//! and `docs/HYBRID_CONTROL_PLANE.md` phase 6 uses for a site's link to
//! its cloud.
//!
//! `silence` is the other way a network fails: connections stay open and
//! new ones are accepted, but nothing crosses in either direction, as
//! across a partition or to a host that has gone away without a reset.
//! `restore` lets the held bytes through. Deep testing D5 takes Postgres
//! and Redis away both ways (`docs/DEEP_TESTING.md`).

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;
use tokio::task::JoinHandle;

pub struct CutLink {
    port: u16,
    target: SocketAddr,
    tasks: Arc<Mutex<Vec<JoinHandle<()>>>>,
    /// True while the link is silent.
    silent: watch::Sender<bool>,
}

impl CutLink {
    /// A proxy on a free loopback port, forwarding to `target`.
    pub async fn open(target: SocketAddr) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let link = Self {
            port,
            target,
            tasks: Arc::new(Mutex::new(Vec::new())),
            silent: watch::channel(false).0,
        };
        link.serve(listener);
        link
    }

    /// A link to the host and port `url` names (`localhost` resolved),
    /// and `url` through it.
    pub async fn for_url(url: &str) -> (Self, String) {
        let rest = url.split_once("://").expect("a URL with a scheme").1;
        let authority = rest.split('/').next().unwrap_or(rest);
        let host_port = authority.rsplit('@').next().unwrap_or(authority);
        let target = tokio::net::lookup_host(host_port)
            .await
            .unwrap_or_else(|e| panic!("{host_port} does not resolve: {e}"))
            .find(|a| a.is_ipv4())
            .unwrap_or_else(|| panic!("{host_port} has no IPv4 address"));
        let link = Self::open(target).await;
        let through = link.url_through(url);
        (link, through)
    }

    /// Where the near side connects.
    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn addr(&self) -> SocketAddr {
        SocketAddr::from(([127, 0, 0, 1], self.port))
    }

    /// `url` with its `host:port` replaced by this link's, so a client
    /// given it connects through the link: `redis://127.0.0.1:6379/3`,
    /// `postgresql://user:pw@db:5432/name`.
    pub fn url_through(&self, url: &str) -> String {
        let (scheme, rest) = url.split_once("://").expect("a URL with a scheme");
        let (authority, path) = match rest.find('/') {
            Some(i) => rest.split_at(i),
            None => (rest, ""),
        };
        let userinfo = match authority.rfind('@') {
            Some(i) => &authority[..=i],
            None => "",
        };
        format!("{scheme}://{userinfo}{}{path}", self.addr())
    }

    fn serve(&self, listener: TcpListener) {
        let target = self.target;
        let tasks = Arc::clone(&self.tasks);
        let silent = self.silent.subscribe();
        let accept = tokio::spawn(async move {
            while let Ok((inbound, _)) = listener.accept().await {
                let silent = silent.clone();
                let pipe = tokio::spawn(async move {
                    if let Ok(outbound) = TcpStream::connect(target).await {
                        pipe(inbound, outbound, silent).await;
                    }
                });
                tasks.lock().unwrap().push(pipe);
            }
        });
        self.tasks.lock().unwrap().push(accept);
    }

    /// Drop every connection and refuse new ones.
    pub fn cut(&self) {
        for task in self.tasks.lock().unwrap().drain(..) {
            task.abort();
        }
    }

    /// Keep every connection, and accept new ones, but carry nothing
    /// either way until [`restore`](Self::restore).
    pub fn silence(&self) {
        self.silent.send_replace(true);
    }

    /// Let connections through again, on the same port: after a cut, a
    /// listener anew; after a silence, the held bytes.
    pub async fn restore(&self) {
        if self.silent.send_replace(false) {
            return;
        }
        let listener = TcpListener::bind(("127.0.0.1", self.port)).await.unwrap();
        self.serve(listener);
    }
}

impl Drop for CutLink {
    fn drop(&mut self) {
        self.cut();
    }
}

/// Copy both ways, holding each direction still while the link is silent.
async fn pipe(inbound: TcpStream, outbound: TcpStream, silent: watch::Receiver<bool>) {
    let (in_read, in_write) = inbound.into_split();
    let (out_read, out_write) = outbound.into_split();
    tokio::select! {
        _ = copy(in_read, out_write, silent.clone()) => {}
        _ = copy(out_read, in_write, silent) => {}
    }
}

async fn copy(
    mut from: impl AsyncRead + Unpin,
    mut to: impl AsyncWrite + Unpin,
    mut silent: watch::Receiver<bool>,
) {
    let mut buf = vec![0u8; 16 * 1024];
    loop {
        // Silent: neither read (so the sender's window fills) nor write.
        if silent.wait_for(|s| !*s).await.is_err() {
            return;
        }
        let n = match from.read(&mut buf).await {
            Ok(0) | Err(_) => return,
            Ok(n) => n,
        };
        if silent.wait_for(|s| !*s).await.is_err() {
            return;
        }
        if to.write_all(&buf[..n]).await.is_err() {
            return;
        }
    }
}
