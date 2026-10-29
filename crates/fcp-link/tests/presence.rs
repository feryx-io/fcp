//! Presence a person sets, at a site (`docs/PRESENCE.md` §3): synced both
//! ways as contacts are. A status chosen or made in the cloud reaches the
//! site through the journal; one chosen or made at the site reaches the
//! cloud through the link's shipping. Each side takes a copy only when it
//! is newer, so a change's echo is let go. A deleted status crosses as a
//! tombstone and a cleared one as a plain `available` row; a tombstone the
//! cloud sweeps goes at the site too.
//! Gated on `FCP_TEST_DATABASE_URL`; skipped with a notice when unset.

mod common;

use common::{cloud_with_site_opts, eventually, CloudOptions};
use fcp_core::presence_status::{ChosenStatus, OwnStatus, PresenceStatusStore, StatusState};
use fcp_core::storage::{CredentialStore, User};
use fcp_core::{TenantId, UserId};
use fcp_storage_postgres::{PostgresConfig, PostgresStorage};
use fcp_testkit::TestDatabase;
use std::time::{Duration, SystemTime};

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

fn chosen(tenant: TenantId, user: UserId, status: &str, at: SystemTime) -> ChosenStatus {
    ChosenStatus {
        tenant_id: tenant,
        user_id: user,
        status: status.to_string(),
        note: Some(format!("{status} note")),
        until: None,
        updated_at: at,
    }
}

/// The status a side says the user chose, as its name.
async fn chosen_on(store: &PostgresStorage, user: &UserId) -> Option<String> {
    store
        .get_chosen_status(user)
        .await
        .unwrap()
        .map(|c| c.status)
}

