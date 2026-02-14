//! MongoDB storage backend for FCP
//!
//! This crate provides a MongoDB implementation of the FCP storage traits.
//!
//! ## Features
//! - Document-oriented storage with native BSON support
//! - Native nested JSON storage (no serialization overhead)
//! - Flexible schema with optional fields
//! - Horizontal scalability with sharding
//! - Built-in replication support
//!
//! ## Usage
//! ```no_run
//! use fcp_storage_mongodb::{MongodbStorage, MongodbConfig};
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     let config = MongodbConfig::new("mongodb://localhost:27017/fcp");
//!     let storage = MongodbStorage::new(config).await?;
//!     Ok(())
//! }
//! ```

use anyhow::Result;
use async_trait::async_trait;
use bson::{doc, DateTime as BsonDateTime, Document};
use fcp_core::storage::{
    AccountStatus, CallDetailRecord, CallDisposition, CdrQuery, CdrStore, ConferenceStore,
    CredentialStore, DeviceStore, EmergencyContact, ExtensionStore, HangupCause, HangupParty,
    LineBindingStore, MediaPreferences, PhoneNumberStore, Registration, RegistrationStore,
    TenantStore, User,
};
use fcp_core::{
    AudioCodec, CallId, Device, DeviceBinding, DeviceId, DeviceType, Extension, ExtensionId,
    ExtensionTarget, LineBinding, LineBindingId, PhoneNumber, PhoneNumberId, RegistrationState,
    RingStrategy, ServiceId, Tenant, TenantId, UserId, VoicemailBoxId,
};
use fcp_core::{BindingType, NumberState};
use mongodb::{
    options::{ClientOptions, IndexOptions},
    Client, Collection, Database, IndexModel,
};
use std::str::FromStr;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tracing::{debug, info};

/// Configuration for MongoDB storage
#[derive(Debug, Clone)]
pub struct MongodbConfig {
    /// MongoDB connection string (e.g., "mongodb://localhost:27017/fcp")
    pub connection_string: String,
    /// Database name (extracted from connection string if not provided)
    pub database_name: String,
    /// Connection timeout in seconds
    pub timeout: Duration,
}

impl MongodbConfig {
    /// Create a new MongoDB configuration
    ///
    /// # Arguments
    /// * `connection_string` - MongoDB connection string
    ///
    /// # Example
    /// ```
    /// use fcp_storage_mongodb::MongodbConfig;
    /// let config = MongodbConfig::new("mongodb://localhost:27017/fcp");
    /// ```
    pub fn new(connection_string: impl Into<String>) -> Self {
        let connection_string = connection_string.into();
        let database_name = Self::extract_database_name(&connection_string);

        Self {
            connection_string,
            database_name,
            timeout: Duration::from_secs(30),
        }
    }

    /// Set connection timeout
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Set database name explicitly
    pub fn with_database(mut self, database_name: impl Into<String>) -> Self {
        self.database_name = database_name.into();
        self
    }

    /// Extract database name from connection string
    fn extract_database_name(connection_string: &str) -> String {
        // Extract database name from connection string
        // mongodb://host:port/database or mongodb://user:pass@host:port/database
        connection_string
            .split('/')
            .last()
            .and_then(|s| s.split('?').next())
            .filter(|s| !s.is_empty())
            .unwrap_or("fcp")
            .to_string()
    }
}

/// MongoDB storage implementation
pub struct MongodbStorage {
    client: Client,
    db: Database,
    users: Collection<Document>,
    registrations: Collection<Document>,
    cdrs: Collection<Document>,
    tenants: Collection<Document>,
    phone_numbers: Collection<Document>,
    line_bindings: Collection<Document>,
    extensions: Collection<Document>,
    devices: Collection<Document>,
    device_bindings: Collection<Document>,
    meetings: Collection<Document>,
}

impl MongodbStorage {
    /// Create a new MongoDB storage instance
    ///
    /// This will connect to MongoDB and ensure all indexes are created.
    pub async fn new(config: MongodbConfig) -> Result<Self> {
        info!(
            connection = %config.connection_string,
            database = %config.database_name,
            "Initializing MongoDB storage"
        );

        // Parse connection string
        let mut client_options = ClientOptions::parse(&config.connection_string)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to parse MongoDB URI: {}", e))?;

        client_options.connect_timeout = Some(config.timeout);

        // Create client
        let client = Client::with_options(client_options)
            .map_err(|e| anyhow::anyhow!("Failed to create MongoDB client: {}", e))?;

        // Get database
        let db = client.database(&config.database_name);

        // Get collections
        let users = db.collection::<Document>("users");
        let registrations = db.collection::<Document>("registrations");
        let cdrs = db.collection::<Document>("cdrs");
        let tenants = db.collection::<Document>("tenants");
        let phone_numbers = db.collection::<Document>("phone_numbers");
        let line_bindings = db.collection::<Document>("line_bindings");
        let extensions = db.collection::<Document>("extensions");
        let devices = db.collection::<Document>("devices");
        let device_bindings = db.collection::<Document>("device_bindings");
        let meetings = db.collection::<Document>("meetings");

        let storage = Self {
            client,
            db,
            users,
            registrations,
            cdrs,
            tenants,
            phone_numbers,
            line_bindings,
            extensions,
            devices,
            device_bindings,
            meetings,
        };

        // Create indexes
        storage.create_indexes().await?;

        info!("MongoDB storage initialized successfully");
        Ok(storage)
    }

    /// Create all necessary indexes
    async fn create_indexes(&self) -> Result<()> {
        debug!("Creating MongoDB indexes");

        // Users indexes
        let users_indexes = vec![
            IndexModel::builder()
                .keys(doc! { "username": 1 })
                .options(IndexOptions::builder().unique(true).build())
                .build(),
            IndexModel::builder().keys(doc! { "id": 1 }).build(),
            IndexModel::builder().keys(doc! { "tenant_id": 1 }).build(),
            IndexModel::builder().keys(doc! { "role": 1 }).build(),
        ];
        self.users
            .create_indexes(users_indexes)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to create users indexes: {}", e))?;

        // Registrations indexes
        let registrations_indexes = vec![
            IndexModel::builder()
                .keys(doc! { "aor": 1, "contact": 1 })
                .options(IndexOptions::builder().unique(true).build())
                .build(),
            IndexModel::builder().keys(doc! { "aor": 1 }).build(),
            IndexModel::builder().keys(doc! { "expires_at": 1 }).build(),
        ];
        self.registrations
            .create_indexes(registrations_indexes)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to create registrations indexes: {}", e))?;

        // CDRs indexes
        let cdrs_indexes = vec![
            IndexModel::builder()
                .keys(doc! { "call_id": 1 })
                .options(IndexOptions::builder().unique(true).build())
                .build(),
            IndexModel::builder().keys(doc! { "tenant_id": 1 }).build(),
            IndexModel::builder().keys(doc! { "start_time": 1 }).build(),
            IndexModel::builder().keys(doc! { "from_uri": 1 }).build(),
            IndexModel::builder().keys(doc! { "to_uri": 1 }).build(),
        ];
        self.cdrs
            .create_indexes(cdrs_indexes)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to create cdrs indexes: {}", e))?;

        // Tenants indexes
        let tenants_indexes = vec![
            IndexModel::builder()
                .keys(doc! { "id": 1 })
                .options(IndexOptions::builder().unique(true).build())
                .build(),
            IndexModel::builder()
                .keys(doc! { "name": 1 })
                .options(IndexOptions::builder().unique(true).build())
                .build(),
            IndexModel::builder().keys(doc! { "status": 1 }).build(),
        ];
        self.tenants
            .create_indexes(tenants_indexes)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to create tenants indexes: {}", e))?;

        // Phone numbers indexes
        let phone_numbers_indexes = vec![
            IndexModel::builder()
                .keys(doc! { "id": 1 })
                .options(IndexOptions::builder().unique(true).build())
                .build(),
            IndexModel::builder()
                .keys(doc! { "e164": 1 })
                .options(IndexOptions::builder().unique(true).build())
                .build(),
            IndexModel::builder().keys(doc! { "tenant_id": 1 }).build(),
        ];
        self.phone_numbers
            .create_indexes(phone_numbers_indexes)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to create phone_numbers indexes: {}", e))?;

        // Line bindings indexes
        let line_bindings_indexes = vec![
            IndexModel::builder()
                .keys(doc! { "id": 1 })
                .options(IndexOptions::builder().unique(true).build())
                .build(),
            IndexModel::builder()
                .keys(doc! { "phone_number_id": 1 })
                .build(),
            IndexModel::builder().keys(doc! { "tenant_id": 1 }).build(),
        ];
        self.line_bindings
            .create_indexes(line_bindings_indexes)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to create line_bindings indexes: {}", e))?;

        // Extensions indexes
        let extensions_indexes = vec![
            IndexModel::builder()
                .keys(doc! { "id": 1 })
                .options(IndexOptions::builder().unique(true).build())
                .build(),
            IndexModel::builder()
                .keys(doc! { "number": 1, "tenant_id": 1 })
                .options(IndexOptions::builder().unique(true).build())
                .build(),
            IndexModel::builder().keys(doc! { "tenant_id": 1 }).build(),
        ];
        self.extensions
            .create_indexes(extensions_indexes)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to create extensions indexes: {}", e))?;

        // Devices indexes
        let devices_indexes = vec![
            IndexModel::builder()
                .keys(doc! { "id": 1 })
                .options(IndexOptions::builder().unique(true).build())
                .build(),
            IndexModel::builder()
                .keys(doc! { "sip_uri": 1 })
                .options(IndexOptions::builder().unique(true).build())
                .build(),
            IndexModel::builder()
                .keys(doc! { "owner_user_id": 1 })
                .build(),
            IndexModel::builder().keys(doc! { "tenant_id": 1 }).build(),
        ];
        self.devices
            .create_indexes(devices_indexes)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to create devices indexes: {}", e))?;

        // Device bindings indexes
        let device_bindings_indexes = vec![
            IndexModel::builder()
                .keys(doc! { "line_binding_id": 1, "device_id": 1 })
                .options(IndexOptions::builder().unique(true).build())
                .build(),
            IndexModel::builder()
                .keys(doc! { "line_binding_id": 1 })
                .build(),
        ];
        self.device_bindings
            .create_indexes(device_bindings_indexes)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to create device_bindings indexes: {}", e))?;

        // Meetings indexes
        let meetings_indexes = vec![
            IndexModel::builder()
                .keys(doc! { "id": 1 })
                .options(IndexOptions::builder().unique(true).build())
                .build(),
            IndexModel::builder()
                .keys(doc! { "tenant_id": 1 })
                .build(),
            IndexModel::builder()
                .keys(doc! { "tenant_id": 1, "room_number": 1 })
                .options(IndexOptions::builder().unique(true).build())
                .build(),
        ];
        self.meetings
            .create_indexes(meetings_indexes)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to create meetings indexes: {}", e))?;

        debug!("MongoDB indexes created successfully");
        Ok(())
    }

    /// Get the underlying MongoDB database
    pub fn database(&self) -> &Database {
        &self.db
    }

    /// Get the MongoDB client
    pub fn client(&self) -> &Client {
        &self.client
    }
}

// Helper functions for timestamp conversion
fn system_time_to_bson_datetime(st: SystemTime) -> BsonDateTime {
    let duration = st.duration_since(UNIX_EPOCH).unwrap_or_default();
    BsonDateTime::from_millis(duration.as_millis() as i64)
}

