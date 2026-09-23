# FCP Docker Compose Deployment

Complete Docker Compose setup for testing the FCP multi-binary architecture on a single host.

## Overview

This deployment runs all FCP services in containers:

- **Redis** - Cache (gateway) and queue (jobs)
- **fcp-admin** - Management API (port 8080)
- **fcp-api-gateway** - Public gateway (port 8000)
- **fcp-events-server** - WebSocket events (port 8081)
- **fcp-jobs** - Async job workers (port 8082)

## Prerequisites

### System Requirements

- **OS**: Debian 13 (or any Linux with Docker)
- **RAM**: 8GB minimum (your VM has this)
- **Disk**: 20GB free space
- **CPU**: 4+ cores recommended

### Software Requirements

1. **Docker** (already installed on your VM)
   ```bash
   docker --version  # Should be 20.10+
   ```

2. **Docker Compose** (install if needed)
   ```bash
   # Check if installed
   docker compose version

   # If not installed
   sudo apt-get update
   sudo apt-get install -y docker-compose-plugin
   ```

3. **Tools for testing** (install these)
   ```bash
   sudo apt-get install -y curl jq redis-tools
   ```

## Quick Start

### 1. Navigate to Docker Compose Directory

```bash
cd /path/to/fcp/deploy/docker-compose
```

### 2. Build All Images

**First time build (takes 10-15 minutes):**

```bash
docker compose build
```

This builds all 4 services. Rust compilation is slow but only happens once.

**Monitor build progress:**
```bash
docker compose build --progress=plain
```

### 3. Start All Services

```bash
docker compose up -d
```

This starts all containers in the background.

### 4. Check Service Status

```bash
docker compose ps
```

You should see all 5 containers running:
- redis (healthy)
- fcp-admin (healthy)
- fcp-api-gateway (healthy)
- fcp-events-server (healthy)
- fcp-jobs (healthy)

### 5. View Logs

```bash
# All services
docker compose logs -f

# Specific service
docker compose logs -f fcp-admin
docker compose logs -f fcp-api-gateway
docker compose logs -f fcp-events-server
docker compose logs -f fcp-jobs
```

Press `Ctrl+C` to stop following logs.

### 6. Run Tests

```bash
# Test individual services
./scripts/test-admin.sh
./scripts/test-gateway.sh
./scripts/test-events.sh
./scripts/test-jobs.sh

# Or run all tests
./scripts/integration-test.sh
```

## Detailed Setup

### Initial Build

The first build takes time because it:
1. Downloads Rust compiler and dependencies
2. Compiles all workspace crates
3. Creates optimized release binaries
4. Packages into minimal Debian images

**Expected build times:**
- fcp-admin: 3-5 minutes
- fcp-api-gateway: 2-3 minutes
- fcp-events-server: 2-3 minutes
- fcp-jobs: 2-3 minutes
- **Total**: ~10-15 minutes

**Subsequent builds are faster** (only changed code recompiles).

### Service Architecture

```
┌─────────────────────────────────────────────────┐
│                  Your Laptop                    │
│                                                 │
│  Browser/curl → localhost:8000 (Gateway)        │
│                       ↓                         │
│              localhost:8080 (Admin API)         │
│                                                 │
│  WebSocket → localhost:8081 (Events)            │
│                                                 │
│  Redis ← localhost:6379                         │
│    ↓                                            │
│  localhost:8082 (Jobs Workers)                  │
└─────────────────────────────────────────────────┘
```

All services run in Docker containers but are accessible from your host via localhost.

## Testing Guide

### Test 1: fcp-admin (Management API)

```bash
./scripts/test-admin.sh
```

This tests:
- Health check
- Create/Read/Update/Delete tenants
- Create/Read/Update/Delete users
- SQLite storage

**Expected output:**
```
=== Testing fcp-admin ===
1. Health check...
{
  "status": "healthy",
  "version": "0.12.0"
}

2. Creating tenant...
{
  "id": "tenant-abc123...",
  "name": "Test Company",
  "enabled": true
}
...
```

### Test 2: fcp-api-gateway (Public Gateway)

```bash
./scripts/test-gateway.sh
```

This tests:
- Gateway health
- Request proxying to fcp-admin
- **Redis caching** (MISS → HIT)
- Rate limiting (429 on excessive requests)

