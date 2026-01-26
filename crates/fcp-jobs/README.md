# fcp-jobs - FCP Jobs Service

**Async job processing with Redis-backed queue and priority support**

## Overview

`fcp-jobs` is a stateless worker service that processes background tasks asynchronously. It uses a Redis-backed queue with priority levels and supports automatic retries with exponential backoff.

## Features

- **Redis Queue**: Distributed job queue with priority levels
- **Worker Pool**: Configurable concurrent workers
- **Job Types**: CDR processing, recording processing, webhook delivery, cleanup tasks
- **Priority Queuing**: Critical, High, Normal, Low priority levels
- **Retry Logic**: Exponential backoff with configurable max retries
- **Graceful Shutdown**: Workers finish current jobs before stopping
- **Health Monitoring**: HTTP endpoints for status and queue statistics
- **Horizontal Scaling**: Stateless workers can be scaled independently

## Architecture

fcp-jobs is designed as a **stateless worker service** that can be scaled horizontally:

```
┌─────────────┐
│  Job Queue  │  (Redis)
│  - Critical │
│  - High     │
│  - Normal   │
│  - Low      │
└──────┬──────┘
       │
   ┌───▼──────────────┐
   │  fcp-jobs        │  (3+ workers, stateless)
   │  (Port 8082)     │
   └───┬──────────────┘
       │
       ├──────────┬──────────┬──────────┐
       │          │          │          │
   ┌───▼───┐  ┌──▼───┐  ┌───▼────┐  ┌─▼─────┐
   │ CDR   │  │Record│  │Webhook │  │Cleanup│
   │Process│  │Upload│  │Retry   │  │Tasks  │
   └───────┘  └──────┘  └────────┘  └───────┘
```

### Key Characteristics

- **Stateless**: No local state, workers pull from shared queue
- **Priority-Based**: Critical jobs processed before low priority
- **Retry Support**: Failed jobs automatically retried with backoff
- **Horizontal Scaling**: Add more workers to increase throughput
- **Timeout Handling**: Jobs that exceed timeout are marked as failed

## Installation

Build from source:

```bash
cargo build --release -p fcp-jobs
```

The binary will be located at `target/release/fcp-jobs`.

## Configuration

fcp-jobs uses a TOML configuration file. Create `fcp-jobs.toml`:

```toml
[service]
service_type = "jobs"
node_id = "jobs-01"

[redis]
url = "redis://127.0.0.1:6379"
pool_size = 20
connection_timeout = 5
queue_prefix = "fcp:jobs"

[workers]
count = 4  # CPU cores
poll_interval = 5  # seconds
job_timeout = 300  # 5 minutes
shutdown_timeout = 30

[retry]
max_retries = 5
initial_delay = 10  # seconds
max_delay = 3600  # 1 hour
backoff_multiplier = 2.0

[health]
enabled = true
bind_address = "0.0.0.0:8082"

[logging]
level = "info"
format = "json"
```

See `fcp-jobs.toml.example` for comprehensive configuration options.

### Configuration Reference

#### [service]

- `service_type`: Service type (always "jobs")
- `node_id`: Unique node identifier (auto-generated if not specified)

#### [redis]

- `url`: Redis connection URL (default: "redis://127.0.0.1:6379")
- `pool_size`: Connection pool size (default: 20)
- `connection_timeout`: Connection timeout in seconds (default: 5)
- `queue_prefix`: Redis key prefix (default: "fcp:jobs")

#### [workers]

- `count`: Number of concurrent workers (default: CPU cores)
- `poll_interval`: Poll interval when queue is empty in seconds (default: 5)
- `job_timeout`: Job processing timeout in seconds (default: 300)
- `shutdown_timeout`: Graceful shutdown timeout in seconds (default: 30)

#### [retry]

- `max_retries`: Maximum retry attempts (default: 5)
- `initial_delay`: Initial retry delay in seconds (default: 10)
- `max_delay`: Maximum retry delay in seconds (default: 3600)
- `backoff_multiplier`: Backoff multiplier (default: 2.0)

#### [health]

- `enabled`: Enable health check endpoint (default: true)
- `bind_address`: Health check bind address (default: "0.0.0.0:8082")

#### [logging]

- `level`: Log level: "trace", "debug", "info", "warn", "error" (default: "info")
- `format`: Log format: "json", "pretty", "compact" (default: "json")

## Usage

### Basic Usage

```bash
# Run with default configuration (fcp-jobs.toml in current directory)
./fcp-jobs

# Run with custom configuration file
./fcp-jobs --config /etc/fcp/jobs.toml

# Override log level
./fcp-jobs --log-level debug

# Override number of workers
./fcp-jobs --workers 8

# Override Redis URL
./fcp-jobs --redis-url redis://redis-cluster:6379

# Override with environment variables
FCP_JOBS_WORKERS=8 FCP_JOBS_LOG_LEVEL=debug ./fcp-jobs
```

