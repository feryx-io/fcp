# SQLite Storage Backend - Implementation Status

**Date:** 2026-01-09
**Status:** 🟡 In Progress (40% Complete)

---

## ✅ Completed

### 1. Crate Structure (100%)
- ✅ Created `fcp-storage-sqlite` crate in workspace
- ✅ Configured Cargo.toml with all dependencies
- ✅ Set up migrations directory
- ✅ Created basic module structure

### 2. Dependencies (100%)
```toml
sqlx = { version = "0.8", features = ["runtime-tokio", "sqlite", "macros", "migrate"] }
tokio = { version = "1.48", features = ["full"] }
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
async-trait = "0.1"
```

### 3. Database Schema (100%)
Created `migrations/001_initial_schema.sql` with complete schema:

**Users Table:**
- Primary key: (tenant_id, username)
- Fields: password_hash, display_name, enabled, timestamps, media_preferences (JSON)
- Indexes: tenant_id, username

**Registrations Table:**
- Primary key: (aor, contact)
- Fields: expires_at, call_id, cseq, user_agent, q-value, registered_at
- Indexes: aor, expires_at

**CDRs Table:**
- Primary key: call_id
- 30+ fields covering all CDR requirements
- Indexes: tenant_id, start_time, from/to URIs, disposition, sip_call_id

### 4. Configuration (100%)
```rust
pub struct SqliteConfig {
    pub database_path: PathBuf,      // Database file location
    pub max_connections: u32,         // Pool size (default: 5)
    pub connection_timeout: Duration, // Timeout (default: 30s)
    pub enable_wal: bool,            // WAL mode (default: true)
    pub create_if_missing: bool,     // Auto-create (default: true)
}
```

### 5. Storage Backend Structure (100%)
```rust
pub struct SqliteStorage {
    pool: Pool<Sqlite>,  // Connection pool
}

impl SqliteStorage {
    pub async fn new(config: SqliteConfig) -> Result<Self> {
        // ✅ Create parent directory
        // ✅ Set up connection options
        // ✅ Enable WAL mode
        // ✅ Create connection pool
        // ✅ Run migrations
    }

    async fn run_migrations(pool: &SqlitePool) -> Result<()> {
        // ✅ Execute migration SQL
        // ✅ Parse and run statements
    }
}
```

---

## ⏳ In Progress / Pending

### 6. CredentialStore Trait Implementation (0%)

**Methods to Implement:**
```rust
#[async_trait]
impl CredentialStore for SqliteStorage {
    async fn get_user(&self, username: &str) -> Result<Option<User>> {
        // TODO: SELECT from users table
        // TODO: Parse media_preferences JSON
        // TODO: Convert timestamps
    }

    async fn put_user(&self, user: User) -> Result<()> {
        // TODO: INSERT OR REPLACE with UPSERT
        // TODO: Serialize media_preferences to JSON
    }

    async fn delete_user(&self, username: &str) -> Result<()> {
        // TODO: DELETE FROM users
    }

    async fn list_users(&self) -> Result<Vec<User>> {
        // TODO: SELECT all users
        // TODO: Parse into User objects
    }

    // Note: verify_credentials() has default implementation
}
```

**Key Implementation Details:**
- Serialize/deserialize `MediaPreferences` as JSON
- Convert SystemTime ↔ Unix timestamp (i64)
- Handle boolean as INTEGER (0/1)
- Parse TenantId from string

### 7. RegistrationStore Trait Implementation (0%)

**Methods to Implement:**
```rust
#[async_trait]
impl RegistrationStore for SqliteStorage {
    async fn get_registrations(&self, aor: &str) -> Result<Vec<Registration>> {
        // TODO: SELECT WHERE aor = ? ORDER BY q DESC
    }

    async fn put_registration(&self, registration: Registration) -> Result<()> {
        // TODO: INSERT OR REPLACE (aor, contact)
    }

    async fn delete_registration(&self, aor: &str, contact: &str) -> Result<()> {
        // TODO: DELETE WHERE aor = ? AND contact = ?
    }

    async fn delete_all_registrations(&self, aor: &str) -> Result<()> {
        // TODO: DELETE WHERE aor = ?
    }

    async fn cleanup_expired(&self) -> Result<usize> {
        // TODO: DELETE WHERE expires_at <= NOW()
        // TODO: Return rows_affected
    }
}
```

