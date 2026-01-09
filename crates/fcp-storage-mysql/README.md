# fcp-storage-mysql

MySQL storage backend for FCP (Ferrous Communications Platform).

## Features

- **Full Storage Trait Implementation**: Complete support for `CredentialStore`, `RegistrationStore`, and `CdrStore`
- **Connection Pooling**: Efficient connection management with configurable pool size
- **JSON Type**: Native MySQL JSON for `media_preferences` and `custom_data` fields
- **DATETIME(6)**: Microsecond precision timestamps
- **InnoDB Engine**: ACID compliance with row-level locking
- **Multi-Tenancy**: All tables include `tenant_id` for isolation
- **Production Ready**: Designed for medium to large-scale deployments

## Installation

Add to your `Cargo.toml`:

```toml
[dependencies]
fcp-storage-mysql = { path = "../fcp-storage-mysql" }
```

## Usage

### Basic Setup

```rust
use fcp_storage_mysql::{MysqlStorage, MysqlConfig};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Configure MySQL connection
    let config = MysqlConfig::new("mysql://user:password@localhost/fcp");

    // Initialize storage (runs migrations automatically)
    let storage = MysqlStorage::new(config).await?;

    // Use via storage traits
    // storage.get_user("alice").await?;

    Ok(())
}
```

### Configuration Options

```rust
let config = MysqlConfig::new("mysql://user:password@localhost/fcp")
    .with_max_connections(20)  // Default: 10
    .with_timeout(Duration::from_secs(60));  // Default: 30s
```

### Connection String Formats

```
# Basic
mysql://user:password@localhost/database

# With explicit port
mysql://user:password@localhost:3306/database

# With SSL
mysql://user:password@localhost/database?ssl-mode=required

# With connection parameters
mysql://user:password@localhost/database?charset=utf8mb4&wait_timeout=28800
```

## Database Schema

The MySQL backend creates three main tables:

### Users Table

```sql
CREATE TABLE users (
    username VARCHAR(255) NOT NULL,
    password_hash TEXT NOT NULL,
    display_name VARCHAR(255),
    tenant_id VARCHAR(255) NOT NULL,
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    created_at DATETIME(6) NOT NULL DEFAULT CURRENT_TIMESTAMP(6),
    updated_at DATETIME(6) NOT NULL DEFAULT CURRENT_TIMESTAMP(6) ON UPDATE CURRENT_TIMESTAMP(6),
    media_preferences JSON,
    PRIMARY KEY (tenant_id, username)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
```

### Registrations Table

```sql
CREATE TABLE registrations (
    aor VARCHAR(255) NOT NULL,
    contact VARCHAR(512) NOT NULL,
    expires_at DATETIME(6) NOT NULL,
    call_id VARCHAR(255) NOT NULL,
    cseq INT NOT NULL,
    user_agent VARCHAR(255),
    q DOUBLE,
    registered_at DATETIME(6) NOT NULL DEFAULT CURRENT_TIMESTAMP(6),
    PRIMARY KEY (aor, contact)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
```

### CDRs Table

```sql
CREATE TABLE cdrs (
    call_id VARCHAR(255) PRIMARY KEY,
    tenant_id VARCHAR(255) NOT NULL,
    sip_call_id VARCHAR(255) NOT NULL,
    -- ... 30+ fields
    custom_data JSON
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
```

## MySQL-Specific Features

### JSON Type Advantages

- **Native Storage**: Efficient binary storage and validation
- **Automatic Validation**: Invalid JSON is rejected at insert time
- **Path Expressions**: Query JSON fields directly

```sql
-- Query by JSON field
SELECT * FROM users WHERE JSON_EXTRACT(media_preferences, '$.codec') = 'PCMU';

-- JSON contains
SELECT * FROM cdrs WHERE JSON_CONTAINS(custom_data, '{"call_type": "internal"}');
```

### DATETIME(6) Benefits

- **Microsecond Precision**: 6 decimal places (0.000001 second resolution)
- **Auto-Update**: `ON UPDATE CURRENT_TIMESTAMP(6)` for updated_at
- **Range Queries**: Efficient timestamp filtering

```sql
-- Time-based queries
SELECT * FROM cdrs WHERE start_time >= DATE_SUB(NOW(6), INTERVAL 24 HOUR);
```

### InnoDB Features

