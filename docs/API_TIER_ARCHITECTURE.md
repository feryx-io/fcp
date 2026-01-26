# FCP API Tier Architecture - Design Document

## Overview

This document defines a **multi-tier API architecture** for FCP that scales independently from call processing infrastructure. The design separates API concerns into specialized layers, each optimized for specific workloads and resource profiles.

## Problem Statement

### Current State

FCP currently includes a comprehensive REST API within `fcp-server`:
- 17+ endpoint groups (users, devices, calls, CDRs, voicemail, phone numbers, etc.)
- WebSocket event streaming
- Shared state with call processing (storage, call manager, dialog manager, media manager)
- Bearer token authentication
- Direct database access from API handlers

**Issues at Scale:**

1. **Resource Contention**
   - API queries compete with call processing for CPU/memory
   - Slow API queries can delay SIP responses
   - WebSocket connections consume memory on call processing nodes

2. **Database Connection Exhaustion**
   - PostgreSQL default max_connections = 100
   - Call processing nodes: 75 connections
   - API layer: only 25 connections remaining
   - Dashboard refreshes exhaust connection pool

3. **Unpredictable Load**
   - Call processing: predictable load based on CPS (calls per second)
   - API layer: bursty (dashboard refreshes, bulk exports, reports)
   - Cannot tune resources for both profiles

4. **Security Isolation**
   - API nodes exposed to external networks (webhooks, integrations)
   - Call processing nodes should be internal-only
   - API compromise shouldn't affect call processing

5. **WebSocket Statefulness**
   - Event streaming creates long-lived stateful connections
   - Cannot horizontally scale without sticky sessions
   - Event buffering consumes memory unpredictably

6. **Heavy Query Impact**
   - CDR exports can return 100K+ records
   - Call history reports require complex joins
   - Analytics queries block transactional operations

## Proposed Architecture

### Four-Tier API Layer

```
┌─────────────────────────────────────────────────────────────┐
│                         Internet                             │
│            (Customer Integrations, Dashboards)               │
└────────────────────────┬────────────────────────────────────┘
                         │
              ┌──────────▼──────────┐
              │  Load Balancer      │
              │  (HAProxy/nginx)    │
              └──────────┬──────────┘
                         │
         ┌───────────────┼───────────────┐
         │               │               │
    ┌────▼─────┐   ┌────▼─────┐   ┌────▼─────┐
    │Tier 1:   │   │Tier 2:   │   │Tier 3:   │
    │API       │   │API       │   │Event     │
    │Gateway   │   │Workers   │   │Streaming │
    │          │   │          │   │          │
    │(Cache,   │   │(CRUD     │   │(WebSocket│
    │ Rate     │   │ ops)     │   │ only)    │
    │ Limit)   │   │          │   │          │
    └────┬─────┘   └────┬─────┘   └────┬─────┘
         │              │              │
         │              │              │
         └──────────────┼──────────────┘
                        │
              ┌─────────▼─────────┐
              │  Tier 4: Jobs     │
              │  (Async ops)      │
              └─────────┬─────────┘
                        │
         ┌──────────────┼──────────────┐
         │              │              │
    ┌────▼────┐    ┌───▼────┐    ┌───▼────┐
    │PostgreSQL│    │Redis   │    │S3/Blob │
    │Primary + │    │Cluster │    │Storage │
    │Replica   │    │        │    │        │
    └─────────┘    └────────┘    └────────┘
```

### Resource Allocation by Tier

| Tier | Purpose | Instances | CPU/RAM | Database | Stateful |
|------|---------|-----------|---------|----------|----------|
| Gateway | Cache, rate limit | 3 | 2/4GB | No | No |
| Workers | CRUD operations | 10 | 2/8GB | Yes | No |
| Events | WebSocket streams | 3 | 4/16GB | Read-only | Yes |
| Jobs | Long operations | 2 | 2/8GB | Yes | Partial |

---

## Tier 1: API Gateway (fcp-api-gateway)

### Purpose

**Public-facing API** with heavy caching and rate limiting. Protects backend services from abuse and reduces database load.

### Responsibilities

1. **Request Routing**
   - Route to API workers (CRUD)
   - Route to event streaming nodes (WebSocket)
   - Route to jobs API (async operations)

2. **Response Caching**
   - Redis-backed cache for GET operations
   - Tenant-aware cache keys
   - Cache invalidation on writes

3. **Rate Limiting**
   - Per-tenant limits (requests per minute)
   - Per-endpoint limits (expensive operations)
   - Burst protection

4. **Authentication**
   - JWT/Bearer token validation (no database)
   - Token caching (Redis)
   - API key validation

5. **Aggregation**
   - Batch multiple requests (GraphQL-style)
   - Reduce N+1 query problem

### Configuration

```toml
# fcp-api-gateway.toml

[service]
type = "api-gateway"
node_id = "api-gw-01"

[network]
bind_address = "0.0.0.0:443"
tls_cert = "/etc/fcp/api.crt"
tls_key = "/etc/fcp/api.key"
enable_http2 = true

[upstream]
# Backend API workers (round-robin)
api_workers = [
    "http://api-worker-01.internal:8080",
    "http://api-worker-02.internal:8080",
    "http://api-worker-03.internal:8080",
]

# Event streaming nodes (sticky session by client IP)
event_nodes = [
    "http://events-01.internal:8081",
    "http://events-02.internal:8081",
]

# Jobs API
jobs_api = "http://jobs-01.internal:8082"

[cache]
# Redis cache for read-heavy endpoints
redis_url = "redis://cache:6379"
redis_pool_size = 100
default_ttl = 60  # seconds

# Cache these endpoints aggressively
cached_endpoints = [
    "GET /api/v1/users/*",
    "GET /api/v1/extensions/*",
    "GET /api/v1/devices/*",
    "GET /api/v1/phone-numbers/*",
    "GET /api/v1/tenants/*",
]

# Cache invalidation patterns (on write)
invalidation_rules = [
    { method = "PUT", path = "/api/v1/users/:username", invalidate = "api:v1:users:{username}:*" },
    { method = "DELETE", path = "/api/v1/users/:username", invalidate = "api:v1:users:{username}:*" },
    { method = "PATCH", path = "/api/v1/extensions/:number", invalidate = "api:v1:extensions:*:{number}:*" },
]

[cache.exclude]
# Never cache these (live data)
patterns = [
    "/api/v1/calls/*",           # Active call state
    "/api/v1/registrations/*",   # Live registrations
    "/api/v1/debug/*",            # Debug endpoints
]

[rate_limit]
# Global rate limit (all tenants combined)
global_requests_per_second = 10000
global_burst = 1000

# Per-tenant rate limits
tenant_requests_per_minute = 1000
tenant_burst = 100

# Per-IP rate limits (prevents single client abuse)
ip_requests_per_minute = 100
ip_burst = 20

# Expensive endpoint limits
expensive_endpoints = [
    "GET /api/v1/cdrs",              # Large result sets
    "GET /api/v1/registrations",     # Many records
    "POST /api/v1/calls",             # Call origination
    "POST /api/v1/jobs",              # Job creation
]
expensive_limit_per_minute = 60

[auth]
# JWT validation (no database hit)
jwt_secret = "${JWT_SECRET}"
jwt_algorithm = "HS256"
token_cache_ttl = 300  # Cache validated tokens for 5 minutes

# Bearer token validation (Redis cache)
bearer_token_cache = true
bearer_token_cache_ttl = 300

[compression]
# Response compression
enable_gzip = true
enable_brotli = true
min_size = 1024  # bytes

[metrics]
enabled = true
bind_address = "0.0.0.0:9090"
endpoint = "/metrics"

[logging]
level = "info"
format = "json"
```

