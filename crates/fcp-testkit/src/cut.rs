//! A link a test can cut: a TCP proxy standing in for the network between
//! two stacks. `cut` drops every connection through it and refuses new
//! ones; `restore` lets them through again, on the same port. What
//! `fcp-cluster/tests/partition.rs` used for a node's link to its store,
//! and `docs/HYBRID_CONTROL_PLANE.md` phase 6 uses for a site's link to
//! its cloud.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;

pub struct CutLink {
    port: u16,
    target: SocketAddr,
    tasks: Arc<Mutex<Vec<JoinHandle<()>>>>,
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
        };
        link.serve(listener);
        link
    }

    /// Where the near side connects.
    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn addr(&self) -> SocketAddr {
        SocketAddr::from(([127, 0, 0, 1], self.port))
    }

    fn serve(&self, listener: TcpListener) {
        let target = self.target;
        let tasks = Arc::clone(&self.tasks);
        let accept = tokio::spawn(async move {
            while let Ok((mut inbound, _)) = listener.accept().await {
                let pipe = tokio::spawn(async move {
                    if let Ok(mut outbound) = TcpStream::connect(target).await {
                        let _ = tokio::io::copy_bidirectional(&mut inbound, &mut outbound).await;
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

    /// Let connections through again, on the same port.
    pub async fn restore(&self) {
        let listener = TcpListener::bind(("127.0.0.1", self.port)).await.unwrap();
        self.serve(listener);
    }
}

impl Drop for CutLink {
    fn drop(&mut self) {
        self.cut();
    }
}
