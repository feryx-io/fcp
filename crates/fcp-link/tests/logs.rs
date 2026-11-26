//! A site's warnings and errors reach the cloud's Logs page
//! (`docs/LOG_VIEWER.md` §5, L5): the link reads the site's services'
//! streams in the site's Redis and carries what is `WARN` or worse up the
//! link; the cloud keeps it as `site:<site>:<service>`, every line the
//! site's tenant's. Gated on `FCP_TEST_DATABASE_URL` and `FCP_TEST_REDIS_URL`.

mod common;

use common::{admin, cloud_with_site_opts, CloudOptions, A_WHILE};
use fcp_cluster::logging::{self, LogRecord};
use fcp_testkit::cluster::private_redis_url;
use std::collections::BTreeMap;

fn line(level: &str, message: &str) -> LogRecord {
    LogRecord {
        ts: 0,
        level: level.into(),
        target: "fcp_sip::handler".into(),
        message: message.into(),
        fields: BTreeMap::new(),
        // What a site's line says of its tenant is not believed.
        tenant_id: Some("11111111-1111-1111-1111-111111111111".into()),
        call_id: None,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_sites_warnings_reach_the_clouds_logs() {
    let (Some(cloud_redis), Some(site_redis)) = (private_redis_url(), private_redis_url()) else {
        eprintln!("FCP_TEST_REDIS_URL not set; skipping the site logs test");
        return;
    };
    let Some(cloud) = cloud_with_site_opts(CloudOptions {
        cloud_redis_url: Some(cloud_redis),
        site_redis_url: Some(site_redis.clone()),
        ..Default::default()
    })
    .await
    else {
        eprintln!("FCP_TEST_DATABASE_URL not set; skipping the site logs test");
        return;
    };
    let site = fcp_storage_redis::RedisStorage::new(fcp_storage_redis::RedisConfig {
        url: site_redis,
        ..Default::default()
    })
    .await
    .unwrap();
    // A service at the site that started after the link: read from its start.
    let ship = |lines: Vec<LogRecord>| {
        let site = site.clone();
        async move {
            let mut conn = site.connection();
            for l in lines {
                let _: String = redis::cmd("XADD")
                    .arg(site.key(&logging::stream_key("site-cm-1")))
                    .arg("*")
                    .arg("r")
                    .arg(serde_json::to_string(&l).unwrap())
                    .query_async(&mut conn)
                    .await
                    .unwrap();
            }
        }
    };
    // The link reads the site's streams every few seconds; this waits out
    // its first look, which takes every stream there as already sent.
    tokio::time::sleep(std::time::Duration::from_secs(7)).await;
    ship(vec![
        line("INFO", "site: a call answered"),
        line("WARN", "site: a trunk is slow"),
        line("ERROR", "site: the database refused a write"),
    ])
    .await;

    let path = format!(
        "/api/v1/logs?service=site:{}:site-cm-1&tenant=all",
        cloud.site_id
    );
    let deadline = tokio::time::Instant::now() + A_WHILE;
    let lines = loop {
        let (code, body) = admin(cloud.http, "GET", &path, None, None).await;
        assert_eq!(code, 200, "{body}");
        let lines = body["data"].as_array().cloned().unwrap_or_default();
        if lines.len() >= 2 {
            break lines;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the site's warnings in the cloud: {body}"
        );
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    };
    let messages: Vec<&str> = lines.iter().filter_map(|l| l["message"].as_str()).collect();
    assert_eq!(
        messages,
        vec![
            "site: the database refused a write",
            "site: a trunk is slow"
        ],
        "warnings and errors only, newest first"
    );
    let tenant = cloud
        .tenant_id
        .strip_prefix("tenant-")
        .unwrap_or(&cloud.tenant_id)
        .to_string();
    assert!(
        lines
            .iter()
            .all(|l| l["tenant_id"].as_str() == Some(tenant.as_str())),
        "every line the site's tenant's: {lines:?}"
    );

    cloud.finish().await;
}