### Cache Key Design

```rust
/// Cache key structure for tenant-aware caching
pub fn cache_key(tenant_id: &TenantId, endpoint: &str, params: &HashMap<String, String>) -> String {
    // Format: api:v1:{endpoint}:{tenant_id}:{params}:v{version}
    // Example: api:v1:users:tenant-123:alice:v1

    let param_str = params.iter()
        .sorted_by_key(|(k, _)| *k)
        .map(|(k, v)| format!("{}={}", k, v))
        .collect::<Vec<_>>()
        .join("&");

    format!("api:v1:{}:{}:{}:v1", endpoint, tenant_id, param_str)
}

/// Cache invalidation on write operations
pub async fn invalidate_cache(redis: &Redis, pattern: &str) -> Result<()> {
    // Find all keys matching pattern
    let keys: Vec<String> = redis.keys(pattern).await?;

    // Delete all matching keys
    for key in keys {
        redis.del(&key).await?;
    }

    Ok(())
}
```

### Rate Limiting Implementation

```rust
/// Token bucket rate limiter
pub struct RateLimiter {
    redis: RedisClient,
}

impl RateLimiter {
    /// Check if request is allowed
    pub async fn check(&self, tenant_id: &TenantId, endpoint: &str) -> Result<bool> {
        let key = format!("ratelimit:{}:{}", tenant_id, endpoint);

        // Use Redis INCR with EXPIRE for sliding window
        let count: i64 = self.redis.incr(&key, 1).await?;

        if count == 1 {
            // First request, set expiry
            self.redis.expire(&key, 60).await?;
        }

        // Check against limit
        let limit = self.get_limit(endpoint);
        Ok(count <= limit)
    }
}
```

### Load Balancing Strategy

**API Workers (Stateless):** Round-robin
- All workers are equivalent
- Any worker can handle any request
- Simple and efficient

**Event Nodes (Stateful):** Sticky session by client IP
- WebSocket connections must stay on same node
- Use consistent hashing or IP-based affinity
- Handle node failures gracefully (reconnect)

---

## Tier 2: API Workers (fcp-admin)

### Purpose

**Stateless CRUD operations** with optimized database access. Handles the bulk of REST API traffic.

### Responsibilities

1. **CRUD Operations**
   - User management
   - Device provisioning
   - Extension configuration
   - Phone number management
   - Tenant administration

2. **Database Access**
   - Dedicated connection pool (50 connections per worker)
   - Statement timeout (prevent slow queries)
   - Read replica routing for heavy queries

3. **Business Logic**
   - Validation
   - Authorization (RBAC)
   - State transitions (e.g., phone number activation)

4. **Event Publishing**
   - Publish changes to event bus
   - Webhook triggers (async)

### Configuration

```toml
# fcp-admin.toml

[service]
type = "admin"
node_id = "api-worker-01"

[network]
bind_address = "0.0.0.0:8080"  # Internal only (behind gateway)

[database]
# Primary database (writes)
primary_url = "postgres://api_user:password@db-primary:5432/fcp"
primary_max_connections = 50
primary_min_connections = 10
statement_timeout = "5s"  # Prevent slow queries
idle_timeout = "10m"

# Read replica (heavy queries)
replica_url = "postgres://api_user:password@db-replica:5432/fcp"
replica_max_connections = 50

# Query routing rules
[database.routing]
# Force to primary (strong consistency)
force_primary = [
    "GET /api/v1/calls/*",         # Live call state
    "GET /api/v1/registrations/*", # Live registrations
]

# Route to replica (eventual consistency OK)
prefer_replica = [
    "GET /api/v1/cdrs",            # Historical data
    "GET /api/v1/users",           # User list
    "GET /api/v1/tenants/*/call-history",
]

[database.connection_pool]
# PgBouncer integration
use_pgbouncer = true
pgbouncer_url = "postgres://pgbouncer:6432/fcp"
pool_mode = "transaction"  # Most efficient

[redis]
url = "redis://redis:6379"
pool_size = 100
# Used for:
# - Session cache
# - Token validation cache
# - Live registration lookup
# - Dialog state lookup

[event_bus]
type = "redis"
url = "redis://redis:6379"
stream_name = "fcp:events"
# Publish events for:
# - User created/updated/deleted
# - Device provisioned/deprovisioned
# - Extension assigned/removed
# - Phone number activated/deactivated

[webhooks]
enabled = true
worker_count = 10
max_retries = 3
timeout = "30s"
# Webhooks are fired asynchronously (don't block API response)

[metrics]
enabled = true
bind_address = "0.0.0.0:9090"

[logging]
level = "info"
format = "json"
```

### Database Connection Pooling

**Without PgBouncer:**
```
10 API Workers × 50 connections = 500 connections needed
PostgreSQL max_connections = 100
PROBLEM: Connection exhaustion
```

**With PgBouncer (transaction pooling):**
```
10 API Workers → 500 virtual connections
         ↓
   PgBouncer (transaction pooling)
         ↓
   PostgreSQL ← 50 real connections

Each connection used only during transaction
Multiplexing ratio: 10:1
```

### Read Replica Routing

```rust
/// Database router that selects primary or replica
pub struct DatabaseRouter {
    primary: PgPool,
    replica: Option<PgPool>,
    routing_rules: RoutingRules,
}

impl DatabaseRouter {
    /// Get appropriate connection pool for request
    pub fn get_pool(&self, method: &str, path: &str) -> &PgPool {
        // Writes always go to primary
        if method != "GET" {
            return &self.primary;
        }

        // Check force_primary rules
        if self.routing_rules.force_primary.iter().any(|pattern| path_matches(path, pattern)) {
            return &self.primary;
        }

        // Check prefer_replica rules
        if self.routing_rules.prefer_replica.iter().any(|pattern| path_matches(path, pattern)) {
            if let Some(ref replica) = self.replica {
                return replica;
            }
        }

        // Default to primary for strong consistency
        &self.primary
    }
}
```

### API Worker Responsibilities Matrix

| Endpoint | Database | Redis | Event Bus | Webhooks |
|----------|----------|-------|-----------|----------|
| POST /users | Primary (write) | Invalidate cache | Publish user.created | Yes |
| GET /users/:id | Replica (read) | Cache hit check | No | No |
| PUT /users/:id | Primary (write) | Invalidate cache | Publish user.updated | Yes |
| DELETE /users/:id | Primary (write) | Invalidate cache | Publish user.deleted | Yes |
| GET /cdrs | Replica (read) | No cache | No | No |
| POST /calls | Primary (write) | Call state | Publish call.originated | Yes |

---

## Tier 3: Event Streaming (fcp-events)

### Purpose

**Dedicated WebSocket event streaming** for real-time updates. Stateful connections isolated from API workers.

### Responsibilities

