//! A site's firmware (`docs/PROVISIONING.md` §11, Q11): an image the
//! platform administrator uploads to the cloud and the tenant targets
//! reaches the site — the target synced with its image, the file fetched
//! over the link and checked by hash into the site's cache — and the
//! site's admin serves it to the site's phones under the site's own
//! provisioning URL; the target gone, the site's copy goes too. Skipped
//! without `FCP_TEST_DATABASE_URL`.

mod common;

use common::{admin, cloud_with_site_opts, CloudOptions, ADMIN_TOKEN};
use fcp_testkit::smoke::{free_port, loopback, toml_path};
use fcp_testkit::{bins, Service, TestDatabase};
use std::time::Duration;

const SITE_ADMIN_TOKEN: &str = "site-admin-token-firmware-0123456789";

async fn eventually(what: &str, check: impl Fn() -> bool) {
    for _ in 0..80 {
        if check() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    panic!("{what} never happened");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_targeted_image_reaches_the_site_and_its_phones() {
    let Some(site_db) = TestDatabase::create().await else {
        eprintln!("FCP_TEST_DATABASE_URL not set; skipping");
        return;
    };
    let dirs = tempfile::tempdir().unwrap();
    let cloud_firmware = dirs.path().join("cloud-firmware");
    let site_firmware = dirs.path().join("site-firmware");
    let cloud = cloud_with_site_opts(CloudOptions {
        site_database_url: Some(site_db.url()),
        cloud_firmware_dir: Some(toml_path(&cloud_firmware)),
        site_firmware_dir: Some(toml_path(&site_firmware)),
        ..Default::default()
    })
    .await
    .expect("the database was there a moment ago");

    // The platform administrator uploads an image to the cloud.
    let image_bytes: Vec<u8> = (0..300_000u32).map(|i| (i * 7 % 253) as u8).collect();
    let boundary = "fcp-site-firmware-boundary";
    let mut body = Vec::new();
    for (name, file, bytes) in [
        ("vendor", None, b"yealink".as_slice()),
        ("version", None, b"96.86.0.80".as_slice()),
        ("file", Some("T54W-96.86.0.80.rom"), image_bytes.as_slice()),
    ] {
        body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        match file {
            Some(f) => body.extend_from_slice(
                format!(
                    "Content-Disposition: form-data; name=\"{name}\"; filename=\"{f}\"\r\n\r\n"
                )
                .as_bytes(),
            ),
            None => body.extend_from_slice(
                format!("Content-Disposition: form-data; name=\"{name}\"\r\n\r\n").as_bytes(),
            ),
        }
        body.extend_from_slice(bytes);
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    let uploaded = reqwest::Client::new()
        .post(loopback(cloud.http, "/api/v1/firmware"))
        .bearer_auth(ADMIN_TOKEN)
        .header(
            "content-type",
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(body)
        .send()
        .await
        .unwrap();
    assert_eq!(uploaded.status().as_u16(), 201);
    let uploaded: serde_json::Value = uploaded.json().await.unwrap();
    let image = uploaded["id"].as_str().unwrap().to_string();
    let simple = image.trim_start_matches("fw-").replace('-', "");

    // The tenant targets it, at the cloud.
    let (code, target) = admin(
        cloud.http,
        "POST",
        "/api/v1/firmware/targets",
        Some(&cloud.tenant_id),
        Some(
            &serde_json::json!({"vendor": "yealink", "image_id": image, "stage": "all"})
                .to_string(),
        ),
    )
    .await;
    assert_eq!(code, 201, "{target}");
    let target_id = target["id"].as_str().unwrap().to_string();

    // The site's link fetches the file into the site's cache, whole.
    let cached = site_firmware.join(&simple).join("T54W-96.86.0.80.rom");
    eventually("the image reaching the site", || {
        std::fs::read(&cached).is_ok_and(|b| b == image_bytes)
    })
    .await;

    // The site's admin, on the site's database and the cache, serves it
    // under the site's own URL.
    let site_http = free_port();
    let site_admin_config = format!(
        r#"prompts_dir = "{dir}/site-prompts"

[service]
node_id = "acme-hq-admin"
default_tenant_id = "{tenant}"

[api]
bind_address = "127.0.0.1:{site_http}"
auth_enabled = true
auth_token = "{SITE_ADMIN_TOKEN}"
provisioning_require_https = false
managed_by = "http://127.0.0.1:{cloud_http}"

[api.firmware]
directory = "{firmware}"

[storage]
database_url = "{db}"

[logging]
level = "info"
format = "compact"
"#,
        dir = toml_path(cloud.dir.path()),
        tenant = cloud.tenant_id,
        cloud_http = cloud.http,
        firmware = toml_path(&site_firmware),
        db = site_db.url(),
    );
    let site_admin = Service::start_with(
        "fcp-admin (site)",
        bins::path("fcp-admin", "fcp-admin"),
        &site_admin_config,
        &loopback(site_http, "/api/v1/health"),
        &[],
    )
    .await;
    let client = reqwest::Client::new();
    let key: serde_json::Value = client
        .post(loopback(site_http, "/api/v1/provisioning/key"))
        .bearer_auth(SITE_ADMIN_TOKEN)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let key = key["key"].as_str().expect("the site's own key").to_string();
    let fetched = client
        .get(loopback(
            site_http,
            &format!("/provision/{key}/firmware/{simple}/T54W-96.86.0.80.rom"),
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(fetched.status().as_u16(), 200);
    assert_eq!(
        fetched.bytes().await.unwrap().as_ref(),
        image_bytes.as_slice()
    );

    // Firmware is the cloud's to change, not the site's.
    let refused = client
        .delete(loopback(
            site_http,
            &format!("/api/v1/firmware/targets/{target_id}"),
        ))
        .bearer_auth(SITE_ADMIN_TOKEN)
        .send()
        .await
        .unwrap();
    assert!(refused.status().is_client_error(), "{}", refused.status());

    // The target gone at the cloud, the site's copy goes.
    let (code, _) = admin(
        cloud.http,
        "DELETE",
        &format!("/api/v1/firmware/targets/{target_id}"),
        Some(&cloud.tenant_id),
        None,
    )
    .await;
    assert_eq!(code, 204);
    eventually("the site forgetting the image", || !cached.exists()).await;

    site_admin.stop().await;
    cloud.finish().await;
    site_db.drop_db().await;
}
