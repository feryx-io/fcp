//! The sync journal (`docs/HYBRID_CONTROL_PLANE.md` §3.2, phase 2): every
//! table a site needs has the trigger, and a put, an update and a delete
//! through the stores journal in order, under the row's tenant, with the
//! tenant's own row under its own id. Gated on `FCP_TEST_DATABASE_URL`;
//! skipped with a notice when unset.

use fcp_core::storage::{
    CredentialStore, DeviceStore, ExtensionStore, PhoneNumberStore, TenantStore, User,
};
use fcp_core::{
    Device, DeviceType, Extension, ExtensionTarget, PhoneNumber, SyncOp, SyncStore, Tenant,
    TenantId, SYNC_TABLES,
};
use fcp_storage_postgres::{PostgresConfig, PostgresStorage};
use sqlx::{Connection, PgConnection};

fn swap_database(url: &str, name: &str) -> String {
    let (base, _) = url.rsplit_once('/').expect("a database in the url");
    format!("{base}/{name}")
}

#[tokio::test]
async fn every_synced_table_journals_its_puts_and_deletes_in_order() {
    let Ok(admin_url) = std::env::var("FCP_TEST_DATABASE_URL") else {
        eprintln!("FCP_TEST_DATABASE_URL not set; skipping the sync journal test");
        return;
    };
    let name = format!("fcp_test_{}", uuid::Uuid::new_v4().simple());
    let mut admin = PgConnection::connect(&admin_url).await.unwrap();
    sqlx::query(&format!("CREATE DATABASE {name}"))
        .execute(&mut admin)
        .await
        .unwrap();
    let url = swap_database(&admin_url, &name);
    let storage = PostgresStorage::new(PostgresConfig {
        connection_string: url.clone(),
        max_connections: 2,
        ..Default::default()
    })
    .await
    .expect("the schema");

    // Every synced table carries the trigger.
    let mut conn = PgConnection::connect(&url).await.unwrap();
    for table in SYNC_TABLES {
        let has: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM pg_trigger WHERE tgname = 'sync_journal' AND tgrelid = $1::regclass)",
        )
        .bind(*table)
        .fetch_one(&mut conn)
        .await
        .unwrap();
        assert!(has, "{table} has no sync_journal trigger");
    }
    assert_eq!(
        storage.journal_head().await.unwrap(),
        0,
        "nothing journaled yet"
    );

    // A tenant: its own row journals under its own id.
    let tenant = TenantId::new();
    let mut t = Tenant::new("acme".to_string());
    t.id = tenant;
    t.sip_domains = vec!["acme.local".to_string()];
    storage.put_tenant(t).await.unwrap();

    // A user, put twice (an update), a device put and deleted, a number
    // put and deleted, an extension put, the user deleted.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let user: User = serde_json::from_value(serde_json::json!({
        "username": "alice",
        "password": "alice-pw",
        "sip_ha1_hash": "0000",
        "tenant_id": tenant,
        "display_name": "Alice",
        "extension": "1001",
        "enabled": true,
        "created_at": now,
        "updated_at": now,
    }))
    .unwrap();
    let user_id = user.id;
    storage.put_user(user.clone()).await.unwrap();
    let mut renamed = user.clone();
    renamed.display_name = Some("Alice A.".to_string());
    storage.put_user(renamed).await.unwrap();
    let device = Device::new("alice softphone".to_string(), DeviceType::Softphone, tenant);
    let device_id = device.id;
    storage.put_device(device).await.unwrap();
    storage.delete_device(&device_id).await.unwrap();
    let number = PhoneNumber::new("+15550001001".to_string(), tenant).unwrap();
    let number_id = number.id;
    storage.put_phone_number(number).await.unwrap();
    storage.delete_phone_number(&number_id).await.unwrap();
    let ext = Extension::new("1001".to_string(), ExtensionTarget::User(user_id), tenant).unwrap();
    let ext_id = ext.id;
    storage.put_extension(ext).await.unwrap();
    storage.delete_user("alice").await.unwrap();

    let entries = storage.journal_after(&tenant, 0, 100).await.unwrap();
    let shape: Vec<(String, String, SyncOp)> = entries
        .iter()
        .map(|e| (e.table.clone(), e.row_id.clone(), e.op))
        .collect();
    let expected = vec![
        ("tenants".to_string(), tenant.to_string(), SyncOp::Put),
        ("users".to_string(), user_id.to_string(), SyncOp::Put),
        ("users".to_string(), user_id.to_string(), SyncOp::Put),
        ("devices".to_string(), device_id.to_string(), SyncOp::Put),
        ("devices".to_string(), device_id.to_string(), SyncOp::Delete),
        (
            "phone_numbers".to_string(),
            number_id.to_string(),
            SyncOp::Put,
        ),
        (
            "phone_numbers".to_string(),
            number_id.to_string(),
            SyncOp::Delete,
        ),
        ("extensions".to_string(), ext_id.to_string(), SyncOp::Put),
        ("users".to_string(), user_id.to_string(), SyncOp::Delete),
    ];
    assert_eq!(shape, expected, "{entries:#?}");
    assert!(
        entries.windows(2).all(|w| w[0].seq < w[1].seq),
        "in sequence order"
    );
    assert!(entries.iter().all(|e| e.tenant_id == tenant));
    assert_eq!(
        storage.journal_head().await.unwrap(),
        entries.last().unwrap().seq
    );

    // Paged, and nobody else's.
    let page = storage.journal_after(&tenant, 0, 3).await.unwrap();
    assert_eq!(page.len(), 3);
    let rest = storage
        .journal_after(&tenant, page[2].seq, 100)
        .await
        .unwrap();
    assert_eq!(rest.len(), entries.len() - 3);
    assert!(storage
        .journal_after(&TenantId::new(), 0, 100)
        .await
        .unwrap()
        .is_empty());

    conn.close().await.ok();
    drop(storage);
    sqlx::query(&format!("DROP DATABASE {name} WITH (FORCE)"))
        .execute(&mut admin)
        .await
        .unwrap();
}
