# FCP Storage Backends

FCP supports multiple storage backends to accommodate different deployment scenarios, from small single-node setups to large distributed systems.

## Available Backends

### ✅ SQLite (`fcp-storage-sqlite`)
**Status**: Production Ready
**Location**: `crates/fcp-storage-sqlite/`

**Best For**: Small to medium deployments, development, testing

**Advantages**:
- Zero configuration - single file database
- Excellent single-node performance
- No external dependencies
- Perfect for embedded deployments
- WAL mode for concurrent access

**Limitations**:
- Limited concurrent write scalability
- No built-in replication
- JSON stored as TEXT (limited query capabilities)

**Use Cases**:
- Development and testing
- Small offices (< 100 users)
- Embedded systems
- Edge deployments

---

### ✅ PostgreSQL (`fcp-storage-postgres`)
**Status**: Production Ready
**Location**: `crates/fcp-storage-postgres/`

**Best For**: Medium to large deployments requiring advanced features

**Advantages**:
- JSONB for efficient JSON storage with GIN indexes
- Native TIMESTAMPTZ with timezone support
- Advanced features (partitioning, full-text search, stored procedures)
- Excellent query optimizer
- Built-in streaming replication
- Strong ACID compliance

**Unique Features**:
- JSONB binary format with indexing
- Advanced query optimization
- Mature ecosystem with pgbouncer, pg_stat_statements
- Table partitioning by range/list/hash

**Use Cases**:
- Multi-tenant SaaS deployments
- High-volume call centers (> 1000 users)
- Data analytics requirements
- Complex querying needs

---

### ✅ MySQL (`fcp-storage-mysql`)
**Status**: Production Ready
**Location**: `crates/fcp-storage-mysql/`

**Best For**: Medium to large deployments, especially in MySQL ecosystems

**Advantages**:
- Native JSON type with validation
- DATETIME(6) for microsecond precision
- InnoDB engine with row-level locking
- Wide adoption and tooling
- Multiple replication options
- Commercial support available

**Unique Features**:
- Master-slave replication
- Group Replication (multi-master)
- FULLTEXT search indexes
- utf8mb4 character set for full Unicode

**Use Cases**:
- Organizations with existing MySQL expertise
- Deployments requiring MySQL-specific tooling
- Master-slave replication architectures
- Environments with MySQL DBAs

---

### ✅ MongoDB (`fcp-storage-mongodb`)
**Status**: Production Ready
**Location**: `crates/fcp-storage-mongodb/`

**Best For**: Large-scale NoSQL deployments with flexible schema needs

**Advantages**:
- Native BSON storage (no JSON serialization overhead)
- Flexible schema - add fields without migrations
- Horizontal scalability with built-in sharding
- Excellent for document-oriented data
- Built-in replica sets
- Strong query capabilities with aggregation pipeline

**Unique Features**:
- Document-oriented storage model
- Native support for nested data structures
- Built-in sharding for horizontal scaling
- Change streams for real-time updates
- Time series collections (5.0+)
- MongoDB Atlas cloud option

**Use Cases**:
- Multi-tenant SaaS at massive scale
- Deployments requiring rapid schema evolution
- Global distributed systems
- Real-time analytics and change tracking
- Microservices with document-based data

---

## Feature Comparison Matrix