fn bson_datetime_to_system_time(dt: BsonDateTime) -> SystemTime {
    UNIX_EPOCH + Duration::from_millis(dt.timestamp_millis() as u64)
}

// Helper to convert disposition enum to/from string
fn disposition_to_string(d: CallDisposition) -> String {
    match d {
        CallDisposition::Answered => "answered".to_string(),
        CallDisposition::Busy => "busy".to_string(),
        CallDisposition::NoAnswer => "no_answer".to_string(),
        CallDisposition::Failed => "failed".to_string(),
        CallDisposition::Cancelled => "cancelled".to_string(),
    }
}

fn string_to_disposition(s: &str) -> CallDisposition {
    match s {
        "answered" => CallDisposition::Answered,
        "busy" => CallDisposition::Busy,
        "no_answer" => CallDisposition::NoAnswer,
        "failed" => CallDisposition::Failed,
        "cancelled" => CallDisposition::Cancelled,
        _ => CallDisposition::Failed,
    }
}

fn hangup_cause_to_bson(h: HangupCause) -> bson::Bson {
    match h {
        HangupCause::NormalClearing => bson::bson!("normal_clearing"),
        HangupCause::UserBusy => bson::bson!("user_busy"),
        HangupCause::NoAnswer => bson::bson!("no_answer"),
        HangupCause::CallRejected => bson::bson!("call_rejected"),
        HangupCause::Timeout => bson::bson!("timeout"),
        HangupCause::NetworkError => bson::bson!("network_error"),
        HangupCause::InternalError => bson::bson!("internal_error"),
        HangupCause::Other(code) => bson::bson!({ "other": code as i32 }),
    }
}

fn bson_to_hangup_cause(b: &bson::Bson) -> HangupCause {
    if let bson::Bson::String(s) = b {
        match s.as_str() {
            "normal_clearing" => HangupCause::NormalClearing,
            "user_busy" => HangupCause::UserBusy,
            "no_answer" => HangupCause::NoAnswer,
            "call_rejected" => HangupCause::CallRejected,
            "timeout" => HangupCause::Timeout,
            "network_error" => HangupCause::NetworkError,
            "internal_error" => HangupCause::InternalError,
            _ => HangupCause::Other(0),
        }
    } else if let bson::Bson::Document(doc) = b {
        if let Some(bson::Bson::Int32(code)) = doc.get("other") {
            HangupCause::Other(*code as u16)
        } else {
            HangupCause::Other(0)
        }
    } else {
        HangupCause::Other(0)
    }
}

fn hangup_party_to_string(h: HangupParty) -> String {
    match h {
        HangupParty::Caller => "caller".to_string(),
        HangupParty::Callee => "callee".to_string(),
        HangupParty::System => "system".to_string(),
    }
}

fn string_to_hangup_party(s: &str) -> HangupParty {
    match s {
        "caller" => HangupParty::Caller,
        "callee" => HangupParty::Callee,
        "system" => HangupParty::System,
        _ => HangupParty::System,
    }
}

// ============================================================================
// CredentialStore Implementation
// ============================================================================

#[async_trait]
impl CredentialStore for MongodbStorage {
    async fn get_user(&self, username: &str) -> Result<Option<User>> {
        let filter = doc! {
            "username": username,
        };

        let doc = self
            .users
            .find_one(filter)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to query user: {}", e))?;

        match doc {
            Some(doc) => {
                let user = document_to_user(doc)?;
                debug!(username = %username, "User retrieved from MongoDB");
                Ok(Some(user))
            }
            None => {
                debug!(username = %username, "User not found in MongoDB");
                Ok(None)
            }
        }
    }

    async fn put_user(&self, user: User) -> Result<()> {
        let tenant_id = user.tenant_id.to_string();
        let media_prefs = bson::to_bson(&user.media_preferences)
            .map_err(|e| anyhow::anyhow!("Failed to serialize media preferences: {}", e))?;

        // Serialize role to snake_case string for storage
        let role_str = serde_json::to_string(&user.role)
            .map(|s| s.trim_matches('"').to_string())
            .unwrap_or_else(|_| "end_user".to_string());

        let account_status = user.account_status.to_string();

        let mut doc = doc! {
            "id": user.id.to_string(),
            "username": &user.username,
            "password_hash": &user.password,
            "display_name": user.display_name.as_ref(),
            "tenant_id": &tenant_id,
            "enabled": user.enabled,
            "role": &role_str,
            "created_at": system_time_to_bson_datetime(user.created_at),
            "updated_at": system_time_to_bson_datetime(user.updated_at),
            "media_preferences": media_prefs,
            "language": &user.language,
            "timezone": &user.timezone,
            "account_status": &account_status,
        };

        // Optional string fields
        if let Some(ref v) = user.digest_password { doc.insert("digest_password", v); }
        if let Some(ref v) = user.sip_ha1_hash { doc.insert("sip_ha1_hash", v); }
        if let Some(ref v) = user.department_id { doc.insert("department_id", v); }
        if let Some(ref v) = user.email { doc.insert("email", v); }
        if let Some(ref v) = user.phone_number { doc.insert("phone_number", v); }
        if let Some(ref v) = user.mobile_number { doc.insert("mobile_number", v); }
        if let Some(ref v) = user.pin { doc.insert("pin", v); }
        if let Some(ref v) = user.extension { doc.insert("extension", v); }
        if let Some(ref v) = user.department { doc.insert("department", v); }
        if let Some(ref v) = user.job_title { doc.insert("job_title", v); }
        if let Some(ref v) = user.location { doc.insert("location", v); }
        if let Some(ref v) = user.emergency_contact.name { doc.insert("emergency_contact_name", v); }
        if let Some(ref v) = user.emergency_contact.phone { doc.insert("emergency_contact_phone", v); }
        if let Some(ref v) = user.primary_phone_number_id { doc.insert("primary_phone_number_id", v.to_string()); }
        if let Some(ref v) = user.voicemail_box_id { doc.insert("voicemail_box_id", v.to_string()); }

        // Optional timestamp fields
        if let Some(t) = user.password_last_reset {
            doc.insert("password_last_reset", system_time_to_bson_datetime(t));
        }
        if let Some(t) = user.last_login {
            doc.insert("last_login", system_time_to_bson_datetime(t));
        }

        let filter = doc! {
            "username": &user.username,
        };

        self.users
            .replace_one(filter, doc)
            .upsert(true)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to insert/update user: {}", e))?;

        debug!(username = %user.username, "User stored in MongoDB");
        Ok(())
    }

    async fn delete_user(&self, username: &str) -> Result<()> {
        let filter = doc! {
            "username": username,
        };

        self.users
            .delete_one(filter)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to delete user: {}", e))?;

        debug!(username = %username, "User deleted from MongoDB");
        Ok(())
    }

    async fn list_users(&self) -> Result<Vec<User>> {
        let mut cursor = self
            .users
            .find(doc! {})
            .await
            .map_err(|e| anyhow::anyhow!("Failed to list users: {}", e))?;

        let mut users = Vec::new();
        while cursor
            .advance()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to iterate users cursor: {}", e))?
        {
            let doc = cursor
                .deserialize_current()
                .map_err(|e| anyhow::anyhow!("Failed to deserialize user document: {}", e))?;
            users.push(document_to_user(doc)?);
        }

        debug!(count = users.len(), "Users listed from MongoDB");
        Ok(users)
    }
}

// ============================================================================
// RegistrationStore Implementation
// ============================================================================

#[async_trait]
impl RegistrationStore for MongodbStorage {
    async fn get_registrations(&self, aor: &str) -> Result<Vec<Registration>> {
        let filter = doc! {
            "aor": aor,
        };

        let mut cursor = self
            .registrations
            .find(filter)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to query registrations: {}", e))?;

        let mut registrations = Vec::new();
        while cursor
            .advance()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to iterate registrations cursor: {}", e))?
        {
            let doc = cursor.deserialize_current().map_err(|e| {
                anyhow::anyhow!("Failed to deserialize registration document: {}", e)
            })?;
            registrations.push(document_to_registration(doc)?);
        }

        debug!(
            aor = %aor,
            count = registrations.len(),
            "Registrations retrieved from MongoDB"
        );
        Ok(registrations)
    }

    async fn put_registration(&self, registration: Registration) -> Result<()> {
        let doc = doc! {
            "aor": &registration.aor,
            "contact": &registration.contact,
            "expires_at": system_time_to_bson_datetime(registration.expires_at),
            "call_id": &registration.call_id,
            "cseq": registration.cseq as i64,
            "user_agent": registration.user_agent.as_ref(),
            "q": registration.q.map(|q| q as f64),
            "registered_at": system_time_to_bson_datetime(registration.registered_at),
            "path": registration.path.as_ref(),
        };

        let filter = doc! {
            "aor": &registration.aor,
            "contact": &registration.contact,
        };

        self.registrations
            .replace_one(filter, doc)
            .upsert(true)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to insert/update registration: {}", e))?;

        debug!(aor = %registration.aor, "Registration stored in MongoDB");
        Ok(())
    }

    async fn delete_registration(&self, aor: &str, contact: &str) -> Result<()> {
        let filter = doc! {
            "aor": aor,
            "contact": contact,
        };

        self.registrations
            .delete_one(filter)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to delete registration: {}", e))?;

        debug!(aor = %aor, contact = %contact, "Registration deleted from MongoDB");
        Ok(())
    }

    async fn delete_all_registrations(&self, aor: &str) -> Result<()> {
        let filter = doc! {
            "aor": aor,
        };

        self.registrations
            .delete_many(filter)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to delete all registrations: {}", e))?;

        debug!(aor = %aor, "All registrations deleted from MongoDB");
        Ok(())
    }

    async fn cleanup_expired(&self) -> Result<usize> {
        let now = SystemTime::now();
        let filter = doc! {
            "expires_at": {
                "$lt": system_time_to_bson_datetime(now)
            }
        };

        let result = self
            .registrations
            .delete_many(filter)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to delete expired registrations: {}", e))?;

        debug!(
            count = result.deleted_count,
            "Expired registrations deleted from MongoDB"
        );
        Ok(result.deleted_count as usize)
    }
}

// ============================================================================
// CdrStore Implementation
// ============================================================================

