//! Moving to a new master key (`docs/ENCRYPTION.md`): a node given the new
//! key cannot read the ring the old key wrapped; given the old one too
//! (`FCP_PREVIOUS_MASTER_KEY`) it reads everything and rewraps every key,
//! after which the new key alone reads what was sealed before. The one
//! test of its binary: it sets the environment. Gated on
//! `FCP_TEST_DATABASE_URL`; skipped with a notice when unset.

use fcp_core::EncryptionStore;
use fcp_crypto::EncryptionSettings;
use fcp_storage_postgres::{PostgresConfig, PostgresStorage};
use sqlx::{Connection, PgConnection};

const OLD_KEY: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
const NEW_KEY: &str = "AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE=";

fn swap_database(url: &str, name: &str) -> String {
    let (base, _) = url.rsplit_once('/').expect("a database in the url");
    format!("{base}/{name}")
}

async fn open(url: &str, key: &str) -> anyhow::Result<PostgresStorage> {
    PostgresStorage::new(PostgresConfig {
        connection_string: url.to_string(),
        max_connections: 2,
        encryption: Some(EncryptionSettings::local_from_base64(key).unwrap()),
        ..Default::default()
    })
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_new_master_key_takes_over_the_ring_the_old_one_wrapped() {
    let Ok(admin_url) = std::env::var("FCP_TEST_DATABASE_URL") else {
        eprintln!("FCP_TEST_DATABASE_URL not set; skipping the rewrap test");
        return;
    };
    std::env::remove_var("FCP_PREVIOUS_MASTER_KEY");
    let name = format!("fcp_test_{}", uuid::Uuid::new_v4().simple());
    let mut admin = PgConnection::connect(&admin_url).await.unwrap();
    sqlx::query(&format!("CREATE DATABASE {name}"))
        .execute(&mut admin)
        .await
        .unwrap();
    let url = swap_database(&admin_url, &name);

    // Under the old key: two keys (one rotated out), a secret sealed with each.
    let old = open(&url, OLD_KEY).await.expect("the old key opens");
    let first = old.seal_secret("sealed under the first key").await.unwrap();
    old.rotate_encryption_key().await.unwrap();
    let second = old
        .seal_secret("sealed under the second key")
        .await
        .unwrap();
    let old_kek = old.encryption_status().await.unwrap().kek_id.unwrap();
    drop(old);

    // The new key alone cannot read a ring the old one wrapped.
    let refused = open(&url, NEW_KEY).await;
    assert!(
        refused.is_err(),
        "the new key alone opened a ring the old one wrapped"
    );

    // With the old key as the previous one, it reads everything and
    // rewraps every key.
    std::env::set_var("FCP_PREVIOUS_MASTER_KEY", OLD_KEY);
    let moving = open(&url, NEW_KEY).await.expect("both keys open the ring");
    assert_eq!(
        moving.open_secret(&first).await.unwrap(),
        "sealed under the first key"
    );
    let report = moving.rewrap_encryption_keys().await.unwrap();
    assert_eq!(
        (report.rewrapped, report.already_current),
        (2, 0),
        "{report:?}"
    );
    assert!(report.unreadable.is_empty(), "{report:?}");
    let new_kek = moving.encryption_status().await.unwrap().kek_id.unwrap();
    assert_ne!(new_kek, old_kek);
    drop(moving);

    // The new key alone now reads what either key sealed, and a second
    // pass has nothing to do.
    std::env::remove_var("FCP_PREVIOUS_MASTER_KEY");
    let new = open(&url, NEW_KEY).await.expect("the new key alone opens");
    assert_eq!(
        new.open_secret(&first).await.unwrap(),
        "sealed under the first key"
    );
    assert_eq!(
        new.open_secret(&second).await.unwrap(),
        "sealed under the second key"
    );
    let again = new.rewrap_encryption_keys().await.unwrap();
    assert_eq!(
        (again.rewrapped, again.already_current),
        (0, 2),
        "{again:?}"
    );
    drop(new);

    // And the old key alone no longer can.
    assert!(
        open(&url, OLD_KEY).await.is_err(),
        "the old key still opens the ring"
    );

    drop(admin);
    let mut admin = PgConnection::connect(&admin_url).await.unwrap();
    let _ = sqlx::query(&format!("DROP DATABASE IF EXISTS {name} WITH (FORCE)"))
        .execute(&mut admin)
        .await;
}
