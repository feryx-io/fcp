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
    AccountStatus, CallDetailRecord, CallDisposition, CdrQuery, CdrStore, CredentialStore,
    EmergencyContact, HangupCause, HangupParty, MediaPreferences, Registration, RegistrationStore,
    TenantStore, User,
};
use fcp_core::{CallId, Tenant, TenantId};
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

        let storage = Self {
            client,
            db,
            users,
            registrations,
            cdrs,
            tenants,
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

        let doc = doc! {
            "id": user.id.to_string(),
            "username": &user.username,
            "password_hash": &user.password,
            "display_name": user.display_name.as_ref(),
            "tenant_id": &tenant_id,
            "enabled": user.enabled,
            "role": &role_str,
            "department_id": user.department_id.as_ref(),
            "created_at": system_time_to_bson_datetime(user.created_at),
            "updated_at": system_time_to_bson_datetime(user.updated_at),
            "media_preferences": media_prefs,
        };

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
        primary_phone_number_id: None, // TODO: Add to MongoDB schema
        voicemail_box_id: None,        // TODO: Add to MongoDB schema
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
