//! Several services open an empty database at once, as a compose stack's
//! first boot does (`docs/NIGHTLY_STACK.md` §4): every one of them must
//! come up, and the key ring must hold one key. Before the fix, each saw
//! an empty ring and inserted a first key, the unique index on the active
//! key refused all but one, and the losers exited to be restarted. Gated
//! on `FCP_TEST_DATABASE_URL`; skipped with a notice when unset.

use fcp_crypto::EncryptionSettings;
use fcp_storage_postgres::{PostgresConfig, PostgresStorage};
use sqlx::{Connection, PgConnection};

/// 32 bytes, base64, as `openssl rand -base64 32` writes a key.
const MASTER_KEY: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
const SERVICES: usize = 8;

fn swap_database(url: &str, name: &str) -> String {
    let (base, _) = url.rsplit_once('/').expect("a database in the url");
    format!("{base}/{name}")
}

fn config(url: &str, encryption: EncryptionSettings) -> PostgresConfig {
    PostgresConfig {
        connection_string: url.to_string(),
        max_connections: 2,
        encryption: Some(encryption),
        ..Default::default()
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn services_starting_together_on_an_empty_database_share_one_key() {
    let Ok(admin_url) = std::env::var("FCP_TEST_DATABASE_URL") else {
        eprintln!("FCP_TEST_DATABASE_URL not set; skipping the first-boot test");
        return;
    };
    let name = format!("fcp_test_{}", uuid::Uuid::new_v4().simple());
    let mut admin = PgConnection::connect(&admin_url).await.unwrap();
    sqlx::query(&format!("CREATE DATABASE {name}"))
        .execute(&mut admin)
        .await
        .unwrap();
    let url = swap_database(&admin_url, &name);

    // The schema, with the key ring still empty.
    drop(
        PostgresStorage::new(config(&url, EncryptionSettings::Disabled))
            .await
            .expect("the schema"),
    );

    // Every service at once, each under the same master key.
    let mut services = Vec::new();
    for _ in 0..SERVICES {
        let cfg = config(
            &url,
            EncryptionSettings::local_from_base64(MASTER_KEY).unwrap(),
        );
        services.push(tokio::spawn(async move {
            PostgresStorage::new(cfg).await.map(|_| ())
        }));
    }
    let mut failed = Vec::new();
    for (i, service) in services.into_iter().enumerate() {
        if let Err(e) = service.await.expect("the task ran") {
            failed.push(format!("service {i}: {e:#}"));
        }
    }
    assert!(
        failed.is_empty(),
        "every service must come up on first boot:\n{}",
        failed.join("\n")
    );

    let mut conn = PgConnection::connect(&url).await.unwrap();
    let (keys, active): (i64, i64) =
        sqlx::query_as("SELECT count(*), count(*) FILTER (WHERE active) FROM data_encryption_keys")
            .fetch_one(&mut conn)
            .await
            .unwrap();
    assert_eq!((keys, active), (1, 1), "one key, and it is the active one");
    conn.close().await.ok();

    sqlx::query(&format!("DROP DATABASE {name} WITH (FORCE)"))
        .execute(&mut admin)
        .await
        .unwrap();
}