- **ACID Compliance**: Full transaction support
- **Row-Level Locking**: Better concurrency than table locks
- **Foreign Keys**: Referential integrity (not used yet, but available)
- **Crash Recovery**: Automatic recovery with redo logs

### Partitioning (Optional)

For high-volume deployments, enable table partitioning:

```sql
-- Partition CDRs by range
ALTER TABLE cdrs PARTITION BY RANGE (YEAR(start_time)) (
    PARTITION p2025 VALUES LESS THAN (2026),
    PARTITION p2026 VALUES LESS THAN (2027),
    PARTITION p2027 VALUES LESS THAN (2028),
    PARTITION p_max VALUES LESS THAN MAXVALUE
);
```

Or by hash for even distribution:

```sql
ALTER TABLE cdrs PARTITION BY HASH(MONTH(start_time)) PARTITIONS 12;
```

## Performance Tuning

### Connection Pool

```rust
let config = MysqlConfig::new(connection_string)
    .with_max_connections(50)  // Higher for high concurrency
    .with_timeout(Duration::from_secs(10));
```

### MySQL Configuration

Recommended `my.cnf` / `my.ini` settings for FCP:

```ini
[mysqld]
# Connection settings
max_connections = 200
connect_timeout = 10
max_allowed_packet = 64M

# InnoDB settings
innodb_buffer_pool_size = 2G
innodb_log_file_size = 512M
innodb_flush_log_at_trx_commit = 2
innodb_file_per_table = 1

# Query cache (MySQL 5.7 only, removed in 8.0)
query_cache_type = 1
query_cache_size = 64M

# Temp tables
tmp_table_size = 64M
max_heap_table_size = 64M

# Slow query log
slow_query_log = 1
long_query_time = 1
log_queries_not_using_indexes = 1
```

## Testing

Integration tests require a running MySQL instance:

```bash
# Start MySQL with Docker
docker run --rm -p 3306:3306 \
  -e MYSQL_ROOT_PASSWORD=root \
  -e MYSQL_DATABASE=fcp \
  mysql:8

# Run tests (most are ignored by default)
cargo test -- --ignored

# Or set DATABASE_URL
DATABASE_URL=mysql://root:root@localhost/fcp cargo test
```

## Migration

### From SQLite

Use `mysql_convert` or manual export/import:

```bash
# Export from SQLite
sqlite3 fcp.db .dump > fcp.sql

# Import to MySQL (after some manual adjustments)
mysql -u root -p fcp < fcp.sql
```

### From PostgreSQL

Use `pgloader` or manual migration:

```bash
pgloader postgresql://user:pass@localhost/fcp mysql://user:pass@localhost/fcp
```

## Comparison with Other Backends

| Feature | SQLite | PostgreSQL | MySQL |
|---------|--------|------------|-------|
| Setup Complexity | Low | Medium | Medium |
| Performance (single) | Excellent | Excellent | Excellent |
| Performance (clustered) | N/A | Excellent | Excellent |
| Concurrent Writes | Good (WAL) | Excellent | Excellent |
| Max Database Size | ~281 TB | Unlimited | 64TB (InnoDB) |
| JSON Support | TEXT | JSONB (binary) | JSON (native) |
| Replication | Manual | Built-in | Built-in (master-slave) |
| Backup | File copy | pg_dump | mysqldump |
| Full-Text Search | Basic | Advanced (tsvector) | Good (FULLTEXT) |
| Best For | Small-Medium | Medium-Large | Medium-Large |
| Licensing | Public Domain | PostgreSQL License | GPL/Commercial |

## Best Practices

1. **Use Connection Pooling**: Never create connections on-demand
2. **Enable Slow Query Log**: Monitor and optimize slow queries
3. **Regular Backups**: Use `mysqldump` or `mysqlbackup`
4. **Optimize Tables**: Run `OPTIMIZE TABLE` periodically
5. **Monitor Replication Lag**: If using replication, monitor lag
6. **Use utf8mb4**: For full Unicode support (including emoji)

## Troubleshooting

### Connection Pool Exhaustion

```
Error: timed out waiting for connection
```

**Solution**: Increase `max_connections` in config or MySQL settings.

### Slow Queries

```sql
-- Enable slow query log
SET GLOBAL slow_query_log = 'ON';
SET GLOBAL long_query_time = 1;

-- Check slow queries
SELECT * FROM mysql.slow_log ORDER BY query_time DESC LIMIT 10;
```

### Lock Waits