1. **WebSocket Management**
   - Accept WebSocket connections
   - Maintain connection state
   - Handle ping/pong keepalives

2. **Event Subscription**
   - Subscribe to Redis Streams / NATS
   - Filter events by tenant/call/type
   - Server-side filtering

3. **Event Delivery**
   - Batch events (reduce frame overhead)
   - Backpressure handling
   - Reconnection support

4. **Connection Lifecycle**
   - Authentication (JWT in query param)
   - Idle timeout
   - Graceful shutdown

### Configuration

```toml
# fcp-events.toml

[service]
type = "events"
node_id = "events-01"

[network]
bind_address = "0.0.0.0:8081"  # WebSocket only

[websocket]
max_connections = 1000  # Per node
max_connections_per_tenant = 100  # Prevent single tenant abuse
max_frame_size = "1MB"
ping_interval = "30s"
idle_timeout = "5m"
read_timeout = "10s"
write_timeout = "10s"

# Connection handshake
[websocket.handshake]
# JWT in query param: ws://events:8081/events?token=JWT
require_auth = true
auth_method = "jwt"  # or "bearer"
allow_unauthenticated = false

[event_bus]
type = "redis"
url = "redis://redis:6379"
stream_name = "fcp:events"

# Consumer group per node (ensures each event delivered to only one node)
consumer_group = "fcp-events-node-01"
consumer_name = "fcp-events-node-01-worker"

# Backlog settings
max_backlog_per_connection = 10000  # Events buffered per WebSocket
backlog_drop_strategy = "oldest"    # Drop oldest events if backlog full

[event_bus.batching]
# Batch events to reduce WebSocket frame overhead
enabled = true
batch_size = 100           # Events per WebSocket message
batch_timeout = "100ms"    # Flush if timeout reached

[filtering]
# Server-side filtering (reduces bandwidth)
enable_server_side_filtering = true
max_filter_complexity = 10  # Prevent DoS via complex filters

# Example filter:
# {
#   "tenant_id": "tenant-123",
#   "event_types": ["call", "media"],
#   "call_id": "call-456"
# }

[memory]
# Memory limits per connection
max_memory_per_connection = "10MB"
memory_check_interval = "10s"
# Disconnect if over limit

[metrics]
enabled = true
bind_address = "0.0.0.0:9090"
# Metrics:
# - Active connections
# - Events delivered per second
# - Backlog size per connection
# - Memory usage per connection

[logging]
level = "info"
format = "json"
```

### WebSocket Protocol

**Connection:**
```javascript
// Client connects with JWT
const ws = new WebSocket('wss://api.example.com/api/v1/events?token=JWT_TOKEN');

ws.onopen = () => {
    // Send subscription filter
    ws.send(JSON.stringify({
        type: 'subscribe',
        filter: {
            tenant_id: 'tenant-123',
            event_types: ['call', 'media'],
            call_id: 'call-456'  // Optional
        }
    }));
};

ws.onmessage = (event) => {
    const data = JSON.parse(event.data);

    if (data.type === 'connected') {
        console.log('Connected:', data.message);
    } else if (data.type === 'events') {
        // Batch of events
        data.events.forEach(evt => {
            console.log('Event:', evt);
        });
    }
};
```

**Event Format:**
```json
{
  "type": "events",
  "batch_size": 3,
  "events": [
    {
      "id": "evt-123",
      "timestamp": "2025-01-26T12:00:00Z",
      "tenant_id": "tenant-123",
      "event_type": "call",
      "event_name": "call.created",
      "data": {
        "call_id": "call-456",
        "from": "sip:alice@example.com",
        "to": "sip:bob@example.com"
      }
    },
    {
      "id": "evt-124",
      "timestamp": "2025-01-26T12:00:01Z",
      "tenant_id": "tenant-123",
      "event_type": "call",
      "event_name": "call.answered",
      "data": {
        "call_id": "call-456",
        "answered_at": "2025-01-26T12:00:01Z"
      }
    }
  ]
}
```

### Event Bus Integration

**Redis Streams:**
```rust
pub struct EventStreamSubscriber {
    redis: RedisClient,
    stream_name: String,
    consumer_group: String,
    consumer_name: String,
}

impl EventStreamSubscriber {
    pub async fn subscribe(&self, filter: EventFilter) -> Result<EventStream> {
        // Create consumer group if not exists
        let _ = self.redis
            .xgroup_create(&self.stream_name, &self.consumer_group, "0")
            .await;

        // Read from consumer group
        let stream = self.redis
            .xreadgroup(
                &self.consumer_group,
                &self.consumer_name,
                &self.stream_name,
                ">",  // Only new messages
            )
            .await?;

        // Filter events based on subscription
        let filtered = stream.filter(|event| filter.matches(event));

        Ok(filtered)
    }
}
```

### Sticky Session Implementation

**HAProxy Configuration:**
```
frontend api_events
    bind *:443 ssl crt /etc/haproxy/certs/

    # WebSocket upgrade
    acl is_websocket hdr(Upgrade) -i WebSocket
    acl is_events_path path_beg /api/v1/events

    # Sticky session by source IP (for WebSocket)
    use_backend events_backend if is_websocket is_events_path

backend events_backend
    balance source  # Sticky by client IP
    hash-type consistent  # Consistent hashing

    # Health checks
    option httpchk GET /health

    # Event nodes
    server events-01 events-01.internal:8081 check
    server events-02 events-02.internal:8081 check
    server events-03 events-03.internal:8081 check
```

### Memory Management

```rust
/// Monitor memory usage per WebSocket connection
pub struct ConnectionMemoryMonitor {
    connections: DashMap<ConnectionId, MemoryStats>,
    max_per_connection: usize,
}

impl ConnectionMemoryMonitor {
    pub async fn check_connection(&self, conn_id: &ConnectionId) -> Result<()> {
        let stats = self.connections.get(conn_id).ok_or(Error::NotFound)?;

        // Check backlog size
        let backlog_size = stats.backlog_events * AVG_EVENT_SIZE;

        if backlog_size > self.max_per_connection {
            // Disconnect or drop oldest events
            warn!(
                connection_id = %conn_id,
                backlog_size = backlog_size,
                "Connection backlog exceeded limit"
            );

            return Err(Error::BacklogExceeded);
        }

        Ok(())
    }
}
```

---

## Tier 4: Async Jobs API (fcp-jobs)

### Purpose

**Long-running operations** that don't fit request/response model. Prevents API timeouts and allows tracking of multi-step operations.

### Responsibilities

1. **Job Management**
   - Create jobs
   - Track progress
   - Store results

2. **Job Execution**
   - Worker pool
   - Timeout handling
   - Retry logic

3. **Result Storage**
   - S3 / Blob storage for large results
   - Redis for metadata

4. **Notifications**
   - Webhook callbacks on completion
   - Event bus publishing

### Configuration