#[async_trait]
impl CdrStore for MongodbStorage {
    async fn insert_cdr(&self, cdr: CallDetailRecord) -> Result<()> {
        let custom_data = if let Some(data) = &cdr.custom_data {
            bson::to_bson(data)
                .map_err(|e| anyhow::anyhow!("Failed to serialize custom_data: {}", e))?
        } else {
            bson::Bson::Null
        };

        let doc = doc! {
            "call_id": cdr.call_id.to_string(),
            "tenant_id": cdr.tenant_id.to_string(),
            "sip_call_id": &cdr.sip_call_id,
            "from_uri": &cdr.from_uri,
            "from_display_name": cdr.from_display_name.as_ref(),
            "to_uri": &cdr.to_uri,
            "to_display_name": cdr.to_display_name.as_ref(),
            "start_time": system_time_to_bson_datetime(cdr.start_time),
            "answer_time": cdr.answer_time.map(system_time_to_bson_datetime),
            "end_time": cdr.end_time.map(system_time_to_bson_datetime),
            "setup_duration_ms": cdr.setup_duration_ms.map(|d| d as i64),
            "call_duration_ms": cdr.call_duration_ms.map(|d| d as i64),
            "total_duration_ms": cdr.total_duration_ms.map(|d| d as i64),
            "disposition": disposition_to_string(cdr.disposition),
            "hangup_cause": hangup_cause_to_bson(cdr.hangup_cause),
            "hangup_party": cdr.hangup_party.as_ref().map(|h| hangup_party_to_string(*h)),
            "sip_status_code": cdr.sip_status_code.map(|c| c as i32),
            "sip_reason": cdr.sip_reason.as_ref(),
            "route_id": cdr.route_id.as_ref(),
            "gateway_id": cdr.gateway_id.as_ref(),
            "media_anchored": cdr.media_anchored,
            "codec_a_leg": cdr.codec_a_leg.as_ref(),
            "codec_b_leg": cdr.codec_b_leg.as_ref(),
            "rtp_packets_sent": cdr.rtp_packets_sent.map(|p| p as i64),
            "rtp_packets_received": cdr.rtp_packets_received.map(|p| p as i64),
            "rtp_bytes_sent": cdr.rtp_bytes_sent.map(|b| b as i64),
            "rtp_bytes_received": cdr.rtp_bytes_received.map(|b| b as i64),
            "recording_url": cdr.recording_url.as_ref(),
            "cost": cdr.cost,
            "currency": cdr.currency.as_ref(),
            "custom_data": custom_data,
        };

        let filter = doc! {
            "call_id": cdr.call_id.to_string(),
        };

        self.cdrs
            .replace_one(filter, doc)
            .upsert(true)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to insert/update CDR: {}", e))?;

        debug!(call_id = %cdr.call_id, "CDR stored in MongoDB");
        Ok(())
    }

    async fn get_cdr(&self, call_id: &CallId) -> Result<Option<CallDetailRecord>> {
        let filter = doc! {
            "call_id": call_id.to_string(),
        };

        let doc = self
            .cdrs
            .find_one(filter)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to query CDR: {}", e))?;

        match doc {
            Some(doc) => {
                let cdr = document_to_cdr(doc)?;
                debug!(call_id = %call_id, "CDR retrieved from MongoDB");
                Ok(Some(cdr))
            }
            None => {
                debug!(call_id = %call_id, "CDR not found in MongoDB");
                Ok(None)
            }
        }
    }

    async fn query_cdrs(&self, query: CdrQuery) -> Result<Vec<CallDetailRecord>> {
        let mut filter = doc! {};

        // Tenant filter
        if let Some(tenant_id) = &query.tenant_id {
            filter.insert("tenant_id", tenant_id.to_string());
        }

        // Call ID filter
        if let Some(call_id) = &query.call_id {
            filter.insert("call_id", call_id.to_string());
        }

        // Time range filters
        if let Some(from) = query.start_time_from {
            filter.insert(
                "start_time",
                doc! { "$gte": system_time_to_bson_datetime(from) },
            );
        }
        if let Some(to) = query.start_time_to {
            let existing = filter.get("start_time");
            if let Some(bson::Bson::Document(mut time_filter)) = existing.cloned() {
                time_filter.insert("$lte", system_time_to_bson_datetime(to));
                filter.insert("start_time", time_filter);
            } else {
                filter.insert(
                    "start_time",
                    doc! { "$lte": system_time_to_bson_datetime(to) },
                );
            }
        }

        // From/To URI filters
        if let Some(from_uri) = &query.from_uri {
            filter.insert("from_uri", from_uri);
        }
        if let Some(to_uri) = &query.to_uri {
            filter.insert("to_uri", to_uri);
        }

        // Disposition filter
        if let Some(disposition) = &query.disposition {
            filter.insert("disposition", disposition_to_string(*disposition));
        }

        // Build find options with limit
        let mut find_options = mongodb::options::FindOptions::default();
        if let Some(limit) = query.limit {
            find_options.limit = Some(limit as i64);
        }
        if let Some(offset) = query.offset {
            find_options.skip = Some(offset as u64);
        }
        find_options.sort = Some(doc! { "start_time": -1 }); // Most recent first

        let mut cursor = self
            .cdrs
            .find(filter)
            .with_options(find_options)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to query CDRs: {}", e))?;

        let mut cdrs = Vec::new();
        while cursor
            .advance()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to iterate CDRs cursor: {}", e))?
        {
            let doc = cursor
                .deserialize_current()
                .map_err(|e| anyhow::anyhow!("Failed to deserialize CDR document: {}", e))?;
            cdrs.push(document_to_cdr(doc)?);
        }

        debug!(count = cdrs.len(), "CDRs queried from MongoDB");
        Ok(cdrs)
    }

    async fn count_cdrs(&self, query: CdrQuery) -> Result<usize> {
        let mut filter = doc! {};

        // Tenant filter
        if let Some(tenant_id) = &query.tenant_id {
            filter.insert("tenant_id", tenant_id.to_string());
        }

        // Call ID filter
        if let Some(call_id) = &query.call_id {
            filter.insert("call_id", call_id.to_string());
        }

        // Time range filters
        if let Some(from) = query.start_time_from {
            filter.insert(
                "start_time",
                doc! { "$gte": system_time_to_bson_datetime(from) },
            );
        }
        if let Some(to) = query.start_time_to {
            let existing = filter.get("start_time");
            if let Some(bson::Bson::Document(mut time_filter)) = existing.cloned() {
                time_filter.insert("$lte", system_time_to_bson_datetime(to));
                filter.insert("start_time", time_filter);
            } else {
                filter.insert(
                    "start_time",
                    doc! { "$lte": system_time_to_bson_datetime(to) },
                );
            }
        }

        // From/To URI filters
        if let Some(from_uri) = &query.from_uri {
            filter.insert("from_uri", from_uri);
        }
        if let Some(to_uri) = &query.to_uri {
            filter.insert("to_uri", to_uri);
        }

        // Disposition filter
        if let Some(disposition) = &query.disposition {
            filter.insert("disposition", disposition_to_string(*disposition));
        }

        let count = self
            .cdrs
            .count_documents(filter)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to count CDRs: {}", e))?;

        Ok(count as usize)
    }
}

// ============================================================================
// TenantStore Implementation
// ============================================================================

#[async_trait]
impl TenantStore for MongodbStorage {
    async fn get_tenant(&self, id: &TenantId) -> Result<Option<Tenant>> {
        let filter = doc! {
            "id": id.to_string(),
        };

        let doc = self
            .tenants
            .find_one(filter)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to query tenant: {}", e))?;

        match doc {
            Some(doc) => {
                let tenant = document_to_tenant(doc)?;
                debug!(tenant_id = %id, "Tenant retrieved from MongoDB");
                Ok(Some(tenant))
            }
            None => {
                debug!(tenant_id = %id, "Tenant not found in MongoDB");
                Ok(None)
            }
        }
    }

    async fn get_tenant_by_name(&self, name: &str) -> Result<Option<Tenant>> {
        let filter = doc! {
            "name": name,
        };

        let doc = self
            .tenants
            .find_one(filter)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to query tenant by name: {}", e))?;

        match doc {
            Some(doc) => {
                let tenant = document_to_tenant(doc)?;
                debug!(name = %name, "Tenant retrieved from MongoDB");
                Ok(Some(tenant))
            }
            None => {
                debug!(name = %name, "Tenant not found in MongoDB");
                Ok(None)
            }
        }
    }

    async fn list_tenants(&self) -> Result<Vec<Tenant>> {
        let mut cursor = self
            .tenants
            .find(doc! {})
            .sort(doc! { "name": 1 })
            .await
            .map_err(|e| anyhow::anyhow!("Failed to list tenants: {}", e))?;

        let mut tenants = Vec::new();
        while cursor
            .advance()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to iterate tenants cursor: {}", e))?
        {
            let doc = cursor
                .deserialize_current()
                .map_err(|e| anyhow::anyhow!("Failed to deserialize tenant document: {}", e))?;
            tenants.push(document_to_tenant(doc)?);
        }

        debug!(count = tenants.len(), "Tenants listed from MongoDB");
        Ok(tenants)
    }

    async fn put_tenant(&self, tenant: Tenant) -> Result<()> {
        // Serialize status to snake_case string using serde
        let status_str = serde_json::to_string(&tenant.status)
            .map(|s| s.trim_matches('"').to_string())
            .unwrap_or_else(|_| "active".to_string());

        let doc = doc! {
            "id": tenant.id.to_string(),
            "name": &tenant.name,
            "description": tenant.description.as_ref(),
            "status": &status_str,
            "contact_email": tenant.contact_email.as_ref(),
            "contact_phone": tenant.contact_phone.as_ref(),
            "max_users": tenant.max_users.map(|v| v as i64),
            "max_phone_numbers": tenant.max_phone_numbers.map(|v| v as i64),
            "max_concurrent_calls": tenant.max_concurrent_calls.map(|v| v as i64),
            "billing_id": tenant.billing_id.as_ref(),
            "created_at": system_time_to_bson_datetime(tenant.created_at),
            "updated_at": system_time_to_bson_datetime(tenant.updated_at),
            "trial_expires_at": tenant.trial_expires_at.map(system_time_to_bson_datetime),
            "metadata": tenant.metadata.as_ref(),
        };

        let filter = doc! {
            "id": tenant.id.to_string(),
        };

        self.tenants
            .replace_one(filter, doc)
            .upsert(true)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to insert/update tenant: {}", e))?;

        debug!(tenant_id = %tenant.id, tenant_name = %tenant.name, "Tenant stored in MongoDB");
        Ok(())
    }

    async fn delete_tenant(&self, id: &TenantId) -> Result<()> {
        let filter = doc! {
            "id": id.to_string(),
        };

        self.tenants
            .delete_one(filter)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to delete tenant: {}", e))?;

        debug!(tenant_id = %id, "Tenant deleted from MongoDB");
        Ok(())
    }
}

// ============================================================================
// Document Conversion Helpers
// ============================================================================

/// Parse role string from database to UserRole enum
fn parse_role(role_str: &str) -> fcp_core::UserRole {
    use fcp_core::UserRole;

    // Try deserializing with serde first (handles snake_case like "telecom_admin")
    if let Ok(role) = serde_json::from_str::<UserRole>(&format!("\"{}\"", role_str)) {
        return role;
    }

    // Fallback to manual matching for backward compatibility
    match role_str.to_lowercase().as_str() {
        "superadmin" | "super_admin" => UserRole::SuperAdmin,
        "telecomadmin" | "telecom_admin" => UserRole::TelecomAdmin,
        "useradmin" | "user_admin" => UserRole::UserAdmin,
        "departmentadmin" | "department_admin" => UserRole::DepartmentAdmin,
        "readonly" | "read_only" => UserRole::ReadOnly,
        "enduser" | "end_user" => UserRole::EndUser,
        _ => {
            tracing::warn!("Unknown role '{}', defaulting to EndUser", role_str);
            UserRole::EndUser
        }
    }
}

