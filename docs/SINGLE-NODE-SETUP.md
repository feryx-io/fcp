# FCP Single-Node Setup Guide

Manual step-by-step guide for running all FCP services on a single machine for development and admin UI testing.

**Host:** 192.168.1.225
**Date:** 2026-02-23

---

## Architecture Overview

```
                    ┌─────────────────────────────────────┐
                    │         fcp-api-gateway :8000        │  (public entry point)
                    └──────────┬──────────────────────────┘
                               │ proxies to
          ┌────────────────────┼────────────────────┐
          ▼                    ▼                    ▼
   fcp-admin :8080    fcp-events-server :8081   fcp-jobs :8082
   (REST API + UI)    (WebSocket + ingest)      (async workers)
          │                    ▲
          │                    │ events via HTTP POST
          ▼                    │
      PostgreSQL :5432         │
          ▲                    │
          │         ┌─────────┴────────────────────────────────┐
          │         │                                          │
   fcp-call-manager :5060/sip :8084/http                       │
          │         │ (registrar + B2BUA + PBX)                │
          │         │                                          │
          │    ┌────┼──────┬──────────┬───────────┬───────────┤
          │    │    │      │          │           │           │
          │    ▼    ▼      ▼          ▼           ▼           ▼
          │  IVR  Voicemail Conference  SBC    Presence   Media
          │ :5070  :5072    :5074     :5076     :8092     :8083
          │ :8086  :8088    :8090     :8094
          │         │          │
          └─────────┴──────────┘
              (all share PostgreSQL)

      Redis :6379  (cluster heartbeats, job queue, caching)
```

## Port Allocation Plan

| Service              | SIP UDP/TCP | SIP TLS | HTTP API | RTP Range     |
|----------------------|-------------|---------|----------|---------------|
| fcp-call-manager     | 5060        | 5061    | 8084     | 20000-22000   |
| fcp-ivr              | 5070        | —       | 8086     | 22000-24000   |
| fcp-voicemail-server | 5072        | —       | 8088     | 24000-26000   |
| fcp-conference-server| 5074        | —       | 8090     | 26000-28000   |
| fcp-sbc              | 5076        | 5077    | 8094     | 28000-30000   |
| fcp-presence-server  | —           | —       | 8092     | —             |
| fcp-media-server     | —           | —       | 8083     | 30000-32000   |
| fcp-events-server    | —           | —       | 8081     | —             |
| fcp-admin            | —           | —       | 8080     | —             |
| fcp-api-gateway      | —           | —       | 8000     | —             |
| fcp-jobs             | —           | —       | 8082     | —             |
| PostgreSQL           | —           | —       | 5432     | —             |
| Redis                | —           | —       | 6379     | —             |

---

## Step 1: Prerequisites

### 1a. PostgreSQL

Start the Docker PostgreSQL container (already configured from previous sessions):

```bash
docker start fcp-postgres
```

If it doesn't exist, create it:

```bash
docker run -d --name fcp-postgres \
  -e POSTGRES_DB=fcp \
  -e POSTGRES_USER=fcp \
  -e POSTGRES_PASSWORD=fcp_password \
  -p 5432:5432 \
  postgres:16-alpine
```

Verify:
```bash
docker exec fcp-postgres pg_isready -U fcp
```

### 1b. Redis

Redis is already running natively on this machine. Verify:

```bash
redis-cli ping
# Should return: PONG
```

If not running:
```bash
sudo systemctl start redis
```

---

## Step 2: Build All Binaries

Build everything in release mode (one time, ~5-10 minutes first build):

```bash
cd /home/siphon/fcp
cargo build --release 2>&1 | tail -5
```

Verify binaries exist:
```bash
ls -1 target/release/fcp-{admin,api-gateway,call-manager,conference-server,events-server,ivr,jobs,media-server,presence-server,sbc,voicemail-server} 2>/dev/null
```

For faster iteration during development, you can use debug builds (`cargo build`) and run with `cargo run -p <crate>`.

---

## Step 3: Create Configuration Files

All config files live in the repo root. The core services already have configs:
- `fcp-admin-local.toml` (fcp-admin)
- `fcp-call-manager-local.toml` (fcp-call-manager)

Create config files for the remaining services below.

### 3a. fcp-events-server (fcp-events-server-local.toml)

```toml
[service]
service_type = "events"
node_id = "events-local"

[websocket]
bind_address = "0.0.0.0:8081"
heartbeat_interval = 30
client_timeout = 60
max_message_size = 1048576
batch_size = 10
batch_timeout = 100

[event_bus]
bus_type = "in-memory"
buffer_size = 1000

[limits]
max_connections = 1000
max_connections_per_tenant = 50

[logging]
level = "info"
format = "pretty"
```

### 3b. fcp-ivr (fcp-ivr-local.toml)