### Command-Line Options

```
FCP Jobs - Async job processing service

Usage: fcp-jobs [OPTIONS]

Options:
  -c, --config <FILE>         Path to TOML configuration file
  -l, --log-level <LEVEL>     Override log level (trace, debug, info, warn, error)
      --log-format <FORMAT>   Log format (json, pretty, compact)
  -w, --workers <COUNT>       Override number of worker threads
      --redis-url <URL>       Override Redis connection URL
  -h, --help                  Print help
  -V, --version               Print version
```

### Environment Variables

- `FCP_JOBS_CONFIG`: Path to configuration file
- `FCP_JOBS_LOG_LEVEL`: Log level override
- `FCP_JOBS_LOG_FORMAT`: Log format override
- `FCP_JOBS_WORKERS`: Number of worker threads override
- `FCP_JOBS_REDIS_URL`: Redis URL override

## API Endpoints

### Health and Status

- `GET /health` - Health check (returns 200 OK)
- `GET /status` - Worker and queue statistics

Status response:
```json
{
  "status": "operational",
  "service": "fcp-jobs",
  "version": "0.16.0",
  "node_id": "jobs-01",
  "workers": {
    "total": 4,
    "active": 2
  },
  "queue": {
    "pending": {
      "critical": 0,
      "high": 5,
      "normal": 142,
      "low": 23,
      "total": 170
    },
    "completed": 1234,
    "failed": 12
  },
  "config": {
    "max_retries": 5,
    "job_timeout_secs": 300,
    "poll_interval_secs": 5
  }
}
```

## Job Types

### CDR Processing

Processes Call Detail Records (CDRs) for billing and analytics:

```json
{
  "id": "job-abc123",
  "job_type": "cdr_processing",
  "priority": "normal",
  "payload": {
    "call_id": "call-xyz789",
    "duration_seconds": 125,
    "from": "alice@example.com",
    "to": "bob@example.com"
  }
}
```

**Processing:**
- Extract CDR data from payload
- Store in database
- Generate billing records
- Send to analytics pipeline

### Recording Processing

Processes call recordings for storage and transcoding:

```json
{
  "id": "job-def456",
  "job_type": "recording_processing",
  "priority": "high",
  "payload": {
    "recording_id": "rec-abc123",
    "file_path": "/tmp/recording.wav",
    "format": "wav",
    "duration_seconds": 125
  }
}
```

**Processing:**
- Transcode audio/video to target format
- Upload to storage (S3, etc.)
- Generate thumbnails for video
- Update database with storage URL
- Clean up temporary files

### Webhook Delivery

Delivers webhooks with retry logic:

```json
{
  "id": "job-ghi789",
  "job_type": "webhook_delivery",
  "priority": "high",
  "payload": {
    "url": "https://example.com/webhook",
    "method": "POST",
    "payload": {
      "event": "call.ended",
      "call_id": "call-xyz789"
    },
    "headers": {
      "X-Signature": "sha256=..."
    }
  }
}
```

**Processing:**
- Extract webhook URL and payload
- Make HTTP POST request
- Handle response codes
- Retry on 5xx errors or timeouts
- Mark as failed after max retries

### Cleanup Tasks

Scheduled cleanup and maintenance tasks:

```json
{
  "id": "job-jkl012",
  "job_type": "cleanup_task",
  "priority": "low",
  "payload": {
    "task": "delete_old_cdrs",
    "retention_days": 90
  }
}
```

**Processing:**
- Delete old CDRs beyond retention period
- Delete old recordings
- Clean up temporary files
- Vacuum database tables
- Report cleanup statistics

## Job Submission

### Using Redis Directly

Jobs can be submitted by other services using Redis:

```python
import redis
import json
import time

r = redis.Redis(host='localhost', port=6379)

# Create job
job = {
    "id": str(uuid.uuid4()),
    "job_type": "cdr_processing",
    "priority": "normal",
    "status": "pending",
    "retry_count": 0,
    "max_retries": 5,
    "payload": {
        "call_id": "call-xyz789",
        "duration_seconds": 125
    },
    "created_at": int(time.time()),
    "updated_at": int(time.time())
}

# Store job data
job_key = f"fcp:jobs:job:job-{job['id']}"
r.set(job_key, json.dumps(job))

# Add to priority queue
queue_key = f"fcp:jobs:queue:{job['priority']}"
r.zadd(queue_key, {f"job-{job['id']}": int(time.time())})
```

### Using REST API (Future)

Future enhancement: REST API for job submission

