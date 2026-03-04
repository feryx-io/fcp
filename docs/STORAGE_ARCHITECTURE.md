# FCP Storage Architecture

**Version:** 2.0
**Date:** 2026-03-04

---

## Overview

FCP uses a trait-based storage architecture defined in `fcp-core/src/storage.rs`. Application code depends on abstract traits, and concrete backends are selected at deployment time. This provides pluggable storage without changing application logic.

### Backends

| Backend | Crate | Purpose |
|---------|-------|---------|
| **PostgreSQL** | `fcp-storage-postgres` | Production storage: ACID transactions, JSONB, connection pooling, migrations |
| **Redis** | `fcp-storage-redis` | Event bus, caching, distributed state (parking, circuit breaker) |
| **FileBackedStorage** | `fcp-core` (built-in) | Development only: in-memory with JSON file persistence |

---

## Storage Traits

All storage traits are defined in `fcp-core/src/storage.rs` and are `async_trait` + `Send + Sync`.

### Core Traits

| Trait | Purpose |
|-------|---------|
| `CredentialStore` | User CRUD with bcrypt password hashing |
| `TenantStore` | Multi-tenant configuration |
| `RegistrationStore` | SIP registration bindings |
| `CdrStore` | Call detail record storage and querying |
| `PhoneNumberStore` | DID/phone number management |
| `LineBindingStore` | Device-to-line bindings |
| `ExtensionStore` | Extension definitions and routing targets |
| `DeviceStore` | Device provisioning data |

### Application Traits

| Trait | Purpose |
|-------|---------|
| `VoicemailStore` | Mailbox and message management |
| `QueueStore` | ACD queue configuration and membership |
| `ConferenceStore` | Conference room management |
| `PresenceStore` | Presence status and subscriptions |
| `ResourceListStore` | Presence resource lists |
| `PresenceHistoryStore` | Presence change history |
| `WebhookStore` | Webhook endpoint CRUD and delivery logs |
| `GeoRoutingStore` | Geographic routing rules |

### Infrastructure Traits

| Trait | Purpose |
|-------|---------|
| `ApiTokenStore` | API authentication tokens |
| `ClusterParkingStore` | Distributed call parking across cluster nodes |

### DynStorage

`DynStorage` wraps all storage traits into a single type-erased container:

```rust
let storage = DynStorage::from_all_in_one(postgres_storage);
// or compose from individual backends:
let storage = DynStorage::new(cred_store, reg_store, cdr_store, ...);
```

All API handlers and services receive `Arc<DynStorage>` and call trait methods without knowing the concrete backend.

---

## PostgreSQL Backend

**Crate:** `fcp-storage-postgres`
**Path:** `crates/fcp-storage-postgres/`

### Features

- Implements all storage traits listed above
- Uses `sqlx` with the `postgres` feature
- Connection pooling via `sqlx::PgPool`
- JSONB columns for flexible data (custom fields, media preferences, SDP)
- SQL migrations managed by `sqlx migrate`
- Table partitioning support for CDRs (by time range)

### Configuration

```toml
[storage]
postgres_url = "postgresql://fcp:secret@localhost:5432/fcp"
max_connections = 20
```

---

## Redis Backend

**Crate:** `fcp-storage-redis`
**Path:** `crates/fcp-storage-redis/`

### Features

- Event bus pub/sub for platform-wide event distribution
- Circuit breaker state for trunk health monitoring
- Distributed call parking state across cluster nodes (`ClusterParkingStore`)
- Cache layer for frequently-accessed data

### Configuration

```toml
[redis]
url = "redis://localhost:6379"
```

---

## FileBackedStorage (Development)

**Location:** Built into `fcp-core/src/storage.rs`

### Features

- In-memory `HashMap`-based storage behind `RwLock`
- Optional JSON file persistence (load on startup, save on changes)
- Implements all core storage traits
- Zero external dependencies

### Usage

Suitable for development, testing, and small single-node deployments. Not recommended for production due to:
- No ACID transactions
- No concurrent write safety beyond `RwLock`
- Data loss risk on crash (in-memory between saves)

---

## Multi-Tenancy

All storage operations are tenant-scoped:

- Every stored entity includes a `TenantId` field
- Query methods accept `tenant_id` parameters for isolation
- Indexes include tenant columns for efficient filtered queries
- `DynStorage` propagates tenant context through all operations

---

## Performance Considerations

### PostgreSQL

- B-tree indexes on frequently queried columns (tenant_id, timestamps)
- GIN indexes for JSONB and full-text search
- Connection pooling (10-100 connections based on load)
- CDR table partitioning by time range for query performance

### Redis

- Pub/sub for real-time event delivery (low latency)
- Key expiry for automatic cache invalidation
- Connection pooling for concurrent access

### Caching Strategy

- User credentials: cached in memory (TTL-based)
- Routing rules: cached in memory with invalidation
- Registrations: real-time from storage (no cache)

---

## Security

- Passwords stored as bcrypt hashes (cost factor 12)
- Parameterized queries via `sqlx` (SQL injection prevention)
- TLS for database connections in production (`ssl_mode = "require"`)
- Connection strings from environment variables, not config files
- Tenant isolation enforced at the query level
