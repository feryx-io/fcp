//! A lost connection to Redis comes back within seconds of the link
//! (`docs/MULTI_REGION_FOLLOWUPS.md` §2.3). Gated on `FCP_TEST_REDIS_URL`.
//!
//! The client's default backoff retried a lost connection after one second
//! and then every sixty, and every command waited on the retry: a short
//! outage stalled whatever shared the connection for minutes.

use fcp_storage_redis::{RedisConfig, RedisStorage};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;

/// A TCP proxy standing in for the link: `cut` drops every connection and
/// refuses new ones, `restore` lets them through again.
struct Link {
    port: u16,
    target: SocketAddr,
    tasks: Arc<Mutex<Vec<JoinHandle<()>>>>,
}

impl Link {
    async fn open(target: SocketAddr) -> Self {
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

    fn cut(&self) {
        for task in self.tasks.lock().unwrap().drain(..) {
            task.abort();
        }
    }

    async fn restore(&self) {
        let listener = TcpListener::bind(("127.0.0.1", self.port)).await.unwrap();
        self.serve(listener);
    }
}

async fn ping(storage: &RedisStorage) -> bool {
    let mut conn = storage.connection();
    matches!(
        tokio::time::timeout(
            Duration::from_secs(2),
            redis::cmd("PING").query_async::<String>(&mut conn)
        )
        .await,
        Ok(Ok(_))
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_lost_connection_comes_back_with_the_link() {
    let Ok(url) = std::env::var("FCP_TEST_REDIS_URL") else {
        eprintln!("FCP_TEST_REDIS_URL not set; skipping");
        return;
    };
    let info = redis::Client::open(url.as_str())
        .unwrap()
        .get_connection_info()
        .clone();
    let redis::ConnectionAddr::Tcp(host, port) = &info.addr else {
        panic!("a TCP Redis is needed here");
    };
    let target: SocketAddr = format!("{host}:{port}")
        .parse()
        .unwrap_or_else(|_| SocketAddr::from(([127, 0, 0, 1], *port)));
    let link = Link::open(target).await;
    let storage = RedisStorage::new(RedisConfig {
        url: format!("redis://127.0.0.1:{}/{}", link.port, info.redis.db),
        key_prefix: "test:reconnect:".to_string(),
    })
    .await
    .expect("connect through the link");
    assert!(ping(&storage).await);

    link.cut();
    // Commands fail while it is down.
    for _ in 0..3 {
        assert!(!ping(&storage).await);
    }

    link.restore().await;
    let back = tokio::time::Instant::now();
    let deadline = back + Duration::from_secs(8);
    loop {
        if ping(&storage).await {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the connection did not come back within 8 s of the link"
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    eprintln!("RECONNECT back {:?} after the link", back.elapsed());
}