fn document_to_user(doc: Document) -> Result<User> {
    let media_prefs = doc
        .get("media_preferences")
        .and_then(|v| bson::from_bson::<MediaPreferences>(v.clone()).ok())
        .unwrap_or_default();

    let created_at = doc
        .get_datetime("created_at")
        .ok()
        .map(|dt| bson_datetime_to_system_time(*dt))
        .unwrap_or_else(SystemTime::now);

    let updated_at = doc
        .get_datetime("updated_at")
        .ok()
        .map(|dt| bson_datetime_to_system_time(*dt))
        .unwrap_or_else(SystemTime::now);

    // Parse optional timestamp fields
    let password_last_reset = doc
        .get_datetime("password_last_reset")
        .ok()
        .map(|dt| bson_datetime_to_system_time(*dt));

    let last_login = doc
        .get_datetime("last_login")
        .ok()
        .map(|dt| bson_datetime_to_system_time(*dt));

    // Parse account status
    let account_status_str = doc.get_str("account_status").unwrap_or("active");
    let account_status =
        AccountStatus::from_str(account_status_str).unwrap_or(AccountStatus::Active);

    // Parse role from database (defaults to EndUser if not found or invalid)
    let role_str = doc.get_str("role").unwrap_or("enduser");
    let role = parse_role(role_str);

    // Parse emergency contact
    let emergency_contact = EmergencyContact {
        name: doc.get_str("emergency_contact_name").ok().map(String::from),
        phone: doc
            .get_str("emergency_contact_phone")
            .ok()
            .map(String::from),
    };

    // Parse or generate user ID
    let id = doc
        .get_str("id")
        .ok()
        .and_then(|s| fcp_core::UserId::from_str(s).ok())
        .unwrap_or_else(fcp_core::UserId::new);

    Ok(User {
        id,
        username: doc
            .get_str("username")
            .map_err(|_| anyhow::anyhow!("Missing username field"))?
            .to_string(),
        password: doc
            .get_str("password_hash")
            .map_err(|_| anyhow::anyhow!("Missing password_hash field"))?
            .to_string(),
        digest_password: doc.get_str("digest_password").ok().map(String::from),
        sip_ha1_hash: doc.get_str("sip_ha1_hash").ok().map(String::from),
        display_name: doc.get_str("display_name").ok().map(String::from),
        tenant_id: TenantId::from_str(
            doc.get_str("tenant_id")
                .map_err(|_| anyhow::anyhow!("Missing tenant_id field"))?,
        )
        .unwrap_or_else(|_| TenantId::nil()),
        enabled: doc.get_bool("enabled").unwrap_or(true),
        role,                                                               // User role for RBAC
        department_id: doc.get_str("department_id").ok().map(String::from), // Department ID for scoping
        created_at,
        updated_at,
        media_preferences: media_prefs,
        email: doc.get_str("email").ok().map(String::from),
        password_last_reset,
        phone_number: doc.get_str("phone_number").ok().map(String::from),
        mobile_number: doc.get_str("mobile_number").ok().map(String::from),
        pin: doc.get_str("pin").ok().map(String::from),
        extension: doc.get_str("extension").ok().map(String::from),
        department: doc.get_str("department").ok().map(String::from),
        job_title: doc.get_str("job_title").ok().map(String::from),
        location: doc.get_str("location").ok().map(String::from),
        language: doc.get_str("language").unwrap_or("en-US").to_string(),
        timezone: doc.get_str("timezone").unwrap_or("UTC").to_string(),
        account_status,
        last_login,
        emergency_contact,
        primary_phone_number_id: doc
            .get_str("primary_phone_number_id")
            .ok()
            .and_then(|s| PhoneNumberId::from_str(s).ok()),
        voicemail_box_id: doc
            .get_str("voicemail_box_id")
            .ok()
            .and_then(|s| VoicemailBoxId::from_str(s).ok()),
    })
}

fn document_to_registration(doc: Document) -> Result<Registration> {
    let expires_at = doc
        .get_datetime("expires_at")
        .ok()
        .map(|dt| bson_datetime_to_system_time(*dt))
        .unwrap_or_else(|| SystemTime::now() + Duration::from_secs(3600));

    let registered_at = doc
        .get_datetime("registered_at")
        .ok()
        .map(|dt| bson_datetime_to_system_time(*dt))
        .unwrap_or_else(SystemTime::now);

    Ok(Registration {
        aor: doc
            .get_str("aor")
            .map_err(|_| anyhow::anyhow!("Missing aor field"))?
            .to_string(),
        contact: doc
            .get_str("contact")
            .map_err(|_| anyhow::anyhow!("Missing contact field"))?
            .to_string(),
        expires_at,
        call_id: doc
            .get_str("call_id")
            .map_err(|_| anyhow::anyhow!("Missing call_id field"))?
            .to_string(),
        cseq: doc.get_i64("cseq").unwrap_or(1) as u32,
        user_agent: doc.get_str("user_agent").ok().map(String::from),
        q: doc.get_f64("q").ok().map(|q| q as f32),
        registered_at,
        path: doc.get_str("path").ok().map(String::from),
        flow_token: doc.get_str("flow_token").ok().map(String::from),
    })
}

fn document_to_tenant(doc: Document) -> Result<Tenant> {
    use fcp_core::TenantStatus;
    use std::str::FromStr;

    let created_at = doc
        .get_datetime("created_at")
        .ok()
        .map(|dt| bson_datetime_to_system_time(*dt))
        .unwrap_or_else(SystemTime::now);

    let updated_at = doc
        .get_datetime("updated_at")
        .ok()
        .map(|dt| bson_datetime_to_system_time(*dt))
        .unwrap_or_else(SystemTime::now);

    let trial_expires_at = doc
        .get_datetime("trial_expires_at")
        .ok()
        .map(|dt| bson_datetime_to_system_time(*dt));

    // Parse status from database (defaults to Active if not found or invalid)
    let status_str = doc.get_str("status").unwrap_or("active");
    let status = serde_json::from_str::<TenantStatus>(&format!("\"{}\"", status_str))
        .unwrap_or(TenantStatus::Active);

    Ok(Tenant {
        id: TenantId::from_str(
            doc.get_str("id")
                .map_err(|_| anyhow::anyhow!("Missing id field"))?,
        )
        .map_err(|_| anyhow::anyhow!("Invalid tenant id format"))?,
        name: doc
            .get_str("name")
            .map_err(|_| anyhow::anyhow!("Missing name field"))?
            .to_string(),
        description: doc.get_str("description").ok().map(String::from),
        status,
        contact_email: doc.get_str("contact_email").ok().map(String::from),
        contact_phone: doc.get_str("contact_phone").ok().map(String::from),
        max_users: doc.get_i64("max_users").ok().map(|v| v as u32),
        max_phone_numbers: doc.get_i64("max_phone_numbers").ok().map(|v| v as u32),
        max_concurrent_calls: doc.get_i64("max_concurrent_calls").ok().map(|v| v as u32),
        billing_id: doc.get_str("billing_id").ok().map(String::from),
        created_at,
        updated_at,
        trial_expires_at,
        metadata: doc.get_str("metadata").ok().map(String::from),
    })
}

fn document_to_cdr(doc: Document) -> Result<CallDetailRecord> {
    let custom_data = doc.get("custom_data").and_then(|v| {
        if v.as_null().is_some() {
            None
        } else {
            bson::from_bson::<serde_json::Value>(v.clone()).ok()
        }
    });

    let start_time = doc
        .get_datetime("start_time")
        .ok()
        .map(|dt| bson_datetime_to_system_time(*dt))
        .unwrap_or_else(SystemTime::now);

    let answer_time = doc
        .get_datetime("answer_time")
        .ok()
        .map(|dt| bson_datetime_to_system_time(*dt));

    let end_time = doc
        .get_datetime("end_time")
        .ok()
        .map(|dt| bson_datetime_to_system_time(*dt));

    let hangup_cause = doc
        .get("hangup_cause")
        .map(bson_to_hangup_cause)
        .unwrap_or(HangupCause::Other(0));

    let hangup_party = doc.get_str("hangup_party").ok().map(string_to_hangup_party);

    Ok(CallDetailRecord {
        call_id: CallId::from_str(
            doc.get_str("call_id")
                .map_err(|_| anyhow::anyhow!("Missing call_id field"))?,
        )
        .map_err(|_| anyhow::anyhow!("Invalid call_id format"))?,
        tenant_id: TenantId::from_str(
            doc.get_str("tenant_id")
                .map_err(|_| anyhow::anyhow!("Missing tenant_id field"))?,
        )
        .unwrap_or_else(|_| TenantId::nil()),
        sip_call_id: doc
            .get_str("sip_call_id")
            .map_err(|_| anyhow::anyhow!("Missing sip_call_id field"))?
            .to_string(),
        from_uri: doc
            .get_str("from_uri")
            .map_err(|_| anyhow::anyhow!("Missing from_uri field"))?
            .to_string(),
        from_display_name: doc.get_str("from_display_name").ok().map(String::from),
        to_uri: doc
            .get_str("to_uri")
            .map_err(|_| anyhow::anyhow!("Missing to_uri field"))?
            .to_string(),
        to_display_name: doc.get_str("to_display_name").ok().map(String::from),
        start_time,
        answer_time,
        end_time,
        setup_duration_ms: doc.get_i64("setup_duration_ms").ok().map(|d| d as u64),
        call_duration_ms: doc.get_i64("call_duration_ms").ok().map(|d| d as u64),
        total_duration_ms: doc.get_i64("total_duration_ms").ok().map(|d| d as u64),
        disposition: string_to_disposition(
            doc.get_str("disposition")
                .map_err(|_| anyhow::anyhow!("Missing disposition field"))?,
        ),
        hangup_cause,
        hangup_party,
        sip_status_code: doc.get_i32("sip_status_code").ok().map(|c| c as u16),
        sip_reason: doc.get_str("sip_reason").ok().map(String::from),
        route_id: doc.get_str("route_id").ok().map(String::from),
        gateway_id: doc.get_str("gateway_id").ok().map(String::from),
        media_anchored: doc.get_bool("media_anchored").unwrap_or(false),
        codec_a_leg: doc.get_str("codec_a_leg").ok().map(String::from),
        codec_b_leg: doc.get_str("codec_b_leg").ok().map(String::from),
        rtp_packets_sent: doc.get_i64("rtp_packets_sent").ok().map(|p| p as u64),
        rtp_packets_received: doc.get_i64("rtp_packets_received").ok().map(|p| p as u64),
        rtp_bytes_sent: doc.get_i64("rtp_bytes_sent").ok().map(|b| b as u64),
        rtp_bytes_received: doc.get_i64("rtp_bytes_received").ok().map(|b| b as u64),
        recording_url: doc.get_str("recording_url").ok().map(String::from),
        cost: doc.get_f64("cost").ok(),
        currency: doc.get_str("currency").ok().map(String::from),
        custom_data,
    })
}

// ============================================================================
// PhoneNumberStore Implementation
// ============================================================================

#[async_trait]
impl PhoneNumberStore for MongodbStorage {
    async fn get_phone_number(&self, id: &PhoneNumberId) -> Result<Option<PhoneNumber>> {
        let filter = doc! {
            "id": id.to_string(),
        };

        let doc = self
            .phone_numbers
            .find_one(filter)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to query phone number: {}", e))?;

        match doc {
            Some(doc) => {
                let phone_number = document_to_phone_number(doc)?;
                debug!(id = %id, "Phone number retrieved from MongoDB");
                Ok(Some(phone_number))
            }
            None => {
                debug!(id = %id, "Phone number not found in MongoDB");
                Ok(None)
            }
        }
    }

