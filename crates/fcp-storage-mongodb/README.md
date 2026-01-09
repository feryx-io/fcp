# fcp-storage-mongodb

MongoDB storage backend for FCP (Ferrous Communications Platform).

## Features

- **Full Storage Trait Implementation**: Complete support for `CredentialStore`, `RegistrationStore`, and `CdrStore`
- **Native BSON Storage**: No JSON serialization overhead for nested data
- **Document-Oriented**: Flexible schema with optional fields
- **Automatic Indexing**: Creates all necessary indexes on startup
- **Connection Pooling**: Efficient connection management built into MongoDB driver
- **Horizontal Scalability**: Support for sharding and replica sets
- **Production Ready**: Designed for medium to large-scale NoSQL deployments

## Installation

Add to your `Cargo.toml`:

```toml
[dependencies]
fcp-storage-mongodb = { path = "../fcp-storage-mongodb" }
```

## Usage

### Basic Setup

```rust
use fcp_storage_mongodb::{MongodbStorage, MongodbConfig};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Configure MongoDB connection
    let config = MongodbConfig::new("mongodb://localhost:27017/fcp");

    // Initialize storage (creates indexes automatically)
    let storage = MongodbStorage::new(config).await?;

    // Use via storage traits
    // storage.get_user("alice").await?;

    Ok(())
}
```

### Configuration Options

```rust
let config = MongodbConfig::new("mongodb://localhost:27017/fcp")
    .with_timeout(Duration::from_secs(60))  // Default: 30s
    .with_database("fcp_production");        // Override database name
```

### Connection String Formats

```
# Basic
mongodb://localhost:27017/database

# With authentication
mongodb://user:password@localhost:27017/database

# With replica set
mongodb://host1:27017,host2:27017,host3:27017/database?replicaSet=rs0

# With SSL/TLS
mongodb://localhost:27017/database?tls=true

# MongoDB Atlas (cloud)
mongodb+srv://user:password@cluster.mongodb.net/database

# With connection options
mongodb://localhost:27017/database?retryWrites=true&w=majority
```

## Database Schema

The MongoDB backend uses three collections with automatic indexing:

### Users Collection

```javascript
{
  username: String,              // Username
  password_hash: String,         // Hashed password
  display_name: String?,         // Optional display name
  tenant_id: String,             // Tenant identifier
  enabled: Boolean,              // Account enabled status
  created_at: Date,              // Creation timestamp (BSON DateTime)
  updated_at: Date,              // Last update timestamp (BSON DateTime)
  media_preferences: Object      // Nested BSON document (no serialization)
}

// Indexes:
// - { tenant_id: 1, username: 1 } UNIQUE
// - { tenant_id: 1 }
// - { username: 1 }
```

### Registrations Collection

```javascript
{
  aor: String,                   // Address of Record (e.g., "alice@example.com")
  contact: String,               // Contact URI
  expires_at: Date,              // Expiration timestamp (BSON DateTime)
  call_id: String,               // SIP Call-ID
  cseq: NumberLong,              // CSeq number
  user_agent: String?,           // Optional User-Agent
  q: Double?,                    // Optional q-value (priority)
  registered_at: Date            // Registration timestamp (BSON DateTime)
}

// Indexes:
// - { aor: 1, contact: 1 } UNIQUE
// - { aor: 1 }
// - { expires_at: 1 }
```

### CDRs Collection

```javascript
{
  call_id: String,               // Unique call identifier
  tenant_id: String,             // Tenant identifier
  sip_call_id: String,           // SIP Call-ID
  // Parties
  from_uri: String,
  from_display_name: String?,
  to_uri: String,
  to_display_name: String?,
  // Timestamps (BSON DateTime)
  start_time: Date,
  answer_time: Date?,
  end_time: Date?,
  // Durations (milliseconds)
  setup_duration_ms: NumberLong?,
  call_duration_ms: NumberLong?,
  total_duration_ms: NumberLong?,
  // Outcome
  disposition: String,
  hangup_cause: String,
  hangup_party: String?,
  // SIP details
  sip_status_code: NumberInt?,
  sip_reason: String?,
  // Routing
  route_id: String?,
  gateway_id: String?,
  // Media
  media_anchored: Boolean,
  codec_a_leg: String?,
  codec_b_leg: String?,
  rtp_packets_sent: NumberLong?,
  rtp_packets_received: NumberLong?,
  rtp_bytes_sent: NumberLong?,
  rtp_bytes_received: NumberLong?,
  // Recording
  recording_url: String?,
  // Billing
  cost: Double?,
  currency: String?,
  // Extensibility
  custom_data: Object?           // Nested BSON document
}

// Indexes:
// - { call_id: 1 } UNIQUE
// - { tenant_id: 1 }
// - { start_time: 1 }
// - { from_uri: 1 }
// - { to_uri: 1 }
// - { disposition: 1 }
// - { sip_call_id: 1 }
```

