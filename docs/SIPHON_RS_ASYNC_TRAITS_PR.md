# Proposal: Add Async Trait Support to LocationStore and CredentialStore

## Problem Statement

Currently, `sip-registrar::LocationStore` and `sip-auth::CredentialStore` are synchronous traits designed for in-memory storage. This creates significant limitations for production deployments:

**Current Signatures:**
```rust
// sip-registrar/src/lib.rs
pub trait LocationStore: Send + Sync {
    fn upsert(&self, binding: Binding) -> Result<()>;
    fn remove(&self, aor: &str, contact: &str) -> Result<()>;
    fn remove_all(&self, aor: &str) -> Result<()>;
    fn lookup(&self, aor: &str) -> Result<Vec<Binding>>;
    fn cleanup_expired(&self) -> Result<usize>;
}

// sip-auth/src/lib.rs
pub trait CredentialStore: Send + Sync {
    fn fetch(&self, username: &str, realm: &str) -> Option<Credentials>;
}
```

**Limitations:**
- ❌ Cannot use async databases (PostgreSQL, MySQL, MongoDB)
- ❌ Cannot use async file I/O without blocking
- ❌ Forces `tokio::task::block_in_place()` workarounds
- ❌ Blocks tokio runtime threads during I/O
- ❌ Cannot use distributed stores (Redis, etcd)
- ❌ Cannot integrate with async authentication services (LDAP, OAuth)

## Motivation

Real-world SIP servers require persistent, scalable storage:

### Use Cases

1. **Database-Backed Registrations**
   ```rust
   // PostgreSQL-backed location store
   struct PostgresLocationStore {
       pool: sqlx::PgPool,
   }

   // ❌ Currently impossible without blocking
   // ✅ Should work with async traits
   ```

2. **Distributed Registration (Multi-Node PBX)**
   ```rust
   // Redis-backed for cluster deployments
   struct RedisLocationStore {
       client: redis::aio::MultiplexedConnection,
   }
   ```

3. **External Authentication**
   ```rust
   // LDAP/AD integration
   struct LdapCredentialStore {
       client: ldap3::LdapConnAsync,
   }
   ```

4. **High-Performance File Storage**
   ```rust
   // JSON file persistence without blocking
   struct FileBackedStore {
       // Uses tokio::fs for async I/O
   }
   ```

## Proposed Solution

Add async variants while maintaining backward compatibility.

### Option A: Async Traits with Default Implementations (RECOMMENDED)

**Benefits:**
- ✅ Non-breaking for existing code
- ✅ Allows incremental migration
- ✅ Provides both sync and async paths
- ✅ Uses stable Rust features

**Changes:**

#### 1. sip-registrar/Cargo.toml
```toml
[dependencies]
async-trait = "0.1"
# ... existing deps
```

