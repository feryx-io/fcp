//! A user's own contacts at a site (`docs/RECORD_AND_CONTACTS.md` §3):
//! synced both ways. A contact written in the cloud reaches the site
//! through the journal; one written at the site reaches the cloud through
//! the link's shipping. Each side takes a copy only when it is newer, so
//! a change's echo is let go (the cloud journals the site's contact once,
//! and nothing more comes of it) and of two edits the later wins; a delete
//! crosses as a tombstone, and a row the cloud removes goes at the site
//! and is not brought back.
//! Gated on `FCP_TEST_DATABASE_URL`; skipped with a notice when unset.

mod common;

use common::{cloud_with_site_opts, eventually, CloudOptions};
use fcp_core::contact::{ContactNumber, ContactStore, PersonalContact};
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

fn contact(tenant: TenantId, owner: UserId, name: &str, at: SystemTime) -> PersonalContact {
    PersonalContact {
        id: fcp_core::contact::new_contact_id(),
        tenant_id: tenant,
        owner_user_id: owner,
        name: name.to_string(),
        company: None,
        numbers: vec![ContactNumber {
            label: "mobile".into(),
            number: "0412345678".into(),
        }],
        email: None,
        notes: None,
        ringtone: None,
        created_at: at,
        updated_at: at,
        deleted_at: None,
    }
}

async fn name_of(store: &PostgresStorage, id: &str) -> Option<String> {
    store.get_contact(id).await.unwrap().map(|c| c.name)
}

/// How many times the cloud journalled the row: what it sent down.
async fn journalled(cloud: &PostgresStorage, id: &str) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM sync_journal WHERE table_name = 'personal_contacts' AND row_id = $1",
    )
    .bind(id)
    .fetch_one(cloud.pool())
    .await
    .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_users_contacts_are_synced_both_ways_with_their_site() {
    let Some(site_db) = TestDatabase::create().await else {
        eprintln!("FCP_TEST_DATABASE_URL not set; skipping the contacts test");
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
    let owner = alice.id;
    cloud_db.put_user(alice).await.unwrap();
    eventually("Alice reaching the site", SOON, || async {
        site_db_store
            .get_user_by_id(&owner)
            .await
            .unwrap()
            .is_some()
    })
    .await;

    // Down: a contact written in the cloud reaches the site.
    let from_cloud = contact(tenant, owner, "Bob from the cloud", now);
    assert!(cloud_db.put_contact_if_newer(&from_cloud).await.unwrap());
    eventually("the cloud's contact reaching the site", SOON, || async {
        name_of(&site_db_store, &from_cloud.id).await.as_deref() == Some("Bob from the cloud")
    })
    .await;

    // Up: one written at the site reaches the cloud.
    let from_site = contact(tenant, owner, "Carol from the site", now);
    assert!(site_db_store
        .put_contact_if_newer(&from_site)
        .await
        .unwrap());
    eventually("the site's contact reaching the cloud", SOON, || async {
        name_of(&cloud_db, &from_site.id).await.as_deref() == Some("Carol from the site")
    })
    .await;

    // No echo: the cloud journalled the site's contact once, when it took
    // it; the site let that copy go (no newer), so nothing came back up.
    tokio::time::sleep(Duration::from_secs(5)).await;
    assert_eq!(journalled(&cloud_db, &from_site.id).await, 1);
    assert_eq!(journalled(&cloud_db, &from_cloud.id).await, 1);

    // Two edits: the later wins on both sides, whichever arrives last.
    let later = now + Duration::from_secs(3600);
    let mut site_edit = from_cloud.clone();
    site_edit.name = "Bob, edited at the site later".into();
    site_edit.updated_at = later;
    let mut cloud_edit = from_cloud.clone();
    cloud_edit.name = "Bob, edited in the cloud earlier".into();
    cloud_edit.updated_at = now + Duration::from_secs(60);
    assert!(site_db_store
        .put_contact_if_newer(&site_edit)
        .await
        .unwrap());
    assert!(cloud_db.put_contact_if_newer(&cloud_edit).await.unwrap());
    for (side, store) in [("the cloud", &cloud_db), ("the site", &site_db_store)] {
        eventually(
            &format!("the later edit winning in {side}"),
            SOON,
            || async {
                name_of(store, &from_cloud.id).await.as_deref()
                    == Some("Bob, edited at the site later")
            },
        )
        .await;
    }

    // A delete in the cloud is a tombstone, and crosses.
    let mut tombstone = cloud_db.get_contact(&from_site.id).await.unwrap().unwrap();
    tombstone.updated_at = later + Duration::from_secs(1);
    tombstone.deleted_at = Some(tombstone.updated_at);
    assert!(cloud_db.put_contact_if_newer(&tombstone).await.unwrap());
    eventually("the delete reaching the site", SOON, || async {
        site_db_store
            .get_contact(&from_site.id)
            .await
            .unwrap()
            .is_some_and(|c| c.deleted_at.is_some())
    })
    .await;
    assert!(site_db_store
        .list_contacts(&owner, None, None, 10)
        .await
        .unwrap()
        .iter()
        .all(|c| c.id != from_site.id));

    // The sweep removes the tombstone in the cloud; the row goes at the
    // site too, and a late echo of the tombstone brings nothing back (a
    // tombstone for a contact a side does not have is not kept).
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert_eq!(
        cloud_db
            .sweep_contact_tombstones(later + Duration::from_secs(10))
            .await
            .unwrap(),
        1
    );
    eventually("the swept row going at the site", SOON, || async {
        site_db_store
            .get_contact(&from_site.id)
            .await
            .unwrap()
            .is_none()
    })
    .await;
    assert!(!cloud_db.put_contact_if_newer(&tombstone).await.unwrap());
    assert!(cloud_db.get_contact(&from_site.id).await.unwrap().is_none());

    cloud.finish().await;
}