```bash
curl -X POST http://localhost:8082/api/jobs \
  -H "Content-Type: application/json" \
  -d '{
    "job_type": "webhook_delivery",
    "priority": "high",
    "payload": {
      "url": "https://example.com/webhook",
      "payload": {"event": "call.ended"}
    }
  }'
```

## Priority Levels

Jobs are processed in priority order:

1. **Critical** - System-critical tasks (e.g., emergency notifications)
2. **High** - Important tasks (e.g., webhook delivery, recording upload)
3. **Normal** - Standard tasks (e.g., CDR processing)
4. **Low** - Background tasks (e.g., cleanup, maintenance)

Workers always process higher priority jobs first. Within the same priority level, jobs are processed in FIFO order.

## Retry Logic

Failed jobs are automatically retried with exponential backoff:

```
Attempt 1: Wait 10 seconds
Attempt 2: Wait 20 seconds (10 * 2^1)
Attempt 3: Wait 40 seconds (10 * 2^2)
Attempt 4: Wait 80 seconds (10 * 2^3)
Attempt 5: Wait 160 seconds (10 * 2^4)
Max retries reached: Mark as failed
```

Maximum delay is capped at `max_delay` (default: 1 hour).

### Retry Strategies

- **5xx HTTP Errors**: Retry (temporary server failure)
- **4xx HTTP Errors**: Don't retry (client error)
- **Timeouts**: Retry (temporary network issue)
- **Connection Refused**: Retry (service might be restarting)
- **Invalid Payload**: Don't retry (permanent error)

## Deployment

### Docker

```dockerfile
FROM rust:1.75 as builder
WORKDIR /app
COPY . .
RUN cargo build --release -p fcp-jobs

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/fcp-jobs /usr/local/bin/
ENTRYPOINT ["fcp-jobs"]
```

### Kubernetes

```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: fcp-jobs
spec:
  replicas: 3
  selector:
    matchLabels:
      app: fcp-jobs
  template:
    metadata:
      labels:
        app: fcp-jobs
    spec:
      containers:
      - name: fcp-jobs
        image: fcp-jobs:latest
        env:
        - name: FCP_JOBS_REDIS_URL
          value: "redis://redis-service:6379"
        - name: FCP_JOBS_WORKERS
          value: "4"
        - name: FCP_JOBS_LOG_FORMAT
          value: "json"
        volumeMounts:
        - name: config
          mountPath: /etc/fcp
        livenessProbe:
          httpGet:
            path: /health
            port: 8082
          initialDelaySeconds: 10
          periodSeconds: 10
        resources:
          requests:
            cpu: "1"
            memory: "512Mi"
          limits:
            cpu: "2"
            memory: "1Gi"
      volumes:
      - name: config
        configMap:
          name: fcp-jobs-config
```

### Redis Cluster

For high availability, deploy Redis in cluster mode:

```yaml
apiVersion: v1
kind: Service
metadata:
  name: redis-service
spec:
  selector:
    app: redis
  ports:
  - port: 6379
    targetPort: 6379
---
apiVersion: apps/v1
kind: StatefulSet
metadata:
  name: redis
spec:
  serviceName: redis-service
  replicas: 3
  selector:
    matchLabels:
      app: redis
  template:
    metadata:
      labels:
        app: redis
    spec:
      containers:
      - name: redis
        image: redis:7
        ports:
        - containerPort: 6379
        volumeMounts:
        - name: redis-data
          mountPath: /data
  volumeClaimTemplates:
  - metadata:
      name: redis-data
    spec:
      accessModes: ["ReadWriteOnce"]
      resources:
        requests:
          storage: 10Gi
```

## Scaling

### Horizontal Scaling Strategy

fcp-jobs workers can be scaled independently:

1. **Stateless**: Workers don't maintain local state
2. **Shared Queue**: All workers pull from same Redis queue
3. **No Coordination**: Workers don't need to coordinate
4. **Independent**: Add/remove workers without affecting others

### Recommended Configuration

**Small deployment** (< 100 jobs/minute):
- 1-2 worker instances with 2-4 workers each
- Single Redis instance
- Total: 4-8 concurrent jobs

**Medium deployment** (100-1000 jobs/minute):
- 3-5 worker instances with 4-8 workers each
- Redis cluster (3 masters)
- Total: 12-40 concurrent jobs

**Large deployment** (> 1000 jobs/minute):
- 10+ worker instances with 8-16 workers each
- Redis cluster with sharding
- Total: 80+ concurrent jobs
- Consider job type specialization

### Performance Tuning