#### 2. sip-registrar/src/lib.rs
```rust
use async_trait::async_trait;

/// Async location store for production deployments.
///
/// Implement this trait when using async storage backends like
/// databases, distributed stores, or file systems.
#[async_trait]
pub trait AsyncLocationStore: Send + Sync {
    /// Insert or update a binding
    async fn upsert(&self, binding: Binding) -> Result<()>;

    /// Remove a specific binding
    async fn remove(&self, aor: &str, contact: &str) -> Result<()>;

    /// Remove all bindings for an AOR
    async fn remove_all(&self, aor: &str) -> Result<()>;

    /// Lookup all bindings for an AOR
    async fn lookup(&self, aor: &str) -> Result<Vec<Binding>>;

    /// Cleanup expired bindings
    async fn cleanup_expired(&self) -> Result<usize>;
}

/// Synchronous location store for in-memory implementations.
///
/// This trait remains unchanged for backward compatibility.
/// New implementations should prefer `AsyncLocationStore`.
pub trait LocationStore: Send + Sync {
    fn upsert(&self, binding: Binding) -> Result<()>;
    fn remove(&self, aor: &str, contact: &str) -> Result<()>;
    fn remove_all(&self, aor: &str) -> Result<()>;
    fn lookup(&self, aor: &str) -> Result<Vec<Binding>>;
    fn cleanup_expired(&self) -> Result<usize>;
}

/// Adapter to use AsyncLocationStore where LocationStore is expected.
///
/// This allows async implementations to work with existing sync APIs
/// by blocking on the async operations.
pub struct AsyncToSyncAdapter<T: AsyncLocationStore> {
    inner: T,
}

impl<T: AsyncLocationStore> AsyncToSyncAdapter<T> {
    pub fn new(inner: T) -> Self {
        Self { inner }
    }
}

impl<T: AsyncLocationStore> LocationStore for AsyncToSyncAdapter<T> {
    fn upsert(&self, binding: Binding) -> Result<()> {
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current()
                .block_on(self.inner.upsert(binding))
        })
    }

    fn remove(&self, aor: &str, contact: &str) -> Result<()> {
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current()
                .block_on(self.inner.remove(aor, contact))
        })
    }

    fn remove_all(&self, aor: &str) -> Result<()> {
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current()
                .block_on(self.inner.remove_all(aor))
        })
    }

    fn lookup(&self, aor: &str) -> Result<Vec<Binding>> {
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current()
                .block_on(self.inner.lookup(aor))
        })
    }

    fn cleanup_expired(&self) -> Result<usize> {
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current()
                .block_on(self.inner.cleanup_expired())
        })
    }
}

/// Adapter to use LocationStore as AsyncLocationStore.
///
/// This allows existing sync implementations to work with async APIs
/// by wrapping sync calls in `spawn_blocking`.
pub struct SyncToAsyncAdapter<T: LocationStore> {
    inner: Arc<T>,
}

impl<T: LocationStore> SyncToAsyncAdapter<T> {
    pub fn new(inner: T) -> Self {
        Self {
            inner: Arc::new(inner),
        }
    }
}

#[async_trait]
impl<T: LocationStore + 'static> AsyncLocationStore for SyncToAsyncAdapter<T> {
    async fn upsert(&self, binding: Binding) -> Result<()> {
        let inner = Arc::clone(&self.inner);
        tokio::task::spawn_blocking(move || inner.upsert(binding))
            .await
            .map_err(|e| anyhow::anyhow!("Task join error: {}", e))?
    }

    async fn remove(&self, aor: &str, contact: &str) -> Result<()> {
        let inner = Arc::clone(&self.inner);
        let aor = aor.to_string();
        let contact = contact.to_string();
        tokio::task::spawn_blocking(move || inner.remove(&aor, &contact))
            .await
            .map_err(|e| anyhow::anyhow!("Task join error: {}", e))?
    }

    async fn remove_all(&self, aor: &str) -> Result<()> {
        let inner = Arc::clone(&self.inner);
        let aor = aor.to_string();
        tokio::task::spawn_blocking(move || inner.remove_all(&aor))
            .await
            .map_err(|e| anyhow::anyhow!("Task join error: {}", e))?
    }

    async fn lookup(&self, aor: &str) -> Result<Vec<Binding>> {
        let inner = Arc::clone(&self.inner);
        let aor = aor.to_string();
        tokio::task::spawn_blocking(move || inner.lookup(&aor))
            .await
            .map_err(|e| anyhow::anyhow!("Task join error: {}", e))?
    }

    async fn cleanup_expired(&self) -> Result<usize> {
        let inner = Arc::clone(&self.inner);
        tokio::task::spawn_blocking(move || inner.cleanup_expired())
            .await
            .map_err(|e| anyhow::anyhow!("Task join error: {}", e))?
    }
}
```

#### 3. Update BasicRegistrar to Support Both