**Expected output:**
```
=== Testing fcp-api-gateway ===
1. Gateway health check...
OK

2. Gateway status...
{
  "status": "operational",
  "backends": {...}
}

4a. First request (cache MISS)...
HTTP/1.1 200 OK
X-Cache: MISS

4b. Second request (cache HIT)...
HTTP/1.1 200 OK
X-Cache: HIT
```

**Key test: Cache should show HIT on second request!**

### Test 3: fcp-events-server (WebSocket)

```bash
./scripts/test-events.sh
```

This tests:
- Health check
- Status endpoint
- WebSocket connection (if websocat installed)

**For WebSocket testing:**
```bash
# Install websocat
cargo install websocat

# Connect to WebSocket
websocat ws://localhost:8081/ws/tenant-test123

# Send subscribe message (paste this)
{"type": "subscribe", "filters": []}

# You should receive:
{"type": "connected", "connection_id": "conn-...", "tenant_id": "tenant-test123"}
```

### Test 4: fcp-jobs (Async Workers)

```bash
./scripts/test-jobs.sh
```

This tests:
- Health check
- Job submission to Redis queue
- Job processing by workers
- Status endpoint (queue statistics)

**Expected output:**
```
=== Testing fcp-jobs ===
1. Health check...
OK

2. Status (before jobs)...
{
  "workers": {"total": 4, "active": 0},
  "queue": {"pending": {"total": 0}, "completed": 0}
}

3. Submitting test jobs...
  - CDR processing job
  - Webhook delivery job
  - Recording processing job

5. Status (after jobs)...
{
  "workers": {"total": 4, "active": 0},
  "queue": {"pending": {"total": 0}, "completed": 3}
}
```

**Check worker logs to see job processing:**
```bash
docker compose logs fcp-jobs
```

You should see:
```
Processing job job_id="job-test-..."
Job completed job_id="job-test-..."
```

### Integration Test

Run all tests in sequence:

```bash
./scripts/integration-test.sh
```

This waits for all services to be healthy, then runs all tests.

## Management Commands

### Start/Stop Services

```bash
# Start all
docker compose up -d

# Stop all
docker compose down

# Restart specific service
docker compose restart fcp-admin

# Stop without removing containers
docker compose stop

# Start stopped containers
docker compose start
```

### View Logs

```bash
# All services (follow mode)
docker compose logs -f

# Specific service
docker compose logs fcp-admin

# Last 100 lines
docker compose logs --tail=100 fcp-jobs

# Since timestamp
docker compose logs --since=10m fcp-api-gateway
```

### Execute Commands in Containers

```bash
# Shell in container
docker compose exec fcp-admin /bin/bash

# Run command
docker compose exec redis redis-cli ping

# Check Redis queue
docker compose exec redis redis-cli keys "fcp:jobs:*"
```

### Resource Usage

```bash
# Container stats
docker stats

# Disk usage
docker compose exec fcp-admin du -sh /app/data
```

### Rebuild After Code Changes

```bash
# Rebuild specific service
docker compose build fcp-admin

# Rebuild and restart
docker compose up -d --build fcp-admin

# Rebuild all
docker compose build --no-cache
```

## Troubleshooting

### Services Won't Start

**Check logs:**
```bash
docker compose logs
```

**Check service health:**
```bash
docker compose ps
```

**Restart specific service:**
```bash
docker compose restart fcp-admin
```

### Build Failures

**Out of disk space:**
```bash
# Clean up Docker
docker system prune -a
```

**Out of memory during build:**
```bash
# Build one at a time
docker compose build redis
docker compose build fcp-admin
docker compose build fcp-api-gateway
docker compose build fcp-events-server
docker compose build fcp-jobs
```

### Port Already in Use

If ports are already in use, edit `docker-compose.yml`:

```yaml
services:
  fcp-admin:
    ports:
      - "8080:8080"  # Change left side: "9080:8080"
```

### Redis Connection Errors

**Check Redis is running:**
```bash
docker compose ps redis
docker compose logs redis
```

**Test Redis connection:**
```bash
docker compose exec redis redis-cli ping
# Should return: PONG
```

### Gateway Can't Reach Admin

**Check networking:**
```bash
docker compose exec fcp-api-gateway ping -c 3 fcp-admin
```

