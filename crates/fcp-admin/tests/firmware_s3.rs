//! Firmware images in an S3-compatible bucket (`[api.firmware.s3]`,
//! `docs/PROVISIONING.md` §11): the admin binary uploads an image into the
//! bucket, serves it to a phone under the tenant's URL from there, and
//! removes it. Skipped unless `FCP_TEST_DATABASE_URL` and the S3 test
//! store (`FCP_TEST_S3_ENDPOINT`, `FCP_TEST_S3_BUCKET`,
//! `FCP_TEST_S3_ACCESS_KEY_ID`, `FCP_TEST_S3_SECRET_ACCESS_KEY`,
//! `FCP_TEST_S3_REGION`) are set; run locally against Garage.

use fcp_testkit::smoke::{self, free_port, loopback};
use fcp_testkit::{Service, TestDatabase};
use serde_json::{json, Value};

const BIN: &str = env!("CARGO_BIN_EXE_fcp-admin");
const TOKEN: &str = "firmware-s3-static-token-0123456789abcdef";

fn var(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

fn multipart(parts: &[(&str, Option<&str>, &[u8])]) -> (String, Vec<u8>) {
    const BOUNDARY: &str = "fcp-firmware-s3-boundary";
    let mut body = Vec::new();
    for (name, file, bytes) in parts {
        body.extend_from_slice(format!("--{BOUNDARY}\r\n").as_bytes());
        match file {
            Some(file) => body.extend_from_slice(
                format!(
                    "Content-Disposition: form-data; name=\"{name}\"; filename=\"{file}\"\r\n\
                     Content-Type: application/octet-stream\r\n\r\n"
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
    body.extend_from_slice(format!("--{BOUNDARY}--\r\n").as_bytes());
    (format!("multipart/form-data; boundary={BOUNDARY}"), body)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_image_lives_in_the_bucket_and_is_served_from_it() {
    let (Some(endpoint), Some(bucket), Some(key_id), Some(secret)) = (
        var("FCP_TEST_S3_ENDPOINT"),
        var("FCP_TEST_S3_BUCKET"),
        var("FCP_TEST_S3_ACCESS_KEY_ID"),
        var("FCP_TEST_S3_SECRET_ACCESS_KEY"),
    ) else {
        eprintln!("FCP_TEST_S3_* not set; skipping");
        return;
    };
    let region = var("FCP_TEST_S3_REGION").unwrap_or_else(|| "us-east-1".into());
    let Some(db) = TestDatabase::create().await else {
        eprintln!("FCP_TEST_DATABASE_URL not set; skipping");
        return;
    };
    let dir = smoke::temp_dir();
    let http = free_port();
    let prefix = format!("test-firmware/{}/", uuid::Uuid::new_v4().simple());
    let config = format!(
        r#"prompts_dir = "{dir}/prompts"

[service]
node_id = "firmware-s3-admin"

[api]
bind_address = "127.0.0.1:{http}"
auth_enabled = true
auth_token = "{TOKEN}"
provisioning_require_https = false

[api.firmware.s3]
bucket = "{bucket}"
region = "{region}"
endpoint = "{endpoint}"
path_style = true
access_key_id = "{key_id}"
secret_access_key = "{secret}"
prefix = "{prefix}"

[storage]
database_url = "{db}"

[logging]
level = "info"
format = "compact"
"#,
        db = db.url(),
        dir = smoke::toml_path(dir.path()),
    );
    let admin = Service::start("admin", BIN, &config, &loopback(http, "/api/v1/health")).await;
    let client = reqwest::Client::new();
    let api = |method: reqwest::Method, path: &str, body: Option<Value>| {
        let mut request = client
            .request(method, loopback(http, path))
            .bearer_auth(TOKEN);
        if let Some(body) = body {
            request = request.json(&body);
        }
        async move {
            let response = request.send().await.expect("the admin answers");
            let status = response.status().as_u16();
            let text = response.text().await.unwrap();
            (
                status,
                serde_json::from_str::<Value>(&text).unwrap_or(Value::Null),
                text,
            )
        }
    };

    // Uploaded: into the bucket, under the prefix.
    let image_bytes: Vec<u8> = (0..400_000u32).map(|i| (i % 251) as u8).collect();
    let (content_type, body) = multipart(&[
        ("vendor", None, b"yealink"),
        ("version", None, b"96.86.0.80"),
        ("models", None, b"T5*"),
        ("file", Some("T54W-96.86.0.80.rom"), &image_bytes),
    ]);
    let made = client
        .post(loopback(http, "/api/v1/firmware"))
        .bearer_auth(TOKEN)
        .header("content-type", content_type)
        .body(body)
        .send()
        .await
        .unwrap();
    let status = made.status().as_u16();
    let made: Value = made.json().await.unwrap();
    assert_eq!(status, 201, "{made}");
    let image = made["id"].as_str().unwrap().to_string();
    let simple = image.trim_start_matches("fw-").replace('-', "");
    let bucket_target = fcp_cdr::archive::S3Target::new(fcp_cdr::archive::S3Config {
        region: region.clone(),
        endpoint: Some(endpoint.clone()),
        path_style: true,
        access_key_id: Some(key_id.clone()),
        secret_access_key: Some(secret.clone()),
        ..fcp_cdr::archive::S3Config::new(bucket.clone())
    })
    .unwrap();
    let object = format!("{prefix}{simple}/T54W-96.86.0.80.rom");
    let stored = bucket_target
        .get(&object)
        .await
        .unwrap()
        .expect("in the bucket");
    assert_eq!(
        stored.bytes().await.unwrap().as_ref(),
        image_bytes.as_slice()
    );

    // Served to a phone of a tenant that targets it, from the bucket.
    let (status, _, text) = api(
        reqwest::Method::POST,
        "/api/v1/firmware/targets",
        Some(json!({"vendor": "yealink", "image_id": image, "stage": "all"})),
    )
    .await;
    assert_eq!(status, 201, "{text}");
    let target = serde_json::from_str::<Value>(&text).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let (_, key, _) = api(reqwest::Method::POST, "/api/v1/provisioning/key", None).await;
    let key = key["key"].as_str().unwrap().to_string();
    let fetched = client
        .get(loopback(
            http,
            &format!("/provision/{key}/firmware/{simple}/T54W-96.86.0.80.rom"),
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(fetched.status().as_u16(), 200);
    assert_eq!(fetched.content_length(), Some(image_bytes.len() as u64));
    assert_eq!(
        fetched.bytes().await.unwrap().as_ref(),
        image_bytes.as_slice()
    );

    // Removed: from the bucket too.
    let (status, _, _) = api(
        reqwest::Method::DELETE,
        &format!("/api/v1/firmware/targets/{target}"),
        None,
    )
    .await;
    assert_eq!(status, 204);
    let (status, _, text) = api(
        reqwest::Method::DELETE,
        &format!("/api/v1/firmware/{image}"),
        None,
    )
    .await;
    assert_eq!(status, 204, "{text}");
    assert!(
        bucket_target.get(&object).await.unwrap().is_none(),
        "gone from the bucket"
    );

    admin.stop().await;
    db.drop_db().await;
}
