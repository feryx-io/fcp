//! The fixed-window counter the admin API's rate limits count in
//! (`docs/DEVELOPER_SURFACE.md` §3, phase 2): counts rise, keys expire, and
//! two connections share a count. Gated on `FCP_TEST_REDIS_URL`.

use fcp_storage_redis::{RedisConfig, RedisStorage};
use std::time::Duration;

#[tokio::test]
async fn a_window_counts_across_connections_and_expires() {
    let Ok(url) = std::env::var("FCP_TEST_REDIS_URL") else {
        eprintln!("FCP_TEST_REDIS_URL not set; skipping");
        return;
    };
    let prefix = format!("fcp-test-{}:", uuid::Uuid::new_v4());
    let config = RedisConfig {
        url,
        key_prefix: prefix,
    };
    let one = RedisStorage::new(config.clone()).await.expect("redis");
    let two = RedisStorage::new(config).await.expect("redis");

    let ttl = Duration::from_secs(1);
    assert_eq!(one.count_in_window("k", ttl).await.unwrap(), 1);
    assert_eq!(two.count_in_window("k", ttl).await.unwrap(), 2);
    assert_eq!(one.count_in_window("k", ttl).await.unwrap(), 3);
    assert_eq!(one.count_in_window("other", ttl).await.unwrap(), 1);

    // The first hit set the expiry; later hits did not push it back.
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert_eq!(one.count_in_window("k", ttl).await.unwrap(), 1);
}