```rust
pub struct BasicRegistrar<L, A> {
    location_store: Arc<L>,
    authenticator: Option<A>,
    config: RegistrarConfig,
}

// Add async variant of handle method
impl<L, A> BasicRegistrar<L, A>
where
    L: AsyncLocationStore,
    A: Authenticator,
{
    /// Handle a REGISTER request asynchronously.
    ///
    /// This method should be preferred when using async storage backends.
    pub async fn handle_async(&self, request: &Request) -> Result<Response> {
        // Implementation that uses await on location_store methods

        // Extract AOR from To header
        let aor = extract_to_uri(request)?;

        // Authenticate if required
        if let Some(ref auth) = self.authenticator {
            if !self.check_auth(request, auth)? {
                return Ok(self.create_unauthorized(request, auth)?);
            }
        }

        // Parse Contact header
        let contacts = parse_contacts(request)?;

        // Handle registration
        if contacts.is_empty() {
            // Query existing bindings
            let bindings = self.location_store.lookup(&aor).await?;
            return Ok(self.create_ok_response(request, &bindings)?);
        }

        // Process each contact
        for contact in contacts {
            if contact.expires == 0 || is_wildcard(&contact) {
                // Deregister
                if is_wildcard(&contact) {
                    self.location_store.remove_all(&aor).await?;
                } else {
                    self.location_store.remove(&aor, &contact.uri).await?;
                }
            } else {
                // Register/update
                let binding = self.create_binding(request, &contact, &aor)?;
                self.location_store.upsert(binding).await?;
            }
        }

        // Return success
        let bindings = self.location_store.lookup(&aor).await?;
        Ok(self.create_ok_response(request, &bindings)?)
    }
}

// Keep existing sync handle() method for backward compatibility
impl<L, A> BasicRegistrar<L, A>
where
    L: LocationStore,
    A: Authenticator,
{
    /// Handle a REGISTER request synchronously.
    ///
    /// This method is retained for backward compatibility.
    /// Consider using `handle_async()` with `AsyncLocationStore` instead.
    pub fn handle(&self, request: &Request) -> Result<Response> {
        // Existing implementation unchanged
        // ...
    }
}
```

#### 4. sip-auth: Similar Changes

```rust
use async_trait::async_trait;

/// Async credential store for production deployments.
#[async_trait]
pub trait AsyncCredentialStore: Send + Sync {
    /// Fetch credentials for a user in a specific realm.
    async fn fetch(&self, username: &str, realm: &str) -> Option<Credentials>;
}

/// Synchronous credential store (backward compatible).
pub trait CredentialStore: Send + Sync {
    fn fetch(&self, username: &str, realm: &str) -> Option<Credentials>;
}

// Adapters similar to LocationStore
pub struct AsyncToSyncCredentialAdapter<T: AsyncCredentialStore> {
    inner: T,
}

pub struct SyncToAsyncCredentialAdapter<T: CredentialStore> {
    inner: Arc<T>,
}

// Implementations...
```

#### 5. Update DigestAuthenticator

```rust
impl<S: AsyncCredentialStore> DigestAuthenticator<S> {
    /// Verify credentials asynchronously.
    pub async fn verify_async(
        &self,
        request: &Request,
        authorization: &Authorization,
    ) -> Result<bool> {
        let creds = self.store.fetch(&authorization.username, &self.realm).await;
        // ... rest of verification logic
    }
}
```

### Migration Examples

#### Example 1: PostgreSQL LocationStore

