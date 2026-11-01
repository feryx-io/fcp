//! Every process says it is there (`docs/SERVICES.md` phase S1), against a
//! real Redis. Gated on `FCP_TEST_REDIS_URL`; skipped with a notice when
//! unset.
//!
//! A beacon is written and refreshed; a failing dependency makes the
//! service degraded but still ready, a failing database makes it not
//! ready; draining is said; a shutdown leaves `stopping` behind, which
//! expires; a beacon started while Redis is away says so and writes once
//! Redis is there.

use fcp_cluster::{Check, ServiceBeacon, ServiceIdentity, ServiceRecord};
use fcp_core::service_health::ServiceState;
use fcp_storage_redis::{RedisConfig, RedisStorage};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

fn redis_url() -> Option<String> {
    let url = std::env::var("FCP_TEST_REDIS_URL").ok();
    if url.is_none() {
        eprintln!("FCP_TEST_REDIS_URL not set; skipping the service beacon test");
    }
    url
}

fn config(url: &str, prefix: &str) -> RedisConfig {
    RedisConfig {
        url: url.to_string(),
        key_prefix: prefix.to_string(),
    }
}

async fn read(storage: &RedisStorage, id: &str) -> Option<ServiceRecord> {
    ServiceBeacon::read_all(storage)
        .await
        .unwrap()
        .into_iter()
        .find(|r| r.id == id)
}

async fn until(
    what: &str,
    storage: &RedisStorage,
    id: &str,
    check: impl Fn(&Option<ServiceRecord>) -> bool,
) -> Option<ServiceRecord> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let r = read(storage, id).await;
        if check(&r) {
            return r;
        }
        assert!(tokio::time::Instant::now() < deadline, "{what}: {r:?}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test]
async fn a_service_says_what_it_is_how_it_is_and_when_it_stops() {
    let Some(url) = redis_url() else { return };
    let prefix = format!("svc-{}:", uuid::Uuid::new_v4().simple());
    let storage = RedisStorage::new(config(&url, &prefix)).await.unwrap();

    let tts_up = Arc::new(AtomicBool::new(true));
    let db_up = Arc::new(AtomicBool::new(true));
    let draining = Arc::new(AtomicBool::new(false));
    let (t, d, dr) = (tts_up.clone(), db_up.clone(), draining.clone());
    let beacon = ServiceBeacon::new(
        ServiceIdentity::new("vm-01", "voicemail")
            .version("9.9.9")
            .http("10.0.0.5:8083"),
    )
    .check(Check::database(move || {
        let up = d.load(Ordering::Relaxed);
        async move {
            if up {
                Ok(())
            } else {
                Err("timed out".to_string())
            }
        }
    }))
    .check(Check::new("tts", false, move || {
        let up = t.load(Ordering::Relaxed);
        async move {
            if up {
                Ok(())
            } else {
                Err("refused".to_string())
            }
        }
    }))
    .load(|| async { BTreeMap::from([("calls".to_string(), 3)]) })
    .draining(move || dr.load(Ordering::Relaxed))
    .with_intervals(Duration::from_millis(200), 2);
    let handle = beacon.start(config(&url, &prefix));
    let health = handle.health();

    let r = until("serving", &storage, "vm-01", |r| {
        r.as_ref().is_some_and(|r| r.state == ServiceState::Serving)
    })
    .await
    .unwrap();
    assert_eq!(r.kind, "voicemail");
    assert_eq!(r.version, "9.9.9");
    assert_eq!(r.http_address.as_deref(), Some("10.0.0.5:8083"));
    assert_eq!(r.load.get("calls"), Some(&3));
    let names: Vec<_> = r.checks.iter().map(|c| c.name.as_str()).collect();
    assert!(names.contains(&"database") && names.contains(&"tts") && names.contains(&"redis"));
    assert!(health.ready());

    // A dependency it can serve without.
    tts_up.store(false, Ordering::Relaxed);
    let r = until("degraded", &storage, "vm-01", |r| {
        r.as_ref()
            .is_some_and(|r| r.state == ServiceState::Degraded)
    })
    .await
    .unwrap();
    let tts = r.checks.iter().find(|c| c.name == "tts").unwrap();
    assert!(!tts.ok && tts.error.as_deref() == Some("refused"));
    assert!(health.ready(), "a dependency's outage is not a refusal");

    // The database it cannot.
    db_up.store(false, Ordering::Relaxed);
    until("database failing", &storage, "vm-01", |r| {
        r.as_ref()
            .is_some_and(|r| r.checks.iter().any(|c| c.name == "database" && !c.ok))
    })
    .await;
    assert!(!health.ready());
    db_up.store(true, Ordering::Relaxed);
    tts_up.store(true, Ordering::Relaxed);

    draining.store(true, Ordering::Relaxed);
    until("draining", &storage, "vm-01", |r| {
        r.as_ref()
            .is_some_and(|r| r.state == ServiceState::Draining)
    })
    .await;

    // A shutdown is said, then expires.
    handle.shutdown().await;
    let r = read(&storage, "vm-01").await.unwrap();
    assert_eq!(r.state, ServiceState::Stopping);
    until("expired", &storage, "vm-01", |r| r.is_none()).await;
}

#[tokio::test]
async fn a_beacon_started_without_redis_writes_once_redis_is_there() {
    let Some(url) = redis_url() else { return };
    let prefix = format!("svc-{}:", uuid::Uuid::new_v4().simple());
    // Nothing listens on port 1.
    let handle = ServiceBeacon::new(ServiceIdentity::new("jobs-01", "jobs"))
        .with_intervals(Duration::from_millis(200), 2)
        .start(config("redis://127.0.0.1:1", &prefix));
    let health = handle.health();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while !health.checks().iter().any(|c| c.name == "redis" && !c.ok) {
        assert!(tokio::time::Instant::now() < deadline, "redis said failing");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(!health.ready(), "redis is required");
    handle.shutdown().await;

    // And one with Redis there is listed.
    let storage = RedisStorage::new(config(&url, &prefix)).await.unwrap();
    let handle = ServiceBeacon::new(ServiceIdentity::new("jobs-01", "jobs"))
        .with_intervals(Duration::from_millis(200), 2)
        .start(config(&url, &prefix));
    until("listed", &storage, "jobs-01", |r| r.is_some()).await;
    handle.shutdown().await;
}