```toml
# fcp-jobs.toml

[service]
type = "jobs"
node_id = "jobs-01"

[network]
bind_address = "0.0.0.0:8082"

[queue]
# Redis-based job queue
type = "redis"
url = "redis://redis:6379"
queue_name = "fcp:jobs"

# Worker pool
worker_count = 10  # Parallel job execution
max_job_duration = "30m"
job_timeout = "35m"  # Slightly longer than max duration

# Retry logic
max_retries = 3
retry_backoff = "exponential"  # 1s, 2s, 4s, 8s, ...

[storage]
# S3 for job results
backend = "s3"
bucket = "fcp-job-results"
region = "us-east-1"
endpoint = "https://s3.amazonaws.com"  # Or MinIO endpoint

# Signed URL expiry
presigned_url_ttl = "7d"

# Auto-cleanup
result_ttl = "30d"  # Delete results after 30 days

[database]
# For job metadata (status, progress)
url = "postgres://jobs_user:password@db:5432/fcp"
max_connections = 20

[webhooks]
enabled = true
timeout = "30s"
max_retries = 3

[metrics]
enabled = true
bind_address = "0.0.0.0:9090"
```

### Job Types

#### 1. CDR Export
```rust
pub struct CdrExportJob {
    pub tenant_id: TenantId,
    pub start_date: DateTime<Utc>,
    pub end_date: DateTime<Utc>,
    pub format: ExportFormat,  // CSV, JSON, Parquet
    pub filters: Option<CdrFilters>,
}

impl Job for CdrExportJob {
    async fn execute(&self, ctx: &JobContext) -> Result<JobResult> {
        // Stream CDRs from database
        let mut stream = self.storage.query_cdrs_stream(&self.filters).await?;

        // Write to temporary file
        let mut writer = csv::Writer::new(File::create(&temp_path)?);

        let mut count = 0;
        while let Some(cdr) = stream.next().await {
            writer.serialize(cdr?)?;

            count += 1;
            if count % 1000 == 0 {
                // Update progress
                ctx.update_progress(count).await?;
            }
        }

        writer.flush()?;

        // Upload to S3
        let s3_key = format!("cdr_export_{}_{}.csv", self.tenant_id, Uuid::new_v4());
        self.s3.upload(&temp_path, &s3_key).await?;

        // Generate presigned URL
        let download_url = self.s3.presigned_url(&s3_key, Duration::days(7)).await?;

        Ok(JobResult {
            status: JobStatus::Completed,
            result_url: Some(download_url),
            metadata: json!({
                "record_count": count,
                "file_size": std::fs::metadata(&temp_path)?.len(),
            }),
        })
    }
}
```

#### 2. Bulk Device Provisioning
```rust
pub struct BulkDeviceProvisionJob {
    pub tenant_id: TenantId,
    pub devices: Vec<DeviceConfig>,
}

impl Job for BulkDeviceProvisionJob {
    async fn execute(&self, ctx: &JobContext) -> Result<JobResult> {
        let mut results = Vec::new();

        for (idx, device_config) in self.devices.iter().enumerate() {
            // Provision device
            match self.provision_device(device_config).await {
                Ok(device) => {
                    results.push(json!({
                        "mac_address": device_config.mac_address,
                        "status": "success",
                        "device_id": device.id,
                    }));
                }
                Err(e) => {
                    results.push(json!({
                        "mac_address": device_config.mac_address,
                        "status": "failed",
                        "error": e.to_string(),
                    }));
                }
            }

            // Update progress
            let progress = (idx + 1) * 100 / self.devices.len();
            ctx.update_progress(progress).await?;
        }

        // Store results in S3
        let results_json = serde_json::to_string_pretty(&results)?;
        let s3_key = format!("bulk_provision_{}_{}.json", self.tenant_id, Uuid::new_v4());
        self.s3.upload_string(&results_json, &s3_key).await?;

        let download_url = self.s3.presigned_url(&s3_key, Duration::days(7)).await?;

        Ok(JobResult {
            status: JobStatus::Completed,
            result_url: Some(download_url),
            metadata: json!({
                "total": self.devices.len(),
                "success": results.iter().filter(|r| r["status"] == "success").count(),
                "failed": results.iter().filter(|r| r["status"] == "failed").count(),
            }),
        })
    }
}
```

#### 3. Tenant Migration
```rust
pub struct TenantMigrationJob {
    pub source_tenant_id: TenantId,
    pub dest_tenant_id: TenantId,
    pub include_cdrs: bool,
    pub include_recordings: bool,
}

impl Job for TenantMigrationJob {
    async fn execute(&self, ctx: &JobContext) -> Result<JobResult> {
        // Step 1: Copy users
        ctx.set_status("Migrating users...").await?;
        let users = self.copy_users().await?;
        ctx.update_progress(20).await?;

        // Step 2: Copy devices
        ctx.set_status("Migrating devices...").await?;
        let devices = self.copy_devices().await?;
        ctx.update_progress(40).await?;

        // Step 3: Copy extensions
        ctx.set_status("Migrating extensions...").await?;
        let extensions = self.copy_extensions().await?;
        ctx.update_progress(60).await?;

        // Step 4: Copy CDRs (optional, can be huge)
        if self.include_cdrs {
            ctx.set_status("Migrating CDRs...").await?;
            let cdrs = self.copy_cdrs().await?;
            ctx.update_progress(80).await?;
        }

        // Step 5: Copy recordings (optional, large files)
        if self.include_recordings {
            ctx.set_status("Migrating recordings...").await?;
            let recordings = self.copy_recordings().await?;
            ctx.update_progress(100).await?;
        }

        Ok(JobResult {
            status: JobStatus::Completed,
            metadata: json!({
                "users_migrated": users,
                "devices_migrated": devices,
                "extensions_migrated": extensions,
            }),
        })
    }
}
```

### API Endpoints

**Create Job:**
```bash
POST /api/v1/jobs
Content-Type: application/json
Authorization: Bearer <token>

{
  "type": "cdr_export",
  "params": {
    "start_date": "2025-01-01T00:00:00Z",
    "end_date": "2025-01-31T23:59:59Z",
    "format": "csv"
  },
  "webhook_url": "https://example.com/webhooks/job-completed"
}

Response: 202 Accepted
{
  "job_id": "job-abc123",
  "status": "queued",
  "created_at": "2025-01-26T12:00:00Z",
  "estimated_duration": "5m"
}
```

**Get Job Status:**
```bash
GET /api/v1/jobs/job-abc123

Response: 200 OK
{
  "job_id": "job-abc123",
  "type": "cdr_export",
  "status": "processing",
  "progress": 45,
  "status_message": "Exporting records...",
  "created_at": "2025-01-26T12:00:00Z",
  "started_at": "2025-01-26T12:00:05Z",
  "estimated_completion": "2025-01-26T12:05:00Z",
  "result_url": null
}
```

**Job Completed:**
```bash
GET /api/v1/jobs/job-abc123

Response: 200 OK
{
  "job_id": "job-abc123",
  "type": "cdr_export",
  "status": "completed",
  "progress": 100,
  "status_message": "Export completed",
  "created_at": "2025-01-26T12:00:00Z",
  "started_at": "2025-01-26T12:00:05Z",
  "completed_at": "2025-01-26T12:04:32Z",
  "duration": "4m27s",
  "result_url": "https://s3.amazonaws.com/fcp-job-results/cdr_export_abc123.csv?signature=...",
  "metadata": {
    "record_count": 125000,
    "file_size": 52428800
  }
}
```