## MongoDB-Specific Features

### Native BSON Storage

MongoDB stores documents in BSON (Binary JSON) format:

- **No Serialization Overhead**: `media_preferences` and `custom_data` are stored as native BSON documents
- **Type Preservation**: Numbers, dates, and booleans maintain their types
- **Nested Queries**: Query nested fields directly without parsing

```javascript
// Query nested media preferences
db.users.find({
  "media_preferences.codecs": "PCMU"
})

// Query nested custom_data
db.cdrs.find({
  "custom_data.call_type": "internal"
})
```

### BSON DateTime

- **Native Timestamp Support**: Millisecond precision timestamps
- **Timezone Aware**: Stores UTC timestamps internally
- **Range Queries**: Efficient date range filtering

```javascript
// Time-based queries
db.cdrs.find({
  start_time: {
    $gte: ISODate("2026-01-01T00:00:00Z"),
    $lt: ISODate("2026-02-01T00:00:00Z")
  }
})
```

### Flexible Schema

- **Dynamic Fields**: Add new fields without migrations
- **Optional Fields**: Fields can be missing or null
- **Schema Validation**: Can add validation rules if needed

```javascript
// Add schema validation (optional)
db.createCollection("users", {
  validator: {
    $jsonSchema: {
      required: ["username", "tenant_id"],
      properties: {
        username: { bsonType: "string" },
        tenant_id: { bsonType: "string" },
        enabled: { bsonType: "bool" }
      }
    }
  }
})
```

### Sharding (Horizontal Scalability)

For high-volume deployments, enable sharding:

```javascript
// Enable sharding on database
sh.enableSharding("fcp")

// Shard CDRs collection by tenant_id
sh.shardCollection("fcp.cdrs", { tenant_id: 1, start_time: 1 })

// Shard users collection by tenant_id
sh.shardCollection("fcp.users", { tenant_id: 1, username: 1 })
```

Or by hashed shard key for even distribution:

```javascript
sh.shardCollection("fcp.cdrs", { tenant_id: "hashed" })
```

## Performance Tuning

### Connection Pool

The MongoDB driver automatically manages connection pooling:

```rust
let config = MongodbConfig::new(connection_string)
    .with_timeout(Duration::from_secs(10));  // Connection timeout
```

Connection pool settings can be configured via connection string:

```
mongodb://localhost:27017/fcp?maxPoolSize=50&minPoolSize=10
```

### MongoDB Configuration

Recommended `mongod.conf` settings for FCP:

```yaml
# Network settings
net:
  port: 27017
  bindIp: 0.0.0.0

# Storage
storage:
  dbPath: /var/lib/mongodb
  journal:
    enabled: true
  wiredTiger:
    engineConfig:
      cacheSizeGB: 2

# Replication
replication:
  replSetName: fcp-rs

# Security
security:
  authorization: enabled
```

### Indexes

All indexes are created automatically on startup:

```rust
// Check index status
let indexes = storage.database()
    .collection::<Document>("users")
    .list_indexes()
    .await?;
```

### Query Optimization

Use `explain()` to analyze query performance:

```javascript
// In mongo shell
db.cdrs.find({ tenant_id: "tenant-123" }).explain("executionStats")
```

## Testing

Integration tests require a running MongoDB instance:

```bash
# Start MongoDB with Docker
docker run --rm -p 27017:27017 --name mongodb mongo:latest

# Run tests (most are ignored by default)
cargo test --package fcp-storage-mongodb -- --ignored

# Or with specific connection string
MONGODB_URL=mongodb://localhost:27017/fcp_test cargo test
```

## Migration

### From SQL Databases

Use `mongoimport` to import from JSON:

```bash
# Export from PostgreSQL/MySQL to JSON
psql -d fcp -c "COPY (SELECT row_to_json(users) FROM users) TO STDOUT" > users.json

# Import to MongoDB
mongoimport --db fcp --collection users --file users.json
```

### From MongoDB Atlas

```bash
# Export from Atlas
mongodump --uri="mongodb+srv://user:pass@cluster.mongodb.net/fcp" --out=dump

# Import to local MongoDB
mongorestore --db fcp dump/fcp
```