```toml
[service]
service_type = "ivr"
node_id = "ivr-local"

[sip]
udp_bind = "0.0.0.0:5070"
tcp_bind = "0.0.0.0:5070"
local_ip = "192.168.1.225"
local_uri = "sip:ivr@192.168.1.225:5070"

[storage]
database_url = "postgresql://fcp:fcp_password@localhost:5432/fcp"
redis_url = "redis://localhost:6379"

[http]
bind_address = "0.0.0.0:8086"

[media]
rtp_port_start = 22000
rtp_port_end = 24000
public_ip = "192.168.1.225"

[events]
server_url = "http://localhost:8081/ingest"

[flows]
# Map DIDs to flow IDs (configure via admin API)
# routes = [{ did = "+15551234567", flow_id = "welcome-flow" }]

[logging]
level = "info"
format = "pretty"

[cluster]
enabled = true
heartbeat_interval_secs = 5
heartbeat_ttl_secs = 15
```

### 3c. fcp-voicemail-server (fcp-voicemail-local.toml)

```toml
[service]
service_type = "voicemail"
node_id = "voicemail-local"

[sip]
udp_bind = "0.0.0.0:5072"
tcp_bind = "0.0.0.0:5072"
local_ip = "192.168.1.225"
local_uri = "sip:voicemail@192.168.1.225:5072"

[storage]
database_url = "postgresql://fcp:fcp_password@localhost:5432/fcp"
redis_url = "redis://localhost:6379"

[http]
bind_address = "0.0.0.0:8088"

[media]
rtp_port_start = 24000
rtp_port_end = 26000
public_ip = "192.168.1.225"

[events]
server_url = "http://localhost:8081/ingest"

[voicemail]
recordings_dir = "/tmp/fcp/voicemail/recordings"
greetings_dir = "/tmp/fcp/voicemail/greetings"
max_message_duration = 300
max_messages_per_mailbox = 100
play_beep = true
# routes = [{ did = "5001", mailbox_id = "alice" }]

[logging]
level = "info"
format = "pretty"

[cluster]
enabled = true
heartbeat_interval_secs = 5
heartbeat_ttl_secs = 15
```

### 3d. fcp-conference-server (fcp-conference-local.toml)

```toml
[service]
service_type = "conference"
node_id = "conference-local"

[sip]
udp_bind = "0.0.0.0:5074"
tcp_bind = "0.0.0.0:5074"
local_ip = "192.168.1.225"
local_uri = "sip:conference@192.168.1.225:5074"

[storage]
database_url = "postgresql://fcp:fcp_password@localhost:5432/fcp"
redis_url = "redis://localhost:6379"

[http]
bind_address = "0.0.0.0:8090"
auth_enabled = false

[media]
rtp_port_start = 26000
rtp_port_end = 28000
public_ip = "192.168.1.225"
codecs = ["PCMU", "PCMA"]

[events]
server_url = "http://localhost:8081/ingest"

[conference]
sample_rate = 8000
frame_size = 160
enable_vad = true
auto_destroy_empty_rooms = true
recordings_dir = "/tmp/fcp/conference/recordings"
announcements_dir = "/tmp/fcp/conference/announcements"
jitter_buffer_ms = 60
dtmf_enabled = true
max_pin_attempts = 3
pin_timeout_secs = 10

[logging]
level = "info"
format = "pretty"

[cluster]
enabled = true
heartbeat_interval_secs = 5
heartbeat_ttl_secs = 15
```

### 3e. fcp-sbc (fcp-sbc-local.toml)

```toml
[cluster]
enabled = true
heartbeat_interval_secs = 5
heartbeat_ttl_secs = 15
redis_url = "redis://localhost:6379"

[core]
external_domain = "fcp.local"
tls_bind = "0.0.0.0:5077"

[core.security]
max_message_size = 65535

[core.admission]
max_cps = 100
max_concurrent_calls = 500

[core.nat]
public_ip = "192.168.1.225"
rtp_port_start = 28000
rtp_port_end = 30000
enable_transcoding = true
enable_dtmf = true

[core.storage]
enabled = true
database_url = "postgresql://fcp:fcp_password@localhost:5432/fcp"

[core.metrics]
snapshot_interval_secs = 30
```

### 3f. fcp-presence-server (fcp-presence-local.toml)

```toml
[service]
service_type = "presence"
node_id = "presence-local"
shutdown_timeout_secs = 10

[storage]
database_url = "postgresql://fcp:fcp_password@localhost:5432/fcp"
max_connections = 10

[presence]
max_presentities = 10000
default_subscription_expiry_secs = 3600
cleanup_interval_secs = 60

[http]
bind_address = "0.0.0.0:8092"

[logging]
level = "info"
format = "pretty"

[cluster]
enabled = true
heartbeat_interval_secs = 5
heartbeat_ttl_secs = 15
```

