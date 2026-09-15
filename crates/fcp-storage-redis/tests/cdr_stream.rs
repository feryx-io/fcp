//! The CDR stream against a real Redis (`docs/CDR_PIPELINE.md` §4.2).
//! Gated on `FCP_TEST_REDIS_URL`; skipped with a notice when unset. Each
//! test works under a key prefix of its own.
#![cfg(feature = "cdr")]

use fcp_cdr::consumer::RecordSource;
use fcp_cdr::sink::Sink;
use fcp_cdr::{CdrKind, RecordBuilder};
use fcp_core::storage::CallDetailRecord;
use fcp_core::{CallId, TenantId};
use fcp_storage_redis::cdr_stream::RedisCdrStream;
use fcp_storage_redis::{RedisConfig, RedisStorage};
use std::time::Duration;

async fn storage() -> Option<RedisStorage> {
    let Ok(url) = std::env::var("FCP_TEST_REDIS_URL") else {
        eprintln!("FCP_TEST_REDIS_URL not set; skipping the CDR stream test");
        return None;
    };
    Some(
        RedisStorage::new(RedisConfig {
            url,
            key_prefix: format!("test:{}:", uuid::Uuid::new_v4().simple()),
        })
        .await
        .expect("connect to redis"),
    )
}

fn record() -> CallDetailRecord {
    RecordBuilder::new(CdrKind::Call, CallId::new(), TenantId::new())
        .from("sip:alice@x", None)
        .to("sip:bob@x", None)
        .build()
}

#[tokio::test]
async fn records_go_round_the_stream_and_a_dead_consumers_entries_are_claimed() {
    let Some(storage) = storage().await else {
        return;
    };
    let node = RedisCdrStream::new(storage.clone(), "node", Duration::from_secs(3600));
    let first = RedisCdrStream::new(storage.clone(), "jobs-1", Duration::from_secs(3600));
    let second = RedisCdrStream::new(storage.clone(), "jobs-2", Duration::from_secs(3600));
    first.prepare().await.unwrap();
    second.prepare().await.unwrap(); // idempotent

    let records: Vec<_> = (0..3).map(|_| record()).collect();
    node.ship(records.clone()).await.unwrap();
    assert_eq!(node.appended(), 3);
    // Something that is not a record, as a buggy writer would leave.
    let mut conn = storage.connection();
    let _: String = redis::cmd("XADD")
        .arg(node.stream_key())
        .arg("*")
        .arg("record")
        .arg("{not json")
        .query_async(&mut conn)
        .await
        .unwrap();

    // The first consumer reads everything and dies without acknowledging.
    let read = first.read(10, Duration::from_millis(100)).await.unwrap();
    assert_eq!(read.len(), 4);
    let ids: Vec<_> = read
        .iter()
        .filter_map(|e| e.record.as_ref().ok().map(|r| r.cdr_id))
        .collect();
    assert_eq!(ids, records.iter().map(|r| r.cdr_id).collect::<Vec<_>>());
    let bad = read[3].record.as_ref().unwrap_err();
    assert_eq!(bad.raw, "{not json");
    assert_eq!(first.status().await.unwrap().pending, 4);

    // Nothing is left for a new reader; the second consumer claims them
    // once they have been idle long enough.
    assert!(second
        .read(10, Duration::from_millis(50))
        .await
        .unwrap()
        .is_empty());
    assert!(second
        .claim_stale(Duration::from_secs(60), 10)
        .await
        .unwrap()
        .is_empty());
    tokio::time::sleep(Duration::from_millis(120)).await;
    let claimed = second
        .claim_stale(Duration::from_millis(100), 10)
        .await
        .unwrap();
    assert_eq!(claimed.len(), 4);

    // The bad one is kept, and all four are done.
    second
        .dead_letter(&claimed[3].id, &bad.raw, &bad.reason)
        .await
        .unwrap();
    let all: Vec<String> = claimed.iter().map(|e| e.id.clone()).collect();
    second.ack(&all).await.unwrap();
    let status = second.status().await.unwrap();
    assert_eq!(
        (status.length, status.pending, status.dead_letters),
        (4, 0, 1)
    );
}

#[tokio::test]
async fn trimming_drops_only_finished_entries_older_than_the_window() {
    let Some(storage) = storage().await else {
        return;
    };
    let node = RedisCdrStream::new(storage.clone(), "node", Duration::ZERO);
    let jobs = RedisCdrStream::new(storage.clone(), "jobs", Duration::ZERO);
    // No group yet: nothing is trimmed, whatever its age.
    node.ship(vec![record(), record()]).await.unwrap();
    assert_eq!(jobs.trim().await.unwrap(), 0);

    jobs.prepare().await.unwrap();
    let read = jobs.read(10, Duration::from_millis(100)).await.unwrap();
    assert_eq!(read.len(), 2);
    node.ship(vec![record()]).await.unwrap(); // not yet delivered

    // One acknowledged, one pending, one undelivered: only entries before
    // the pending one may go, and that is none.
    jobs.ack(&[read[0].id.clone()]).await.unwrap();
    tokio::time::sleep(Duration::from_millis(5)).await;
    // The acknowledged first entry is older than the pending second one.
    assert_eq!(jobs.trim().await.unwrap(), 1);
    assert_eq!(jobs.status().await.unwrap().length, 2);

    // With the second acknowledged, the group's last delivered entry is the
    // boundary: the undelivered third stays.
    jobs.ack(&[read[1].id.clone()]).await.unwrap();
    jobs.trim().await.unwrap();
    let status = jobs.status().await.unwrap();
    assert_eq!(
        status.length, 2,
        "the last delivered and the undelivered stay"
    );
    let rest = jobs.read(10, Duration::from_millis(100)).await.unwrap();
    assert_eq!(
        rest.len(),
        1,
        "the undelivered record is still there to read"
    );

    // A long window keeps everything.
    let patient = RedisCdrStream::new(storage.clone(), "jobs", Duration::from_secs(3600));
    jobs.ack(&[rest[0].id.clone()]).await.unwrap();
    assert_eq!(patient.trim().await.unwrap(), 0);
}

#[tokio::test]
async fn dead_letters_are_read_in_pages_and_deleted_by_id() {
    use fcp_core::subject_erasure::DeadLetterStore;

    let Some(storage) = storage().await else {
        return;
    };
    let stream = RedisCdrStream::new(storage, "jobs", Duration::from_secs(3600));
    let records: Vec<_> = (0..5).map(|_| record()).collect();
    for (n, r) in records.iter().enumerate() {
        let raw = serde_json::to_string(r).unwrap();
        stream
            .dead_letter(&format!("{n}-0"), &raw, "refused")
            .await
            .unwrap();
    }
    let (first, next) = stream.dead_letter_page(None, 3).await.unwrap();
    assert_eq!(first.len(), 3);
    let next = next.expect("a second page");
    let (second, last) = stream.dead_letter_page(Some(&next), 3).await.unwrap();
    assert_eq!(second.len(), 2);
    assert!(last.is_none());
    let found: CallDetailRecord = serde_json::from_str(&first[0].1).unwrap();
    assert_eq!(found.cdr_id, records[0].cdr_id);

    let ids: Vec<String> = first.iter().map(|(id, _)| id.clone()).collect();
    assert_eq!(stream.delete_dead_letters(&ids).await.unwrap(), 3);
    let (left, _) = stream.dead_letter_page(None, 10).await.unwrap();
    assert_eq!(left.len(), 2);
    assert_eq!(stream.status().await.unwrap().dead_letters, 2);
}
