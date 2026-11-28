//! A site's own backups seen from the cloud (`docs/SCHEDULED_BACKUPS.md`
//! B5): a destination the site's admin set up, in the site's database, is
//! carried up on the link's heartbeat with its last good backup and
//! whether it is overdue, and shown on the cloud's site, without a secret.
//! Gated on `FCP_TEST_DATABASE_URL`.

mod common;

use common::{cloud_with_site_opts, eventually, CloudOptions};
use fcp_core::backup::{
    BackupDestination, BackupRetention, BackupSchedule, BackupStore, BackupTarget,
};
use fcp_storage_postgres::{PostgresConfig, PostgresStorage};
use fcp_testkit::TestDatabase;
use std::time::{Duration, SystemTime};

const SOON: Duration = Duration::from_secs(30);

#[tokio::test]
async fn a_sites_backups_reach_the_cloud_with_their_overdue() {
    let Some(site_db) = TestDatabase::create().await else {
        eprintln!("FCP_TEST_DATABASE_URL not set; skipping");
        return;
    };
    let cloud = cloud_with_site_opts(CloudOptions {
        site_database_url: Some(site_db.url()),
        ..Default::default()
    })
    .await
    .expect("the database was there a moment ago");
    let site_store = PostgresStorage::new(PostgresConfig {
        connection_string: site_db.url().to_string(),
        read_replica_url: None,
        max_replica_lag: Duration::from_secs(5),
        max_connections: 4,
        connection_timeout: Duration::from_secs(30),
        encryption: None,
    })
    .await
    .unwrap();
    // A NAS on a schedule, set up two days ago and never backed up.
    let made = SystemTime::now() - Duration::from_secs(2 * 86_400);
    let nas = BackupDestination {
        id: uuid::Uuid::new_v4(),
        name: "Site NAS".into(),
        target: BackupTarget::Sftp {
            host: "nas.site.example".into(),
            port: 22,
            user: "fcp".into(),
            path: "/backups".into(),
            private_key: Some("-----BEGIN OPENSSH PRIVATE KEY-----\nsite-secret\n".into()),
            host_key: None,
        },
        password: Some("the site's repository password".into()),
        schedule: BackupSchedule {
            daily_at: Some("03:00".into()),
            cron: None,
        },
        keep: BackupRetention::default(),
        include_key: false,
        enabled: true,
        created_at: made,
        updated_at: made,
    };
    site_store.put_backup_destination(&nas).await.unwrap();

    let path = format!("/api/v1/sites/{}", cloud.site_id);
    eventually("the site's backups on the cloud", SOON, || async {
        let (code, body) = cloud.admin_call("GET", &path, true, None).await;
        assert_eq!(code, 200, "{body}");
        body["backups"]
            .as_array()
            .is_some_and(|b| b.iter().any(|d| d["name"] == "Site NAS"))
    })
    .await;
    let (_, site) = cloud.admin_call("GET", &path, true, None).await;
    let d = &site["backups"][0];
    assert_eq!(d["destination_id"], format!("bkd-{}", nas.id));
    assert_eq!(d["kind"], "sftp");
    assert_eq!(d["scheduled"], true);
    assert_eq!(d["overdue"], true, "{site}");
    assert_eq!(d["last_success_at"], serde_json::Value::Null);
    let text = site.to_string();
    assert!(
        !text.contains("site-secret") && !text.contains("repository password"),
        "{text}"
    );
    cloud.finish().await;
}