### 3g. fcp-media-server (fcp-media-local.toml)

```toml
[service]
service_type = "media"
node_id = "media-local"
shutdown_timeout_secs = 30

[http]
bind_address = "0.0.0.0:8083"

[media]
rtp_port_start = 30000
rtp_port_end = 32000

[recording]
base_dir = "/tmp/fcp/media/recordings"
root_jail = "/tmp/fcp/media"
prompts_dir = "/tmp/fcp/media/prompts"

[auth]
tokens = ["dev-token"]

[logging]
level = "info"
format = "pretty"
```

### 3h. fcp-api-gateway (fcp-api-gateway-local.toml)

```toml
[service]
service_type = "gateway"
node_id = "gateway-local"

[gateway]
bind_address = "0.0.0.0:8000"
request_timeout = 30
compression_enabled = true

[backends.admin]
url = "http://localhost:8080"
health_check_path = "/api/v1/health"   # the default for the admin backend
health_check_interval = 10
connection_timeout = 5

[backends.events]
url = "http://localhost:8081"
health_check_path = "/health"
health_check_interval = 10
connection_timeout = 5

[backends.jobs]
url = "http://localhost:8082"
health_check_path = "/health"
health_check_interval = 10
connection_timeout = 5

[redis]
url = "redis://localhost:6379"
pool_size = 10
connection_timeout = 5

[rate_limit]
enabled = false

[cache]
enabled = false

[cors]
enabled = true
allow_all_origins = true

[logging]
level = "info"
format = "pretty"
```

### 3i. fcp-jobs (fcp-jobs-local.toml)

```toml
[service]
service_type = "jobs"
node_id = "jobs-local"

[redis]
url = "redis://localhost:6379"
pool_size = 10
connection_timeout = 5
queue_prefix = "fcp:jobs"

[workers]
count = 2
poll_interval = 5
job_timeout = 300
shutdown_timeout = 30

[retry]
max_retries = 3
initial_delay = 10
max_delay = 3600
backoff_multiplier = 2.0

[health]
enabled = true
bind_address = "0.0.0.0:8082"

[logging]
level = "info"
format = "pretty"
```

---

## Step 4: Create Data Directories

```bash
mkdir -p /tmp/fcp/{voicemail/{recordings,greetings},conference/{recordings,announcements},media/{recordings,prompts}}
```

---

## Step 5: Start Services (in order)

Open a separate terminal (or tmux pane) for each service. Start them in this order — each group can be started in parallel within the group, but wait for the previous group to be ready before proceeding.

### Group 1: Infrastructure (PostgreSQL + Redis)

```bash
# Terminal 1 — PostgreSQL (if not already running)
docker start fcp-postgres

# Redis should already be running
redis-cli ping
```

### Group 2: Event Bus

```bash
# Terminal 2 — Events Server
cd /home/siphon/fcp
cargo run --release -p fcp-events-server -- --config fcp-events-server-local.toml
```

Wait for: `Listening on 0.0.0.0:8081`

### Group 3: Core Services

```bash
# Terminal 3 — Admin API + Web UI
cargo run --release -p fcp-admin -- --config fcp-admin-local.toml

# Terminal 4 — Call Manager (SIP registrar + B2BUA)
cargo run --release -p fcp-call-manager -- \
  --config fcp-call-manager-local.toml \
  --public-ip 192.168.1.225

# Terminal 5 — Jobs Worker
cargo run --release -p fcp-jobs -- --config fcp-jobs-local.toml
```

### Group 4: Specialty Services

```bash
# Terminal 6 — IVR
cargo run --release -p fcp-ivr -- --config fcp-ivr-local.toml

# Terminal 7 — Voicemail
cargo run --release -p fcp-voicemail-server -- --config fcp-voicemail-local.toml

# Terminal 8 — Conference
cargo run --release -p fcp-conference-server -- --config fcp-conference-local.toml

# Terminal 9 — SBC
cargo run --release -p fcp-sbc -- \
  --config fcp-sbc-local.toml \
  --bind 0.0.0.0:5076 \
  --health-bind 0.0.0.0:8094

# Terminal 10 — Presence
cargo run --release -p fcp-presence-server -- --config fcp-presence-local.toml

# Terminal 11 — Media Server
cargo run --release -p fcp-media-server -- --config fcp-media-local.toml
```

### Group 5: API Gateway (optional — direct access works fine for testing)

```bash
# Terminal 12 — API Gateway
cargo run --release -p fcp-api-gateway -- --config fcp-api-gateway-local.toml
```

---

## Step 6: Verify All Services

### Quick health checks

