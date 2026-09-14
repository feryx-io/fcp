//! The record table at volume (`docs/CDR_PIPELINE.md` §13, phase 4): a
//! database built to migration 058, seeded with `FCP_VOLUME_ROWS` records
//! (default one million) in the unpartitioned table, converted by 059, and
//! then timed at what phase 4 is for — listing, deep pages, counts, the
//! export's walk, the rollup rebuilds, usage from rollups — and phase 5's
//! archive: where its walk starts, a day's tenants, a day written, and what
//! the late-record trigger costs an insert.
//!
//! Not part of the suite: `cargo test -p fcp-storage-postgres --test volume
//! -- --ignored --nocapture`, with `FCP_TEST_DATABASE_URL`. The database is
//! dropped at the end unless `FCP_VOLUME_KEEP` is set.

use chrono::Utc;
use fcp_core::storage::{CdrCursor, CdrQuery, CdrStore};
use fcp_core::TenantId;
use fcp_storage_postgres::{PostgresConfig, PostgresStorage};
use sqlx::{Connection, PgConnection};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

const TENANTS: i64 = 50;

fn swap_database(url: &str, name: &str) -> String {
    let (base, query) = match url.split_once('?') {
        Some((b, q)) => (b, Some(q)),
        None => (url, None),
    };
    let base = &base[..base.rfind('/').expect("a database in the URL")];
    match query {
        Some(q) => format!("{base}/{name}?{q}"),
        None => format!("{base}/{name}"),
    }
}

fn report(what: &str, took: Duration) {
    println!("{what:<58} {:>10.1} ms", took.as_secs_f64() * 1000.0);
}

