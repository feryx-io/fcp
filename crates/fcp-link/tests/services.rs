//! A site's services reach the cloud's record (`docs/SERVICES.md` phase
//! S2): the link writes its own beacon into the site's Redis, carries the
//! site's beacons up on its heartbeat, and the cloud's admin lists them
//! under the site, by name; when the link stops, its own service is said
//! stopped. Gated on `FCP_TEST_DATABASE_URL` and `FCP_TEST_REDIS_URL`.

mod common;

use common::{admin, cloud_with_site_opts, CloudOptions, A_WHILE};
use fcp_testkit::cluster::private_redis_url;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_sites_services_are_kept_by_the_cloud() {
    let (Some(cloud_redis), Some(site_redis)) = (private_redis_url(), private_redis_url()) else {
        eprintln!("FCP_TEST_REDIS_URL not set; skipping the site services test");
        return;
    };
    let Some(mut cloud) = cloud_with_site_opts(CloudOptions {
        cloud_redis_url: Some(cloud_redis),
        site_redis_url: Some(site_redis),
        ..Default::default()
    })
    .await
    else {
        eprintln!("FCP_TEST_DATABASE_URL not set; skipping the site services test");
        return;
    };
    let path = format!("/api/v1/system/services?site_id={}", cloud.site_id);

    let deadline = tokio::time::Instant::now() + A_WHILE;
    let link = loop {
        let (code, body) = admin(cloud.http, "GET", &path, None, None).await;
        assert_eq!(code, 200, "{body}");
        if let Some(s) = body["data"]
            .as_array()
            .and_then(|l| l.iter().find(|s| s["kind"] == "link"))
            // Its first beacons may go out before it has enrolled: degraded,
            // its `cloud` check "not enrolled".
            .filter(|s| s["state"] == "serving")
        {
            break s.clone();
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the site's link among the cloud's services: {body}"
        );
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    };
    assert_eq!(link["site_id"], cloud.site_id, "{link}");
    assert_eq!(link["site_name"], "acme-hq", "{link}");
    assert_eq!(link["version"], env!("CARGO_PKG_VERSION"), "{link}");
    assert!(
        link["checks"]
            .as_array()
            .is_some_and(|c| c.iter().any(|c| c["name"] == "cloud" && c["ok"] == true)),
        "the link reaches the cloud: {link}"
    );
    // Not among the platform's own.
    let (_, platform) = admin(
        cloud.http,
        "GET",
        "/api/v1/system/services?site_id=platform",
        None,
        None,
    )
    .await;
    assert!(
        platform["data"]
            .as_array()
            .unwrap()
            .iter()
            .all(|s| s["kind"] != "link"),
        "{platform}"
    );

    // The link stops: its own beacon said stopping, but the heartbeat that
    // would carry it may not go out first, so the cloud sees the site's
    // link down and the service unknown, or stopped.
    let link_id = link["id"].as_str().unwrap().to_string();
    cloud.link.take().unwrap().stop().await;
    let deadline = tokio::time::Instant::now() + A_WHILE;
    loop {
        let (_, body) = admin(cloud.http, "GET", &path, None, None).await;
        let state = body["data"]
            .as_array()
            .and_then(|l| l.iter().find(|s| s["id"] == link_id))
            .map(|s| s["state"].clone());
        if state
            .as_ref()
            .is_some_and(|s| s == "unknown" || s == "stopped")
        {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the stopped link is no longer serving: {body}"
        );
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }

    cloud.finish().await;
}
