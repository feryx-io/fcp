//! Configuration down (`docs/HYBRID_CONTROL_PLANE.md` §3.2, phase 2): the
//! site's snapshot holds the tenant's slice with a cursor; a user made,
//! changed and deleted in the cloud arrives on the feed as a put with the
//! object, another put, and a delete; the feed held open answers as soon
//! as something changes and empty when nothing does; and the snapshot
//! plus every change reproduce the slice exactly. Gated on
//! `FCP_TEST_DATABASE_URL`; skipped with a notice when unset.

mod common;

use common::cloud_with_site;
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

type Slice = BTreeMap<(String, String), serde_json::Value>;

fn slice_of(snapshot: &serde_json::Value) -> Slice {
    snapshot["objects"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| {
            (
                (
                    o["table"].as_str().unwrap().to_string(),
                    o["id"].as_str().unwrap().to_string(),
                ),
                o["object"].clone(),
            )
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_snapshot_and_the_feed_reproduce_the_tenants_slice() {
    let Some(cloud) = cloud_with_site().await else {
        eprintln!("FCP_TEST_DATABASE_URL not set; skipping the sync test");
        return;
    };
    let client = cloud.site_client();
    let secret = cloud.secret();
    let get = |path: String| {
        let client = client.clone();
        let secret = secret.clone();
        let url = cloud.link_url(&path);
        async move {
            let r = client.get(&url).bearer_auth(secret).send().await.unwrap();
            let code = r.status().as_u16();
            let body: serde_json::Value = r.json().await.unwrap();
            (code, body)
        }
    };

    // The snapshot: the tenant itself, and the cursor to follow from.
    let (code, snapshot) = get("/link/v1/snapshot".to_string()).await;
    assert_eq!(code, 200, "{snapshot}");
    assert_eq!(snapshot["tenant_id"], cloud.tenant_id, "{snapshot}");
    let cursor = snapshot["cursor"].as_i64().expect("a cursor");
    let slice = slice_of(&snapshot);
    assert!(
        slice.contains_key(&("tenants".to_string(), cloud.tenant_id.clone())),
        "the tenant's own row is in its slice: {snapshot}"
    );
    assert!(
        !slice.keys().any(|(t, _)| t == "users"),
        "no users yet: {snapshot}"
    );

    // A user made in the cloud arrives on the feed, with the object.
    let (code, made) = cloud
        .admin_call(
            "POST",
            "/api/v1/users",
            true,
            Some(
                &serde_json::json!({
                    "username": "alice",
                    "password": "Alice-syncs-2026!",
                    "display_name": "Alice",
                    "tenant_id": cloud.tenant_id,
                })
                .to_string(),
            ),
        )
        .await;
    assert_eq!(code, 201, "{made}");
    let user_id = made["id"].as_str().unwrap().to_string();
    let started = Instant::now();
    let (code, changes) = get(format!("/link/v1/changes?since={cursor}&wait_secs=10")).await;
    assert_eq!(code, 200, "{changes}");
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "the feed answered a change without waiting out the hold: {:?}",
        started.elapsed()
    );
    let list = changes["changes"].as_array().unwrap();
    let put = list
        .iter()
        .find(|c| c["table"] == "users" && c["id"] == user_id)
        .unwrap_or_else(|| panic!("the user's put on the feed: {changes}"));
    assert_eq!(put["op"], "put", "{put}");
    assert_eq!(put["object"]["username"], "alice", "{put}");
    assert!(
        put["object"]["sip_ha1_hash"]
            .as_str()
            .is_some_and(|h| !h.is_empty()),
        "the site gets the credentials it challenges with: {put}"
    );
    let cursor = changes["cursor"].as_i64().unwrap();
    assert!(cursor >= put["seq"].as_i64().unwrap());

    // Nothing new: the held feed comes back empty when its wait is up.
    let started = Instant::now();
    let (code, quiet) = get(format!("/link/v1/changes?since={cursor}&wait_secs=2")).await;
    assert_eq!(code, 200, "{quiet}");
    assert!(quiet["changes"].as_array().unwrap().is_empty(), "{quiet}");
    assert_eq!(quiet["cursor"], cursor);
    assert!(
        started.elapsed() >= Duration::from_secs(2),
        "the feed held for its wait: {:?}",
        started.elapsed()
    );

    // A change and a deletion.
    let (code, renamed) = cloud
        .admin_call(
            "PUT",
            "/api/v1/users/alice",
            true,
            Some(&serde_json::json!({ "display_name": "Alice A." }).to_string()),
        )
        .await;
    assert_eq!(code, 200, "{renamed}");
    let (code, gone) = cloud
        .admin_call("DELETE", "/api/v1/users/alice", true, None)
        .await;
    assert!(code == 200 || code == 204, "{gone}");
    // Deleted, then purged without waiting for the grace period.
    let (code, purged) = cloud
        .admin_call("POST", "/api/v1/users/alice/purge", true, None)
        .await;
    assert_eq!(code, 200, "{purged}");
    let (code, changes) = get(format!("/link/v1/changes?since={cursor}&wait_secs=10")).await;
    assert_eq!(code, 200, "{changes}");
    let ops: Vec<&str> = changes["changes"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["table"] == "users" && c["id"] == user_id)
        .map(|c| c["op"].as_str().unwrap())
        .collect();
    assert!(
        ops.last() == Some(&"delete"),
        "the user's last change is its deletion: {changes}"
    );
    assert!(
        changes["changes"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|c| c["op"] == "delete")
            .all(|c| c.get("object").is_none()),
        "a delete carries no object: {changes}"
    );

    // The snapshot plus every change since is exactly the slice now.
    let (_, first) = get("/link/v1/snapshot".to_string()).await;
    let mut rebuilt = slice_of(&snapshot);
    let mut since = snapshot["cursor"].as_i64().unwrap();
    loop {
        let (_, page) = get(format!("/link/v1/changes?since={since}&wait_secs=0")).await;
        let list = page["changes"].as_array().unwrap().clone();
        if list.is_empty() {
            break;
        }
        for c in &list {
            let key = (
                c["table"].as_str().unwrap().to_string(),
                c["id"].as_str().unwrap().to_string(),
            );
            if c["op"] == "delete" {
                rebuilt.remove(&key);
            } else {
                rebuilt.insert(key, c["object"].clone());
            }
        }
        since = page["cursor"].as_i64().unwrap();
        if page["more"] != true {
            break;
        }
    }
    let now = slice_of(&first);
    assert_eq!(
        rebuilt.keys().collect::<Vec<_>>(),
        now.keys().collect::<Vec<_>>(),
        "the same objects"
    );
    for (key, object) in &now {
        assert_eq!(&rebuilt[key], object, "{key:?} as the cloud has it now");
    }

    cloud.finish().await;
}