```rust
use sqlx::PgPool;
use sip_registrar::{AsyncLocationStore, Binding};

pub struct PostgresLocationStore {
    pool: PgPool,
}

#[async_trait]
impl AsyncLocationStore for PostgresLocationStore {
    async fn upsert(&self, binding: Binding) -> Result<()> {
        sqlx::query!(
            r#"
            INSERT INTO registrations (aor, contact, expires, call_id, cseq, q)
            VALUES ($1, $2, $3, $4, $5, $6)
            ON CONFLICT (aor, contact) DO UPDATE
            SET expires = $3, call_id = $4, cseq = $5, q = $6
            "#,
            binding.aor.to_string(),
            binding.contact.to_string(),
            binding.expires as i32,
            binding.call_id.to_string(),
            binding.cseq as i32,
            binding.q,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn lookup(&self, aor: &str) -> Result<Vec<Binding>> {
        let rows = sqlx::query!(
            "SELECT * FROM registrations WHERE aor = $1 AND expires > NOW()",
            aor
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(|row| {
            Binding {
                aor: row.aor.into(),
                contact: row.contact.into(),
                expires: row.expires as u32,
                call_id: row.call_id.into(),
                cseq: row.cseq as u32,
                q: row.q,
            }
        }).collect())
    }

    // ... other methods
}

// Usage
let pool = PgPool::connect("postgres://...").await?;
let store = PostgresLocationStore { pool };
let registrar = BasicRegistrar::builder()
    .location_store(store)
    .build();

// Use async handler
let response = registrar.handle_async(&request).await?;
```

#### Example 2: Redis CredentialStore

```rust
use redis::aio::MultiplexedConnection;
use sip_auth::{AsyncCredentialStore, Credentials};

pub struct RedisCredentialStore {
    conn: MultiplexedConnection,
}

#[async_trait]
impl AsyncCredentialStore for RedisCredentialStore {
    async fn fetch(&self, username: &str, realm: &str) -> Option<Credentials> {
        let key = format!("creds:{}:{}", realm, username);
        let mut conn = self.conn.clone();

        let data: Option<String> = redis::cmd("GET")
            .arg(&key)
            .query_async(&mut conn)
            .await
            .ok()?;

        data.and_then(|json| serde_json::from_str(&json).ok())
    }
}
```

#### Example 3: Existing Code (No Changes Required)

```rust
// Old code continues to work unchanged
let store = MemoryLocationStore::default();
let registrar = BasicRegistrar::builder()
    .location_store(store)
    .build();

// Use sync handler (as before)
let response = registrar.handle(&request)?;
```

## Implementation Plan

### Phase 1: Add Async Traits (Non-Breaking)
- [ ] Add `async-trait` dependency to `sip-registrar` and `sip-auth`
- [ ] Add `AsyncLocationStore` trait alongside `LocationStore`
- [ ] Add `AsyncCredentialStore` trait alongside `CredentialStore`
- [ ] Implement adapters (`AsyncToSyncAdapter`, `SyncToAsyncAdapter`)
- [ ] Update documentation with async examples

### Phase 2: Update BasicRegistrar
- [ ] Add `handle_async()` method that uses async traits
- [ ] Keep existing `handle()` method unchanged
- [ ] Update builder to accept both trait types
- [ ] Add examples for async usage

### Phase 3: Update DigestAuthenticator
- [ ] Add async verification methods
- [ ] Keep existing sync methods
- [ ] Update documentation

### Phase 4: Examples and Tests
- [ ] Add `examples/postgres_registrar.rs`
- [ ] Add `examples/redis_credentials.rs`
- [ ] Add integration tests with tokio-test
- [ ] Update CLAUDE.md with async patterns

### Phase 5: Documentation
- [ ] Update README with async examples
- [ ] Add migration guide
- [ ] Update rustdoc comments
- [ ] Add cookbook entries for common async patterns

## Testing Strategy

```rust
// tests/async_location_store.rs
#[tokio::test]
async fn test_async_location_store() {
    struct TestAsyncStore;

    #[async_trait]
    impl AsyncLocationStore for TestAsyncStore {
        async fn upsert(&self, binding: Binding) -> Result<()> {
            // Simulate async I/O
            tokio::time::sleep(Duration::from_millis(10)).await;
            Ok(())
        }

        async fn lookup(&self, aor: &str) -> Result<Vec<Binding>> {
            tokio::time::sleep(Duration::from_millis(10)).await;
            Ok(vec![])
        }

        // ... other methods
    }

    let store = TestAsyncStore;
    let registrar = BasicRegistrar::builder()
        .location_store(store)
        .build();

    let response = registrar.handle_async(&request).await?;
    assert_eq!(response.start.code, 200);
}

// Test adapter
#[tokio::test]
async fn test_sync_to_async_adapter() {
    let sync_store = MemoryLocationStore::default();
    let async_store = SyncToAsyncAdapter::new(sync_store);

    let binding = create_test_binding();
    async_store.upsert(binding).await?;

    let bindings = async_store.lookup("sip:test@example.com").await?;
    assert_eq!(bindings.len(), 1);
}
```