## Comparison with SQL Backends

| Feature | SQLite | PostgreSQL | MySQL | MongoDB |
|---------|--------|------------|-------|---------|
| Setup Complexity | Low | Medium | Medium | Low |
| Performance (reads) | Excellent | Excellent | Excellent | Excellent |
| Performance (writes) | Good | Excellent | Excellent | Excellent |
| Concurrent Writes | Good (WAL) | Excellent | Excellent | Excellent |
| JSON Support | TEXT | JSONB | JSON | BSON (native) |
| Schema Flexibility | Rigid | Rigid | Rigid | Flexible |
| Horizontal Scaling | No | Limited | Limited | Excellent |
| Sharding | No | Manual | Manual | Built-in |
| Replication | Manual | Built-in | Built-in | Built-in |
| Max Database Size | 281 TB | Unlimited | 64TB | Unlimited |
| Best For | Small-Medium | Medium-Large | Medium-Large | Large-Scale NoSQL |

## Best Practices

1. **Use Replica Sets**: Always deploy with at least 3-node replica set for HA
2. **Enable Authentication**: Use SCRAM-SHA-256 authentication in production
3. **Regular Backups**: Use `mongodump` or MongoDB Atlas backups
4. **Monitor Performance**: Use MongoDB Cloud Manager or Ops Manager
5. **Index Wisely**: Review index usage with `$indexStats`
6. **Connection Pooling**: Reuse connections, don't create on-demand
7. **Write Concerns**: Use `w: majority` for critical data

## Troubleshooting

### Connection Issues

```
Error: Failed to connect to MongoDB
```

**Solution**:
- Check MongoDB is running: `systemctl status mongod`
- Verify connection string format
- Check firewall: `sudo ufw allow 27017/tcp`

### Authentication Errors

```
Error: Authentication failed
```

**Solution**:
```javascript
// Create user with proper permissions
use fcp
db.createUser({
  user: "fcp_user",
  pwd: "secure_password",
  roles: [{ role: "readWrite", db: "fcp" }]
})
```

### Slow Queries

```javascript
// Enable profiling
db.setProfilingLevel(2)

// Check slow queries
db.system.profile.find({ millis: { $gt: 100 } }).sort({ ts: -1 })

// Check missing indexes
db.system.profile.find({ planSummary: "COLLSCAN" })
```

### High Memory Usage

```javascript
// Check WiredTiger cache usage
db.serverStatus().wiredTiger.cache

// Adjust cache size in mongod.conf
storage:
  wiredTiger:
    engineConfig:
      cacheSizeGB: 4
```

## High Availability

### Replica Set Setup

```javascript
// Initialize replica set
rs.initiate({
  _id: "fcp-rs",
  members: [
    { _id: 0, host: "mongodb1:27017" },
    { _id: 1, host: "mongodb2:27017" },
    { _id: 2, host: "mongodb3:27017" }
  ]
})

// Check replica set status
rs.status()
```

### Read Preferences

```rust
// Configure read preference in connection string
let config = MongodbConfig::new(
    "mongodb://mongodb1:27017,mongodb2:27017,mongodb3:27017/fcp?replicaSet=fcp-rs&readPreference=secondaryPreferred"
);
```

### Write Concerns

```rust
// Use majority write concern for critical data
// Set in connection string:
// mongodb://host/fcp?w=majority&journal=true
```

## Sharding (Advanced)

For multi-tenant SaaS deployments with high volume:

```javascript
// Enable sharding
sh.enableSharding("fcp")

// Shard collections by tenant_id
sh.shardCollection("fcp.users", { tenant_id: 1, username: 1 })
sh.shardCollection("fcp.cdrs", { tenant_id: 1, start_time: 1 })

// Check shard distribution
db.cdrs.getShardDistribution()
```

## Monitoring

### Key Metrics

```javascript
// Server status
db.serverStatus()

// Connection stats
db.serverStatus().connections

// Operation counters
db.serverStatus().opcounters

// Lock statistics
db.serverStatus().locks

// WiredTiger cache
db.serverStatus().wiredTiger.cache
```

### Collection Statistics

```javascript
// Collection stats
db.users.stats()
db.cdrs.stats()

// Index sizes
db.users.stats().indexSizes
```

### Performance Monitoring

```javascript
// Current operations
db.currentOp()

// Kill slow operations
db.killOp(<opid>)

// Database profiler
db.setProfilingLevel(1, { slowms: 100 })
db.system.profile.find().limit(10).sort({ ts: -1 }).pretty()
```