```sql
-- Check InnoDB lock waits
SELECT * FROM information_schema.innodb_lock_waits;

-- Check InnoDB transactions
SELECT * FROM information_schema.innodb_trx;
```

### High Memory Usage

Check InnoDB buffer pool:

```sql
SHOW VARIABLES LIKE 'innodb_buffer_pool_size';
SHOW STATUS LIKE 'Innodb_buffer_pool%';
```

## Security

### SSL/TLS

Always use SSL in production:

```rust
let config = MysqlConfig::new(
    "mysql://user:password@localhost/fcp?ssl-mode=required"
);
```

### User Permissions

Grant minimal permissions:

```sql
CREATE USER 'fcp_app'@'%' IDENTIFIED BY 'secure_password';
GRANT SELECT, INSERT, UPDATE, DELETE ON fcp.* TO 'fcp_app'@'%';
FLUSH PRIVILEGES;
```

### Password Security

Use strong passwords and consider using MySQL's `mysql_native_password` or `caching_sha2_password`:

```sql
ALTER USER 'fcp_app'@'%' IDENTIFIED WITH caching_sha2_password BY 'secure_password';
```

## High Availability

### MySQL Replication

Setup master-slave replication:

```sql
-- On master
CREATE USER 'repl'@'%' IDENTIFIED BY 'replpassword';
GRANT REPLICATION SLAVE ON *.* TO 'repl'@'%';

-- On slave
CHANGE MASTER TO
  MASTER_HOST='master.example.com',
  MASTER_USER='repl',
  MASTER_PASSWORD='replpassword',
  MASTER_LOG_FILE='mysql-bin.000001',
  MASTER_LOG_POS=107;
START SLAVE;
```

### MySQL Group Replication

For multi-master setup (MySQL 5.7.17+):

```ini
[mysqld]
plugin-load-add=group_replication.so
group_replication_group_name="aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"
group_replication_start_on_boot=off
group_replication_local_address="127.0.0.1:33061"
group_replication_group_seeds="127.0.0.1:33061,127.0.0.1:33062,127.0.0.1:33063"
```

## Advanced Features

### Full-Text Search

Create FULLTEXT indexes for searching:

```sql
ALTER TABLE cdrs ADD FULLTEXT INDEX idx_fulltext (from_uri, to_uri, sip_reason);

-- Search with MATCH AGAINST
SELECT * FROM cdrs
WHERE MATCH(from_uri, to_uri) AGAINST ('alice@example.com' IN BOOLEAN MODE);
```

### Stored Procedures

Create stored procedures for complex queries:

```sql
DELIMITER //
CREATE PROCEDURE GetCallStatsByTenant(IN p_tenant_id VARCHAR(255))
BEGIN
    SELECT
        disposition,
        COUNT(*) as count,
        AVG(call_duration_ms) as avg_duration
    FROM cdrs
    WHERE tenant_id = p_tenant_id
    GROUP BY disposition;
END //
DELIMITER ;
```

### Triggers

Auto-calculate durations:

```sql
CREATE TRIGGER calc_durations BEFORE INSERT ON cdrs
FOR EACH ROW
BEGIN
    IF NEW.end_time IS NOT NULL AND NEW.start_time IS NOT NULL THEN
        SET NEW.total_duration_ms = TIMESTAMPDIFF(MICROSECOND, NEW.start_time, NEW.end_time) / 1000;
    END IF;
END;
```

## Monitoring

### Key Metrics

```sql
-- Connection usage
SHOW STATUS LIKE 'Threads_connected';
SHOW STATUS LIKE 'Max_used_connections';

-- Query statistics
SHOW STATUS LIKE 'Questions';
SHOW STATUS LIKE 'Queries';

-- InnoDB statistics
SHOW ENGINE INNODB STATUS;

-- Table sizes
SELECT
    table_name,
    ROUND(((data_length + index_length) / 1024 / 1024), 2) AS size_mb
FROM information_schema.tables
WHERE table_schema = 'fcp';
```

### Performance Schema

Enable for detailed monitoring (MySQL 5.7+):

```sql
-- Enable performance schema
UPDATE performance_schema.setup_instruments SET ENABLED = 'YES', TIMED = 'YES';

-- Query performance
SELECT * FROM performance_schema.events_statements_summary_by_digest
ORDER BY SUM_TIMER_WAIT DESC LIMIT 10;
```

## License

Same as FCP project.