**Key Implementation Details:**
- Convert SystemTime ↔ Unix timestamp for expires_at, registered_at
- Handle Optional fields (user_agent, q)
- Return count of deleted rows for cleanup

### 8. CdrStore Trait Implementation (0%)

**Methods to Implement:**
```rust
#[async_trait]
impl CdrStore for SqliteStorage {
    async fn insert_cdr(&self, cdr: CallDetailRecord) -> Result<()> {
        // TODO: INSERT INTO cdrs with 30+ fields
        // TODO: Serialize disposition, hangup_cause, hangup_party enums
        // TODO: Serialize custom_data as JSON
        // TODO: Convert timestamps
    }

    async fn get_cdr(&self, call_id: &CallId) -> Result<Option<CallDetailRecord>> {
        // TODO: SELECT WHERE call_id = ?
        // TODO: Deserialize enums
        // TODO: Parse JSON fields
    }

    async fn query_cdrs(&self, query: CdrQuery) -> Result<Vec<CallDetailRecord>> {
        // TODO: Build dynamic WHERE clause
        // TODO: Handle tenant_id, call_id, from_uri, to_uri filters
        // TODO: Handle time range filters
        // TODO: Handle disposition filter
        // TODO: Apply LIMIT and OFFSET
    }

    async fn count_cdrs(&self, query: CdrQuery) -> Result<usize> {
        // TODO: SELECT COUNT(*) with same filters as query_cdrs
    }
}
```

**Key Implementation Details:**
- Serialize enums as strings (serde_json)
- Handle `HangupCause::Other(u16)` special case
- Dynamic query building for flexible filters
- Handle Optional fields (many in CDR)
- Convert SystemTime ↔ Unix timestamp
- Serialize custom_data as JSON TEXT

### 9. Helper Functions (0%)

**Needed:**
```rust
fn row_to_user(row: SqliteRow) -> Result<User> {
    // Convert database row to User struct
}

fn row_to_registration(row: SqliteRow) -> Result<Registration> {
    // Convert database row to Registration struct
}

fn row_to_cdr(row: SqliteRow) -> Result<CallDetailRecord> {
    // Convert database row to CDR struct
    // Most complex due to 30+ fields
}
```

### 10. Tests (0%)

**Test Cases Needed:**
```rust
#[tokio::test]
async fn test_user_crud() {
    // Create, read, update, delete user
}

#[tokio::test]
async fn test_registration_lifecycle() {
    // Add, query, expire, cleanup registrations
}

#[tokio::test]
async fn test_cdr_storage_and_query() {
    // Insert CDRs, query with filters
}

#[tokio::test]
async fn test_concurrent_access() {
    // Multiple connections, WAL mode verification
}

#[tokio::test]
async fn test_migration() {
    // Verify migrations run correctly
}
```

---

## 📋 Implementation Checklist

### Phase 1: User & Registration Storage
- [ ] Implement `get_user()`
- [ ] Implement `put_user()`
- [ ] Implement `delete_user()`
- [ ] Implement `list_users()`
- [ ] Implement `get_registrations()`
- [ ] Implement `put_registration()`
- [ ] Implement `delete_registration()`
- [ ] Implement `delete_all_registrations()`
- [ ] Implement `cleanup_expired()`
- [ ] Add `row_to_user()` helper
- [ ] Add `row_to_registration()` helper
- [ ] Write user CRUD tests
- [ ] Write registration lifecycle tests