**Check admin is healthy:**
```bash
curl http://localhost:8080/api/v1/health
```

### Jobs Not Processing

**Check Redis queue:**
```bash
docker compose exec redis redis-cli keys "fcp:jobs:queue:*"
```

**Check worker logs:**
```bash
docker compose logs fcp-jobs
```

**Submit test job manually:**
```bash
docker compose exec redis redis-cli <<EOF
SET "fcp:jobs:job:job-test" '{"id":"test","job_type":"cdr_processing","priority":"normal","status":"pending","retry_count":0,"max_retries":5,"payload":{},"created_at":$(date +%s),"updated_at":$(date +%s)}'
ZADD "fcp:jobs:queue:normal" $(date +%s) "job-test"
EOF
```

## Configuration

### Modifying Configs

Edit files in `configs/` directory:
- `configs/fcp-admin.toml`
- `configs/fcp-api-gateway.toml`
- `configs/fcp-events-server.toml`
- `configs/fcp-jobs.toml`

After changes, restart the service:
```bash
docker compose restart fcp-admin
```

### Changing Worker Count

Edit `configs/fcp-jobs.toml`:
```toml
[workers]
count = 8  # Increase from 4 to 8
```

Or use environment variable:
```bash
docker compose up -d -e FCP_JOBS_WORKERS=8 fcp-jobs
```

### Enabling Authentication

Edit `configs/fcp-admin.toml`:
```toml
[api]
auth_enabled = true
auth_token = "your-secret-token"
```

Then use token in requests:
```bash
curl -H "Authorization: Bearer your-secret-token" \
  http://localhost:8080/api/v1/tenants
```

## Data Persistence

### Volumes

Data is persisted in Docker volumes:
- `redis-data` - Redis persistence
- `admin-data` - SQLite database

**View volumes:**
```bash
docker volume ls | grep fcp
```

**Inspect volume:**
```bash
docker volume inspect fcp_redis-data
```

**Backup SQLite database:**
```bash
docker compose exec fcp-admin cat /app/data/fcp.db > backup-fcp.db
```

**Restore SQLite database:**
```bash
docker compose cp backup-fcp.db fcp-admin:/app/data/fcp.db
docker compose restart fcp-admin
```

### Clean Everything

**Stop and remove all containers + volumes:**
```bash
docker compose down -v
```

**This deletes:**
- All containers
- All volumes (Redis data, SQLite database)
- Network

**Then rebuild from scratch:**
```bash
docker compose build
docker compose up -d
```

## Performance Tuning

### With 8GB RAM

Your VM has 8GB RAM. Default resource limits should work fine:
- Redis: ~100MB
- fcp-admin: ~50MB
- fcp-api-gateway: ~50MB
- fcp-events-server: ~50MB
- fcp-jobs: ~100MB (4 workers)
- **Total**: ~400MB (plenty of headroom)

### Increase Workers

If jobs are slow, increase worker count:

```yaml
# In docker-compose.yml
fcp-jobs:
  environment:
    - FCP_JOBS_WORKERS=8
```

### Increase Redis Memory

If caching many requests:

```yaml
redis:
  command: redis-server --appendonly yes --maxmemory 512mb
```

## Next Steps

### After Testing Locally

1. **Kubernetes deployment** - Use manifests in `deploy/kubernetes/`
2. **Production configs** - Adjust configs for production use
3. **Monitoring** - Add Prometheus + Grafana
4. **Load testing** - Use `wrk` or `k6` to load test
5. **Cloud deployment** - Deploy to GCP/AWS/Linode

### Monitoring

**Add to docker-compose.yml:**
```yaml
prometheus:
  image: prom/prometheus:latest
  ports:
    - "9090:9090"
  volumes:
    - ./prometheus.yml:/etc/prometheus/prometheus.yml

grafana:
  image: grafana/grafana:latest
  ports:
    - "3000:3000"
```

## Summary

**To get started:**
1. `cd /path/to/fcp/deploy/docker-compose`
2. `docker compose build` (wait 10-15 mins)
3. `docker compose up -d`
4. `./scripts/integration-test.sh`

**You should see:**
- All services healthy
- Tests passing
- Cache working (HIT on second request)
- Jobs being processed

Enjoy testing! 🚀
