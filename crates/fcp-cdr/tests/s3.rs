//! The S3 target against a real S3-compatible store: one PUT, a multipart
//! upload, confirmation, and a refusal. Skipped unless
//! `FCP_TEST_S3_ENDPOINT`, `FCP_TEST_S3_BUCKET`, `FCP_TEST_S3_ACCESS_KEY_ID`
//! and `FCP_TEST_S3_SECRET_ACCESS_KEY` are set (`FCP_TEST_S3_REGION`
//! defaults to `us-east-1`); the bucket is addressed path-style. Run locally
//! against Garage or MinIO.

use fcp_cdr::archive::{sha256_file, ArchiveTarget, ObjectInfo, S3Config, S3Target};

fn config() -> Option<S3Config> {
    let var = |name: &str| std::env::var(name).ok().filter(|v| !v.is_empty());
    let endpoint = var("FCP_TEST_S3_ENDPOINT")?;
    Some(S3Config {
        endpoint: Some(endpoint),
        region: var("FCP_TEST_S3_REGION").unwrap_or_else(|| "us-east-1".into()),
        path_style: true,
        access_key_id: Some(var("FCP_TEST_S3_ACCESS_KEY_ID")?),
        secret_access_key: Some(var("FCP_TEST_S3_SECRET_ACCESS_KEY")?),
        ..S3Config::new(var("FCP_TEST_S3_BUCKET")?)
    })
}

macro_rules! require_s3 {
    () => {
        match config() {
            Some(c) => c,
            None => {
                eprintln!("FCP_TEST_S3_* not set; skipping the S3 target test");
                return;
            }
        }
    };
}

async fn object(path: &std::path::Path, data: &[u8], records: u64) -> ObjectInfo {
    std::fs::write(path, data).unwrap();
    ObjectInfo {
        bytes: data.len() as u64,
        sha256: sha256_file(path).await.unwrap(),
        records,
    }
}

/// Bytes that do not compress, from a fixed seed.
fn noise(len: usize, mut seed: u64) -> Vec<u8> {
    (0..len)
        .map(|_| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed as u8
        })
        .collect()
}

#[tokio::test]
async fn puts_one_object_whole_and_confirms_it() {
    let config = require_s3!();
    let target = S3Target::new(config).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let key = format!("test/{}/2026/09/13.jsonl.gz", uuid::Uuid::new_v4());
    let file = dir.path().join("object");

    let first = object(&file, b"{\"a\":1}\n", 1).await;
    assert!(
        !target.confirm(&key, &first).await.unwrap(),
        "not there yet"
    );
    target.put(&key, &file, &first).await.unwrap();
    assert!(target.confirm(&key, &first).await.unwrap());

    // Same size, other bytes: not the object that is there.
    let other = object(&file, b"{\"b\":2}\n", 1).await;
    assert!(!target.confirm(&key, &other).await.unwrap());
    // Replaced, it is.
    target.put(&key, &file, &other).await.unwrap();
    assert!(target.confirm(&key, &other).await.unwrap());
    assert!(!target.confirm(&key, &first).await.unwrap());

    // A body that is not the one whose hash was signed is refused, and
    // the object there is left as it was.
    let lying = ObjectInfo {
        sha256: first.sha256.clone(),
        ..other.clone()
    };
    std::fs::write(&file, b"{\"c\":3}\n").unwrap();
    assert!(target.put(&key, &file, &lying).await.is_err());
    assert!(target.confirm(&key, &other).await.unwrap());
}

#[tokio::test]
async fn puts_a_large_object_in_parts() {
    let config = require_s3!();
    let target = S3Target::new(S3Config {
        multipart_threshold: 6 * 1024 * 1024,
        part_size: 5 * 1024 * 1024,
        ..config
    })
    .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let key = format!("test/{}/2026/09/14.jsonl.gz", uuid::Uuid::new_v4());
    let file = dir.path().join("object");

    // Three parts: two full, one short.
    let big = object(&file, &noise(11 * 1024 * 1024 + 123, 7), 42).await;
    target.put(&key, &file, &big).await.unwrap();
    assert!(target.confirm(&key, &big).await.unwrap());

    // Exactly two full parts.
    let even = object(&file, &noise(10 * 1024 * 1024, 11), 43).await;
    target.put(&key, &file, &even).await.unwrap();
    assert!(target.confirm(&key, &even).await.unwrap());
}

#[tokio::test]
async fn a_wrong_secret_is_refused_without_retrying_forever() {
    let config = require_s3!();
    let target = S3Target::new(S3Config {
        secret_access_key: Some("not-the-secret".into()),
        ..config
    })
    .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("object");
    let info = object(&file, b"x", 1).await;
    let started = std::time::Instant::now();
    let error = target.put("test/refused", &file, &info).await.unwrap_err();
    assert!(error.to_string().contains("403"), "{error}");
    assert!(started.elapsed() < std::time::Duration::from_secs(5));
}

#[tokio::test]
async fn reads_an_object_back_and_deletes_it() {
    let config = require_s3!();
    let target = S3Target::new(config).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let key = format!("test/{}/firmware.rom", uuid::Uuid::new_v4());
    let file = dir.path().join("object");
    let data = noise(300 * 1024, 7);
    let info = object(&file, &data, 0).await;

    assert!(target.get(&key).await.unwrap().is_none(), "not there yet");
    target.put(&key, &file, &info).await.unwrap();
    let response = target.get(&key).await.unwrap().expect("there now");
    assert_eq!(response.content_length(), Some(data.len() as u64));
    assert_eq!(response.bytes().await.unwrap().as_ref(), data.as_slice());

    target.delete(&key).await.unwrap();
    assert!(target.get(&key).await.unwrap().is_none(), "gone");
    // Deleting what is not there is no error.
    target.delete(&key).await.unwrap();
}

/// A small object put from memory (a stream's segment or playlist,
/// `docs/WEBINARS.md` W6.3c): served back with the content type and
/// `Cache-Control` it was put with, at the URL `object_url` names.
#[tokio::test]
async fn puts_bytes_with_their_content_type_and_cache_control() {
    let config = require_s3!();
    let target = S3Target::new(config).unwrap();
    let key = format!("test/{}/hls/seg-1.m4s", uuid::Uuid::new_v4());
    target
        .put_bytes(
            &key,
            b"segment".to_vec(),
            "video/mp4",
            Some("public, max-age=31536000, immutable"),
        )
        .await
        .unwrap();
    let got = target.get(&key).await.unwrap().expect("the object");
    assert_eq!(got.headers()["content-type"], "video/mp4");
    assert_eq!(
        got.headers()["cache-control"],
        "public, max-age=31536000, immutable"
    );
    assert_eq!(got.bytes().await.unwrap().as_ref(), b"segment");
    assert!(
        target.object_url(&key).ends_with(&key),
        "{}",
        target.object_url(&key)
    );
    target.delete(&key).await.unwrap();
    assert!(target.get(&key).await.unwrap().is_none());
}