## Security

### Authentication

Always enable authentication in production:

```yaml
# mongod.conf
security:
  authorization: enabled
```

Create application user with minimal permissions:

```javascript
use fcp
db.createUser({
  user: "fcp_app",
  pwd: "secure_password",
  roles: [
    { role: "readWrite", db: "fcp" }
  ]
})
```

### SSL/TLS

Enable TLS for encrypted connections:

```yaml
# mongod.conf
net:
  tls:
    mode: requireTLS
    certificateKeyFile: /etc/ssl/mongodb.pem
    CAFile: /etc/ssl/ca.pem
```

Connection string with TLS:

```
mongodb://user:pass@localhost:27017/fcp?tls=true&tlsCAFile=/path/to/ca.pem
```

### Encryption at Rest

Enable WiredTiger encryption:

```yaml
# mongod.conf
security:
  enableEncryption: true
  encryptionKeyFile: /path/to/keyfile
```

## Backup and Restore

### mongodump / mongorestore

```bash
# Full database backup
mongodump --uri="mongodb://localhost:27017/fcp" --out=/backup/$(date +%Y%m%d)

# Restore from backup
mongorestore --uri="mongodb://localhost:27017" /backup/20260109/fcp

# Backup specific collection
mongodump --uri="mongodb://localhost:27017/fcp" --collection=cdrs --out=/backup/cdrs

# Backup with compression
mongodump --uri="mongodb://localhost:27017/fcp" --gzip --out=/backup/compressed
```

### MongoDB Atlas Backups

MongoDB Atlas provides automated continuous backups:
- Point-in-time restore (up to 72 hours)
- Snapshot frequency: hourly/daily/weekly
- Cross-region backup replication

### Continuous Backup

For self-hosted deployments, use MongoDB Ops Manager or:

```bash
# Filesystem snapshots (requires WiredTiger)
# 1. Stop writes
# 2. fsync and lock: db.fsyncLock()
# 3. Take LVM/EBS snapshot
# 4. Unlock: db.fsyncUnlock()
```

## MongoDB Atlas (Cloud)

FCP works seamlessly with MongoDB Atlas:

```rust
let config = MongodbConfig::new(
    "mongodb+srv://user:password@cluster.mongodb.net/fcp?retryWrites=true&w=majority"
);
```

**Atlas Benefits:**
- Fully managed service
- Automatic backups
- Built-in monitoring
- Free tier available (512MB)
- Auto-scaling
- Multi-region deployment

## Advanced Features

### Aggregation Pipeline

Complex queries using aggregation:

```javascript
// Call statistics by tenant
db.cdrs.aggregate([
  { $match: { tenant_id: "tenant-123" } },
  { $group: {
      _id: "$disposition",
      count: { $sum: 1 },
      avg_duration: { $avg: "$call_duration_ms" }
  }},
  { $sort: { count: -1 } }
])
```

### Change Streams

Real-time notifications on data changes:

```rust
// Watch for registration changes
let pipeline = vec![];
let mut change_stream = storage.database()
    .collection::<Document>("registrations")
    .watch()
    .await?;

while let Some(event) = change_stream.next().await {
    println!("Registration changed: {:?}", event);
}
```

### Time Series Collections (MongoDB 5.0+)

Optimize CDR storage with time series:

```javascript
db.createCollection("cdrs_timeseries", {
  timeseries: {
    timeField: "start_time",
    metaField: "tenant_id",
    granularity: "minutes"
  }
})
```

### Text Search

Full-text search on URI fields:

```javascript
// Create text index
db.cdrs.createIndex({
  from_uri: "text",
  to_uri: "text",
  sip_reason: "text"
})

// Search
db.cdrs.find({
  $text: { $search: "alice example.com" }
})
```

## Deployment Scenarios

### Single Node (Development)

```bash
docker run -d -p 27017:27017 --name mongodb mongo:latest
```

### Replica Set (Production)

```bash
# Use docker-compose or Kubernetes with 3+ nodes
# See: https://www.mongodb.com/docs/manual/tutorial/deploy-replica-set/
```

### Sharded Cluster (Enterprise)

```bash
# Requires config servers, mongos routers, and shard servers
# See: https://www.mongodb.com/docs/manual/tutorial/deploy-shard-cluster/
```

### MongoDB Atlas (Cloud)

Sign up at https://www.mongodb.com/cloud/atlas and create a cluster.

## License

Same as FCP project.
