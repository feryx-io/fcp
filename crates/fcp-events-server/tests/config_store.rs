//! The events server's operational sections are stored in the database and
//! reconciled with its TOML at every start. Needs `FCP_TEST_DATABASE_URL`;
//! skipped otherwise.

use fcp_core::ConfigStore;
use fcp_testkit::smoke::{free_port, loopback};
use fcp_testkit::{Seed, Service, TestDatabase};

const BIN: &str = env!("CARGO_BIN_EXE_fcp-events-server");

fn config(node_id: &str, db: &str, http: u16) -> String {
    format!(
        r#"[service]
node_id = "{node_id}"

[auth]
token = "config-store-token"

[websocket]
bind_address = "127.0.0.1:{http}"

[event_bus]
bus_type = "in-memory"

[webhook]
enabled = true
database_url = "{db}"

[logging]
level = "info"
format = "compact"
"#
    )
}

/// #341: the consumer name defaulted to a name made at each start, so the
/// stored `[event_bus]` section was rewritten, and its version bumped, at
/// every start. It is now this instance's own, from its file or its node
/// id, and not part of the stored section at all: a second server sharing
/// the database does not touch it either.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_event_bus_section_is_not_rewritten_at_every_start() {
    let Some(db) = TestDatabase::create().await else {
        eprintln!("FCP_TEST_DATABASE_URL not set; skipping");
        return;
    };
    let seed = Seed::new(db.url(), "config-store.local").await;
    let storage = seed.storage();
    let boot = |node_id: &'static str| {
        let http = free_port();
        let config = config(node_id, db.url(), http);
        async move { Service::start("events-server", BIN, &config, &loopback(http, "/health")).await }
    };
    let stored = || async {
        storage
            .get_config("events-server", "event_bus", "_system")
            .await
            .unwrap()
            .expect("the section is stored")
    };

    boot("events-a").await.stop().await;
    let first = stored().await;
    assert!(
        first.value.get("consumer_name").is_none(),
        "the consumer is no part of the stored section: {}",
        first.value
    );

    boot("events-a").await.stop().await;
    boot("events-b").await.stop().await;
    let after = stored().await;
    assert_eq!(
        after.version, first.version,
        "the same file, or another server's, rewrote the section: {}",
        after.value
    );
    db.drop_db().await;
}
