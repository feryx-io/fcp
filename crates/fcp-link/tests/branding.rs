//! A site's brand (`docs/BRANDING.md`, `docs/SOFTPHONE.md` phase F3): the
//! tenant's brand made in the cloud reaches the site — the brand synced
//! with what its assets are, the bytes fetched over the link and checked
//! by hash into the site's database — and the site's admin serves the
//! assets to the site's softphones; an asset the brand stops naming goes
//! from the site, and so does the brand. Skipped without
//! `FCP_TEST_DATABASE_URL`.

mod common;

use common::{admin, cloud_with_site_at, ADMIN_TOKEN};
use fcp_testkit::smoke::{free_port, loopback, toml_path};
use fcp_testkit::{bins, Service, TestDatabase};
use std::time::Duration;

const SITE_ADMIN_TOKEN: &str = "site-admin-token-branding-0123456789";

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_tenants_brand_reaches_the_site() {
    let Some(site_db) = TestDatabase::create().await else {
        eprintln!("FCP_TEST_DATABASE_URL not set; skipping");
        return;
    };
    let cloud = cloud_with_site_at(Some(site_db.url()))
        .await
        .expect("the database was there a moment ago");
    let client = reqwest::Client::new();

    // Two assets uploaded to the cloud, and a brand naming them.
    let upload = |bytes: Vec<u8>| {
        let client = client.clone();
        let url = loopback(cloud.http, "/api/v1/branding/assets");
        let tenant = cloud.tenant_id.clone();
        async move {
            let res = client
                .post(url)
                .bearer_auth(ADMIN_TOKEN)
                .header("x-fcp-tenant-id", tenant)
                .body(bytes)
                .send()
                .await
                .unwrap();
            assert_eq!(res.status().as_u16(), 201);
            let asset: serde_json::Value = res.json().await.unwrap();
            asset["sha256"].as_str().unwrap().to_string()
        }
    };
    let mut logo_bytes = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
    logo_bytes.extend_from_slice(&[0, 0, 1, 0, 0, 0, 1, 0, 8, 6, 0, 0, 0]);
    logo_bytes.extend((0..20_000u32).map(|i| (i % 251) as u8));
    let ring_bytes: Vec<u8> = b"OggS\0\x02"
        .iter()
        .copied()
        .chain((0..9_000u32).map(|i| (i % 7) as u8))
        .collect();
    let logo = upload(logo_bytes.clone()).await;
    let ring = upload(ring_bytes.clone()).await;
    let brand = |app_name: &str, ringtones: serde_json::Value| {
        serde_json::json!({
            "app_name": app_name,
            "colors": {"primary": "#123456"},
            "logo": logo,
            "ringtones": ringtones,
        })
        .to_string()
    };
    let (code, set) = admin(
        cloud.http,
        "PUT",
        "/api/v1/branding",
        Some(&cloud.tenant_id),
        Some(&brand(
            "Acme HQ",
            serde_json::json!([{"name": "Acme", "sha256": ring}]),
        )),
    )
    .await;
    assert_eq!(code, 200, "{set}");

    // The site's admin, on the site's database.
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
managed_by = "http://127.0.0.1:{cloud_http}"

[storage]
database_url = "{db}"

[logging]
level = "info"
format = "compact"
"#,
        dir = toml_path(cloud.dir.path()),
        tenant = cloud.tenant_id,
        cloud_http = cloud.http,
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
    let site_brand = || async {
        let res = client
            .get(loopback(site_http, "/api/v1/branding"))
            .bearer_auth(SITE_ADMIN_TOKEN)
            .send()
            .await
            .unwrap();
        let status = res.status().as_u16();
        (
            status,
            res.json::<serde_json::Value>().await.unwrap_or_default(),
        )
    };
    let site_asset = |hash: String| async move {
        let res = reqwest::get(loopback(site_http, &format!("/brand/v1/assets/{hash}")))
            .await
            .unwrap();
        (res.status().as_u16(), res.bytes().await.unwrap().to_vec())
    };
    let mut synced = serde_json::Value::Null;
    for _ in 0..80 {
        let (status, body) = site_brand().await;
        if status == 200 && body["app_name"] == "Acme HQ" {
            synced = body;
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    assert_eq!(
        synced["app_name"], "Acme HQ",
        "the brand never reached the site"
    );
    assert_eq!(synced["assets"].as_array().unwrap().len(), 2, "{synced}");
    // The site serves the bytes it fetched, unchanged.
    assert_eq!(site_asset(logo.clone()).await, (200, logo_bytes.clone()));
    assert_eq!(site_asset(ring.clone()).await, (200, ring_bytes.clone()));

    // The brand is the cloud's to change, not the site's.
    let refused = client
        .put(loopback(site_http, "/api/v1/branding"))
        .bearer_auth(SITE_ADMIN_TOKEN)
        .header("content-type", "application/json")
        .body(brand("Local", serde_json::json!([])))
        .send()
        .await
        .unwrap();
    assert_eq!(refused.status().as_u16(), 403);

    // The ringtone dropped at the cloud: it goes from the site.
    let (code, _) = admin(
        cloud.http,
        "PUT",
        "/api/v1/branding",
        Some(&cloud.tenant_id),
        Some(&brand("Acme HQ 2", serde_json::json!([]))),
    )
    .await;
    assert_eq!(code, 200);
    for _ in 0..80 {
        if site_brand().await.1["app_name"] == "Acme HQ 2" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    assert_eq!(
        site_asset(ring.clone()).await.0,
        404,
        "the dropped ringtone"
    );
    assert_eq!(site_asset(logo.clone()).await.0, 200, "the logo stays");

    // The brand gone at the cloud, gone at the site.
    let (code, _) = admin(
        cloud.http,
        "DELETE",
        "/api/v1/branding",
        Some(&cloud.tenant_id),
        None,
    )
    .await;
    assert_eq!(code, 204);
    for _ in 0..80 {
        if site_brand().await.0 == 404 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    assert_eq!(site_brand().await.0, 404, "the brand stayed at the site");
    assert_eq!(site_asset(logo).await.0, 404, "its logo stayed at the site");

    site_admin.stop().await;
    cloud.finish().await;
    site_db.drop_db().await;
}