### Phase 2: CDR Storage
- [ ] Implement `insert_cdr()`
- [ ] Implement `get_cdr()`
- [ ] Implement `query_cdrs()` with dynamic filtering
- [ ] Implement `count_cdrs()`
- [ ] Add `row_to_cdr()` helper
- [ ] Handle enum serialization (disposition, hangup_cause, hangup_party)
- [ ] Handle JSON custom_data field
- [ ] Write CDR storage tests
- [ ] Write CDR query tests

### Phase 3: Integration & Polish
- [ ] Add connection pool health checks
- [ ] Add query performance logging
- [ ] Write concurrent access tests
- [ ] Add migration rollback support
- [ ] Document usage examples
- [ ] Performance benchmarks
- [ ] Error handling improvements
- [ ] Add prepared statement optimization

---

## 🔧 Technical Details

### Enum Serialization Strategy

**Simple Enums (disposition, hangup_party):**
```rust
// Serialize as JSON string
let disposition = serde_json::to_string(&cdr.disposition)?;
// "answered", "busy", "no_answer", "failed", "cancelled"
```

**Complex Enum (HangupCause::Other):**
```rust
let hangup_cause = match cdr.hangup_cause {
    HangupCause::Other(code) => format!("other:{}", code),
    _ => serde_json::to_string(&cdr.hangup_cause)?.trim_matches('"').to_string(),
};
```

### Timestamp Conversion

**SystemTime → i64:**
```rust
let timestamp = system_time
    .duration_since(UNIX_EPOCH)
    .unwrap_or_default()
    .as_secs() as i64;
```

**i64 → SystemTime:**
```rust
let system_time = UNIX_EPOCH + Duration::from_secs(timestamp as u64);
```

### Dynamic Query Building

```rust
let mut sql = String::from("SELECT * FROM cdrs WHERE 1=1");
let mut bindings = Vec::new();

if let Some(tenant_id) = &query.tenant_id {
    sql.push_str(" AND tenant_id = ?");
    bindings.push(tenant_id.to_string());
}

if let Some(start_from) = query.start_time_from {
    sql.push_str(" AND start_time >= ?");
    bindings.push(start_from.duration_since(UNIX_EPOCH)?.as_secs());
}

// ... more filters
```

---

## 📊 Estimated Completion Time

- **Phase 1 (User & Registration):** 4-6 hours
- **Phase 2 (CDR Storage):** 6-8 hours
- **Phase 3 (Integration & Polish):** 2-4 hours

**Total:** 12-18 hours of focused development

---

## 🚀 Next Steps

1. **Implement CredentialStore trait** (start with user CRUD)
2. **Implement RegistrationStore trait** (simpler than CDR)
3. **Implement CdrStore trait** (most complex)
4. **Add helper functions** (row conversion)
5. **Write comprehensive tests**
6. **Document usage and examples**
7. **Performance testing and optimization**

---

## 📚 Reference Implementation

For reference, see:
- **FileBackedStorage** in `fcp-core/src/storage.rs` (existing implementation)
- **sqlx examples**: https://github.com/launchbadge/sqlx/tree/main/examples
- **Migration patterns**: Include SQL files, run on startup
- **Connection pooling**: SqlitePoolOptions configuration

---

## ⚠️ Known Challenges

1. **Dynamic Query Building**: Need careful handling of query parameters
2. **Enum Serialization**: HangupCause::Other(u16) needs special handling
3. **JSON Fields**: media_preferences, custom_data need serialize/deserialize
4. **Timestamp Conversion**: SystemTime ↔ Unix epoch, handle Optional<SystemTime>
5. **Boolean Storage**: SQLite INTEGER (0/1) ↔ Rust bool
6. **Connection Pool Tuning**: Balance between connections and performance

---

## 🎯 Success Criteria

- ✅ All three storage traits fully implemented
- ✅ All tests passing
- ✅ WAL mode enabled and working
- ✅ Migrations run successfully
- ✅ Connection pool performs well under load
- ✅ Proper error handling and logging
- ✅ Documentation with usage examples
- ✅ Zero data loss in concurrent scenarios

---

**Current Progress:** 40% Complete
**Estimated Remaining:** 60% (12-18 hours)