| Feature | SQLite | PostgreSQL | MySQL | MongoDB |
|---------|--------|------------|-------|---------|
| **Setup Complexity** | ⭐ Lowest | ⭐⭐ Medium | ⭐⭐ Medium | ⭐⭐ Medium |
| **Performance (reads)** | ⭐⭐⭐ Excellent | ⭐⭐⭐ Excellent | ⭐⭐⭐ Excellent | ⭐⭐⭐ Excellent |
| **Performance (writes)** | ⭐⭐ Good | ⭐⭐⭐ Excellent | ⭐⭐⭐ Excellent | ⭐⭐⭐ Excellent |
| **Concurrent Writes** | ⭐⭐ Good (WAL) | ⭐⭐⭐ Excellent | ⭐⭐⭐ Excellent | ⭐⭐⭐ Excellent |
| **JSON Support** | ⭐ TEXT only | ⭐⭐⭐ JSONB | ⭐⭐ Native JSON | ⭐⭐⭐ Native BSON |
| **Schema Flexibility** | ❌ Rigid | ❌ Rigid | ❌ Rigid | ⭐⭐⭐ Flexible |
| **Full-Text Search** | ⭐ Basic | ⭐⭐⭐ Advanced | ⭐⭐ Good | ⭐⭐ Good |
| **Replication** | ❌ Manual only | ⭐⭐⭐ Built-in | ⭐⭐⭐ Built-in | ⭐⭐⭐ Built-in |
| **Clustering** | ❌ No | ⭐⭐⭐ Yes | ⭐⭐ Limited | ⭐⭐⭐ Yes |
| **Sharding** | ❌ No | ⭐ Manual | ⭐ Manual | ⭐⭐⭐ Built-in |
| **Max Database Size** | 281 TB | Unlimited | 64 TB (InnoDB) | Unlimited |
| **Operating Cost** | Free | Free | Free/Commercial | Free/Cloud |
| **Ecosystem** | ⭐⭐ Good | ⭐⭐⭐ Excellent | ⭐⭐⭐ Excellent | ⭐⭐⭐ Excellent |

## Technical Details

### Schema Compatibility

All backends use compatible schemas with minor variations:

| Type | SQLite | PostgreSQL | MySQL | MongoDB |
|------|--------|------------|-------|---------|
| **Timestamps** | INTEGER (Unix epoch) | TIMESTAMPTZ | DATETIME(6) | BSON DateTime |
| **JSON** | TEXT | JSONB | JSON | BSON (native) |
| **Boolean** | INTEGER (0/1) | BOOLEAN | BOOLEAN | Boolean |
| **Text** | TEXT | TEXT | VARCHAR/TEXT | String |
| **Decimals** | REAL | DECIMAL | DECIMAL | Double |
| **Schema** | Table-based | Table-based | Table-based | Document-based |

### Connection Strings

```rust
// SQLite
let config = SqliteConfig::new("/var/lib/fcp/fcp.db");

// PostgreSQL
let config = PostgresConfig::new("postgresql://user:pass@localhost/fcp");

// MySQL
let config = MysqlConfig::new("mysql://user:pass@localhost/fcp");

// MongoDB
let config = MongodbConfig::new("mongodb://user:pass@localhost:27017/fcp");

// MongoDB Atlas (cloud)
let config = MongodbConfig::new("mongodb+srv://user:pass@cluster.mongodb.net/fcp");
```

### Storage Traits Implemented

All four backends fully implement:
- ✅ `CredentialStore` - User authentication and management
- ✅ `RegistrationStore` - SIP registration tracking
- ✅ `CdrStore` - Call Detail Record storage and querying

## Performance Benchmarks (Estimated)

*Based on typical hardware (SSD storage, 8-core CPU, 16GB RAM)*

| Operation | SQLite | PostgreSQL | MySQL | MongoDB |
|-----------|--------|------------|-------|---------|
| **User Lookup** | < 1ms | < 1ms | < 1ms | < 1ms |
| **Registration Query** | < 1ms | < 1ms | < 1ms | < 1ms |
| **CDR Insert** | 1-2ms | 1-2ms | 1-2ms | 1-2ms |
| **CDR Query (filtered)** | 5-50ms | 5-50ms | 5-50ms | 5-50ms |
| **Nested JSON Query** | N/A (TEXT) | 2-10ms | 10-30ms | 1-5ms |
| **Concurrent Connections** | ~100 | ~500+ | ~500+ | ~1,000+ |
| **Writes/Second** | ~1,000 | ~10,000+ | ~10,000+ | ~10,000+ |
| **Horizontal Scaling** | No | Limited | Limited | Excellent |

*Note: Actual performance depends on workload, hardware, and configuration.*

## Migration Between Backends

### SQLite → PostgreSQL

```bash
# Using pgloader
pgloader sqlite:///var/lib/fcp/fcp.db postgresql://user:pass@localhost/fcp
```

### SQLite → MySQL

```bash
# Using mysql_convert or manual export
sqlite3 fcp.db .dump > fcp.sql
# Manual adjustments to SQL
mysql -u root -p fcp < fcp.sql
```

### PostgreSQL → MySQL

```bash
# Using pgloader
pgloader postgresql://user:pass@localhost/fcp mysql://user:pass@localhost/fcp
```