    async fn get_phone_number_by_e164(&self, e164: &str) -> Result<Option<PhoneNumber>> {
        let filter = doc! {
            "e164": e164,
        };

        let doc = self
            .phone_numbers
            .find_one(filter)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to query phone number by e164: {}", e))?;

        match doc {
            Some(doc) => {
                let phone_number = document_to_phone_number(doc)?;
                debug!(e164 = %e164, "Phone number retrieved from MongoDB");
                Ok(Some(phone_number))
            }
            None => {
                debug!(e164 = %e164, "Phone number not found in MongoDB");
                Ok(None)
            }
        }
    }

    async fn list_phone_numbers(&self, tenant_id: &TenantId) -> Result<Vec<PhoneNumber>> {
        let filter = doc! {
            "tenant_id": tenant_id.to_string(),
        };

        let mut cursor = self
            .phone_numbers
            .find(filter)
            .sort(doc! { "e164": 1 })
            .await
            .map_err(|e| anyhow::anyhow!("Failed to list phone numbers: {}", e))?;

        let mut phone_numbers = Vec::new();
        while cursor
            .advance()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to iterate phone numbers cursor: {}", e))?
        {
            let doc = cursor.deserialize_current().map_err(|e| {
                anyhow::anyhow!("Failed to deserialize phone number document: {}", e)
            })?;
            phone_numbers.push(document_to_phone_number(doc)?);
        }

        debug!(
            tenant_id = %tenant_id,
            count = phone_numbers.len(),
            "Phone numbers listed from MongoDB"
        );
        Ok(phone_numbers)
    }

    async fn put_phone_number(&self, phone_number: PhoneNumber) -> Result<()> {
        // Serialize state to snake_case string
        let state_str = match phone_number.state {
            NumberState::Unassigned => "unassigned",
            NumberState::Reserved => "reserved",
            NumberState::Active => "active",
            NumberState::Quarantine => "quarantine",
            NumberState::Ported => "ported",
        };

        // Serialize e911_address as nested document
        let e911_address = if let Some(addr) = &phone_number.e911_address {
            bson::to_bson(addr)
                .map_err(|e| anyhow::anyhow!("Failed to serialize e911_address: {}", e))?
        } else {
            bson::Bson::Null
        };

        // Serialize lists
        let allow_list = bson::to_bson(&phone_number.allow_list)
            .map_err(|e| anyhow::anyhow!("Failed to serialize allow_list: {}", e))?;
        let block_list = bson::to_bson(&phone_number.block_list)
            .map_err(|e| anyhow::anyhow!("Failed to serialize block_list: {}", e))?;
        let tags = bson::to_bson(&phone_number.tags)
            .map_err(|e| anyhow::anyhow!("Failed to serialize tags: {}", e))?;

        let doc = doc! {
            "id": phone_number.id.to_string(),
            "e164": &phone_number.e164,
            "country_code": &phone_number.country_code,
            "npa": &phone_number.npa,
            "nxx": &phone_number.nxx,
            "line": &phone_number.line,
            "state": state_str,
            "tenant_id": phone_number.tenant_id.to_string(),
            "e911_address": e911_address,
            "e911_callback": phone_number.e911_callback.as_ref(),
            "cnam": phone_number.cnam.as_ref(),
            "spam_score": phone_number.spam_score,
            "carrier": phone_number.carrier.as_ref(),
            "lrn": phone_number.lrn.as_ref(),
            "ocn": phone_number.ocn.as_ref(),
            "rate_center": phone_number.rate_center.as_ref(),
            "lata": phone_number.lata.as_ref(),
            "acquired_at": system_time_to_bson_datetime(phone_number.acquired_at),
            "activated_at": phone_number.activated_at.map(system_time_to_bson_datetime),
            "ported_at": phone_number.ported_at.map(system_time_to_bson_datetime),
            "loa_date": phone_number.loa_date.map(system_time_to_bson_datetime),
            "foc_date": phone_number.foc_date.map(system_time_to_bson_datetime),
            "recording_enabled": phone_number.recording_enabled,
            "block_anonymous": phone_number.block_anonymous,
            "allow_list": allow_list,
            "block_list": block_list,
            "tags": tags,
            "created_at": system_time_to_bson_datetime(phone_number.created_at),
            "updated_at": system_time_to_bson_datetime(phone_number.updated_at),
        };

        let filter = doc! {
            "id": phone_number.id.to_string(),
        };

        self.phone_numbers
            .replace_one(filter, doc)
            .upsert(true)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to insert/update phone number: {}", e))?;

        debug!(id = %phone_number.id, e164 = %phone_number.e164, "Phone number stored in MongoDB");
        Ok(())
    }

    async fn delete_phone_number(&self, id: &PhoneNumberId) -> Result<()> {
        let filter = doc! {
            "id": id.to_string(),
        };

        self.phone_numbers
            .delete_one(filter)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to delete phone number: {}", e))?;

        debug!(id = %id, "Phone number deleted from MongoDB");
        Ok(())
    }
}

// ============================================================================
// LineBindingStore Implementation
// ============================================================================

impl MongodbStorage {
    /// Get device bindings for a line binding
    async fn get_device_bindings(
        &self,
        line_binding_id: &LineBindingId,
    ) -> Result<Vec<DeviceBinding>> {
        let filter = doc! {
            "line_binding_id": line_binding_id.to_string(),
        };

        let mut cursor = self
            .device_bindings
            .find(filter)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to query device bindings: {}", e))?;

        let mut bindings = Vec::new();
        while cursor
            .advance()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to iterate device bindings cursor: {}", e))?
        {
            let doc = cursor.deserialize_current().map_err(|e| {
                anyhow::anyhow!("Failed to deserialize device binding document: {}", e)
            })?;
            bindings.push(document_to_device_binding(doc)?);
        }

        Ok(bindings)
    }

    /// Store a device binding
    async fn put_device_binding(
        &self,
        line_binding_id: &LineBindingId,
        binding: &DeviceBinding,
    ) -> Result<()> {
        let doc = doc! {
            "line_binding_id": line_binding_id.to_string(),
            "device_id": binding.device_id.to_string(),
            "priority": binding.priority as i32,
            "line_appearance": binding.line_appearance as i32,
            "shared_line": binding.shared_line,
            "enabled": binding.enabled,
        };

        let filter = doc! {
            "line_binding_id": line_binding_id.to_string(),
            "device_id": binding.device_id.to_string(),
        };

        self.device_bindings
            .replace_one(filter, doc)
            .upsert(true)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to insert/update device binding: {}", e))?;

        Ok(())
    }
}

#[async_trait]
impl LineBindingStore for MongodbStorage {
    async fn get_line_binding(&self, id: &LineBindingId) -> Result<Option<LineBinding>> {
        let filter = doc! {
            "id": id.to_string(),
        };

        let doc = self
            .line_bindings
            .find_one(filter)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to query line binding: {}", e))?;

        match doc {
            Some(doc) => {
                let mut binding = document_to_line_binding(doc)?;
                binding.devices = self.get_device_bindings(id).await?;
                debug!(id = %id, "Line binding retrieved from MongoDB");
                Ok(Some(binding))
            }
            None => {
                debug!(id = %id, "Line binding not found in MongoDB");
                Ok(None)
            }
        }
    }

    async fn get_line_binding_by_phone_number(
        &self,
        phone_number_id: &PhoneNumberId,
    ) -> Result<Option<LineBinding>> {
        let filter = doc! {
            "phone_number_id": phone_number_id.to_string(),
        };

        let doc =
            self.line_bindings.find_one(filter).await.map_err(|e| {
                anyhow::anyhow!("Failed to query line binding by phone number: {}", e)
            })?;

        match doc {
            Some(doc) => {
                let mut binding = document_to_line_binding(doc)?;
                binding.devices = self.get_device_bindings(&binding.id).await?;
                debug!(phone_number_id = %phone_number_id, "Line binding retrieved from MongoDB");
                Ok(Some(binding))
            }
            None => {
                debug!(phone_number_id = %phone_number_id, "Line binding not found in MongoDB");
                Ok(None)
            }
        }
    }

    async fn list_line_bindings(&self, tenant_id: &TenantId) -> Result<Vec<LineBinding>> {
        let filter = doc! {
            "tenant_id": tenant_id.to_string(),
        };

        let mut cursor = self
            .line_bindings
            .find(filter)
            .sort(doc! { "created_at": 1 })
            .await
            .map_err(|e| anyhow::anyhow!("Failed to list line bindings: {}", e))?;

        let mut bindings = Vec::new();
        while cursor
            .advance()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to iterate line bindings cursor: {}", e))?
        {
            let doc = cursor.deserialize_current().map_err(|e| {
                anyhow::anyhow!("Failed to deserialize line binding document: {}", e)
            })?;
            let mut binding = document_to_line_binding(doc)?;
            binding.devices = self.get_device_bindings(&binding.id).await?;
            bindings.push(binding);
        }

        debug!(
            tenant_id = %tenant_id,
            count = bindings.len(),
            "Line bindings listed from MongoDB"
        );
        Ok(bindings)
    }

    async fn list_line_bindings_by_user(&self, user_id: &UserId) -> Result<Vec<LineBinding>> {
        let filter = doc! {
            "owner_user_id": user_id.to_string(),
        };

        let mut cursor = self
            .line_bindings
            .find(filter)
            .sort(doc! { "created_at": 1 })
            .await
            .map_err(|e| anyhow::anyhow!("Failed to list line bindings by user: {}", e))?;

        let mut bindings = Vec::new();
        while cursor
            .advance()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to iterate line bindings cursor: {}", e))?
        {
            let doc = cursor.deserialize_current().map_err(|e| {
                anyhow::anyhow!("Failed to deserialize line binding document: {}", e)
            })?;
            let mut binding = document_to_line_binding(doc)?;
            binding.devices = self.get_device_bindings(&binding.id).await?;
            bindings.push(binding);
        }

        debug!(
            user_id = %user_id,
            count = bindings.len(),
            "Line bindings listed from MongoDB"
        );
        Ok(bindings)
    }

    async fn list_line_bindings_by_device(&self, device_id: &DeviceId) -> Result<Vec<LineBinding>> {
        // First find all line_binding_ids that have this device
        let filter = doc! {
            "device_id": device_id.to_string(),
        };

        let mut cursor = self
            .device_bindings
            .find(filter)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to query device bindings: {}", e))?;

        let mut line_binding_ids = Vec::new();
        while cursor
            .advance()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to iterate device bindings cursor: {}", e))?
        {
            let doc = cursor.deserialize_current().map_err(|e| {
                anyhow::anyhow!("Failed to deserialize device binding document: {}", e)
            })?;
            if let Ok(line_binding_id) = doc.get_str("line_binding_id") {
                line_binding_ids.push(line_binding_id.to_string());
            }
        }

        // Now fetch all line bindings
        let mut bindings = Vec::new();
        for line_binding_id_str in line_binding_ids {
            if let Ok(line_binding_id) = LineBindingId::from_str(&line_binding_id_str) {
                if let Some(binding) = self.get_line_binding(&line_binding_id).await? {
                    bindings.push(binding);
                }
            }
        }

        debug!(
            device_id = %device_id,
            count = bindings.len(),
            "Line bindings listed from MongoDB"
        );
        Ok(bindings)
    }

