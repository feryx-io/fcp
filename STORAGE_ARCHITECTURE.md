# FCP Storage Architecture

**Version:** 1.0
**Date:** 2026-01-09

---

## Overview

FCP implements a flexible, pluggable storage architecture that allows deployments to choose the storage backend based on their scale and requirements. The storage layer uses trait-based abstractions to support multiple backends without changing application code.

## Design Principles

1. **Pluggable Backends** - Swap storage implementations via configuration
2. **Async-First** - All storage operations are async for scalability
3. **Type-Safe** - Strongly-typed APIs prevent runtime errors
4. **Multi-Tenant** - All storage includes tenant isolation
5. **Scale-Appropriate** - Small deployments use files/SQLite, large deployments use PostgreSQL/MySQL
6. **Cloud-Ready** - Hooks for cloud-native storage (S3, GCS, CloudSQL)

---

## Storage Backends

### Tier 1: Small Deployments (< 1000 users)

#### 1. **FileBackedStorage** (Default)
- **Use Case**: Development, small office PBX, embedded systems
- **Implementation**: In-memory with JSON file persistence
- **Pros**: Zero dependencies, simple, fast for small datasets
- **Cons**: Limited scalability, no transactions, manual conflict resolution
- **Status**: ✅ Implemented (crates/fcp-core/src/storage.rs)

#### 2. **SQLiteStorage**
- **Use Case**: Small to medium deployments, single-node production
- **Implementation**: Embedded SQLite database with connection pooling
- **Pros**: ACID transactions, good performance, zero server setup, embedded
- **Cons**: Single-writer limitation, not ideal for multi-node clustering
- **Status**: ⏳ To Implement
- **Dependencies**: `sqlx` with `sqlite` feature

### Tier 2: Medium Deployments (1,000 - 100,000 users)

#### 3. **PostgreSQLStorage**
- **Use Case**: Production deployments, multi-tenant SaaS, clustering
- **Implementation**: PostgreSQL client with connection pooling
- **Pros**: ACID, excellent concurrency, JSON support, extensions (PostGIS, timescaledb)
- **Cons**: Requires database server
- **Status**: ⏳ To Implement
- **Dependencies**: `sqlx` with `postgres` feature

#### 4. **MySQLStorage** / **MariaDBStorage**
- **Use Case**: Production deployments with existing MySQL infrastructure
- **Implementation**: MySQL client with connection pooling
- **Pros**: ACID, widely supported, good ecosystem
- **Cons**: Requires database server, JSON support less mature than PostgreSQL
- **Status**: ⏳ To Implement
- **Dependencies**: `sqlx` with `mysql` feature

### Tier 3: Large/Cloud Deployments (> 100,000 users)

#### 5. **CloudSQLStorage** (GCP Cloud SQL, AWS RDS)
- **Use Case**: Cloud-native deployments on GCP/AWS
- **Implementation**: Wrapper around PostgreSQL/MySQL with cloud-specific features
- **Pros**: Managed service, automatic backups, replication, scaling
- **Cons**: Cost, vendor lock-in
- **Status**: ⏳ To Implement (Phase 2)
- **Cloud Integration**:
  - GCP: Cloud SQL Proxy, IAM authentication
  - AWS: RDS Proxy, IAM authentication

#### 6. **CloudStorageHooks** (S3, GCS)
- **Use Case**: CDR archival, recording storage, logs, backups
- **Implementation**: Async upload hooks for blob storage
- **Pros**: Infinite scale, cheap, durable
- **Cons**: High latency, not suitable for transactional data
- **Status**: ⏳ To Implement (Phase 2)
- **Use Cases**:
  - CDR exports (daily/weekly batch jobs)
  - Call recordings (upload after call completes)
  - Configuration backups
  - Log archival

---

## Storage Traits

### Core Traits (Already Implemented)