#[tokio::test]
#[ignore = "a volume benchmark; run on purpose"]
async fn the_record_table_at_volume() {
    let Ok(admin_url) = std::env::var("FCP_TEST_DATABASE_URL") else {
        eprintln!("FCP_TEST_DATABASE_URL not set");
        return;
    };
    let rows: i64 = std::env::var("FCP_VOLUME_ROWS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1_000_000);
    let name = format!("fcp_volume_{}", uuid::Uuid::new_v4().simple());
    let mut admin = PgConnection::connect(&admin_url).await.unwrap();
    sqlx::query(&format!("CREATE DATABASE {name}"))
        .execute(&mut admin)
        .await
        .unwrap();
    let url = swap_database(&admin_url, &name);
    println!("database {name}, {rows} records over twelve months, {TENANTS} tenants");

    // 1. The schema as it was before phase 4.
    let pool = sqlx::PgPool::connect(&url).await.unwrap();
    let full = sqlx::migrate!("./migrations");
    let before: Vec<_> = full
        .migrations
        .iter()
        .filter(|m| m.version < 59)
        .cloned()
        .collect();
    let old = sqlx::migrate::Migrator {
        migrations: std::borrow::Cow::Owned(before),
        ignore_missing: false,
        locking: true,
        no_tx: false,
    };
    old.run(&pool).await.unwrap();

    // 2. Records: twelve months back from now, fifty tenants, one in ten a
    // queue or conference record, one in three answered with a cost. The
    // old table's secondary indexes are dropped for the load (the
    // conversion scans the table and never uses them); its key stays.
    for index in [
        "idx_cdrs_tenant",
        "idx_cdrs_start_time",
        "idx_cdrs_from_uri",
        "idx_cdrs_to_uri",
        "idx_cdrs_disposition",
        "idx_cdrs_sip_call_id",
        "idx_cdrs_custom_data",
        "idx_cdrs_call_record",
        "idx_cdrs_call_id",
        "idx_cdrs_tenant_start",
    ] {
        sqlx::query(&format!("DROP INDEX IF EXISTS {index}"))
            .execute(&pool)
            .await
            .unwrap();
    }
    let t = Instant::now();
    sqlx::query(
        "INSERT INTO cdrs (
             cdr_id, call_id, tenant_id, sip_call_id, from_uri, to_uri, start_time,
             answer_time, end_time, setup_duration_ms, call_duration_ms, total_duration_ms,
             disposition, hangup_cause, media_anchored, kind, direction, cost, currency, created_at)
         SELECT gen_random_uuid(),
                'call-' || gen_random_uuid(),
                'tenant-00000000-0000-0000-0000-' || lpad((g % $2)::text, 12, '0'),
                g || '@volume',
                'sip:' || (1000 + g % 500) || '@volume.example',
                'sip:+1415555' || lpad((g % 10000)::text, 4, '0') || '@carrier.example',
                ts,
                CASE WHEN g % 3 = 0 THEN ts + interval '3 seconds' END,
                ts + interval '90 seconds',
                CASE WHEN g % 3 = 0 THEN 3000 END,
                CASE WHEN g % 3 = 0 THEN 87000 END,
                90000,
                CASE WHEN g % 3 = 0 THEN 'answered' ELSE 'no_answer' END,
                'normal_clearing',
                true,
                CASE WHEN g % 10 = 0 THEN 'queue' WHEN g % 10 = 1 THEN 'conference' ELSE 'call' END,
                CASE WHEN g % 2 = 0 THEN 'outbound' ELSE 'inbound' END,
                CASE WHEN g % 3 = 0 THEN 0.0300 END,
                CASE WHEN g % 3 = 0 THEN 'USD' END,
                now()
         FROM (SELECT g, now() - (g::double precision / $1 * interval '365 days') AS ts
               FROM generate_series(1, $1) g) s",
    )
    .bind(rows)
    .bind(TENANTS)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("ANALYZE cdrs").execute(&pool).await.unwrap();
    report("seed the unpartitioned table", t.elapsed());
    let size: String = sqlx::query_scalar("SELECT pg_size_pretty(pg_total_relation_size('cdrs'))")
        .fetch_one(&pool)
        .await
        .unwrap();
    println!("unpartitioned table with its indexes: {size}");

    // 3. The conversion, and the rollup tables.
    let t = Instant::now();
    full.run(&pool).await.unwrap();
    report(
        "migrations 059 and 060 (the partition conversion)",
        t.elapsed(),
    );
    sqlx::query("ANALYZE cdrs").execute(&pool).await.unwrap();
    let size: String = sqlx::query_scalar(
        "SELECT pg_size_pretty(sum(pg_total_relation_size(inhrelid)))
         FROM pg_inherits WHERE inhparent = 'cdrs'::regclass",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    println!("partitioned table with its indexes: {size}");
    pool.close().await;

    let s = Arc::new(
        PostgresStorage::new(PostgresConfig {
            connection_string: url.clone(),
            max_connections: 4,
            connection_timeout: Duration::from_secs(30),
            encryption: None,
            read_replica_url: None,
            max_replica_lag: Duration::from_secs(5),
        })
        .await
        .unwrap(),
    );
    let tenant: TenantId = "00000000-0000-0000-0000-000000000007".parse().unwrap();
    let per_tenant = CdrQuery {
        tenant_id: Some(tenant),
        limit: Some(100),
        ..Default::default()
    };

    // 4. Listing: the first page, a page reached by walking ten deep with
    // the cursor, and the deepest offset the API allows.
    let t = Instant::now();
    let first = s.query_cdrs(per_tenant.clone()).await.unwrap();
    report("first page of a tenant's listing (100)", t.elapsed());
    let mut cursor = first.last().map(CdrCursor::after);
    let t = Instant::now();
    for _ in 0..10 {
        let page = s
            .query_cdrs(CdrQuery {
                cursor,
                ..per_tenant.clone()
            })
            .await
            .unwrap();
        cursor = page.last().map(CdrCursor::after);
    }
    report("ten further pages by cursor, each", t.elapsed() / 10);
    let t = Instant::now();
    s.query_cdrs(CdrQuery {
        offset: Some(10_000),
        ..per_tenant.clone()
    })
    .await
    .unwrap();
    report("a page at offset 10,000", t.elapsed());

    // 5. Counts: a tenant's, and the platform's, under the listing's budget.
    for (what, query) in [
        (
            "count, one tenant (2 s budget)",
            CdrQuery {
                tenant_id: Some(tenant),
                ..Default::default()
            },
        ),
        ("count, every tenant (2 s budget)", CdrQuery::default()),
        (
            "count, every tenant, one filter (2 s budget)",
            CdrQuery {
                to_uri: Some("5550042".into()),
                ..Default::default()
            },
        ),
    ] {
        let t = Instant::now();
        let count = s
            .count_cdrs_within(query, Duration::from_secs(2))
            .await
            .unwrap();
        report(
            &format!(
                "{what}: {} {}",
                count.count,
                if count.exact { "exact" } else { "estimated" }
            ),
            t.elapsed(),
        );
    }

    // 6. The export's walk: a tenant's whole history in pages of 1,000.
    let t = Instant::now();
    let mut walked = 0usize;
    let mut cursor = None;
    loop {
        let page = s
            .query_cdrs(CdrQuery {
                tenant_id: Some(tenant),
                limit: Some(1000),
                cursor,
                ..Default::default()
            })
            .await
            .unwrap();
        if page.is_empty() {
            break;
        }
        walked += page.len();
        cursor = page.last().map(CdrCursor::after);
        if page.len() < 1000 {
            break;
        }
    }
    let took = t.elapsed();
    report(
        &format!(
            "export walk, one tenant: {walked} records, {:.0} records/s",
            walked as f64 / took.as_secs_f64()
        ),
        took,
    );

    // 7. Rollups: the first rebuild (all of history), then the ten-minute
    // one over 48 hours; usage for a month from rollups and from records.
    let t = Instant::now();
    let run = s
        .rebuild_usage_rollups(Duration::from_secs(48 * 3600))
        .await
        .unwrap();
    report(
        &format!(
            "first rollup rebuild: {} hours, {} rows",
            run.hours, run.rows
        ),
        t.elapsed(),
    );
    let t = Instant::now();
    let run = s
        .rebuild_usage_rollups(Duration::from_secs(48 * 3600))
        .await
        .unwrap();
    report(
        &format!("rollup rebuild, 48 h window: {} hours", run.hours),
        t.elapsed(),
    );
    let month_ago = SystemTime::now() - Duration::from_secs(30 * 86_400);
    let t = Instant::now();
    let rolled = s
        .aggregate_usage(&tenant, month_ago, SystemTime::now())
        .await
        .unwrap();
    report(
        &format!(
            "usage for 30 days from rollups: {} calls",
            rolled.total_calls
        ),
        t.elapsed(),
    );
    sqlx::query("DELETE FROM cdr_rollup_state")
        .execute(s.pool())
        .await
        .unwrap();
    let t = Instant::now();
    let live = s
        .aggregate_usage(&tenant, month_ago, SystemTime::now())
        .await
        .unwrap();
    report(
        &format!("usage for 30 days from records: {} calls", live.total_calls),
        t.elapsed(),
    );
    assert_eq!(rolled.total_calls, live.total_calls);
    assert_eq!(rolled.total_cost, live.total_cost);

    // 8. The archive: where the walk starts, the tenants on a day, and
    // every tenant's records for that day written to a directory.
    use fcp_cdr::archive::{ArchiveConfig, Archiver, DirectoryTarget};
    let t = Instant::now();
    let oldest = s.cdr_archive_next_day().await.unwrap().expect("records");
    report(
        &format!("archive: the oldest record's day ({oldest})"),
        t.elapsed(),
    );
    let day = (Utc::now() - chrono::Duration::days(30)).date_naive();
    let t = Instant::now();
    let tenants = s.cdr_tenants_on(day).await.unwrap();
    report(
        &format!(
            "archive: the tenants with records on a day ({})",
            tenants.len()
        ),
        t.elapsed(),
    );
    let dir = tempfile::tempdir().unwrap();
    let work = dir.path().join("work");
    std::fs::create_dir_all(&work).unwrap();
    let archiver = Archiver::new(
        s.clone(),
        Arc::new(DirectoryTarget::new(dir.path().join("archive"))),
        ArchiveConfig {
            work_dir: work,
            ..ArchiveConfig::default()
        },
    );
    let t = Instant::now();
    let (mut records, mut bytes) = (0u64, 0u64);
    for tenant in &tenants {
        let generation = s.begin_cdr_archive(tenant, day).await.unwrap();
        let written = archiver.write(tenant, day, generation).await.unwrap();
        records += written.records;
        bytes += written.bytes;
    }
    let took = t.elapsed();
    report(
        &format!(
            "archive: one day, every tenant: {records} records, {} KiB, {:.0} records/s",
            bytes / 1024,
            records as f64 / took.as_secs_f64()
        ),
        took,
    );

    // What the late-record trigger costs the processor's batched insert,
    // disabled and then enabled with the walk under way (the batches are
    // today's records, so none is late).
    use fcp_core::storage::CdrKind;
    let batch = |n: usize| -> Vec<_> {
        (0..n)
            .map(|_| {
                fcp_cdr::RecordBuilder::new(CdrKind::Call, fcp_core::CallId::new(), tenant)
                    .from("sip:1000@volume.example", None)
                    .to("sip:+14155550100@carrier.example", None)
                    .started_at(SystemTime::now() - Duration::from_secs(60))
                    .ended_at(Some(SystemTime::now()))
                    .build()
            })
            .collect()
    };
    // Alternated, twice each, so a cold first batch does not decide it.
    for round in 1..=2 {
        for (state, sql) in [
            (
                "no trigger",
                "ALTER TABLE cdrs DISABLE TRIGGER cdrs_archive_note_late",
            ),
            (
                "trigger on",
                "ALTER TABLE cdrs ENABLE TRIGGER cdrs_archive_note_late",
            ),
        ] {
            sqlx::query(sql).execute(s.pool()).await.unwrap();
            let t = Instant::now();
            for _ in 0..40 {
                s.insert_cdrs(batch(500)).await.unwrap();
            }
            report(
                &format!("insert 20,000 records in batches of 500, {state} ({round})"),
                t.elapsed(),
            );
        }
    }
    println!("finished {}", Utc::now());

    drop(s);
    if std::env::var("FCP_VOLUME_KEEP").is_err() {
        sqlx::query(&format!("DROP DATABASE {name} WITH (FORCE)"))
            .execute(&mut admin)
            .await
            .unwrap();
    }
}