**Webhook Callback (when complete):**
```bash
POST https://example.com/webhooks/job-completed
Content-Type: application/json

{
  "job_id": "job-abc123",
  "status": "completed",
  "result_url": "https://s3.amazonaws.com/fcp-job-results/cdr_export_abc123.csv?signature=...",
  "metadata": {
    "record_count": 125000,
    "file_size": 52428800
  }
}
```

### Job Queue Implementation

```rust
/// Redis-based job queue
pub struct JobQueue {
    redis: RedisClient,
    queue_name: String,
}

impl JobQueue {
    /// Enqueue a new job
    pub async fn enqueue(&self, job: Job) -> Result<JobId> {
        let job_id = JobId::new();

        // Store job data
        let job_key = format!("job:{}", job_id);
        let job_data = serde_json::to_string(&job)?;
        self.redis.set(&job_key, &job_data).await?;

        // Add to queue (sorted set by priority)
        self.redis.zadd(&self.queue_name, job_id.to_string(), 0).await?;

        Ok(job_id)
    }

    /// Dequeue next job
    pub async fn dequeue(&self) -> Result<Option<(JobId, Job)>> {
        // Pop from queue (blocking with timeout)
        let result: Option<String> = self.redis
            .bzpopmin(&self.queue_name, 5.0)
            .await?;

        if let Some(job_id_str) = result {
            let job_id = JobId::from_str(&job_id_str)?;
            let job_key = format!("job:{}", job_id);
            let job_data: String = self.redis.get(&job_key).await?;
            let job: Job = serde_json::from_str(&job_data)?;

            Ok(Some((job_id, job)))
        } else {
            Ok(None)
        }
    }
}
```

---

## Database Access Patterns

### Primary vs Replica Routing

**Strong Consistency (Primary):**
- Live call state: `GET /api/v1/calls/:id`
- Live registrations: `GET /api/v1/registrations/:aor`
- User authentication: `POST /api/v1/auth/login`
- All writes: `POST`, `PUT`, `DELETE`, `PATCH`

**Eventual Consistency OK (Replica):**
- Historical data: `GET /api/v1/cdrs`
- User lists: `GET /api/v1/users`
- Device inventory: `GET /api/v1/devices`
- Call history: `GET /api/v1/tenants/:id/call-history`
- Reports and analytics

### Connection Pooling Architecture

```
┌──────────────────────────────────────────────────┐
│          API Workers (10 instances)              │
│  Each with 50 virtual connections = 500 total    │
└────────────────────┬─────────────────────────────┘
                     │
          ┌──────────▼──────────┐
          │     PgBouncer       │
          │  (Transaction Pool) │
          │  - Pool size: 50    │
          │  - Mode: transaction│
          │  - Max client: 5000 │
          └──────────┬──────────┘
                     │
      ┌──────────────┴──────────────┐
      │                             │
┌─────▼──────┐            ┌────────▼────────┐
│PostgreSQL  │            │  PostgreSQL     │
│  Primary   │◄───────────│   Replica       │
│            │ Replication│                 │
│max_conn:50 │            │  max_conn: 50   │
└────────────┘            └─────────────────┘
```

**Benefits:**
1. API workers can have 500 virtual connections
2. Only 50 real connections to PostgreSQL
3. Multiplexing ratio: 10:1
4. Connections released immediately after transaction

### Query Optimization

**Problem Query (N+1):**
```rust
// BAD: N+1 query problem
pub async fn list_extensions_with_devices(tenant_id: &TenantId) -> Result<Vec<ExtensionWithDevices>> {
    let extensions = storage.list_extensions(tenant_id).await?;

    let mut result = Vec::new();
    for ext in extensions {
        // N queries (one per extension)
        let devices = storage.get_extension_devices(&ext.id).await?;
        result.push(ExtensionWithDevices { ext, devices });
    }

    Ok(result)
}
```

**Optimized (Single Query with JOIN):**
```rust
// GOOD: Single query with JOIN
pub async fn list_extensions_with_devices(tenant_id: &TenantId) -> Result<Vec<ExtensionWithDevices>> {
    let rows = sqlx::query!(
        r#"
        SELECT
            e.id, e.number, e.name,
            d.id as device_id, d.mac_address, d.type
        FROM extensions e
        LEFT JOIN device_bindings db ON db.extension_id = e.id
        LEFT JOIN devices d ON d.id = db.device_id
        WHERE e.tenant_id = $1
        ORDER BY e.number, d.mac_address
        "#,
        tenant_id.as_uuid()
    )
    .fetch_all(&pool)
    .await?;

    // Group by extension
    let result = group_by_extension(rows);
    Ok(result)
}
```

### Index Strategy

**Critical Indexes for API Performance:**
```sql
-- User lookups
CREATE INDEX idx_users_tenant_username ON users(tenant_id, username);
CREATE INDEX idx_users_tenant_email ON users(tenant_id, email);

-- Device lookups
CREATE INDEX idx_devices_tenant_mac ON devices(tenant_id, mac_address);
CREATE INDEX idx_device_bindings_extension ON device_bindings(extension_id);

-- Extension lookups
CREATE INDEX idx_extensions_tenant_number ON extensions(tenant_id, number);

-- CDR queries (most common filters)
CREATE INDEX idx_cdrs_tenant_created ON cdrs(tenant_id, created_at DESC);
CREATE INDEX idx_cdrs_call_id ON cdrs(call_id);
CREATE INDEX idx_cdrs_from_number ON cdrs(from_number);
CREATE INDEX idx_cdrs_to_number ON cdrs(to_number);

-- Registration lookups
CREATE INDEX idx_registrations_aor ON registrations(aor);
CREATE INDEX idx_registrations_tenant_user ON registrations(tenant_id, user_id);

-- Phone numbers
CREATE INDEX idx_phone_numbers_e164 ON phone_numbers(e164);
CREATE INDEX idx_phone_numbers_tenant_state ON phone_numbers(tenant_id, state);
```

---

## Caching Strategy

### Three-Level Caching

**L1: Gateway Cache (Redis)**
- Response caching for GET operations
- TTL: 60 seconds
- Invalidation on writes
- Tenant-aware keys

**L2: Worker In-Memory Cache (LRU)**
- Frequently accessed reference data
- TTL: 5 minutes
- Size: 1000 entries per worker
- Invalidated via event bus

**L3: Database Query Cache (PostgreSQL)**
- PostgreSQL shared_buffers
- Automatically managed
- 25% of RAM recommended

### Cache Invalidation Strategies

**1. Write-Through Invalidation**
```rust
// On write operation, invalidate cache
pub async fn update_user(&self, user: User) -> Result<()> {
    // Write to database
    self.storage.put_user(user.clone()).await?;

    // Invalidate cache (gateway layer)
    let cache_key = format!("api:v1:users:{}:{}:*", user.tenant_id, user.username);
    self.cache.invalidate(&cache_key).await?;

    // Publish event (invalidates worker L2 cache)
    self.event_bus.publish(Event::UserUpdated { user }).await?;

    Ok(())
}
```