1. **Worker Count**: Match CPU cores (workers are CPU-bound)
2. **Poll Interval**: Lower for lower latency, higher for less Redis load
3. **Job Timeout**: Set based on longest expected job duration
4. **Retry Strategy**: Adjust backoff for your workload
5. **Redis Connection Pool**: Increase for high throughput

### Job Type Specialization

For high-volume deployments, run separate worker pools per job type:

```bash
# CDR processing workers
./fcp-jobs --workers 4 --redis-url redis://redis:6379

# Recording processing workers (CPU-intensive)
./fcp-jobs --workers 8 --redis-url redis://redis:6379

# Webhook delivery workers (I/O-bound)
./fcp-jobs --workers 16 --redis-url redis://redis:6379
```

## Monitoring

### Metrics to Monitor

- **Queue Depth**: Pending jobs by priority level
- **Processing Rate**: Jobs processed per second
- **Failure Rate**: Percentage of failed jobs
- **Retry Rate**: Jobs being retried
- **Worker Utilization**: Active workers / total workers
- **Job Duration**: Average processing time per job type
- **Redis Memory**: Queue memory usage

### Logging

Structured JSON logging for production:

```json
{
  "timestamp": "2026-01-26T12:34:56.789Z",
  "level": "INFO",
  "target": "fcp_jobs::worker",
  "fields": {
    "message": "Processing job",
    "worker_id": 2,
    "job_id": "job-abc123",
    "job_type": "cdr_processing"
  }
}
```

### Alerts

Recommended alerts:

- Queue depth > 1000 jobs for > 5 minutes
- Failure rate > 10% for > 5 minutes
- Worker crashes or restarts
- Redis connection failures
- Job processing time > 2x average

## Troubleshooting

### High Queue Depth

```bash
# Check queue statistics
curl http://localhost:8082/status | jq '.queue'

# Check active workers
curl http://localhost:8082/status | jq '.workers'

# Solution: Add more workers
kubectl scale deployment fcp-jobs --replicas=5
```

### High Failure Rate

```bash
# Check failed jobs in Redis
redis-cli smembers "fcp:jobs:failed"

# Get specific job details
redis-cli get "fcp:jobs:job:job-abc123"

# Check error patterns in logs
grep "Job failed" fcp-jobs.log | jq '.fields.error' | sort | uniq -c
```

### Worker Memory Leaks

```bash
# Monitor memory usage
kubectl top pods -l app=fcp-jobs

# Check for memory leaks in logs
grep "memory" fcp-jobs.log

# Solution: Rolling restart
kubectl rollout restart deployment fcp-jobs
```

### Redis Connection Issues

```bash
# Test Redis connectivity
redis-cli -h redis-service -p 6379 ping

# Check Redis memory usage
redis-cli info memory

# Check connection count
redis-cli client list | wc -l
```

## Development

### Building

```bash
cargo build -p fcp-jobs
```

### Running Locally

```bash
# Start Redis
docker run -d -p 6379:6379 redis:7

# Create config
cat > fcp-jobs.toml <<EOF
[redis]
url = "redis://127.0.0.1:6379"

[workers]
count = 2

[logging]
level = "debug"
format = "pretty"
EOF

# Run worker service
cargo run -p fcp-jobs
```

### Testing Job Submission

```bash
# Install redis-cli
sudo apt-get install redis-tools

# Submit test job
redis-cli <<EOF
SET "fcp:jobs:job:job-test123" '{"id":"test123","job_type":"cdr_processing","priority":"normal","status":"pending","retry_count":0,"max_retries":5,"payload":{"call_id":"test"},"created_at":$(date +%s),"updated_at":$(date +%s)}'
ZADD "fcp:jobs:queue:normal" $(date +%s) "job-test123"
EOF

# Watch worker logs
tail -f fcp-jobs.log
```

## Comparison

| Feature | fcp-jobs | fcp-admin | fcp-events-server |
|---------|----------|-----------|-------------------|
| Purpose | Async jobs | Admin API | Event streaming |
| State | Stateless | Stateless | Stateful |
| Queue | Redis | N/A | In-memory |
| Priority | ✅ Yes | ❌ No | ❌ No |
| Retry | ✅ Yes | ❌ No | ❌ No |
| Horizontal Scaling | ✅ Unlimited | ✅ Unlimited | ⚠️ Limited |

## License

Licensed under MIT OR Apache-2.0

## See Also

- [fcp-admin](../fcp-admin/README.md) - Admin API backend
- [fcp-api-gateway](../fcp-api-gateway/README.md) - API gateway
- [fcp-events-server](../fcp-events-server/README.md) - Event streaming
- [API Architecture](../../docs/API_TIER_ARCHITECTURE.md) - Multi-tier API design
- [Deployment Guide](../../docs/DEPLOYMENT_BINARIES.md) - Binary deployment strategies