/// How many times the cloud journalled the row: what it sent down.
async fn journalled(cloud: &PostgresStorage, table: &str, id: &str) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM sync_journal WHERE table_name = $1 AND row_id = $2")
        .bind(table)
        .bind(id)
        .fetch_one(cloud.pool())
        .await
        .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn presence_a_person_sets_is_synced_both_ways_with_their_site() {
    let Some(site_db) = TestDatabase::create().await else {
        eprintln!("FCP_TEST_DATABASE_URL not set; skipping the presence test");
        return;
    };
    let cloud = cloud_with_site_opts(CloudOptions {
        site_database_url: Some(site_db.url()),
        ..Default::default()
    })
    .await
    .expect("the database was there a moment ago");
    let tenant: TenantId = cloud.tenant_id.parse().unwrap();
    let cloud_db = storage(cloud.db.url()).await;
    let site_db_store = storage(site_db.url()).await;

    // Alice, made in the cloud and synced to her site.
    let now = SystemTime::now();
    let epoch = now
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let alice: User = serde_json::from_value(serde_json::json!({
        "username": "alice", "password": "Alice-pw-1", "tenant_id": tenant,
        "enabled": true, "created_at": epoch, "updated_at": epoch,
    }))
    .unwrap();
    let user = alice.id;
    cloud_db.put_user(alice).await.unwrap();
    eventually("Alice reaching the site", SOON, || async {
        site_db_store.get_user_by_id(&user).await.unwrap().is_some()
    })
    .await;

    // Down: a status of her own made in the cloud, and chosen there,
    // reach the site.
    let wfh = OwnStatus {
        id: fcp_core::presence_status::new_status_id(),
        tenant_id: tenant,
        user_id: user,
        label: "WFH".into(),
        state: StatusState::Available,
        position: 0,
        updated_at: now,
        deleted_at: None,
    };
    cloud_db.put_own_status(&wfh).await.unwrap();
    cloud_db
        .put_chosen_status(&chosen(tenant, user, &wfh.id, now))
        .await
        .unwrap();
    eventually("the cloud's status reaching the site", SOON, || async {
        site_db_store
            .list_own_statuses(&user)
            .await
            .unwrap()
            .iter()
            .any(|s| s.id == wfh.id && s.label == "WFH")
            && chosen_on(&site_db_store, &user).await.as_deref() == Some(wfh.id.as_str())
    })
    .await;

    // Up: a status chosen at the site, and one made there, reach the
    // cloud.
    let later = now + Duration::from_secs(60);
    site_db_store
        .put_chosen_status(&chosen(tenant, user, "busy", later))
        .await
        .unwrap();
    let focus = OwnStatus {
        id: fcp_core::presence_status::new_status_id(),
        label: "Focus".into(),
        state: StatusState::Dnd,
        position: 1,
        updated_at: later,
        ..wfh.clone()
    };
    site_db_store.put_own_status(&focus).await.unwrap();
    eventually("the site's status reaching the cloud", SOON, || async {
        chosen_on(&cloud_db, &user).await.as_deref() == Some("busy")
            && cloud_db
                .list_own_statuses(&user)
                .await
                .unwrap()
                .iter()
                .any(|s| s.id == focus.id && s.label == "Focus")
    })
    .await;

    // No echo: the cloud journalled the chosen status twice (its own
    // choice, then the site's) and each own status once; the site let
    // each copy back from the cloud go (no newer), so nothing came back
    // up.
    tokio::time::sleep(Duration::from_secs(5)).await;
    let user_row = user.to_string();
    assert_eq!(journalled(&cloud_db, "user_presence", &user_row).await, 2);
    assert_eq!(journalled(&cloud_db, "presence_statuses", &wfh.id).await, 1);
    assert_eq!(
        journalled(&cloud_db, "presence_statuses", &focus.id).await,
        1
    );

    // An older copy is let go on either side.
    assert!(!cloud_db
        .put_chosen_status_if_newer(&chosen(tenant, user, "away", now))
        .await
        .unwrap());
    assert!(!site_db_store
        .put_chosen_status_if_newer(&chosen(tenant, user, "away", now))
        .await
        .unwrap());
    let mut stale = wfh.clone();
    stale.label = "Stale".into();
    assert!(!site_db_store.put_own_status_if_newer(&stale).await.unwrap());
    assert_eq!(chosen_on(&cloud_db, &user).await.as_deref(), Some("busy"));

    // A status deleted at the site crosses as a tombstone.
    assert!(site_db_store
        .delete_own_status(&user, &focus.id)
        .await
        .unwrap());
    eventually("the site's delete reaching the cloud", SOON, || async {
        cloud_db
            .get_own_status(&focus.id)
            .await
            .unwrap()
            .is_some_and(|s| s.deleted_at.is_some())
    })
    .await;
    assert!(cloud_db
        .list_own_statuses(&user)
        .await
        .unwrap()
        .iter()
        .all(|s| s.id != focus.id));

    // A status cleared in the cloud is a plain `available` row, and
    // crosses: the site reads no status.
    cloud_db.delete_chosen_status(&user).await.unwrap();
    assert_eq!(chosen_on(&cloud_db, &user).await, None);
    eventually("the cloud's clearing reaching the site", SOON, || async {
        chosen_on(&site_db_store, &user).await.is_none()
            && site_db_store
                .get_chosen_status_row(&user)
                .await
                .unwrap()
                .is_some_and(|c| c.is_plain_available())
    })
    .await;

    // The cloud sweeps the tombstone; the row goes at the site too, and a
    // late echo of the tombstone brings nothing back.
    let tombstone = cloud_db.get_own_status(&focus.id).await.unwrap().unwrap();
    assert_eq!(
        cloud_db
            .sweep_status_tombstones(SystemTime::now() + Duration::from_secs(10))
            .await
            .unwrap(),
        1
    );
    eventually("the swept row going at the site", SOON, || async {
        site_db_store
            .get_own_status(&focus.id)
            .await
            .unwrap()
            .is_none()
    })
    .await;
    assert!(!cloud_db.put_own_status_if_newer(&tombstone).await.unwrap());
    assert!(cloud_db.get_own_status(&focus.id).await.unwrap().is_none());

    cloud.finish().await;
}
