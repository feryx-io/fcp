//! A shared line's appearances in the cluster's Redis
//! (`docs/ENTERPRISE_CALLING.md` §3.9): what one node claims another sees
//! and cannot take, a seize becomes its call, a lapsed entry frees its
//! index, and only an owner releases. Skipped without `FCP_TEST_REDIS_URL`.

use fcp_cluster::LineAppearances;
use fcp_core::sca::{Appearance, AppearanceState};
use fcp_storage_redis::{RedisConfig, RedisStorage};
use std::time::{SystemTime, UNIX_EPOCH};

async fn node(prefix: &str) -> Option<LineAppearances> {
    let url = std::env::var("FCP_TEST_REDIS_URL").ok()?;
    let storage = RedisStorage::new(RedisConfig {
        url,
        key_prefix: prefix.to_string(),
    })
    .await
    .ok()?;
    Some(LineAppearances::new(storage))
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

fn entry(owner: &str, node: &str, state: AppearanceState, expires_at: u64) -> Appearance {
    Appearance {
        index: 0,
        state,
        owner: owner.into(),
        node: node.into(),
        node_uri: Some(format!("sip:{node}:5060")),
        handoff: Some("token".into()),
        contact: None,
        remote: None,
        expires_at,
    }
}

#[tokio::test]
async fn two_nodes_share_a_lines_appearances() {
    let cluster = format!("test:{}:", uuid::Uuid::new_v4());
    let Some(one) = node(&cluster).await else {
        eprintln!("skipped: FCP_TEST_REDIS_URL is not set");
        return;
    };
    let two = node(&cluster).await.expect("a second node");
    let far = now() + 600;
    let (t, l) = ("tenant-a", "line-a");

    assert_eq!(
        one.claim(
            t,
            l,
            None,
            2,
            entry("call:1", "one", AppearanceState::Alerting, far),
            None
        )
        .await
        .unwrap(),
        Some(1)
    );
    assert_eq!(
        two.claim(
            t,
            l,
            None,
            2,
            entry("call:2", "two", AppearanceState::Progressing, far),
            None
        )
        .await
        .unwrap(),
        Some(2),
        "the second node's call takes the next index"
    );
    assert_eq!(
        two.claim(
            t,
            l,
            None,
            2,
            entry("call:3", "two", AppearanceState::Alerting, far),
            None
        )
        .await
        .unwrap(),
        None,
        "the line is full on every node"
    );
    // The owner moves its own appearance on; nobody else can.
    assert_eq!(
        one.claim(
            t,
            l,
            Some(1),
            2,
            entry("call:1", "one", AppearanceState::Held, far),
            None
        )
        .await
        .unwrap(),
        Some(1)
    );
    let seen = two.list(t, l).await.unwrap();
    assert_eq!(seen.len(), 2);
    assert_eq!((seen[0].index, seen[0].state), (1, AppearanceState::Held));
    assert_eq!(seen[0].node_uri.as_deref(), Some("sip:one:5060"));

    two.release(t, l, 1, "call:2").await.unwrap();
    assert_eq!(
        two.list(t, l).await.unwrap().len(),
        2,
        "only the owner frees an index"
    );
    one.release(t, l, 1, "call:1").await.unwrap();
    assert_eq!(two.list(t, l).await.unwrap().len(), 1);
}

#[tokio::test]
async fn a_seize_becomes_its_call_and_a_lapsed_one_holds_nothing() {
    let Some(table) = node(&format!("test:{}:", uuid::Uuid::new_v4())).await else {
        eprintln!("skipped: FCP_TEST_REDIS_URL is not set");
        return;
    };
    let (t, l) = ("tenant-b", "line-b");
    table
        .claim(
            t,
            l,
            Some(3),
            4,
            entry("seize:x", "one", AppearanceState::Seized, now() + 30),
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        table
            .claim(
                t,
                l,
                Some(3),
                4,
                entry("call:y", "one", AppearanceState::Progressing, now() + 600),
                None
            )
            .await
            .unwrap(),
        None,
        "another phone's seize holds the index"
    );
    assert_eq!(
        table
            .claim(
                t,
                l,
                Some(3),
                4,
                entry("call:y", "one", AppearanceState::Progressing, now() + 600),
                Some("seize:x")
            )
            .await
            .unwrap(),
        Some(3),
        "the seizing phone's call takes its own seize"
    );
    table
        .claim(
            t,
            l,
            Some(2),
            4,
            entry("seize:z", "one", AppearanceState::Seized, now()),
            None,
        )
        .await
        .unwrap();
    assert!(
        table.list(t, l).await.unwrap().iter().all(|a| a.index != 2),
        "a lapsed seize is not listed"
    );
    assert_eq!(
        table
            .claim(
                t,
                l,
                None,
                4,
                entry("call:w", "one", AppearanceState::Alerting, now() + 600),
                None
            )
            .await
            .unwrap(),
        Some(1)
    );
    assert_eq!(
        table
            .claim(
                t,
                l,
                Some(2),
                4,
                entry("call:v", "one", AppearanceState::Alerting, now() + 600),
                None
            )
            .await
            .unwrap(),
        Some(2),
        "a lapsed seize's index is free"
    );
}
