//! Fixtures for a throwaway database, written the way the binaries read
//! them: through `PostgresStorage` and the fcp-core store traits, so the
//! rows are exactly what the platform's own code would have made.
//! Storage rather than HTTP because the call manager has no route that
//! creates a user.

use fcp_core::e911::{EmergencyCallRecord, EmergencyStore};
use fcp_core::storage::{
    password::compute_ha1, ConferenceStore, CredentialStore, DeviceStore, ExtensionStore,
    SiteLocationStore, TenantStore, User,
};
use fcp_core::{
    Device, DeviceType, Extension, ExtensionTarget, LocationId, Meeting, MeetingId, SiteLocation,
    Tenant, TenantId,
};
use fcp_storage_postgres::emergency::PostgresEmergencyStore;
use fcp_storage_postgres::{PostgresConfig, PostgresStorage};
use std::time::{Duration, SystemTime};

/// A seeded database: the default tenant and whatever the test added.
pub struct Seed {
    storage: PostgresStorage,
    /// The realm SIP users authenticate in; the same one the config
    /// gives the registrar.
    pub realm: String,
    /// The tenant everything is created in: the nil one the binaries
    /// default to and the registrar falls back to.
    pub tenant_id: TenantId,
}

/// A user the seed made, with the password a phone registers with.
#[derive(Debug, Clone)]
pub struct SeededUser {
    pub username: String,
    pub password: String,
    pub extension: String,
}

impl Seed {
    /// Connect to `database_url` (running the migrations, as a binary
    /// would) and make the default tenant with `realm` as its SIP domain.
    pub async fn new(database_url: &str, realm: &str) -> Self {
        let storage = PostgresStorage::new(PostgresConfig {
            connection_string: database_url.to_string(),
            read_replica_url: None,
            max_replica_lag: Duration::from_secs(5),
            max_connections: 4,
            connection_timeout: Duration::from_secs(30),
            encryption: None,
        })
        .await
        .expect("seed connects to the test database");
        let tenant_id = TenantId::nil();
        let mut tenant = Tenant::new("e2e".to_string());
        tenant.id = tenant_id;
        tenant.sip_domains = vec![realm.to_string()];
        storage.put_tenant(tenant).await.expect("seed tenant");
        Self {
            storage,
            realm: realm.to_string(),
            tenant_id,
        }
    }

    /// A SIP user with a digest password, a softphone device on the
    /// user's AOR (a call to the user rings the user's devices, and a
    /// device's contacts are the registrations on its SIP URI), and an
    /// extension that dials them.
    pub async fn user(&self, username: &str, password: &str, extension: &str) -> SeededUser {
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let user: User = serde_json::from_value(serde_json::json!({
            "username": username,
            "password": password,
            "sip_ha1_hash": compute_ha1(username, &self.realm, password),
            "tenant_id": self.tenant_id,
            "display_name": username,
            "extension": extension,
            "enabled": true,
            "created_at": now,
            "updated_at": now,
        }))
        .expect("user fixture deserializes");
        let user_id = user.id;
        self.storage.put_user(user).await.expect("seed user");
        let mut device = Device::new(
            format!("{username} softphone"),
            DeviceType::Softphone,
            self.tenant_id,
        );
        device.owner_user_id = Some(user_id);
        device.sip_uri = Some(format!("sip:{username}@{}", self.realm));
        self.storage.put_device(device).await.expect("seed device");
        let ext = Extension::new(
            extension.to_string(),
            ExtensionTarget::User(user_id),
            self.tenant_id,
        )
        .expect("extension number is valid");
        self.storage
            .put_extension(ext)
            .await
            .expect("seed extension");
        SeededUser {
            username: username.to_string(),
            password: password.to_string(),
            extension: extension.to_string(),
        }
    }