    async fn put_line_binding(&self, binding: LineBinding) -> Result<()> {
        // Serialize binding_type to snake_case string
        let binding_type_str = match binding.binding_type {
            BindingType::User => "user",
            BindingType::Ivr => "ivr",
            BindingType::Queue => "queue",
            BindingType::HuntGroup => "hunt_group",
            BindingType::ConferenceBridge => "conference_bridge",
            BindingType::Voicemail => "voicemail",
            BindingType::Parking => "parking",
            BindingType::Paging => "paging",
            BindingType::Fax => "fax",
            BindingType::Emergency => "emergency",
        };

        // Serialize ring_strategy to snake_case string
        let ring_strategy_str = match binding.ring_strategy {
            RingStrategy::Simultaneous => "simultaneous",
            RingStrategy::Sequential => "sequential",
            RingStrategy::RoundRobin => "round_robin",
            RingStrategy::LongestIdle => "longest_idle",
            RingStrategy::Primary => "primary",
        };

        let doc = doc! {
            "id": binding.id.to_string(),
            "phone_number_id": binding.phone_number_id.to_string(),
            "owner_user_id": binding.owner_user_id.as_ref().map(|u| u.to_string()),
            "binding_type": binding_type_str,
            "ring_strategy": ring_strategy_str,
            "tenant_id": binding.tenant_id.to_string(),
            "no_answer_timeout_secs": binding.no_answer_timeout.as_secs() as i64,
            "forward_on_busy": binding.forward_on_busy.as_ref(),
            "forward_on_no_answer": binding.forward_on_no_answer.as_ref(),
            "forward_unconditional": binding.forward_unconditional.as_ref(),
            "outbound_caller_id": binding.outbound_caller_id.as_ref(),
            "outbound_cnam": binding.outbound_cnam.as_ref(),
            "created_at": system_time_to_bson_datetime(binding.created_at),
            "updated_at": system_time_to_bson_datetime(binding.updated_at),
        };

        let filter = doc! {
            "id": binding.id.to_string(),
        };

        self.line_bindings
            .replace_one(filter, doc)
            .upsert(true)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to insert/update line binding: {}", e))?;

        // Delete existing device bindings
        let delete_filter = doc! {
            "line_binding_id": binding.id.to_string(),
        };
        self.device_bindings
            .delete_many(delete_filter)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to delete old device bindings: {}", e))?;

        // Insert new device bindings
        for device in &binding.devices {
            self.put_device_binding(&binding.id, device).await?;
        }

        debug!(id = %binding.id, "Line binding stored in MongoDB");
        Ok(())
    }

    async fn delete_line_binding(&self, id: &LineBindingId) -> Result<()> {
        let filter = doc! {
            "id": id.to_string(),
        };

        self.line_bindings
            .delete_one(filter)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to delete line binding: {}", e))?;

        // Also delete device bindings
        let device_filter = doc! {
            "line_binding_id": id.to_string(),
        };
        self.device_bindings
            .delete_many(device_filter)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to delete device bindings: {}", e))?;

        debug!(id = %id, "Line binding deleted from MongoDB");
        Ok(())
    }
}

// ============================================================================
// ExtensionStore Implementation
// ============================================================================

#[async_trait]
impl ExtensionStore for MongodbStorage {
    async fn get_extension(&self, id: &ExtensionId) -> Result<Option<Extension>> {
        let filter = doc! {
            "id": id.to_string(),
        };

        let doc = self
            .extensions
            .find_one(filter)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to query extension: {}", e))?;

        match doc {
            Some(doc) => {
                let extension = document_to_extension(doc)?;
                debug!(id = %id, "Extension retrieved from MongoDB");
                Ok(Some(extension))
            }
            None => {
                debug!(id = %id, "Extension not found in MongoDB");
                Ok(None)
            }
        }
    }

    async fn get_extension_by_number(
        &self,
        tenant_id: &TenantId,
        number: &str,
    ) -> Result<Option<Extension>> {
        let filter = doc! {
            "tenant_id": tenant_id.to_string(),
            "number": number,
        };

        let doc = self
            .extensions
            .find_one(filter)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to query extension by number: {}", e))?;

        match doc {
            Some(doc) => {
                let extension = document_to_extension(doc)?;
                debug!(tenant_id = %tenant_id, number = %number, "Extension retrieved from MongoDB");
                Ok(Some(extension))
            }
            None => {
                debug!(tenant_id = %tenant_id, number = %number, "Extension not found in MongoDB");
                Ok(None)
            }
        }
    }

    async fn list_extensions(&self, tenant_id: &TenantId) -> Result<Vec<Extension>> {
        let filter = doc! {
            "tenant_id": tenant_id.to_string(),
        };

        let mut cursor = self
            .extensions
            .find(filter)
            .sort(doc! { "number": 1 })
            .await
            .map_err(|e| anyhow::anyhow!("Failed to list extensions: {}", e))?;

        let mut extensions = Vec::new();
        while cursor
            .advance()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to iterate extensions cursor: {}", e))?
        {
            let doc = cursor
                .deserialize_current()
                .map_err(|e| anyhow::anyhow!("Failed to deserialize extension document: {}", e))?;
            extensions.push(document_to_extension(doc)?);
        }

        debug!(
            tenant_id = %tenant_id,
            count = extensions.len(),
            "Extensions listed from MongoDB"
        );
        Ok(extensions)
    }

    async fn put_extension(&self, extension: Extension) -> Result<()> {
        let (target_type, target_id) = match &extension.target {
            ExtensionTarget::User(user_id) => ("user", user_id.to_string()),
            ExtensionTarget::PhoneNumber(num_id) => ("phone_number", num_id.to_string()),
            ExtensionTarget::Device(dev_id) => ("device", dev_id.to_string()),
            ExtensionTarget::Service(svc_id) => ("service", svc_id.to_string()),
        };

        let doc = doc! {
            "id": extension.id.to_string(),
            "number": &extension.number,
            "target_type": target_type,
            "target_id": &target_id,
            "tenant_id": extension.tenant_id.to_string(),
            "description": extension.description.as_ref(),
            "enabled": extension.enabled,
            "created_at": system_time_to_bson_datetime(extension.created_at),
            "updated_at": system_time_to_bson_datetime(extension.updated_at),
        };

        let filter = doc! {
            "id": extension.id.to_string(),
        };

        self.extensions
            .replace_one(filter, doc)
            .upsert(true)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to insert/update extension: {}", e))?;

        debug!(id = %extension.id, number = %extension.number, "Extension stored in MongoDB");
        Ok(())
    }

    async fn delete_extension(&self, id: &ExtensionId) -> Result<()> {
        let filter = doc! {
            "id": id.to_string(),
        };

        self.extensions
            .delete_one(filter)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to delete extension: {}", e))?;

        debug!(id = %id, "Extension deleted from MongoDB");
        Ok(())
    }
}

// ============================================================================
// DeviceStore Implementation
// ============================================================================

#[async_trait]
impl DeviceStore for MongodbStorage {
    async fn get_device(&self, id: &DeviceId) -> Result<Option<Device>> {
        let filter = doc! {
            "id": id.to_string(),
        };

        let doc = self
            .devices
            .find_one(filter)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to query device: {}", e))?;

        match doc {
            Some(doc) => {
                let device = document_to_device(doc)?;
                debug!(id = %id, "Device retrieved from MongoDB");
                Ok(Some(device))
            }
            None => {
                debug!(id = %id, "Device not found in MongoDB");
                Ok(None)
            }
        }
    }

    async fn get_device_by_sip_uri(&self, sip_uri: &str) -> Result<Option<Device>> {
        let filter = doc! {
            "sip_uri": sip_uri,
        };

        let doc = self
            .devices
            .find_one(filter)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to query device by sip_uri: {}", e))?;

        match doc {
            Some(doc) => {
                let device = document_to_device(doc)?;
                debug!(sip_uri = %sip_uri, "Device retrieved from MongoDB");
                Ok(Some(device))
            }
            None => {
                debug!(sip_uri = %sip_uri, "Device not found in MongoDB");
                Ok(None)
            }
        }
    }

    async fn list_devices(&self, tenant_id: &TenantId) -> Result<Vec<Device>> {
        let filter = doc! {
            "tenant_id": tenant_id.to_string(),
        };

        let mut cursor = self
            .devices
            .find(filter)
            .sort(doc! { "name": 1 })
            .await
            .map_err(|e| anyhow::anyhow!("Failed to list devices: {}", e))?;

        let mut devices = Vec::new();
        while cursor
            .advance()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to iterate devices cursor: {}", e))?
        {
            let doc = cursor
                .deserialize_current()
                .map_err(|e| anyhow::anyhow!("Failed to deserialize device document: {}", e))?;
            devices.push(document_to_device(doc)?);
        }

        debug!(
            tenant_id = %tenant_id,
            count = devices.len(),
            "Devices listed from MongoDB"
        );
        Ok(devices)
    }

    async fn list_devices_by_user(&self, user_id: &UserId) -> Result<Vec<Device>> {
        let filter = doc! {
            "owner_user_id": user_id.to_string(),
        };

        let mut cursor = self
            .devices
            .find(filter)
            .sort(doc! { "name": 1 })
            .await
            .map_err(|e| anyhow::anyhow!("Failed to list devices by user: {}", e))?;

        let mut devices = Vec::new();
        while cursor
            .advance()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to iterate devices cursor: {}", e))?
        {
            let doc = cursor
                .deserialize_current()
                .map_err(|e| anyhow::anyhow!("Failed to deserialize device document: {}", e))?;
            devices.push(document_to_device(doc)?);
        }

        debug!(
            user_id = %user_id,
            count = devices.len(),
            "Devices listed from MongoDB"
        );
        Ok(devices)
    }

    async fn put_device(&self, device: Device) -> Result<()> {
        // Serialize device_type to snake_case string
        let device_type_str = match device.device_type {
            DeviceType::DeskPhone => "desk_phone",
            DeviceType::Softphone => "softphone",
            DeviceType::WebRtc => "webrtc",
            DeviceType::ConferenceRoom => "conference_room",
            DeviceType::Ata => "ata",
            DeviceType::Gateway => "gateway",
            DeviceType::Mobile => "mobile",
            DeviceType::Other => "other",
        };

        // Serialize registration_state to snake_case string
        let registration_state_str = match device.registration_state {
            RegistrationState::Unregistered => "unregistered",
            RegistrationState::Registered => "registered",
            RegistrationState::Failed => "failed",
            RegistrationState::Expired => "expired",
        };

        // Serialize codec_support as array of strings
        let codec_support: Vec<String> = device
            .codec_support
            .iter()
            .map(|codec| match codec {
                AudioCodec::Pcmu => "pcmu".to_string(),
                AudioCodec::Pcma => "pcma".to_string(),
                AudioCodec::G722 => "g722".to_string(),
                AudioCodec::G729 => "g729".to_string(),
                AudioCodec::Opus => "opus".to_string(),
                AudioCodec::Ilbc => "ilbc".to_string(),
                AudioCodec::Gsm => "gsm".to_string(),
                AudioCodec::Speex => "speex".to_string(),
            })
            .collect();

        let codec_support_bson = bson::to_bson(&codec_support)
            .map_err(|e| anyhow::anyhow!("Failed to serialize codec_support: {}", e))?;

        // Serialize e911_override as nested document
        let e911_override = if let Some(addr) = &device.e911_override {
            bson::to_bson(addr)
                .map_err(|e| anyhow::anyhow!("Failed to serialize e911_override: {}", e))?
        } else {
            bson::Bson::Null
        };

        let doc = doc! {
            "id": device.id.to_string(),
            "name": &device.name,
            "device_type": device_type_str,
            "mac_address": device.mac_address.as_ref(),
            "model": device.model.as_ref(),
            "firmware_version": device.firmware_version.as_ref(),
            "tenant_id": device.tenant_id.to_string(),
            "owner_user_id": device.owner_user_id.as_ref().map(|u| u.to_string()),
            "sip_uri": device.sip_uri.as_ref(),
            "registration_state": registration_state_str,
            "last_registration": device.last_registration.map(system_time_to_bson_datetime),
            "last_ip_address": device.last_ip_address.as_ref(),
            "user_agent": device.user_agent.as_ref(),
            "max_lines": device.max_lines as i32,
            "supports_video": device.supports_video,
            "supports_blf": device.supports_blf,
            "codec_support": codec_support_bson,
            "location": device.location.as_ref(),
            "e911_override": e911_override,
            "provisioning_url": device.provisioning_url.as_ref(),
            "config_template_id": device.config_template_id.as_ref(),
            "enabled": device.enabled,
            "created_at": system_time_to_bson_datetime(device.created_at),
            "updated_at": system_time_to_bson_datetime(device.updated_at),
        };

        let filter = doc! {
            "id": device.id.to_string(),
        };

        self.devices
            .replace_one(filter, doc)
            .upsert(true)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to insert/update device: {}", e))?;

        debug!(id = %device.id, name = %device.name, "Device stored in MongoDB");
        Ok(())
    }

