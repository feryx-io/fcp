# fcp-storage-postgres

PostgreSQL storage backend for FCP (Ferrous Communications Platform).

## Features

- **Full Storage Trait Implementation**: Complete support for `CredentialStore`, `RegistrationStore`, and `CdrStore`
- **Connection Pooling**: Efficient connection management with configurable pool size
- **JSONB Support**: Native PostgreSQL JSONB for `media_preferences` and `custom_data` fields
- **TIMESTAMPTZ**: Native timestamp handling with timezone support
- **GIN Indexes**: Fast JSON field querying
- **Multi-Tenancy**: All tables include `tenant_id` for isolation
- **Production Ready**: Designed for medium to large-scale deployments

## Installation

Add to your `Cargo.toml`:

```toml
[dependencies]
fcp-storage-postgres = { path = "../fcp-storage-postgres" }
```

## Usage

### Basic Setup

```rust
use fcp_storage_postgres::{PostgresStorage, PostgresConfig};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Configure PostgreSQL connection
    let config = PostgresConfig::new("postgresql://user:password@localhost/fcp");

    // Initialize storage (runs migrations automatically)
    let storage = PostgresStorage::new(config).await?;

    // Use via storage traits
    // storage.get_user("alice").await?;

    Ok(())
}
```

### Configuration Options

```rust
let config = PostgresConfig::new("postgresql://user:password@localhost/fcp")
    .with_max_connections(20)  // Default: 10
    .with_timeout(Duration::from_secs(60));  // Default: 30s
```

### Connection String Formats

```
# Basic
postgresql://user:password@localhost/database

# With explicit port
postgresql://user:password@localhost:5432/database

# With SSL
postgresql://user:password@localhost/database?sslmode=require

# With connection parameters
postgresql://user:password@localhost/database?application_name=fcp&connect_timeout=10
```

## Database Schema

The PostgreSQL backend creates three main tables:

### Users Table

```sql
CREATE TABLE users (
    username TEXT NOT NULL,
    password_hash TEXT NOT NULL,
    display_name TEXT,
    tenant_id TEXT NOT NULL,
    enabled BOOLEAN NOT NULL DEFAULT true,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    media_preferences JSONB,
    PRIMARY KEY (tenant_id, username)
);
```

### Registrations Table

```sql
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
```

### CDRs Table

```sql
CREATE TABLE cdrs (
    call_id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    sip_call_id TEXT NOT NULL,
    -- ... 30+ fields
    custom_data JSONB
);
```

## PostgreSQL-Specific Features

### JSONB Advantages

- **Fast Querying**: GIN indexes enable efficient JSON field searches
- **Storage Efficiency**: Binary format reduces storage size
- **Native Operations**: Use PostgreSQL JSON operators directly

```sql
-- Query by JSON field
SELECT * FROM users WHERE media_preferences->>'codec' = 'PCMU';

-- JSON containment
SELECT * FROM cdrs WHERE custom_data @> '{"call_type": "internal"}';
```

### TIMESTAMPTZ Benefits

- **Timezone Awareness**: Automatic timezone conversion
- **Range Queries**: Efficient timestamp filtering
- **Native Functions**: Use PostgreSQL date/time functions

```sql
-- Time-based queries
SELECT * FROM cdrs WHERE start_time >= NOW() - INTERVAL '24 hours';
```

### Partitioning (Optional)

For high-volume deployments, enable table partitioning:

```sql
-- Partition CDRs by month
ALTER TABLE cdrs PARTITION BY RANGE (start_time);

CREATE TABLE cdrs_2026_01 PARTITION OF cdrs
    FOR VALUES FROM ('2026-01-01') TO ('2026-02-01');

CREATE TABLE cdrs_2026_02 PARTITION OF cdrs
    FOR VALUES FROM ('2026-02-01') TO ('2026-03-01');
```

## Performance Tuning

### Connection Pool

```rust
let config = PostgresConfig::new(connection_string)
    .with_max_connections(50)  // Higher for high concurrency
    .with_timeout(Duration::from_secs(10));
```

### PostgreSQL Configuration

Recommended `postgresql.conf` settings for FCP:

```conf
# Connection settings
max_connections = 200
shared_buffers = 2GB
effective_cache_size = 6GB

# WAL settings
wal_buffers = 16MB
checkpoint_completion_target = 0.9

# Query planning
random_page_cost = 1.1  # For SSD storage
effective_io_concurrency = 200

# Maintenance
autovacuum = on
autovacuum_naptime = 30s
```

## Encryption at rest

Secret columns (SIP digest/HA1/provisioning passwords, trunk passwords,
conference PINs, webhook secrets) are sealed with AES-256-GCM when
`FCP_ENCRYPTION_PROVIDER` is set (`local` master key or `aws-kms`, the latter
behind the `aws-kms` cargo feature). Legacy plaintext stays readable; see
[docs/ENCRYPTION.md](../../docs/ENCRYPTION.md) for the key model, rotation and
the re-encrypt endpoint.

## Testing

`tests/integration.rs` exercises the backend against a real PostgreSQL:
migrations, tenants, users, registrations, CDR query/count/aggregates, rate
plans and balances (including the one-settlement-per-call guarantee under
concurrency). The tests are gated on `FCP_TEST_DATABASE_URL` and skip when it
is unset, so `cargo test` stays green without a database. Each test creates
its own `fcp_test_<hex>` database, migrates it, and drops it at the end, so
the connection must be a superuser (or have CREATEDB).

```bash
# Start a throwaway PostgreSQL (or use scripts/pg-test.sh, which does this for you)
docker run --rm -d --name fcp-pg -p 5432:5432 -e POSTGRES_PASSWORD=postgres postgres:16

# Run the integration tests
FCP_TEST_DATABASE_URL=postgresql://postgres:postgres@localhost:5432/postgres \
  cargo test -p fcp-storage-postgres --test integration

docker rm -f fcp-pg
```

CI runs the same tests against a `postgres:16` service on every push. A test
that panics leaves its `fcp_test_*` database behind; drop stale ones with
`DROP DATABASE … WITH (FORCE)`.

## Migration

### From SQLite

Use `pgloader` for seamless migration:

```bash
pgloader sqlite:///path/to/fcp.db postgresql://user:pass@localhost/fcp
```

### From Other Databases

The schema is standard SQL and can be adapted to other databases with minimal changes.

## Comparison with SQLite Backend

| Feature | SQLite | PostgreSQL |
|---------|--------|------------|
| Setup Complexity | Low | Medium |
| Performance (single node) | Excellent | Excellent |
| Performance (clustered) | N/A | Excellent |
| Concurrent Writes | Good (WAL) | Excellent |
| Max Database Size | ~281 TB | Unlimited |
| JSON Querying | Limited (TEXT) | Native (JSONB) |
| Replication | Manual | Built-in |
| Backup | File copy | pg_dump/pg_basebackup |
| Best For | Small-Medium | Medium-Large |

## Best Practices

1. **Use Connection Pooling**: Never create connections on-demand
2. **Enable Prepared Statements**: Reuse query plans (automatic with sqlx)
3. **Monitor Slow Queries**: Use `pg_stat_statements` extension
4. **Regular VACUUM**: Ensure autovacuum is properly configured
5. **Index Maintenance**: Run `REINDEX` periodically on heavy-write tables
6. **Backup Strategy**: Use `pg_basebackup` for point-in-time recovery

## Troubleshooting

### Connection Pool Exhaustion

```
Error: timed out waiting for connection
```

**Solution**: Increase `max_connections` in config or PostgreSQL settings.

### Slow Queries

```sql
-- Enable query logging
ALTER DATABASE fcp SET log_min_duration_statement = 1000;  -- Log queries > 1s

-- Check slow queries
SELECT * FROM pg_stat_statements ORDER BY mean_exec_time DESC LIMIT 10;
```

### High Memory Usage

Check `shared_buffers` and reduce if necessary. Monitor with:

```sql
SELECT * FROM pg_stat_database WHERE datname = 'fcp';
```

## Security

### SSL/TLS

Always use SSL in production:

```rust
let config = PostgresConfig::new(
    "postgresql://user:password@localhost/fcp?sslmode=require"
);
```

### User Permissions

Grant minimal permissions:

```sql
CREATE USER fcp_app WITH PASSWORD 'secure_password';
GRANT CONNECT ON DATABASE fcp TO fcp_app;
GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA public TO fcp_app;
GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA public TO fcp_app;
```

## License

Same as FCP project.