```rust
/// User credential storage
#[async_trait]
pub trait CredentialStore: Send + Sync {
    async fn get_user(&self, username: &str) -> Result<Option<User>>;
    async fn put_user(&self, user: User) -> Result<()>;
    async fn delete_user(&self, username: &str) -> Result<()>;
    async fn list_users(&self) -> Result<Vec<User>>;
    async fn verify_credentials(&self, username: &str, password: &str) -> Result<bool>;
}

/// SIP registration storage
#[async_trait]
pub trait RegistrationStore: Send + Sync {
    async fn get_registrations(&self, aor: &str) -> Result<Vec<Registration>>;
    async fn put_registration(&self, registration: Registration) -> Result<()>;
    async fn delete_registration(&self, aor: &str, contact: &str) -> Result<()>;
    async fn delete_all_registrations(&self, aor: &str) -> Result<()>;
    async fn cleanup_expired(&self) -> Result<usize>;
}
```

### New Traits (To Implement)

```rust
/// Call Detail Record storage
#[async_trait]
pub trait CdrStore: Send + Sync {
    /// Insert a new CDR
    async fn insert_cdr(&self, cdr: CallDetailRecord) -> Result<()>;

    /// Get CDRs by call ID
    async fn get_cdr(&self, call_id: &CallId) -> Result<Option<CallDetailRecord>>;

    /// Query CDRs with filters
    async fn query_cdrs(&self, query: CdrQuery) -> Result<Vec<CallDetailRecord>>;

    /// Export CDRs to external storage (cloud bucket, etc.)
    async fn export_cdrs(&self, query: CdrQuery, destination: &str) -> Result<usize>;

    /// Archive old CDRs (move to cold storage)
    async fn archive_old_cdrs(&self, older_than: Duration) -> Result<usize>;
}

/// Configuration storage (routes, trunks, etc.)
#[async_trait]
pub trait ConfigStore: Send + Sync {
    /// Get a configuration value
    async fn get_config(&self, tenant_id: &TenantId, key: &str) -> Result<Option<String>>;

    /// Set a configuration value
    async fn set_config(&self, tenant_id: &TenantId, key: &str, value: String) -> Result<()>;

    /// Delete a configuration value
    async fn delete_config(&self, tenant_id: &TenantId, key: &str) -> Result<()>;

    /// List all configuration keys for a tenant
    async fn list_config(&self, tenant_id: &TenantId) -> Result<Vec<(String, String)>>;
}

/// Route storage (least-cost routing, number routing)
#[async_trait]
pub trait RouteStore: Send + Sync {
    /// Find routes for a destination number
    async fn find_routes(&self, tenant_id: &TenantId, destination: &str) -> Result<Vec<Route>>;

    /// Add or update a route
    async fn put_route(&self, route: Route) -> Result<()>;

    /// Delete a route
    async fn delete_route(&self, route_id: &RouteId) -> Result<()>;

    /// List all routes for a tenant
    async fn list_routes(&self, tenant_id: &TenantId) -> Result<Vec<Route>>;
}

/// Unified storage interface
pub trait Storage: CredentialStore + RegistrationStore + CdrStore + ConfigStore + RouteStore + Send + Sync {}
```

---

## Data Models