    async fn delete_device(&self, id: &DeviceId) -> Result<()> {
        let filter = doc! {
            "id": id.to_string(),
        };

        self.devices
            .delete_one(filter)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to delete device: {}", e))?;

        debug!(id = %id, "Device deleted from MongoDB");
        Ok(())
    }
}

#[async_trait]
impl ConferenceStore for MongodbStorage {
    async fn get_meeting(
        &self,
        id: &fcp_core::MeetingId,
    ) -> Result<Option<fcp_core::Meeting>> {
        let filter = doc! { "id": id.to_string() };
        let result = self
            .meetings
            .find_one(filter)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to get meeting: {}", e))?;

        match result {
            Some(doc) => Ok(Some(document_to_meeting(doc)?)),
            None => Ok(None),
        }
    }

    async fn get_meeting_by_room_number(
        &self,
        tenant_id: &TenantId,
        room_number: &str,
    ) -> Result<Option<fcp_core::Meeting>> {
        let filter = doc! {
            "tenant_id": tenant_id.to_string(),
            "room_number": room_number,
        };
        let result = self
            .meetings
            .find_one(filter)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to get meeting by room number: {}", e))?;

        match result {
            Some(doc) => Ok(Some(document_to_meeting(doc)?)),
            None => Ok(None),
        }
    }

    async fn list_meetings(&self, tenant_id: &TenantId) -> Result<Vec<fcp_core::Meeting>> {
        let filter = doc! { "tenant_id": tenant_id.to_string() };
        let mut cursor = self
            .meetings
            .find(filter)
            .sort(doc! { "room_number": 1 })
            .await
            .map_err(|e| anyhow::anyhow!("Failed to list meetings: {}", e))?;

        let mut meetings = Vec::new();
        while cursor.advance().await.map_err(|e| anyhow::anyhow!("Cursor error: {}", e))? {
            let doc = cursor.deserialize_current()
                .map_err(|e| anyhow::anyhow!("Deserialize error: {}", e))?;
            if let Ok(meeting) = document_to_meeting(doc) {
                meetings.push(meeting);
            }
        }

        Ok(meetings)
    }

    async fn put_meeting(&self, meeting: fcp_core::Meeting) -> Result<()> {
        let created_at = system_time_to_bson_datetime(meeting.created_at);
        let updated_at = system_time_to_bson_datetime(SystemTime::now());

        let doc = doc! {
            "id": meeting.id.to_string(),
            "tenant_id": meeting.tenant_id.to_string(),
            "room_number": &meeting.room_number,
            "name": &meeting.name,
            "guest_pin": meeting.guest_pin.as_deref(),
            "host_pin": meeting.host_pin.as_deref(),
            "require_guest_pin": meeting.require_guest_pin,
            "max_participants": meeting.max_participants as i64,
            "wait_for_moderator": meeting.wait_for_moderator,
            "enabled": meeting.enabled,
            "auto_record": meeting.auto_record,
            "default_locked": meeting.default_locked,
            "permanent": meeting.permanent,
            "created_at": created_at,
            "updated_at": updated_at,
        };

        let filter = doc! { "id": meeting.id.to_string() };
        self.meetings
            .replace_one(filter, doc)
            .upsert(true)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to store meeting: {}", e))?;

        debug!(id = %meeting.id, "Meeting stored in MongoDB");
        Ok(())
    }

    async fn delete_meeting(&self, id: &fcp_core::MeetingId) -> Result<()> {
        let filter = doc! { "id": id.to_string() };
        self.meetings
            .delete_one(filter)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to delete meeting: {}", e))?;

        debug!(id = %id, "Meeting deleted from MongoDB");
        Ok(())
    }
}

// ============================================================================
// Document Conversion Helpers for New Types
// ============================================================================

fn document_to_meeting(doc: Document) -> Result<fcp_core::Meeting> {
    let id = fcp_core::MeetingId::from_str(
        doc.get_str("id")
            .map_err(|_| anyhow::anyhow!("Missing id field"))?,
    )
    .map_err(|e| anyhow::anyhow!("Invalid MeetingId: {}", e))?;

    let tenant_id = TenantId::from_str(
        doc.get_str("tenant_id")
            .map_err(|_| anyhow::anyhow!("Missing tenant_id field"))?,
    )
    .map_err(|e| anyhow::anyhow!("Invalid TenantId: {}", e))?;

    let room_number = doc
        .get_str("room_number")
        .map_err(|_| anyhow::anyhow!("Missing room_number field"))?
        .to_string();

    let name = doc
        .get_str("name")
        .map_err(|_| anyhow::anyhow!("Missing name field"))?
        .to_string();

    let guest_pin = doc.get_str("guest_pin").ok().map(|s| s.to_string());
    let host_pin = doc.get_str("host_pin").ok().map(|s| s.to_string());

    let require_guest_pin = doc.get_bool("require_guest_pin").unwrap_or(false);
    let max_participants = doc.get_i64("max_participants").unwrap_or(0) as usize;
    let wait_for_moderator = doc.get_bool("wait_for_moderator").unwrap_or(false);
    let enabled = doc.get_bool("enabled").unwrap_or(true);
    let auto_record = doc.get_bool("auto_record").unwrap_or(false);
    let default_locked = doc.get_bool("default_locked").unwrap_or(false);
    let permanent = doc.get_bool("permanent").unwrap_or(false);

    let created_at = doc
        .get_datetime("created_at")
        .map(|dt| bson_datetime_to_system_time(*dt))
        .unwrap_or(SystemTime::now());

    let updated_at = doc
        .get_datetime("updated_at")
        .map(|dt| bson_datetime_to_system_time(*dt))
        .unwrap_or(SystemTime::now());

    Ok(fcp_core::Meeting {
        id,
        tenant_id,
        room_number,
        name,
        guest_pin,
        host_pin,
        require_guest_pin,
        max_participants,
        wait_for_moderator,
        enabled,
        auto_record,
        default_locked,
        permanent,
        created_at,
        updated_at,
    })
}

fn document_to_phone_number(doc: Document) -> Result<PhoneNumber> {
    let id = PhoneNumberId::from_str(
        doc.get_str("id")
            .map_err(|_| anyhow::anyhow!("Missing id field"))?,
    )?;
    let tenant_id = TenantId::from_str(
        doc.get_str("tenant_id")
            .map_err(|_| anyhow::anyhow!("Missing tenant_id field"))?,
    )?;

    let state_str = doc
        .get_str("state")
        .map_err(|_| anyhow::anyhow!("Missing state field"))?;
    let state = match state_str {
        "unassigned" => NumberState::Unassigned,
        "reserved" => NumberState::Reserved,
        "active" => NumberState::Active,
        "quarantine" => NumberState::Quarantine,
        "ported" => NumberState::Ported,
        _ => NumberState::Unassigned,
    };

    let e911_address = doc.get("e911_address").and_then(|v| {
        if v.as_null().is_some() {
            None
        } else {
            bson::from_bson(v.clone()).ok()
        }
    });

    let allow_list: Vec<String> = doc
        .get("allow_list")
        .and_then(|v| bson::from_bson(v.clone()).ok())
        .unwrap_or_default();

    let block_list: Vec<String> = doc
        .get("block_list")
        .and_then(|v| bson::from_bson(v.clone()).ok())
        .unwrap_or_default();

    let tags: Vec<String> = doc
        .get("tags")
        .and_then(|v| bson::from_bson(v.clone()).ok())
        .unwrap_or_default();

    let created_at = doc
        .get_datetime("created_at")
        .ok()
        .map(|dt| bson_datetime_to_system_time(*dt))
        .unwrap_or_else(SystemTime::now);

    let updated_at = doc
        .get_datetime("updated_at")
        .ok()
        .map(|dt| bson_datetime_to_system_time(*dt))
        .unwrap_or_else(SystemTime::now);

    let acquired_at = doc
        .get_datetime("acquired_at")
        .ok()
        .map(|dt| bson_datetime_to_system_time(*dt))
        .unwrap_or_else(SystemTime::now);

    let activated_at = doc
        .get_datetime("activated_at")
        .ok()
        .map(|dt| bson_datetime_to_system_time(*dt));

    let ported_at = doc
        .get_datetime("ported_at")
        .ok()
        .map(|dt| bson_datetime_to_system_time(*dt));

    let loa_date = doc
        .get_datetime("loa_date")
        .ok()
        .map(|dt| bson_datetime_to_system_time(*dt));

    let foc_date = doc
        .get_datetime("foc_date")
        .ok()
        .map(|dt| bson_datetime_to_system_time(*dt));

    Ok(PhoneNumber {
        id,
        e164: doc
            .get_str("e164")
            .map_err(|_| anyhow::anyhow!("Missing e164 field"))?
            .to_string(),
        country_code: doc
            .get_str("country_code")
            .map_err(|_| anyhow::anyhow!("Missing country_code field"))?
            .to_string(),
        npa: doc
            .get_str("npa")
            .map_err(|_| anyhow::anyhow!("Missing npa field"))?
            .to_string(),
        nxx: doc
            .get_str("nxx")
            .map_err(|_| anyhow::anyhow!("Missing nxx field"))?
            .to_string(),
        line: doc
            .get_str("line")
            .map_err(|_| anyhow::anyhow!("Missing line field"))?
            .to_string(),
        state,
        tenant_id,
        e911_address,
        e911_callback: doc.get_str("e911_callback").ok().map(String::from),
        cnam: doc.get_str("cnam").ok().map(String::from),
        spam_score: doc.get_f64("spam_score").ok().map(|v| v as f32),
        carrier: doc.get_str("carrier").ok().map(String::from),
        lrn: doc.get_str("lrn").ok().map(String::from),
        ocn: doc.get_str("ocn").ok().map(String::from),
        rate_center: doc.get_str("rate_center").ok().map(String::from),
        lata: doc.get_str("lata").ok().map(String::from),
        acquired_at,
        activated_at,
        ported_at,
        loa_date,
        foc_date,
        recording_enabled: doc.get_bool("recording_enabled").unwrap_or(false),
        block_anonymous: doc.get_bool("block_anonymous").unwrap_or(false),
        allow_list,
        block_list,
        tags,
        created_at,
        updated_at,
    })
}