    /// A conference room: `room` is what callers dial. A guest PIN, when
    /// given, is required of every caller; a host PIN makes its caller a
    /// moderator.
    pub async fn meeting(&self, room: &str, guest_pin: Option<&str>, host_pin: Option<&str>) {
        let meeting: Meeting = serde_json::from_value(serde_json::json!({
            "id": MeetingId::new(),
            "tenant_id": self.tenant_id,
            "room_number": room,
            "name": format!("Room {room}"),
            "guest_pin": guest_pin,
            "host_pin": host_pin,
            "require_guest_pin": guest_pin.is_some(),
            "max_participants": 10,
            "wait_for_moderator": false,
            "enabled": true,
            "auto_record": false,
            "default_locked": false,
            "permanent": true,
            "created_at": SystemTime::now(),
            "updated_at": SystemTime::now(),
        }))
        .expect("meeting fixture deserializes");
        self.storage
            .put_meeting(meeting)
            .await
            .expect("seed meeting");
    }

    /// A site with a complete civic address and `elin` as its E911
    /// callback number, with `username` placed at it: the dispatchable
    /// location of that user's emergency calls (`docs/E911.md`).
    pub async fn site_for(&self, username: &str, elin: &str) -> LocationId {
        let mut site = SiteLocation::new(format!("{username}'s office"), self.tenant_id);
        site.address_line1 = Some("100 State St".to_string());
        site.city = Some("Albany".to_string());
        site.state = Some("NY".to_string());
        site.postal_code = Some("12207".to_string());
        site.country = Some("US".to_string());
        site.floor = Some("3".to_string());
        site.e911_callback_number = Some(elin.to_string());
        let id = site.id;
        self.storage.put_location(site).await.expect("seed site");
        let mut user = self
            .storage
            .get_user_in_tenant(username, &self.tenant_id)
            .await
            .expect("load user")
            .expect("user was seeded");
        user.location_id = Some(id);
        self.storage.put_user(user).await.expect("place user");
        id
    }

    /// The emergency call records, newest first, once at least `n` are
    /// there and `ready` holds for every one. Panics after `within`.
    pub async fn emergency_calls(
        &self,
        n: usize,
        within: std::time::Duration,
        ready: impl Fn(&EmergencyCallRecord) -> bool,
    ) -> Vec<EmergencyCallRecord> {
        let store = PostgresEmergencyStore::new(self.storage.pool().clone());
        let deadline = tokio::time::Instant::now() + within;
        loop {
            let records = store
                .list_emergency_calls(&self.tenant_id, 100)
                .await
                .expect("list emergency calls");
            if records.len() >= n && records.iter().all(&ready) {
                return records;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "{} of {n} emergency call records ready after {within:?}: {records:#?}",
                records.len()
            );
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
    }

    /// The storage, for fixtures this helper has no shorthand for.
    pub fn storage(&self) -> &PostgresStorage {
        &self.storage
    }

    /// The records of `kind` (every kind for `None`) in the test database,
    /// once at least `n` are there, oldest first. Panics after `within`.
    pub async fn cdrs(
        &self,
        kind: Option<fcp_core::storage::CdrKind>,
        n: usize,
        within: std::time::Duration,
    ) -> Vec<fcp_core::storage::CallDetailRecord> {
        use fcp_core::storage::{CdrQuery, CdrStore};
        let deadline = tokio::time::Instant::now() + within;
        loop {
            let mut records = self
                .storage
                .query_cdrs(CdrQuery {
                    kind,
                    ..Default::default()
                })
                .await
                .expect("query records");
            if records.len() >= n {
                records.reverse();
                return records;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "{} of {n} {kind:?} records after {within:?}: {records:#?}",
                records.len()
            );
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
    }

    /// Run one SQL statement against the test database, for a scenario
    /// that has to break something on purpose (the CDR pipeline's outage
    /// test takes the table away and gives it back).
    pub async fn execute(&self, sql: &str) {
        sqlx::query(sql)
            .execute(self.storage.pool())
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"));
    }
}