**2. TTL-Based Expiration**
```rust
// Cache with TTL for data that changes infrequently
pub async fn get_tenant(&self, tenant_id: &TenantId) -> Result<Tenant> {
    let cache_key = format!("api:v1:tenants:{}:v1", tenant_id);

    // Check cache
    if let Some(cached) = self.cache.get(&cache_key).await? {
        return Ok(serde_json::from_str(&cached)?);
    }

    // Cache miss - fetch from database
    let tenant = self.storage.get_tenant(tenant_id).await?;

    // Cache with 5 minute TTL
    let tenant_json = serde_json::to_string(&tenant)?;
    self.cache.set(&cache_key, &tenant_json, Duration::minutes(5)).await?;

    Ok(tenant)
}
```

**3. Event-Based Invalidation**
```rust
// Worker subscribes to events for cache invalidation
pub struct WorkerCacheInvalidator {
    cache: Arc<Mutex<LruCache<String, Tenant>>>,
    event_subscriber: EventSubscriber,
}

impl WorkerCacheInvalidator {
    pub async fn run(&self) {
        let mut stream = self.event_subscriber.subscribe().await;

        while let Some(event) = stream.next().await {
            match event {
                Event::TenantUpdated { tenant_id } => {
                    // Remove from L2 cache
                    let key = format!("tenant:{}", tenant_id);
                    self.cache.lock().unwrap().pop(&key);
                }
                Event::UserUpdated { user } => {
                    let key = format!("user:{}:{}", user.tenant_id, user.username);
                    self.cache.lock().unwrap().pop(&key);
                }
                _ => {}
            }
        }
    }
}
```

### Cache Key Versioning

```rust
/// Cache key with version (for schema changes)
pub fn cache_key_with_version(
    entity_type: &str,
    tenant_id: &TenantId,
    entity_id: &str,
    version: u32,
) -> String {
    format!("api:v{}:{}:{}:{}:v{}",
        API_VERSION, entity_type, tenant_id, entity_id, version)
}

// Example:
// api:v1:users:tenant-123:alice:v1
//     ^    ^       ^         ^    ^
//     |    |       |         |    Schema version
//     |    |       |         Entity ID
//     |    |       Tenant ID
//     |    Entity type
//     API version
```

When schema changes:
1. Increment version number
2. All old cache entries automatically miss
3. New entries use new schema
4. No need to flush entire cache

---

## Security and Authentication

### Multi-Tier Authentication

**Gateway Layer:**
- JWT validation (no database hit)
- API key validation (Redis cache)
- Rate limiting by tenant

**Worker Layer:**
- Authorization (RBAC)
- Tenant isolation validation
- Audit logging

**Events Layer:**
- WebSocket authentication (JWT in query param)
- Tenant filtering

### JWT Design

```rust
/// JWT claims for API access
#[derive(Serialize, Deserialize)]
pub struct ApiClaims {
    pub sub: String,          // User ID or service account ID
    pub tenant_id: TenantId,  // Tenant context
    pub roles: Vec<String>,   // RBAC roles
    pub scopes: Vec<String>,  // API scopes (read:users, write:devices, etc.)
    pub exp: i64,             // Expiration
    pub iat: i64,             // Issued at
    pub jti: String,          // JWT ID (for revocation)
}

impl ApiClaims {
    /// Check if JWT has required scope
    pub fn has_scope(&self, scope: &str) -> bool {
        self.scopes.iter().any(|s| s == scope || s == "*")
    }

    /// Check if JWT has required role
    pub fn has_role(&self, role: &str) -> bool {
        self.roles.iter().any(|r| r == role || r == "admin")
    }
}
```

### RBAC Implementation

```rust
/// Role-based access control
pub struct RbacChecker {
    redis: RedisClient,
}

impl RbacChecker {
    /// Check if user can access resource
    pub async fn check_access(
        &self,
        user_id: &UserId,
        tenant_id: &TenantId,
        resource: &str,
        action: &str,
    ) -> Result<bool> {
        // Get user roles (cached in Redis)
        let roles = self.get_user_roles(user_id, tenant_id).await?;

        // Check permissions
        for role in roles {
            let perms = self.get_role_permissions(&role).await?;

            if perms.allows(resource, action) {
                return Ok(true);
            }
        }

        Ok(false)
    }
}

/// Example usage in API handler
pub async fn update_user(
    State(state): State<ApiState>,
    claims: JwtClaims,  // Extracted by middleware
    Path(username): Path<String>,
    Json(req): Json<UpdateUserRequest>,
) -> Result<impl IntoResponse, ApiError> {
    // Check RBAC
    let can_update = state.rbac
        .check_access(&claims.sub, &claims.tenant_id, "users", "write")
        .await?;

    if !can_update {
        return Err(ApiError::Forbidden("Insufficient permissions".into()));
    }

    // Proceed with update
    // ...
}
```

### Tenant Isolation

```rust
/// Middleware to enforce tenant isolation
pub async fn enforce_tenant_isolation(
    claims: JwtClaims,
    Path(params): Path<HashMap<String, String>>,
    req: Request,
    next: Next,
) -> Result<Response, ApiError> {
    // Extract tenant_id from path/query/body
    let requested_tenant = extract_tenant_id(&params, &req)?;

    // Verify JWT tenant matches requested tenant
    if claims.tenant_id != requested_tenant {
        return Err(ApiError::Forbidden(
            "Cannot access resources for different tenant".into()
        ));
    }

    // Tenant isolation enforced, proceed
    Ok(next.run(req).await)
}
```

---

## Monitoring and Observability

### Metrics per Tier

**Gateway Metrics:**
- `api_gateway_requests_total{endpoint, method, status}` - Total requests
- `api_gateway_cache_hits_total{endpoint}` - Cache hits
- `api_gateway_cache_misses_total{endpoint}` - Cache misses
- `api_gateway_rate_limit_exceeded_total{tenant_id}` - Rate limit violations
- `api_gateway_request_duration_seconds{endpoint, method}` - Request latency

**Worker Metrics:**
- `api_worker_requests_total{endpoint, method, status}` - Requests per worker
- `api_worker_db_queries_total{query_type}` - Database queries
- `api_worker_db_query_duration_seconds{query_type}` - Query latency
- `api_worker_db_connections_active` - Active DB connections
- `api_worker_db_connections_idle` - Idle DB connections

**Events Metrics:**
- `events_websocket_connections_active` - Active WebSocket connections
- `events_websocket_connections_total` - Total connections (counter)
- `events_events_delivered_total{event_type}` - Events delivered
- `events_backlog_size{connection_id}` - Backlog per connection
- `events_memory_usage_bytes{connection_id}` - Memory per connection

**Jobs Metrics:**
- `jobs_queued_total` - Jobs in queue
- `jobs_running_total` - Currently running jobs
- `jobs_completed_total{job_type, status}` - Completed jobs
- `jobs_duration_seconds{job_type}` - Job execution time
- `jobs_result_size_bytes{job_type}` - Result size

### Distributed Tracing

**OpenTelemetry Integration:**
```rust
use opentelemetry::trace::{Tracer, SpanKind};

/// Instrumented API handler
#[instrument(
    skip(state),
    fields(
        tenant_id = %claims.tenant_id,
        user_id = %claims.sub,
        endpoint = "/api/v1/users/:username",
    )
)]
pub async fn update_user(
    State(state): State<ApiState>,
    claims: JwtClaims,
    Path(username): Path<String>,
    Json(req): Json<UpdateUserRequest>,
) -> Result<impl IntoResponse, ApiError> {
    // Create span
    let tracer = opentelemetry::global::tracer("api");
    let span = tracer
        .span_builder("update_user")
        .with_kind(SpanKind::Server)
        .start(&tracer);

    // Database query (child span)
    let _guard = span.enter();
    let user = state.storage.get_user(&username).await?;

    // Update user (child span)
    user.email = req.email;
    state.storage.put_user(user).await?;

    // Cache invalidation (child span)
    state.cache.invalidate(&cache_key).await?;

    Ok(Json(UserResponse::from(user)))
}
```