## Breaking Changes

**None** - This is fully backward compatible:

- ✅ Existing `LocationStore` implementations unchanged
- ✅ Existing `CredentialStore` implementations unchanged
- ✅ Existing `BasicRegistrar::handle()` unchanged
- ✅ New async traits are opt-in additions
- ✅ Adapters provide seamless interop

## Documentation Updates

### README.md additions:

```markdown
## Async Storage Support

siphon-rs supports both synchronous and asynchronous storage backends.

### Using Async Storage

For production deployments with databases or distributed stores:

\`\`\`rust
use sip_registrar::{AsyncLocationStore, BasicRegistrar};

// Implement async trait for your storage
#[async_trait]
impl AsyncLocationStore for MyDatabaseStore {
    async fn upsert(&self, binding: Binding) -> Result<()> {
        // Your async database code
    }
    // ...
}

// Use async handler
let registrar = BasicRegistrar::builder()
    .location_store(my_db_store)
    .build();

let response = registrar.handle_async(&request).await?;
\`\`\`

### Using Sync Storage (In-Memory)

For simple deployments or testing:

\`\`\`rust
let store = MemoryLocationStore::default();
let registrar = BasicRegistrar::builder()
    .location_store(store)
    .build();

let response = registrar.handle(&request)?;  // Sync API
\`\`\`
```

## Benefits

1. **Production Ready**: Enables real database backends
2. **Performance**: Avoids blocking tokio runtime
3. **Backward Compatible**: No breaking changes
4. **Flexible**: Supports both sync and async use cases
5. **Future Proof**: Aligns with modern Rust async ecosystem
6. **Interoperable**: Adapters allow mixing sync/async code

## Alternative Considered: Breaking Change Migration

We could make the traits async directly (breaking change):

**Pros:**
- Cleaner API, no trait duplication
- Forces ecosystem to modernize

**Cons:**
- ❌ Breaks all existing code
- ❌ Forces users to update immediately
- ❌ May not be ready for sync->async migration

**Verdict:** Non-breaking approach (Option A) is better for ecosystem stability.

## Checklist for Implementation

- [ ] Add `async-trait` to `sip-registrar/Cargo.toml`
- [ ] Add `async-trait` to `sip-auth/Cargo.toml`
- [ ] Implement `AsyncLocationStore` trait
- [ ] Implement `AsyncCredentialStore` trait
- [ ] Implement `AsyncToSyncAdapter`
- [ ] Implement `SyncToAsyncAdapter`
- [ ] Add `BasicRegistrar::handle_async()`
- [ ] Add `DigestAuthenticator::verify_async()`
- [ ] Write comprehensive tests
- [ ] Add PostgreSQL example
- [ ] Add Redis example
- [ ] Update documentation
- [ ] Update CHANGELOG.md
- [ ] Bump minor version (0.x.0)

## Estimated Effort

- Implementation: 8-12 hours
- Testing: 4-6 hours
- Documentation: 2-4 hours
- **Total: 14-22 hours**

## Questions for Review

1. Should we add `async-trait` to workspace dependencies?
2. Do we need feature flags for async support, or make it default?
3. Should adapters be in separate module (`sip-registrar::adapters`)?
4. Any preferred naming convention for async methods? (`handle_async` vs `async_handle`)

---

**Ready to implement!** Let me know if you'd like any changes to the proposal.
