//! The cluster's count of a tenant's calls, against real Redis
//! (`docs/CAPACITY_AND_SURVIVABILITY.md` §7, §11).
//!
//! What the unit tests in `fcp-sbc-core` cannot show is the part that
//! lives in Redis: that a lease is idempotent, that one node sees
//! another's, that a dead node's leases stop counting when their time
//! passes, and that a refresh keeps a long call's place. Skipped without
//! `FCP_TEST_REDIS_URL`.

use fcp_cluster::{CallLeases, LeaseSubject};
use fcp_storage_redis::{RedisConfig, RedisStorage};
use std::time::Duration;

/// A lease store on its own key prefix, so tests do not see each other.
async fn leases(ttl_secs: u64) -> Option<CallLeases> {
    leases_sharing(&format!("test:{}:", uuid::Uuid::new_v4()), ttl_secs).await
}

/// A lease store on a named prefix: two of these are two nodes of one
/// cluster, which is what the count is supposed to be shared across.
async fn leases_sharing(prefix: &str, ttl_secs: u64) -> Option<CallLeases> {
    let url = std::env::var("FCP_TEST_REDIS_URL").ok()?;
    let storage = RedisStorage::new(RedisConfig {
        url,
        key_prefix: prefix.to_string(),
    })
    .await
    .ok()?;
    Some(CallLeases::new(storage).with_ttl(ttl_secs))
}

macro_rules! leases_or_skip {
    ($ttl:expr) => {
        match leases($ttl).await {
            Some(l) => l,
            None => {
                eprintln!("skipped: FCP_TEST_REDIS_URL is not set");
                return;
            }
        }
    };
}

/// What one node takes, another node counts: the point of the thing.
#[tokio::test]
async fn a_lease_is_the_clusters_not_a_nodes() {
    let cluster = format!("test:{}:", uuid::Uuid::new_v4());
    let Some(one) = leases_sharing(&cluster, 60).await else {
        eprintln!("skipped: FCP_TEST_REDIS_URL is not set");
        return;
    };
    let two = leases_sharing(&cluster, 60).await.expect("a second node");
    let elsewhere = leases(60).await.expect("a store of its own");
    let tenant = LeaseSubject::Tenant("acme");

    assert_eq!(one.take(tenant, "call-a").await.unwrap(), 1);
    // The second node sees the first's call and counts itself after it.
    assert_eq!(two.take(tenant, "call-b").await.unwrap(), 2);
    assert_eq!(one.held(tenant).await.unwrap(), 2, "one cluster, one count");
    assert_eq!(two.held(tenant).await.unwrap(), 2);
    // Another cluster's tenant of the same name is not this one.
    assert_eq!(elsewhere.held(tenant).await.unwrap(), 0);

    // A call ending on one node frees the contract on the other.
    one.release(tenant, "call-a").await.unwrap();
    assert_eq!(two.held(tenant).await.unwrap(), 1);
    // Releasing what was never held is not an error.
    one.release(tenant, "call-a").await.unwrap();
    two.release(tenant, "never-taken").await.unwrap();
    assert_eq!(one.held(tenant).await.unwrap(), 1);
}

/// A retried INVITE takes the same lease, not a second one.
#[tokio::test]
async fn taking_the_same_call_twice_counts_once() {
    let leases = leases_or_skip!(60);
    let tenant = LeaseSubject::Tenant("acme");
    assert_eq!(leases.take(tenant, "call-a").await.unwrap(), 1);
    assert_eq!(leases.take(tenant, "call-a").await.unwrap(), 1);
    assert_eq!(leases.held(tenant).await.unwrap(), 1);
}

/// A tenant and a trunk are counted apart.
#[tokio::test]
async fn subjects_do_not_share_a_count() {
    let leases = leases_or_skip!(60);
    leases
        .take(LeaseSubject::Tenant("acme"), "call-a")
        .await
        .unwrap();
    leases
        .take(LeaseSubject::Trunk("carrier-1"), "call-a")
        .await
        .unwrap();
    assert_eq!(leases.held(LeaseSubject::Tenant("acme")).await.unwrap(), 1);
    assert_eq!(
        leases.held(LeaseSubject::Trunk("carrier-1")).await.unwrap(),
        1
    );
    assert_eq!(leases.held(LeaseSubject::Tenant("other")).await.unwrap(), 0);
}

/// A node that dies cannot give its leases back; they stop counting when
/// their time passes, rather than holding a tenant's contract for ever.
#[tokio::test]
async fn a_lease_nobody_gives_back_lapses() {
    let leases = leases_or_skip!(1);
    let tenant = LeaseSubject::Tenant("acme");
    assert_eq!(leases.take(tenant, "call-a").await.unwrap(), 1);
    assert_eq!(leases.held(tenant).await.unwrap(), 1);

    tokio::time::sleep(Duration::from_millis(1100)).await;
    assert_eq!(
        leases.held(tenant).await.unwrap(),
        0,
        "the dead node's call stopped counting"
    );
    // And the next call to arrive counts from there, not from two.
    assert_eq!(leases.take(tenant, "call-b").await.unwrap(), 1);
}

/// A call longer than the lease keeps its place, because its node says
/// so on the settling tick.
#[tokio::test]
async fn a_refreshed_lease_outlives_its_ttl() {
    let leases = leases_or_skip!(2);
    let tenant = LeaseSubject::Tenant("acme");
    leases.take(tenant, "long-call").await.unwrap();

    for _ in 0..3 {
        tokio::time::sleep(Duration::from_millis(700)).await;
        leases
            .refresh(tenant, &["long-call".to_string()])
            .await
            .unwrap();
    }
    assert_eq!(
        leases.held(tenant).await.unwrap(),
        1,
        "still counted after longer than its lifetime"
    );
    // Refreshing nothing is not an error.
    leases.refresh(tenant, &[]).await.unwrap();
}