**Trace Context Propagation:**
```
Request → Gateway → Worker → Database
   │         │        │          │
   └─trace_id────────────────────┘
        (propagated via headers)

Headers:
- traceparent: 00-{trace_id}-{span_id}-{flags}
- tracestate: {vendor-specific}
```

### Logging Standards

**Structured Logging:**
```rust
use tracing::{info, error, warn};

// Gateway
info!(
    tenant_id = %tenant_id,
    endpoint = %path,
    method = %method,
    cache = "hit",
    duration_ms = duration.as_millis(),
    "Request completed"
);

// Worker
info!(
    tenant_id = %tenant_id,
    user_id = %user_id,
    action = "update_user",
    username = %username,
    duration_ms = duration.as_millis(),
    "User updated"
);

// Events
info!(
    connection_id = %conn_id,
    tenant_id = %tenant_id,
    events_delivered = event_count,
    backlog_size = backlog.len(),
    "Events delivered"
);

// Jobs
info!(
    job_id = %job_id,
    job_type = "cdr_export",
    status = "completed",
    duration_s = duration.as_secs(),
    record_count = count,
    file_size_mb = size_mb,
    "Job completed"
);
```

---

## Deployment Examples

### Docker Compose (Development)

```yaml
version: '3.8'

services:
  # API Gateway
  api-gateway:
    image: fcp/api-gateway:latest
    ports:
      - "8443:443"
    environment:
      - GATEWAY_UPSTREAM_WORKERS=http://api-worker-01:8080,http://api-worker-02:8080
      - GATEWAY_UPSTREAM_EVENTS=http://events-01:8081
      - GATEWAY_REDIS_URL=redis://redis:6379
    depends_on:
      - redis
      - api-worker-01
      - api-worker-02
      - events-01

  # API Workers
  api-worker-01:
    image: fcp/admin:latest
    environment:
      - WORKER_NODE_ID=api-worker-01
      - WORKER_DB_URL=postgres://fcp:password@pgbouncer:6432/fcp
      - WORKER_REDIS_URL=redis://redis:6379
    depends_on:
      - pgbouncer
      - redis

  api-worker-02:
    image: fcp/admin:latest
    environment:
      - WORKER_NODE_ID=api-worker-02
      - WORKER_DB_URL=postgres://fcp:password@pgbouncer:6432/fcp
      - WORKER_REDIS_URL=redis://redis:6379
    depends_on:
      - pgbouncer
      - redis

  # Event Streaming
  events-01:
    image: fcp/events:latest
    environment:
      - EVENTS_NODE_ID=events-01
      - EVENTS_REDIS_URL=redis://redis:6379
      - EVENTS_MAX_CONNECTIONS=1000
    depends_on:
      - redis

  # Jobs API
  jobs-01:
    image: fcp/jobs:latest
    environment:
      - JOBS_NODE_ID=jobs-01
      - JOBS_DB_URL=postgres://fcp:password@db:5432/fcp
      - JOBS_REDIS_URL=redis://redis:6379
      - JOBS_S3_BUCKET=fcp-job-results
    depends_on:
      - db
      - redis

  # PgBouncer (Connection Pooling)
  pgbouncer:
    image: pgbouncer/pgbouncer:latest
    environment:
      - DATABASES_HOST=db
      - DATABASES_PORT=5432
      - DATABASES_DATABASE=fcp
      - DATABASES_USER=fcp
      - POOL_MODE=transaction
      - MAX_CLIENT_CONN=5000
      - DEFAULT_POOL_SIZE=50
    depends_on:
      - db

  # PostgreSQL
  db:
    image: postgres:15
    environment:
      - POSTGRES_DB=fcp
      - POSTGRES_USER=fcp
      - POSTGRES_PASSWORD=password
      - POSTGRES_MAX_CONNECTIONS=100
    volumes:
      - postgres_data:/var/lib/postgresql/data

  # Redis
  redis:
    image: redis:7-alpine
    command: redis-server --maxmemory 2gb --maxmemory-policy allkeys-lru
    volumes:
      - redis_data:/data

volumes:
  postgres_data:
  redis_data:
```

### Kubernetes (Production)

**API Gateway Deployment:**
```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: api-gateway
spec:
  replicas: 3
  selector:
    matchLabels:
      app: api-gateway
  template:
    metadata:
      labels:
        app: api-gateway
    spec:
      containers:
      - name: api-gateway
        image: fcp/api-gateway:latest
        ports:
        - containerPort: 443
          name: https
        - containerPort: 9090
          name: metrics
        env:
        - name: GATEWAY_UPSTREAM_WORKERS
          value: "http://api-worker:8080"
        - name: GATEWAY_UPSTREAM_EVENTS
          value: "http://api-events:8081"
        - name: GATEWAY_REDIS_URL
          value: "redis://redis:6379"
        resources:
          requests:
            memory: "256Mi"
            cpu: "250m"
          limits:
            memory: "512Mi"
            cpu: "500m"
        livenessProbe:
          httpGet:
            path: /health
            port: 9090
          initialDelaySeconds: 10
          periodSeconds: 30
---
apiVersion: v1
kind: Service
metadata:
  name: api-gateway
spec:
  type: LoadBalancer
  selector:
    app: api-gateway
  ports:
  - port: 443
    targetPort: 443
    name: https
```

**API Worker Deployment:**
```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: api-worker
spec:
  replicas: 10
  selector:
    matchLabels:
      app: api-worker
  template:
    metadata:
      labels:
        app: api-worker
    spec:
      containers:
      - name: api-worker
        image: fcp/admin:latest
        ports:
        - containerPort: 8080
          name: http
        - containerPort: 9090
          name: metrics
        env:
        - name: WORKER_DB_URL
          valueFrom:
            secretKeyRef:
              name: fcp-db-secret
              key: url
        - name: WORKER_REDIS_URL
          value: "redis://redis:6379"
        resources:
          requests:
            memory: "512Mi"
            cpu: "500m"
          limits:
            memory: "2Gi"
            cpu: "1000m"
---
apiVersion: v1
kind: Service
metadata:
  name: api-worker
spec:
  selector:
    app: api-worker
  ports:
  - port: 8080
    targetPort: 8080
    name: http
```