### SQL → MongoDB

```bash
# Export from SQL database to JSON
psql -d fcp -c "COPY (SELECT row_to_json(users) FROM users) TO STDOUT" > users.json
psql -d fcp -c "COPY (SELECT row_to_json(registrations) FROM registrations) TO STDOUT" > registrations.json
psql -d fcp -c "COPY (SELECT row_to_json(cdrs) FROM cdrs) TO STDOUT" > cdrs.json

# Import to MongoDB
mongoimport --uri="mongodb://localhost:27017/fcp" --collection users --file users.json
mongoimport --uri="mongodb://localhost:27017/fcp" --collection registrations --file registrations.json
mongoimport --uri="mongodb://localhost:27017/fcp" --collection cdrs --file cdrs.json
```

### MongoDB → SQL

```bash
# Export from MongoDB to JSON
mongoexport --uri="mongodb://localhost:27017/fcp" --collection users --out users.json
mongoexport --uri="mongodb://localhost:27017/fcp" --collection registrations --out registrations.json
mongoexport --uri="mongodb://localhost:27017/fcp" --collection cdrs --out cdrs.json

# Import to SQL (requires custom script for data transformation)
# JSON structure needs to be adapted to SQL schema
```

## Deployment Recommendations

### Scenario: Small Office (10-50 users)
**Recommendation**: SQLite
**Why**: Simple setup, no maintenance, sufficient performance

### Scenario: Medium Business (50-500 users)
**Recommendation**: PostgreSQL or MySQL
**Why**: Better concurrency, replication support, room for growth

### Scenario: Large Enterprise (500+ users)
**Recommendation**: PostgreSQL
**Why**: Best query optimizer, JSONB indexing, mature replication

### Scenario: Multi-Tenant SaaS
**Recommendation**: PostgreSQL
**Why**: Advanced features, excellent multi-tenancy support, JSONB

### Scenario: Existing MySQL Infrastructure
**Recommendation**: MySQL
**Why**: Leverage existing expertise and tooling

### Scenario: Global Multi-Region Deployment
**Recommendation**: MongoDB
**Why**: Built-in sharding, replica sets across regions, flexible schema

### Scenario: Rapid Development with Changing Requirements
**Recommendation**: MongoDB
**Why**: Schema-less design allows quick iteration without migrations

### Scenario: Microservices Architecture
**Recommendation**: MongoDB
**Why**: Document model aligns well with microservices, easy horizontal scaling

## Configuration Examples

### SQLite (Development)

```rust
use fcp_storage_sqlite::{SqliteStorage, SqliteConfig};

let config = SqliteConfig::new("./fcp_dev.db")
    .with_max_connections(5);

let storage = SqliteStorage::new(config).await?;
```

### PostgreSQL (Production)

```rust
use fcp_storage_postgres::{PostgresStorage, PostgresConfig};

let config = PostgresConfig::new("postgresql://fcp:password@db.example.com/fcp")
    .with_max_connections(20)
    .with_timeout(Duration::from_secs(30));

let storage = PostgresStorage::new(config).await?;
```

### MySQL (Production)

```rust
use fcp_storage_mysql::{MysqlStorage, MysqlConfig};

let config = MysqlConfig::new("mysql://fcp:password@db.example.com/fcp")
    .with_max_connections(20)
    .with_timeout(Duration::from_secs(30));

let storage = MysqlStorage::new(config).await?;
```

### MongoDB (Production)

```rust
use fcp_storage_mongodb::{MongodbStorage, MongodbConfig};

let config = MongodbConfig::new("mongodb://fcp:password@db.example.com:27017/fcp")
    .with_timeout(Duration::from_secs(30));

let storage = MongodbStorage::new(config).await?;
```

### MongoDB Atlas (Cloud)

```rust
use fcp_storage_mongodb::{MongodbStorage, MongodbConfig};

let config = MongodbConfig::new(
    "mongodb+srv://fcp:password@cluster.mongodb.net/fcp?retryWrites=true&w=majority"
);

let storage = MongodbStorage::new(config).await?;
```

## Backup Strategies

### SQLite
```bash
# Simple file copy (while service is stopped)
cp /var/lib/fcp/fcp.db /backup/fcp-$(date +%Y%m%d).db

# Or use SQLite backup API (online backup)
sqlite3 /var/lib/fcp/fcp.db ".backup /backup/fcp-$(date +%Y%m%d).db"
```