### Call Detail Record (CDR)

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallDetailRecord {
    /// Unique call identifier (FCP internal)
    pub call_id: CallId,

    /// Tenant ID for multi-tenancy
    pub tenant_id: TenantId,

    /// SIP Call-ID header
    pub sip_call_id: String,

    /// Caller information
    pub from_uri: String,
    pub from_display_name: Option<String>,

    /// Callee information
    pub to_uri: String,
    pub to_display_name: Option<String>,

    /// Call timestamps
    pub start_time: SystemTime,
    pub answer_time: Option<SystemTime>,
    pub end_time: Option<SystemTime>,

    /// Call duration (milliseconds)
    pub setup_duration_ms: Option<u64>,
    pub call_duration_ms: Option<u64>,
    pub total_duration_ms: Option<u64>,

    /// Call disposition
    pub disposition: CallDisposition,
    pub hangup_cause: HangupCause,
    pub hangup_party: Option<HangupParty>,

    /// SIP response codes
    pub sip_status_code: Option<u16>,
    pub sip_reason: Option<String>,

    /// Routing information
    pub route_id: Option<RouteId>,
    pub gateway_id: Option<GatewayId>,

    /// Media information
    pub media_anchored: bool,
    pub codec_a_leg: Option<String>,
    pub codec_b_leg: Option<String>,
    pub rtp_packets_sent: Option<u64>,
    pub rtp_packets_received: Option<u64>,
    pub rtp_bytes_sent: Option<u64>,
    pub rtp_bytes_received: Option<u64>,

    /// Recording
    pub recording_url: Option<String>,

    /// Cost (for billing)
    pub cost: Option<f64>,
    pub currency: Option<String>,

    /// Custom fields (JSON)
    pub custom_data: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum CallDisposition {
    Answered,
    Busy,
    NoAnswer,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum HangupCause {
    NormalClearing,
    UserBusy,
    NoAnswer,
    CallRejected,
    Timeout,
    NetworkError,
    InternalError,
    Other(u16),
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum HangupParty {
    Caller,
    Callee,
    System,
}
```

### Route

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Route {
    /// Unique route identifier
    pub id: RouteId,

    /// Tenant ID
    pub tenant_id: TenantId,

    /// Route name/description
    pub name: String,

    /// Destination prefix pattern (e.g., "1", "1415", "+44")
    pub prefix: String,

    /// Priority (lower = higher priority)
    pub priority: i32,

    /// Gateway IDs to try (in order)
    pub gateway_ids: Vec<GatewayId>,

    /// Number manipulation (strip digits, add prefix)
    pub strip_digits: Option<u32>,
    pub prepend_digits: Option<String>,

    /// Enabled flag
    pub enabled: bool,

    /// Timestamps
    pub created_at: SystemTime,
    pub updated_at: SystemTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Gateway {
    /// Unique gateway identifier
    pub id: GatewayId,

    /// Tenant ID
    pub tenant_id: TenantId,

    /// Gateway name
    pub name: String,

    /// SIP URI (e.g., "sip:trunk.example.com:5060")
    pub uri: String,

    /// Authentication
    pub auth_username: Option<String>,
    pub auth_password: Option<String>,

    /// Codec preferences
    pub codec_preferences: Vec<String>,

    /// Health check
    pub health_check_enabled: bool,
    pub last_health_check: Option<SystemTime>,
    pub is_healthy: bool,

    /// Enabled flag
    pub enabled: bool,
}
```

---

## Database Schemas

### SQLite Schema

```sql
-- Users table
CREATE TABLE users (
    username TEXT PRIMARY KEY,
    password_hash TEXT NOT NULL,
    display_name TEXT,
    tenant_id TEXT NOT NULL,
    enabled BOOLEAN NOT NULL DEFAULT 1,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    media_preferences JSON,
    UNIQUE(tenant_id, username)
);
CREATE INDEX idx_users_tenant ON users(tenant_id);

-- Registrations table
CREATE TABLE registrations (
    aor TEXT NOT NULL,
    contact TEXT NOT NULL,
    expires_at INTEGER NOT NULL,
    call_id TEXT NOT NULL,
    cseq INTEGER NOT NULL,
    user_agent TEXT,
    q REAL,
    registered_at INTEGER NOT NULL,
    PRIMARY KEY (aor, contact)
);
CREATE INDEX idx_registrations_expires ON registrations(expires_at);

-- CDRs table
CREATE TABLE cdrs (
    call_id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    sip_call_id TEXT NOT NULL,
    from_uri TEXT NOT NULL,
    from_display_name TEXT,
    to_uri TEXT NOT NULL,
    to_display_name TEXT,
    start_time INTEGER NOT NULL,
    answer_time INTEGER,
    end_time INTEGER,
    setup_duration_ms INTEGER,
    call_duration_ms INTEGER,
    total_duration_ms INTEGER,
    disposition TEXT NOT NULL,
    hangup_cause TEXT NOT NULL,
    hangup_party TEXT,
    sip_status_code INTEGER,
    sip_reason TEXT,
    route_id TEXT,
    gateway_id TEXT,
    media_anchored BOOLEAN,
    codec_a_leg TEXT,
    codec_b_leg TEXT,
    rtp_packets_sent INTEGER,
    rtp_packets_received INTEGER,
    rtp_bytes_sent INTEGER,
    rtp_bytes_received INTEGER,
    recording_url TEXT,
    cost REAL,
    currency TEXT,
    custom_data JSON
);
CREATE INDEX idx_cdrs_tenant ON cdrs(tenant_id);
CREATE INDEX idx_cdrs_start_time ON cdrs(start_time);
CREATE INDEX idx_cdrs_from_uri ON cdrs(from_uri);
CREATE INDEX idx_cdrs_to_uri ON cdrs(to_uri);

-- Routes table
CREATE TABLE routes (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    name TEXT NOT NULL,
    prefix TEXT NOT NULL,
    priority INTEGER NOT NULL DEFAULT 100,
    gateway_ids JSON NOT NULL,
    strip_digits INTEGER,
    prepend_digits TEXT,
    enabled BOOLEAN NOT NULL DEFAULT 1,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE INDEX idx_routes_tenant_prefix ON routes(tenant_id, prefix);
CREATE INDEX idx_routes_priority ON routes(priority);

-- Gateways table
CREATE TABLE gateways (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    name TEXT NOT NULL,
    uri TEXT NOT NULL,
    auth_username TEXT,
    auth_password TEXT,
    codec_preferences JSON,
    health_check_enabled BOOLEAN NOT NULL DEFAULT 1,
    last_health_check INTEGER,
    is_healthy BOOLEAN NOT NULL DEFAULT 1,
    enabled BOOLEAN NOT NULL DEFAULT 1
);
CREATE INDEX idx_gateways_tenant ON gateways(tenant_id);

-- Config table (key-value store)
CREATE TABLE config (
    tenant_id TEXT NOT NULL,
    key TEXT NOT NULL,
    value TEXT NOT NULL,
    PRIMARY KEY (tenant_id, key)
);
```

### PostgreSQL Schema

```sql
-- Enable UUID extension
CREATE EXTENSION IF NOT EXISTS "uuid-ossp";

-- Users table
CREATE TABLE users (
    username TEXT NOT NULL,
    password_hash TEXT NOT NULL,
    display_name TEXT,
    tenant_id UUID NOT NULL,
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    media_preferences JSONB,
    PRIMARY KEY (tenant_id, username)
);
CREATE INDEX idx_users_tenant ON users(tenant_id);

-- Registrations table
CREATE TABLE registrations (
    aor TEXT NOT NULL,
    contact TEXT NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    call_id TEXT NOT NULL,
    cseq INTEGER NOT NULL,
    user_agent TEXT,
    q REAL,
    registered_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (aor, contact)
);
CREATE INDEX idx_registrations_expires ON registrations(expires_at);

-- CDRs table (partitioned by start_time for performance)
CREATE TABLE cdrs (
    call_id UUID PRIMARY KEY,
    tenant_id UUID NOT NULL,
    sip_call_id TEXT NOT NULL,
    from_uri TEXT NOT NULL,
    from_display_name TEXT,
    to_uri TEXT NOT NULL,
    to_display_name TEXT,
    start_time TIMESTAMPTZ NOT NULL,
    answer_time TIMESTAMPTZ,
    end_time TIMESTAMPTZ,
    setup_duration_ms INTEGER,
    call_duration_ms INTEGER,
    total_duration_ms INTEGER,
    disposition TEXT NOT NULL,
    hangup_cause TEXT NOT NULL,
    hangup_party TEXT,
    sip_status_code SMALLINT,
    sip_reason TEXT,
    route_id UUID,
    gateway_id UUID,
    media_anchored BOOLEAN,
    codec_a_leg TEXT,
    codec_b_leg TEXT,
    rtp_packets_sent BIGINT,
    rtp_packets_received BIGINT,
    rtp_bytes_sent BIGINT,
    rtp_bytes_received BIGINT,
    recording_url TEXT,
    cost DECIMAL(10, 4),
    currency TEXT,
    custom_data JSONB
) PARTITION BY RANGE (start_time);

-- Create monthly partitions (example for January 2026)
CREATE TABLE cdrs_2026_01 PARTITION OF cdrs
    FOR VALUES FROM ('2026-01-01') TO ('2026-02-01');

CREATE INDEX idx_cdrs_tenant ON cdrs(tenant_id);
CREATE INDEX idx_cdrs_start_time ON cdrs(start_time);
CREATE INDEX idx_cdrs_from_uri ON cdrs USING GIN (from_uri gin_trgm_ops);
CREATE INDEX idx_cdrs_to_uri ON cdrs USING GIN (to_uri gin_trgm_ops);
CREATE INDEX idx_cdrs_custom_data ON cdrs USING GIN (custom_data);

-- Routes table
CREATE TABLE routes (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    tenant_id UUID NOT NULL,
    name TEXT NOT NULL,
    prefix TEXT NOT NULL,
    priority INTEGER NOT NULL DEFAULT 100,
    gateway_ids UUID[] NOT NULL,
    strip_digits INTEGER,
    prepend_digits TEXT,
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX idx_routes_tenant_prefix ON routes(tenant_id, prefix);
CREATE INDEX idx_routes_priority ON routes(priority);

-- Gateways table
CREATE TABLE gateways (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    tenant_id UUID NOT NULL,
    name TEXT NOT NULL,
    uri TEXT NOT NULL,
    auth_username TEXT,
    auth_password TEXT,
    codec_preferences TEXT[],
    health_check_enabled BOOLEAN NOT NULL DEFAULT TRUE,
    last_health_check TIMESTAMPTZ,
    is_healthy BOOLEAN NOT NULL DEFAULT TRUE,
    enabled BOOLEAN NOT NULL DEFAULT TRUE
);
CREATE INDEX idx_gateways_tenant ON gateways(tenant_id);

-- Config table
CREATE TABLE config (
    tenant_id UUID NOT NULL,
    key TEXT NOT NULL,
    value TEXT NOT NULL,
    PRIMARY KEY (tenant_id, key)
);
```

---

## Storage Configuration

### Configuration File (fcp.toml)

```toml
[storage]
# Backend type: file, sqlite, postgres, mysql
backend = "sqlite"

# File backend (default)
[storage.file]
data_dir = "/var/lib/fcp/data"

# SQLite backend
[storage.sqlite]
database_path = "/var/lib/fcp/fcp.db"
connection_pool_size = 5
journal_mode = "WAL"  # Write-Ahead Logging for better concurrency

# PostgreSQL backend
[storage.postgres]
host = "localhost"
port = 5432
database = "fcp"
username = "fcp"
password = "secret"
connection_pool_size = 20
connection_timeout_seconds = 30
# SSL mode: disable, require, verify-ca, verify-full
ssl_mode = "require"
ssl_ca_cert = "/path/to/ca-cert.pem"

# MySQL backend
[storage.mysql]
host = "localhost"
port = 3306
database = "fcp"
username = "fcp"
password = "secret"
connection_pool_size = 20

# Cloud storage (optional)
[storage.cloud]
# CDR export destination
cdr_export_enabled = true
cdr_export_provider = "s3"  # s3, gcs
cdr_export_bucket = "fcp-cdrs"
cdr_export_region = "us-east-1"
cdr_export_interval_hours = 24

# Recording storage
recording_provider = "s3"
recording_bucket = "fcp-recordings"
recording_region = "us-east-1"

# AWS credentials (or use IAM role)
[storage.cloud.aws]
access_key_id = "AKIAIOSFODNN7EXAMPLE"
secret_access_key = "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY"

# GCP credentials (or use service account)
[storage.cloud.gcp]
credentials_file = "/path/to/service-account.json"
project_id = "my-fcp-project"
```

---

## Implementation Plan

### Phase 1: Core Storage Traits (Week 1)

1. ✅ `CredentialStore` - Already implemented
2. ✅ `RegistrationStore` - Already implemented
3. ⏳ `CdrStore` - Add trait and models
4. ⏳ `ConfigStore` - Add trait and models
5. ⏳ `RouteStore` - Add trait and models
6. ⏳ `Storage` - Unified trait

### Phase 2: SQL Backends (Week 2)

1. **SQLiteStorage** (2 days)
   - Use `sqlx` with SQLite runtime
   - Implement all storage traits
   - WAL mode for concurrency
   - Migrations with `sqlx migrate`

2. **PostgreSQLStorage** (2 days)
   - Use `sqlx` with PostgreSQL runtime
   - Connection pooling
   - Table partitioning for CDRs
   - JSON support for custom fields

3. **MySQLStorage** (1 day)
   - Use `sqlx` with MySQL runtime
   - Similar to PostgreSQL implementation

### Phase 3: Cloud Integration (Week 3)

1. **CloudStorageHooks** (2 days)
   - S3 SDK integration (rusoto or aws-sdk-rust)
   - GCS SDK integration (google-cloud-storage)
   - Async upload after call completion
   - Retry logic with exponential backoff

2. **Cloud SQL Integration** (1 day)
   - GCP Cloud SQL Proxy support
   - AWS RDS IAM authentication
   - Connection string builders

### Phase 4: Migrations & Testing (Week 3-4)

1. **Database Migrations** (2 days)
   - SQLite migrations
   - PostgreSQL migrations
   - MySQL migrations
   - Migration CLI tool

2. **Testing** (2 days)
   - Unit tests for each backend
   - Integration tests with test databases
   - Performance benchmarks
   - Migration tests

---

## Storage Factory

```rust
pub enum StorageBackend {
    File(PathBuf),
    SQLite(SQLiteConfig),
    PostgreSQL(PostgreSQLConfig),
    MySQL(MySQLConfig),
}

pub struct StorageFactory;

impl StorageFactory {
    pub async fn create(backend: StorageBackend) -> Result<Arc<dyn Storage>> {
        match backend {
            StorageBackend::File(data_dir) => {
                Ok(Arc::new(FileBackedStorage::new(data_dir).await?))
            }
            StorageBackend::SQLite(config) => {
                Ok(Arc::new(SQLiteStorage::new(config).await?))
            }
            StorageBackend::PostgreSQL(config) => {
                Ok(Arc::new(PostgreSQLStorage::new(config).await?))
            }
            StorageBackend::MySQL(config) => {
                Ok(Arc::new(MySQLStorage::new(config).await?))
            }
        }
    }
}
```

---

## Performance Considerations

### Indexing Strategy

**SQLite:**
- Simple indexes on frequently queried columns
- Avoid too many indexes (slow writes)

**PostgreSQL:**
- B-tree indexes for exact matches
- GIN indexes for full-text search (from_uri, to_uri)
- Partial indexes for filtered queries
- Table partitioning for CDRs (monthly)

**MySQL:**
- Similar to PostgreSQL
- InnoDB for transactions

### Connection Pooling

- **SQLite**: 1-5 connections (single-writer)
- **PostgreSQL**: 10-100 connections (depends on load)
- **MySQL**: 10-100 connections

### Batch Operations

- CDR insertion: Batch up to 100 records
- Registration cleanup: Batch delete expired
- Cloud uploads: Batch upload CDRs hourly/daily

### Caching

- User credentials: Cache in memory (TTL: 5 minutes)
- Routes: Cache in memory (TTL: 1 minute)
- Registrations: No cache (real-time updates)

---

## Security Considerations

1. **Password Storage**: bcrypt with cost 12 (already implemented)
2. **SQL Injection**: Use parameterized queries (sqlx handles this)
3. **Connection Strings**: Store in environment variables, not config files
4. **Encryption at Rest**: Database-level encryption (PostgreSQL TDE, MySQL encryption)
5. **TLS**: Require TLS for database connections in production
6. **Audit Logging**: Log all credential changes, route modifications
7. **Cloud Credentials**: Use IAM roles instead of access keys when possible

---

## Migration Strategy

### Upgrading from File to SQL

```bash
# Export existing data
fcp-cli storage export --format json --output /tmp/fcp-export.json

# Configure SQL backend in fcp.toml
vim /etc/fcp/fcp.toml

# Run migrations
fcp-cli storage migrate --backend sqlite

# Import data
fcp-cli storage import --input /tmp/fcp-export.json

# Verify
fcp-cli storage verify
```

---

## Future Enhancements (Phase 5+)

1. **Sharding**: Horizontal sharding by tenant_id for massive scale
2. **Replication**: Master-slave replication for read scaling
3. **TimescaleDB**: Time-series database for CDR analytics
4. **Elasticsearch**: Full-text search for CDRs
5. **Redis**: Session state, rate limiting, caching
6. **ClickHouse**: OLAP queries for analytics
7. **Kafka**: Event streaming for real-time CDR processing

---

## Conclusion

This storage architecture provides FCP with the flexibility to scale from small deployments (< 1000 users) with file-based storage to large cloud deployments (> 100,000 users) with PostgreSQL and cloud storage integration. The trait-based design ensures backward compatibility while allowing new backends to be added without changing application code.