fn document_to_line_binding(doc: Document) -> Result<LineBinding> {
    let id = LineBindingId::from_str(
        doc.get_str("id")
            .map_err(|_| anyhow::anyhow!("Missing id field"))?,
    )?;
    let phone_number_id = PhoneNumberId::from_str(
        doc.get_str("phone_number_id")
            .map_err(|_| anyhow::anyhow!("Missing phone_number_id field"))?,
    )?;
    let tenant_id = TenantId::from_str(
        doc.get_str("tenant_id")
            .map_err(|_| anyhow::anyhow!("Missing tenant_id field"))?,
    )?;

    let owner_user_id = doc
        .get_str("owner_user_id")
        .ok()
        .and_then(|s| UserId::from_str(s).ok());

    let binding_type_str = doc
        .get_str("binding_type")
        .map_err(|_| anyhow::anyhow!("Missing binding_type field"))?;
    let binding_type = match binding_type_str {
        "user" => BindingType::User,
        "ivr" => BindingType::Ivr,
        "queue" => BindingType::Queue,
        "hunt_group" => BindingType::HuntGroup,
        "conference_bridge" => BindingType::ConferenceBridge,
        "voicemail" => BindingType::Voicemail,
        "parking" => BindingType::Parking,
        "paging" => BindingType::Paging,
        "fax" => BindingType::Fax,
        "emergency" => BindingType::Emergency,
        _ => BindingType::User,
    };

    let ring_strategy_str = doc
        .get_str("ring_strategy")
        .map_err(|_| anyhow::anyhow!("Missing ring_strategy field"))?;
    let ring_strategy = match ring_strategy_str {
        "simultaneous" => RingStrategy::Simultaneous,
        "sequential" => RingStrategy::Sequential,
        "round_robin" => RingStrategy::RoundRobin,
        "longest_idle" => RingStrategy::LongestIdle,
        "primary" => RingStrategy::Primary,
        _ => RingStrategy::Simultaneous,
    };

    let no_answer_timeout_secs = doc.get_i64("no_answer_timeout_secs").unwrap_or(30);
    let no_answer_timeout = Duration::from_secs(no_answer_timeout_secs as u64);

    let created_at = doc
        .get_datetime("created_at")
        .ok()
        .map(|dt| bson_datetime_to_system_time(*dt))
        .unwrap_or_else(SystemTime::now);

    let updated_at = doc
        .get_datetime("updated_at")
        .ok()
        .map(|dt| bson_datetime_to_system_time(*dt))
        .unwrap_or_else(SystemTime::now);

    Ok(LineBinding {
        id,
        phone_number_id,
        owner_user_id,
        binding_type,
        devices: Vec::new(), // Will be populated by caller
        ring_strategy,
        tenant_id,
        no_answer_timeout,
        forward_on_busy: doc.get_str("forward_on_busy").ok().map(String::from),
        forward_on_no_answer: doc.get_str("forward_on_no_answer").ok().map(String::from),
        forward_unconditional: doc.get_str("forward_unconditional").ok().map(String::from),
        outbound_caller_id: doc.get_str("outbound_caller_id").ok().map(String::from),
        outbound_cnam: doc.get_str("outbound_cnam").ok().map(String::from),
        created_at,
        updated_at,
    })
}

fn document_to_device_binding(doc: Document) -> Result<DeviceBinding> {
    let device_id = DeviceId::from_str(
        doc.get_str("device_id")
            .map_err(|_| anyhow::anyhow!("Missing device_id field"))?,
    )?;

    let priority = doc.get_i32("priority").unwrap_or(1) as u8;

    let line_appearance = doc.get_i32("line_appearance").unwrap_or(1) as u8;

    let shared_line = doc.get_bool("shared_line").unwrap_or(false);
    let enabled = doc.get_bool("enabled").unwrap_or(true);

    Ok(DeviceBinding {
        device_id,
        priority,
        line_appearance,
        shared_line,
        enabled,
    })
}

fn document_to_extension(doc: Document) -> Result<Extension> {
    let id = ExtensionId::from_str(
        doc.get_str("id")
            .map_err(|_| anyhow::anyhow!("Missing id field"))?,
    )?;
    let tenant_id = TenantId::from_str(
        doc.get_str("tenant_id")
            .map_err(|_| anyhow::anyhow!("Missing tenant_id field"))?,
    )?;

    let target_type = doc
        .get_str("target_type")
        .map_err(|_| anyhow::anyhow!("Missing target_type field"))?;
    let target_id = doc
        .get_str("target_id")
        .map_err(|_| anyhow::anyhow!("Missing target_id field"))?;

    let target = match target_type {
        "user" => ExtensionTarget::User(UserId::from_str(target_id)?),
        "phone_number" => ExtensionTarget::PhoneNumber(PhoneNumberId::from_str(target_id)?),
        "device" => ExtensionTarget::Device(DeviceId::from_str(target_id)?),
        "service" => ExtensionTarget::Service(
            ServiceId::from_str(target_id).unwrap_or_else(|_| ServiceId::new()),
        ),
        _ => {
            return Err(anyhow::anyhow!(
                "Unknown extension target type: {}",
                target_type
            ))
        }
    };

    let created_at = doc
        .get_datetime("created_at")
        .ok()
        .map(|dt| bson_datetime_to_system_time(*dt))
        .unwrap_or_else(SystemTime::now);

    let updated_at = doc
        .get_datetime("updated_at")
        .ok()
        .map(|dt| bson_datetime_to_system_time(*dt))
        .unwrap_or_else(SystemTime::now);

    Ok(Extension {
        id,
        number: doc
            .get_str("number")
            .map_err(|_| anyhow::anyhow!("Missing number field"))?
            .to_string(),
        target,
        tenant_id,
        description: doc.get_str("description").ok().map(String::from),
        enabled: doc.get_bool("enabled").unwrap_or(true),
        created_at,
        updated_at,
    })
}

fn document_to_device(doc: Document) -> Result<Device> {
    let id = DeviceId::from_str(
        doc.get_str("id")
            .map_err(|_| anyhow::anyhow!("Missing id field"))?,
    )?;
    let tenant_id = TenantId::from_str(
        doc.get_str("tenant_id")
            .map_err(|_| anyhow::anyhow!("Missing tenant_id field"))?,
    )?;

    let owner_user_id = doc
        .get_str("owner_user_id")
        .ok()
        .and_then(|s| UserId::from_str(s).ok());

    let device_type_str = doc
        .get_str("device_type")
        .map_err(|_| anyhow::anyhow!("Missing device_type field"))?;
    let device_type = match device_type_str {
        "desk_phone" => DeviceType::DeskPhone,
        "softphone" => DeviceType::Softphone,
        "webrtc" => DeviceType::WebRtc,
        "conference_room" => DeviceType::ConferenceRoom,
        "ata" => DeviceType::Ata,
        "gateway" => DeviceType::Gateway,
        "mobile" => DeviceType::Mobile,
        _ => DeviceType::Other,
    };

    let registration_state_str = doc
        .get_str("registration_state")
        .map_err(|_| anyhow::anyhow!("Missing registration_state field"))?;
    let registration_state = match registration_state_str {
        "unregistered" => RegistrationState::Unregistered,
        "registered" => RegistrationState::Registered,
        "failed" => RegistrationState::Failed,
        "expired" => RegistrationState::Expired,
        _ => RegistrationState::Unregistered,
    };

    let last_registration = doc
        .get_datetime("last_registration")
        .ok()
        .map(|dt| bson_datetime_to_system_time(*dt));

    let codec_support_strings: Vec<String> = doc
        .get("codec_support")
        .and_then(|v| bson::from_bson(v.clone()).ok())
        .unwrap_or_default();

    let codec_support: Vec<AudioCodec> = codec_support_strings
        .into_iter()
        .filter_map(|s| match s.to_lowercase().as_str() {
            "pcmu" => Some(AudioCodec::Pcmu),
            "pcma" => Some(AudioCodec::Pcma),
            "g722" => Some(AudioCodec::G722),
            "g729" => Some(AudioCodec::G729),
            "opus" => Some(AudioCodec::Opus),
            "ilbc" => Some(AudioCodec::Ilbc),
            "gsm" => Some(AudioCodec::Gsm),
            "speex" => Some(AudioCodec::Speex),
            _ => None,
        })
        .collect();

    let e911_override = doc.get("e911_override").and_then(|v| {
        if v.as_null().is_some() {
            None
        } else {
            bson::from_bson(v.clone()).ok()
        }
    });

    let created_at = doc
        .get_datetime("created_at")
        .ok()
        .map(|dt| bson_datetime_to_system_time(*dt))
        .unwrap_or_else(SystemTime::now);

    let updated_at = doc
        .get_datetime("updated_at")
        .ok()
        .map(|dt| bson_datetime_to_system_time(*dt))
        .unwrap_or_else(SystemTime::now);

    Ok(Device {
        id,
        name: doc
            .get_str("name")
            .map_err(|_| anyhow::anyhow!("Missing name field"))?
            .to_string(),
        device_type,
        mac_address: doc.get_str("mac_address").ok().map(String::from),
        model: doc.get_str("model").ok().map(String::from),
        firmware_version: doc.get_str("firmware_version").ok().map(String::from),
        tenant_id,
        owner_user_id,
        sip_uri: doc.get_str("sip_uri").ok().map(String::from),
        registration_state,
        last_registration,
        last_ip_address: doc.get_str("last_ip_address").ok().map(String::from),
        user_agent: doc.get_str("user_agent").ok().map(String::from),
        max_lines: doc.get_i32("max_lines").unwrap_or(1) as u8,
        supports_video: doc.get_bool("supports_video").unwrap_or(false),
        supports_blf: doc.get_bool("supports_blf").unwrap_or(false),
        codec_support,
        location: doc.get_str("location").ok().map(String::from),
        e911_override,
        provisioning_url: doc.get_str("provisioning_url").ok().map(String::from),
        config_template_id: doc.get_str("config_template_id").ok().map(String::from),
        enabled: doc.get_bool("enabled").unwrap_or(true),
        created_at,
        updated_at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore] // Requires MongoDB instance
    async fn test_connection() {
        // This test requires a running MongoDB instance
        // 1. Start MongoDB: docker run --rm -p 27017:27017 mongo:latest
        // 2. Run: cargo test --package fcp-storage-mongodb -- --ignored

        let config = MongodbConfig::new("mongodb://localhost:27017/fcp_test");
        let storage = MongodbStorage::new(config).await;
        assert!(storage.is_ok(), "Failed to connect to MongoDB");
    }

    #[test]
    fn test_extract_database_name() {
        assert_eq!(
            MongodbConfig::extract_database_name("mongodb://localhost:27017/fcp"),
            "fcp"
        );
        assert_eq!(
            MongodbConfig::extract_database_name("mongodb://user:pass@localhost:27017/mydb"),
            "mydb"
        );
        assert_eq!(
            MongodbConfig::extract_database_name("mongodb://localhost:27017/fcp?ssl=true"),
            "fcp"
        );
        assert_eq!(
            MongodbConfig::extract_database_name("mongodb://localhost:27017/"),
            "fcp"
        );
    }
}