### PostgreSQL
```bash
# Logical backup
pg_dump -U fcp -d fcp -F c -f /backup/fcp-$(date +%Y%m%d).dump

# Physical backup
pg_basebackup -D /backup/pg_basebackup -F tar -z -P
```

### MySQL
```bash
# Logical backup
mysqldump -u fcp -p --single-transaction fcp > /backup/fcp-$(date +%Y%m%d).sql

# Physical backup (with Percona XtraBackup)
xtrabackup --backup --target-dir=/backup/mysql-$(date +%Y%m%d)
```

### MongoDB
```bash
# Full database backup
mongodump --uri="mongodb://fcp:pass@localhost:27017/fcp" --out=/backup/$(date +%Y%m%d)

# Restore from backup
mongorestore --uri="mongodb://localhost:27017" /backup/20260109/fcp

# Backup with compression
mongodump --uri="mongodb://fcp:pass@localhost:27017/fcp" --gzip --out=/backup/fcp-compressed

# MongoDB Atlas: Automated backups via dashboard
```

## Monitoring

### SQLite
```rust
// Check database size
let size = std::fs::metadata("/var/lib/fcp/fcp.db")?.len();

// WAL mode check
let wal_mode: String = sqlx::query_scalar("PRAGMA journal_mode")
    .fetch_one(&pool).await?;
```

### PostgreSQL
```sql
-- Connection count
SELECT count(*) FROM pg_stat_activity WHERE datname = 'fcp';

-- Database size
SELECT pg_size_pretty(pg_database_size('fcp'));

-- Table sizes
SELECT relname, pg_size_pretty(pg_total_relation_size(relid))
FROM pg_catalog.pg_statio_user_tables
ORDER BY pg_total_relation_size(relid) DESC;
```

### MySQL
```sql
-- Connection count
SHOW STATUS LIKE 'Threads_connected';

-- Database size
SELECT table_schema, SUM(data_length + index_length) / 1024 / 1024 AS size_mb
FROM information_schema.tables
WHERE table_schema = 'fcp'
GROUP BY table_schema;

-- Table sizes
SELECT table_name,
       ROUND(((data_length + index_length) / 1024 / 1024), 2) AS size_mb
FROM information_schema.tables
WHERE table_schema = 'fcp'
ORDER BY size_mb DESC;
```

### MongoDB
```javascript
// Server status
db.serverStatus()

// Connection count
db.serverStatus().connections

// Database size
db.stats()

// Collection sizes
db.users.stats()
db.registrations.stats()
db.cdrs.stats()

// Current operations
db.currentOp()

// Index usage
db.cdrs.aggregate([{ $indexStats: {} }])
```

## Future Backends (Planned)

### Cloud Storage Backends

**Amazon RDS / Aurora**
- Managed MySQL/PostgreSQL
- Automatic backups and updates
- Multi-AZ replication

**Google Cloud SQL**
- Managed MySQL/PostgreSQL
- Automatic scaling
- Integration with GCP services

**Azure Database**
- Managed MySQL/PostgreSQL
- Built-in high availability
- Integration with Azure services

### NoSQL Options (Under Consideration)

**ScyllaDB / Cassandra**
- Horizontal scalability
- Multi-datacenter replication
- Best for write-heavy workloads

**MongoDB**
- Native JSON/BSON support
- Flexible schema
- Aggregation framework

## Summary

FCP provides four production-ready storage backends:

1. **SQLite** - Start simple, perfect for development and small deployments
2. **PostgreSQL** - Scale up with advanced features and excellent query capabilities
3. **MySQL** - Proven reliability with wide adoption and tooling
4. **MongoDB** - Document-oriented NoSQL for massive scale and flexible schemas

All backends implement the same storage traits, making it easy to switch between them as your needs evolve. Choose based on your deployment size, expertise, and requirements.

**Recommendations:**
- **Development**: SQLite (zero configuration)
- **Production (SQL)**: PostgreSQL (best SQL features) or MySQL (existing infrastructure)
- **Production (NoSQL)**: MongoDB (horizontal scaling, flexible schema)
- **Global Scale**: MongoDB with sharding and replica sets