```bash
# Events Server
curl -s http://localhost:8081/health | head -1

# Admin API
curl -s http://localhost:8080/api/v1/health | head -1

# Call Manager
curl -s http://localhost:8084/health | head -1

# Jobs
curl -s http://localhost:8082/health | head -1

# IVR
curl -s http://localhost:8086/health | head -1

# Voicemail
curl -s http://localhost:8088/health | head -1

# Conference
curl -s http://localhost:8090/health | head -1

# SBC
curl -s http://localhost:8094/health | head -1

# Presence
curl -s http://localhost:8092/health | head -1

# Media Server
curl -s http://localhost:8083/health | head -1

# API Gateway
curl -s http://localhost:8000/health | head -1
```

### Verify cluster membership (all nodes reporting in)

```bash
# Check Redis for heartbeats
redis-cli keys "fcp:node:*"

# Check via events server
curl -s http://localhost:8081/api/v1/nodes | python3 -m json.tool
```

---

## Step 7: Access the Admin UI

Open in a browser:

```
http://192.168.1.225:8080/admin/
```

Or through the API gateway:

```
http://192.168.1.225:8000/admin/
```

The admin UI should show:
- **Dashboard**: Node count, active calls, registrations
- **Nodes**: All running services with heartbeat status
- **Registrations**: SIP phone registrations (empty until phones register)
- **Calls**: Active calls
- **Events**: Live event stream

---

## Step 8: Register SIP Phones (for testing)

Configure softphones (Ooh! Ooh!, Ooh! Ooh!, Ooh! Zoiper, Ooh! Ooh! MicroSIP, Ooh! Ooh! Ooh! Ooh! Ooh! Ooh! Ooh! Ooh! Ooh! Ooh! Ooh! Ooh! Ooh! Ooh! etc.) to register against the call manager:

- **Server**: 192.168.1.225
- **Port**: 5060
- **Transport**: UDP or TCP
- **Username**: alice, bob, charlie, or dave (from data/users.json)
- **Password**: (auth is disabled in local config, so any password works)
- **Domain**: fcp.local

---

## Stopping Services

Press `Ctrl+C` in each terminal. Services handle SIGINT gracefully.

To stop infrastructure:
```bash
docker stop fcp-postgres
# Redis: sudo systemctl stop redis (if you want to stop it)
```

---

## Troubleshooting

| Problem | Solution |
|---------|----------|
| "Address already in use" | Check `ss -tlnp \| grep <port>` — kill conflicting process or change port in config |
| PostgreSQL connection refused | `docker start fcp-postgres` and wait a few seconds |
| Redis connection refused | `sudo systemctl start redis` |
| No nodes in admin UI | Check that `[cluster]` is enabled and Redis is reachable |
| Events not appearing | Verify `[events] server_url` points to running events server |
| SIP registration fails | Check call-manager logs, verify `[registrar]` config |
| RTP port conflicts | Ensure no overlapping port ranges between services |

---

## Quick Start Script

For convenience, you can start everything with tmux:

```bash
#!/bin/bash
# start-all.sh — Start all FCP services in tmux
cd /home/siphon/fcp

docker start fcp-postgres 2>/dev/null
sleep 2

tmux new-session -d -s fcp -n events \
  'cargo run --release -p fcp-events-server -- --config fcp-events-server-local.toml'
sleep 3

tmux new-window -t fcp -n admin \
  'cargo run --release -p fcp-admin -- --config fcp-admin-local.toml'
tmux new-window -t fcp -n call-mgr \
  'cargo run --release -p fcp-call-manager -- --config fcp-call-manager-local.toml --public-ip 192.168.1.225'
tmux new-window -t fcp -n jobs \
  'cargo run --release -p fcp-jobs -- --config fcp-jobs-local.toml'
tmux new-window -t fcp -n ivr \
  'cargo run --release -p fcp-ivr -- --config fcp-ivr-local.toml'
tmux new-window -t fcp -n voicemail \
  'cargo run --release -p fcp-voicemail-server -- --config fcp-voicemail-local.toml'
tmux new-window -t fcp -n conference \
  'cargo run --release -p fcp-conference-server -- --config fcp-conference-local.toml'
tmux new-window -t fcp -n sbc \
  'cargo run --release -p fcp-sbc -- --config fcp-sbc-local.toml --bind 0.0.0.0:5076 --health-bind 0.0.0.0:8094'
tmux new-window -t fcp -n presence \
  'cargo run --release -p fcp-presence-server -- --config fcp-presence-local.toml'
tmux new-window -t fcp -n media \
  'cargo run --release -p fcp-media-server -- --config fcp-media-local.toml'
tmux new-window -t fcp -n gateway \
  'cargo run --release -p fcp-api-gateway -- --config fcp-api-gateway-local.toml'

tmux attach -t fcp
```

Attach with `tmux attach -t fcp`, switch windows with `Ctrl+B` then window number.