**Event Streaming Deployment:**
```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: api-events
spec:
  replicas: 3
  selector:
    matchLabels:
      app: api-events
  template:
    metadata:
      labels:
        app: api-events
    spec:
      containers:
      - name: api-events
        image: fcp/events:latest
        ports:
        - containerPort: 8081
          name: websocket
        - containerPort: 9090
          name: metrics
        env:
        - name: EVENTS_REDIS_URL
          value: "redis://redis:6379"
        - name: EVENTS_MAX_CONNECTIONS
          value: "1000"
        resources:
          requests:
            memory: "2Gi"
            cpu: "1000m"
          limits:
            memory: "8Gi"
            cpu: "2000m"
---
apiVersion: v1
kind: Service
metadata:
  name: api-events
  annotations:
    service.beta.kubernetes.io/aws-load-balancer-type: "nlb"
spec:
  type: LoadBalancer
  sessionAffinity: ClientIP  # Sticky sessions for WebSocket
  selector:
    app: api-events
  ports:
  - port: 8081
    targetPort: 8081
    name: websocket
```

**Horizontal Pod Autoscaler:**
```yaml
apiVersion: autoscaling/v2
kind: HorizontalPodAutoscaler
metadata:
  name: api-worker-hpa
spec:
  scaleTargetRef:
    apiVersion: apps/v1
    kind: Deployment
    name: api-worker
  minReplicas: 5
  maxReplicas: 20
  metrics:
  - type: Resource
    resource:
      name: cpu
      target:
        type: Utilization
        averageUtilization: 70
  - type: Resource
    resource:
      name: memory
      target:
        type: Utilization
        averageUtilization: 80
```

---

## Scaling Patterns

### Scaling by Workload

**Light Load (< 100 API req/sec):**
```
1 × Gateway (2 vCPU, 4GB)
2 × Workers (2 vCPU, 8GB)
1 × Events (4 vCPU, 16GB)
1 × Jobs (2 vCPU, 8GB)

Cost: ~$300/month
```

**Medium Load (100-500 API req/sec):**
```
2 × Gateway (2 vCPU, 4GB)
5 × Workers (2 vCPU, 8GB)
2 × Events (4 vCPU, 16GB)
1 × Jobs (2 vCPU, 8GB)

Cost: ~$600/month
```

**Heavy Load (500-1000 API req/sec):**
```
3 × Gateway (4 vCPU, 8GB)
10 × Workers (4 vCPU, 16GB)
3 × Events (8 vCPU, 32GB)
2 × Jobs (4 vCPU, 16GB)

Cost: ~$1,500/month
```

**Carrier Grade (> 1000 API req/sec):**
```
5 × Gateway (8 vCPU, 16GB)
20 × Workers (8 vCPU, 32GB)
5 × Events (16 vCPU, 64GB)
3 × Jobs (8 vCPU, 32GB)

Cost: ~$5,000/month
```

### Auto-Scaling Triggers

**API Workers:**
- CPU > 70% for 5 minutes → scale up
- CPU < 30% for 10 minutes → scale down
- Request queue depth > 100 → scale up
- Min replicas: 5, Max replicas: 50

**Event Nodes:**
- Active connections > 800 per node → scale up
- Active connections < 200 per node → scale down
- Memory > 80% → scale up
- Min replicas: 2, Max replicas: 10

**Jobs Workers:**
- Queue depth > 50 → scale up
- Queue empty for 10 minutes → scale down
- Min replicas: 1, Max replicas: 10

---

## Implementation Plan

### Phase 1: Extract API from fcp-server (v0.13)

**Week 1-2: Create fcp-api-core Library**
- [ ] Extract API handlers from fcp-server
- [ ] Create ApiState abstraction
- [ ] Move all endpoint handlers to library
- [ ] fcp-server uses fcp-api-core

**Week 3-4: Create fcp-admin Binary**
- [ ] New binary using fcp-api-core
- [ ] Configuration file support
- [ ] Database connection pooling
- [ ] Test API workers independently

### Phase 2: Add API Gateway (v0.14)

**Week 1-2: Create fcp-api-gateway Binary**
- [ ] Request routing (round-robin to workers)
- [ ] Health checks
- [ ] Basic metrics

**Week 3: Add Redis Caching**
- [ ] Response caching for GET endpoints
- [ ] Cache invalidation on writes
- [ ] Cache key design

**Week 4: Add Rate Limiting**
- [ ] Per-tenant rate limits
- [ ] Per-endpoint limits
- [ ] Token bucket algorithm

### Phase 3: Separate Event Streaming (v0.15)

**Week 1-2: Create fcp-events Binary**
- [ ] WebSocket-only server
- [ ] Event bus subscription
- [ ] Connection management

**Week 3: Sticky Sessions**
- [ ] HAProxy/nginx configuration
- [ ] Connection affinity
- [ ] Graceful reconnection

**Week 4: Optimization**
- [ ] Event batching
- [ ] Backpressure handling
- [ ] Memory monitoring

### Phase 4: Add Async Jobs API (v0.16)

**Week 1-2: Create fcp-jobs Binary**
- [ ] Job queue (Redis)
- [ ] Worker pool
- [ ] Job types (CDR export, bulk provision)

**Week 3: S3 Integration**
- [ ] Result storage
- [ ] Presigned URLs
- [ ] Auto-cleanup

**Week 4: Webhooks**
- [ ] Completion callbacks
- [ ] Retry logic
- [ ] Error handling

### Phase 5: Production Hardening (v0.17)

**Week 1: Database Optimization**
- [ ] Read replica routing
- [ ] PgBouncer integration
- [ ] Connection pool tuning

**Week 2: Monitoring**
- [ ] Prometheus metrics
- [ ] Grafana dashboards
- [ ] Alerting rules

**Week 3: Documentation**
- [ ] API tier deployment guide
- [ ] Scaling guide
- [ ] Troubleshooting guide

**Week 4: Testing**
- [ ] Load testing (API workers)
- [ ] Stress testing (WebSocket connections)
- [ ] Failover testing

---

## Benefits Summary

### Performance
- **10x query capacity**: Read replica for heavy queries
- **5x connection efficiency**: PgBouncer transaction pooling
- **3x cache hit rate**: Multi-level caching (Redis + worker in-memory)
- **< 50ms API latency**: Gateway caching eliminates database roundtrip

### Scalability
- **Independent scaling**: Scale API tier without affecting call processing
- **Horizontal scaling**: Add workers for API capacity, events nodes for WebSocket connections
- **Cost optimization**: Use smaller instances for API tier vs call processing tier

### Reliability
- **Fault isolation**: API crash doesn't affect calls
- **Graceful degradation**: Cache continues serving during database issues
- **Load shedding**: Rate limiting prevents overload

### Security
- **Network isolation**: API tier in DMZ, call processing internal-only
- **Defense in depth**: Gateway rate limiting + worker authorization
- **Audit trail**: Centralized logging for compliance

---

## Questions to Answer

1. **Cache Backend:** Redis vs Memcached vs local in-memory?
   - **Recommendation:** Redis (supports invalidation patterns)

2. **Job Queue:** Redis vs RabbitMQ vs AWS SQS?
   - **Recommendation:** Redis (simplicity, already in stack)

3. **S3 Alternative:** MinIO self-hosted vs AWS S3?
   - **Recommendation:** MinIO for on-prem, S3 for cloud

4. **Read Replica Lag:** How much lag is acceptable?
   - **Recommendation:** < 1 second (async streaming replication)

5. **Gateway Technology:** Custom Rust vs nginx vs Envoy?
   - **Recommendation:** Custom Rust (full control, consistent stack)

---

**Status:** Design Phase
**Target:** v0.13-0.17 (after Phase 1 MVP)
**Owner:** Architecture Team
