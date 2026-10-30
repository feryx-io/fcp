//! Parking at a site (`docs/PARKING.md` phase 4): a lot made in the cloud
//! reaches the site's database through the journal, its changes and its
//! removal too; a call parked at the site — a claim in the site's Redis,
//! as the site's call manager writes it — is carried up on the link's
//! heartbeat and shown on the cloud's board under the site.
//! Gated on `FCP_TEST_DATABASE_URL` and `FCP_TEST_REDIS_URL`.

mod common;

use common::{cloud_with_site_opts, eventually, CloudOptions};
use fcp_core::parking::{ParkingLot, ParkingLotStore};
use fcp_core::storage::{ClusterParkedCall, ClusterParkingStore};
use fcp_core::TenantId;
use fcp_storage_postgres::{PostgresConfig, PostgresStorage};
use fcp_testkit::cluster::private_redis_url;
use fcp_testkit::TestDatabase;
use std::time::Duration;

const SOON: Duration = Duration::from_secs(30);

async fn storage(url: &str) -> PostgresStorage {
    PostgresStorage::new(PostgresConfig {
        connection_string: url.to_string(),
        read_replica_url: None,
        max_replica_lag: Duration::from_secs(5),
        max_connections: 4,
        connection_timeout: Duration::from_secs(30),
        encryption: None,
    })
    .await
    .expect("the database")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_lot_reaches_its_site_and_the_sites_parked_calls_reach_the_cloud() {
    let (Some(site_db), Some(site_redis)) = (TestDatabase::create().await, private_redis_url())
    else {
        eprintln!("FCP_TEST_DATABASE_URL or FCP_TEST_REDIS_URL not set; skipping");
        return;
    };
    let cloud = cloud_with_site_opts(CloudOptions {
        site_database_url: Some(site_db.url()),
        site_redis_url: Some(site_redis.clone()),
        ..Default::default()
    })
    .await
    .expect("the database was there a moment ago");
    let tenant: TenantId = cloud.tenant_id.parse().unwrap();
    let cloud_db = storage(cloud.db.url()).await;
    let site_store = storage(site_db.url()).await;

    // Down: the cloud's lot, then its change, then its removal.
    let mut lot = ParkingLot::new(tenant, "Lobby", "4320", "4329");
    lot.park_number = Some("4300".into());
    cloud_db.put_parking_lot(&lot).await.unwrap();
    let at_site = |name: &'static str| {
        let (store, id) = (&site_store, lot.id.clone());
        async move {
            store
                .get_parking_lot(&tenant, &id)
                .await
                .unwrap()
                .is_some_and(|l| l.name == name)
        }
    };
    eventually("the lot reaching the site", SOON, || at_site("Lobby")).await;
    lot.name = "Front desk".into();
    cloud_db.put_parking_lot(&lot).await.unwrap();
    eventually("the lot's change reaching the site", SOON, || {
        at_site("Front desk")
    })
    .await;

    // Up: a call parked at the site, in its own Redis, on the cloud's board.
    let redis = fcp_storage_redis::RedisStorage::new(fcp_storage_redis::RedisConfig {
        url: site_redis,
        ..Default::default()
    })
    .await
    .unwrap();
    let parking = fcp_storage_redis::parking::RedisParkingStore::new(redis);
    let parked = ClusterParkedCall {
        call_id: "call-11111111-2222-3333-4444-555555555555".into(),
        node_id: "site-cm-1".into(),
        parker_extension: "1002".into(),
        tenant_id: tenant.to_string(),
        slot: "4321".into(),
        lot_id: lot.id.clone(),
        parked_at_unix: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs(),
    };
    assert!(parking
        .cluster_park(&tenant, "4321", parked, 300)
        .await
        .unwrap());
    eventually(
        "the site's parked call on the cloud's board",
        SOON,
        || async {
            let (code, body) = cloud
                .admin_call("GET", "/api/v1/parking/calls", true, None)
                .await;
            assert_eq!(code, 200, "{body}");
            body["data"]
                .as_array()
                .is_some_and(|calls| calls.iter().any(|c| c["slot"] == "4321"))
        },
    )
    .await;
    let (_, board) = cloud
        .admin_call("GET", "/api/v1/parking/calls", true, None)
        .await;
    let call = board["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["slot"] == "4321")
        .unwrap()
        .clone();
    assert_eq!(call["site_id"], cloud.site_id, "{call}");
    assert_eq!(call["site_name"], "acme-hq", "{call}");
    assert_eq!(call["parker_extension"], "1002");
    assert_eq!(call["timeout_secs"], 60, "the lot's own wait");

    // Taken back at the site: gone from the board.
    parking.cluster_unpark(&tenant, "4321").await.unwrap();
    eventually("the site's call off the cloud's board", SOON, || async {
        let (_, body) = cloud
            .admin_call("GET", "/api/v1/parking/calls", true, None)
            .await;
        body["data"]
            .as_array()
            .is_some_and(|calls| calls.iter().all(|c| c["slot"] != "4321"))
    })
    .await;

    // The lot removed in the cloud goes at the site.
    cloud_db.delete_parking_lot(&tenant, &lot.id).await.unwrap();
    eventually("the lot's removal reaching the site", SOON, || async {
        site_store
            .get_parking_lot(&tenant, &lot.id)
            .await
            .unwrap()
            .is_none()
    })
    .await;
    cloud.finish().await;
}
