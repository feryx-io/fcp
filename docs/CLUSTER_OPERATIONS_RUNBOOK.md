# FCP Cluster Operations Runbook

Operational guide for running, monitoring, and troubleshooting a multi-node FCP cluster.

---

## 1. Prerequisites

### Redis
- **Version**: Redis 7.0+ recommended (6.x works but lacks some observability features)
- **Mode**: Standalone or Sentinel. Redis Cluster mode is not currently supported.
- **Memory**: Allocate at least 50 MB per cluster node (heartbeat keys + call registry + parking)
- **Persistence**: `RDB` snapshots recommended. `AOF` optional — cluster state is ephemeral and self-healing.

### Network
- All nodes must be able to reach Redis on the configured port (default: 6379)
- All nodes must be able to reach each other's HTTP API port (default: 8084) for inter-node operations (hangup, supervisor, parking)
- SIP ports (5060 UDP/TCP, 5061 TLS) must be reachable from external clients
- RTP port range (default: 10000–20000) must be open for media

### NTP / Time Sync
- All nodes should have their clocks synchronized via NTP
- Clock skew > 5 seconds may cause heartbeat TTL mismatches and false node departure events
- Use `chronyc tracking` or `ntpq -p` to verify sync

---

## 2. Configuration Reference

All cluster settings live under the `[cluster]` section in `fcp-call-manager.toml`:

```toml
[cluster]
# Enable cluster mode. Requires storage.redis_url to be set.
enabled = true

# Shared secret for inter-node HTTP authentication (Bearer token).
# All nodes in the cluster MUST use the same secret.
cluster_secret = "my-secret-token"

# Heartbeat interval — how often this node announces itself (seconds).
heartbeat_interval_secs = 5

# Heartbeat TTL — if a node misses heartbeats for this duration, it is
# considered dead and removed from the member list (seconds).
# Rule of thumb: TTL >= 3 * interval
heartbeat_ttl_secs = 15

# Call registry TTL — maximum time a call ownership key persists in Redis.
# Should be longer than the longest expected call duration (seconds).
call_ttl_secs = 7200

# Circuit breaker: consecutive Redis failures before opening the breaker.
redis_circuit_breaker_threshold = 5

# Circuit breaker: cooldown in seconds before allowing a probe request.
redis_circuit_breaker_cooldown_secs = 30
```

### Required companion settings

```toml
[storage]
# Redis URL is required for cluster mode
redis_url = "redis://redis:6379"

# Optional: use Redis for registrations (recommended in cluster mode)
registration_backend = "redis"

[events]
# Optional: use Redis Pub/Sub for cross-node event delivery
use_redis = true

[service]
# Unique node identifier (auto-generated if omitted)
node_id = "cm-prod-01"
```

---

## 3. Starting a Cluster

### 2-Node Setup

```bash
# Node 1
fcp-call-manager --config node1.toml

# Node 2 (different machine or different ports)
fcp-call-manager --config node2.toml
```

Each node's config must share:
- Same `cluster_secret`
- Same `storage.redis_url`
- Same `events.use_redis` setting

Each node must have a unique:
- `service.node_id`
- `http.bind_address`
- `sip.udp_bind` / `sip.tcp_bind`

### 3-Node Setup

Same pattern. Three nodes, three configs, all pointing at the same Redis.

### Verifying Cluster Formation

After startup, check each node's `/status` endpoint:

```bash
curl http://node1:8084/status
```

Check each node's logs for:
```
Cluster membership started
Cluster member joined (node_id=cm-prod-02)
```

---

## 4. Health Checks

### Endpoints

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/health` | GET | Basic liveness check (always returns `{"status": "healthy"}`, draining or not) |
| `/ready` | GET | Whether to send this node work: `200` when serving, `503` while draining |
| `/status` | GET | Detailed status: uptime, call stats, version, `draining` |
| `/drain` | POST / DELETE | Leave service, or return to it (`docs/CAPACITY_AND_SURVIVABILITY.md` §6) |
| `/calls` | GET | List all active calls on this node |
| `/registrations` | GET | List SIP registrations (optional `?aor=` filter) |
| `/metrics` | GET | Prometheus metrics in text exposition format |

`/health` and `/ready` answer different questions and are not
interchangeable: `/health` says the process is alive, `/ready` says it
should be given new work. A liveness probe pointed at `/ready` kills a
node in the middle of a drain.

Every routing binary answers all of these: the call manager, the SBC, the
registrar, the IVR, the voicemail server, the presence server, the WebRTC
gateway and the conference server. `/drain` and `/status` need the
binary's operator token on every one of them — `[http] api_token` on the
call manager, the registrar, the IVR, the voicemail server and the presence
server; `--health-api-token` on the SBC; `[http] auth_token` on the
conference server; `[auth] tokens` on the gateway — and a binary refuses a
non-loopback HTTP bind without one. `/ready` and `/health` never ask.

### Health Check Response

```json
{
  "status": "healthy",
  "service": "fcp-call-manager",
  "version": "0.14.96",
  "uptime_seconds": 3600,
  "active_calls": 12,
  "total_calls": 4567,
  "established_calls": 10,
  "terminated_calls": 4553,
  "failed_calls": 4
}
```

### Load Balancer Configuration

Point your load balancer at `GET /ready`, with 5-second intervals and a
3-failure threshold: it turns `503` the moment a node starts draining, so
new calls stop arriving before the node stops taking them. Point a
liveness probe (Kubernetes `livenessProbe`, systemd watchdog) at
`GET /health`, which stays `200` throughout a drain.

---

## 5. Monitoring

### Prometheus Metrics

Scrape `/metrics` on each node. Key metrics:

#### Redis Operations
| Metric | Type | Labels | Description |
|--------|------|--------|-------------|
| `fcp_redis_operations_total` | Counter | `operation`, `result` | Total ops (result: ok/err) |
| `fcp_redis_operation_duration_seconds` | Histogram | `operation` | Latency per operation |
| `fcp_redis_connection_errors_total` | Counter | — | Connection failures |

Operations tracked: `get`, `set`, `del`, `scan`, `hgetall`, `hset`, `expire`, `pipe`

#### Cache
| Metric | Type | Labels | Description |
|--------|------|--------|-------------|
| `fcp_cache_hits_total` | Counter | `store` | Cache hits |
| `fcp_cache_misses_total` | Counter | `store` | Cache misses |
| `fcp_cache_evictions_total` | Counter | `store` | Evictions |
| `fcp_cache_size` | Gauge | `store` | Current entry count |

### Alert Thresholds

| Condition | Severity | Description |
|-----------|----------|-------------|
| `fcp_redis_connection_errors_total` rate > 0 for 1m | Warning | Redis connectivity issues |
| `fcp_redis_operation_duration_seconds` p99 > 100ms | Warning | Redis latency spike |
| Node missing from `/status` for > 30s | Critical | Node departure |
| `fcp_cache_hits_total` / (`hits` + `misses`) < 0.5 | Info | Low cache hit rate |

### Grafana Tips

- Create a dashboard per node with Redis latency, operation throughput, and active calls
- Use `rate(fcp_redis_operations_total[5m])` for throughput graphs
- Use `histogram_quantile(0.99, rate(fcp_redis_operation_duration_seconds_bucket[5m]))` for p99 latency

---

## 6. Common Operations

### Rolling Restart

Restart nodes one at a time, and drain each one first so the restart costs
no calls (`docs/CAPACITY_AND_SURVIVABILITY.md` §6). Set
`[service] drain_wait_secs` on every node before you need it — it is `0`
by default, which means "stop where you stand":

```toml
[service]
drain_wait_secs = 300    # wait up to five minutes for calls to end
```

Then, per node:

```bash
# 1. Leave service. The node stops taking calls, keeps the ones it has,
#    and says so in its heartbeat, so placement elsewhere skips it.
curl -sX POST http://node1:8084/drain
#    -> {"status":"draining","active_calls":12,"wait_secs":300}

# 2. Watch it empty. /ready is 503 from here on; /health stays 200.
watch -n5 'curl -s http://node1:8084/status | jq "{draining,active_calls}"'

# 3. Stop it once it is empty — or straight away: SIGTERM runs the same
#    drain, waits drain_wait_secs, then BYEs whatever is left with a
#    Reason header and writes each record.
kill -TERM $(pidof fcp-call-manager)
# Wait for "Shutdown complete" in the logs.

# 4. Restart, and confirm it is serving before moving on.
fcp-call-manager --config node1.toml
curl -sf http://node1:8084/ready && echo serving

# Repeat for node 2, node 3, ...
```

A second SIGTERM during the wait ends the remaining calls immediately —
each still gets its BYE and its record — which is what to send when a
container runtime's grace period is about to expire.

What each kind of node does with what it is carrying:

| Node | On a drain |
|---|---|
| Call manager, SBC | Keeps its calls until the deadline, then BYEs both legs with `Reason: SIP;cause=200;text="node draining"` and completes each record |
| Registrar, call manager | Grants registrations of at most `[admission] drain_register_expiry_secs` (60 s), so devices come back on a serving node |
| IVR, voicemail | Lets callers finish; the voicemail server also waits for the messages already spoken to be written |
| Presence | Grants short subscriptions, then sends `terminated;reason=probation` to whoever is left |
| Conference | Moves its callers to other nodes by REFER (`docs/DISTRIBUTED_CONFERENCING.md` phase 5), then waits, then hangs up |
| WebRTC gateway | Tells each browser in a room to join another gateway (`docs/WEBRTC.md`), waits, then ends what is left |

Emergency calls are admitted on a draining node, as they are past every
other limit.

### Add a New Node

1. Prepare the config file with the same `cluster_secret` and `redis_url`
2. Assign a unique `service.node_id`
3. Start the node: `fcp-call-manager --config new-node.toml`
4. Verify it appears in other nodes' logs: `Cluster member joined`

### Remove a Node

1. Gracefully stop: `kill -TERM $(pidof fcp-call-manager)`
2. The node removes its heartbeat key on shutdown
3. Other nodes detect departure within one heartbeat cycle

### Drain a Node (without restarting it)

A drain is not a shutdown: the node keeps running and can be put back.

```bash
curl -sX POST   http://node:8084/drain   # leave service
curl -s         http://node:8084/status  # {"draining":true,"active_calls":…}
curl -sX DELETE http://node:8084/drain   # serve again; the calls it kept carry on
```

`DELETE /drain` before the deadline returns the node to service with
everything it was carrying untouched — which is what makes starting a
drain by hand safe. After the deadline the node has ended what was left,
and calling the drain off only makes it take new calls again.

The admin UI's cluster page shows a `draining` badge per node and a count
of draining nodes, from the same heartbeat field.

### Reload Queue Configuration (No Restart)

Send SIGHUP to reload queue configs without restarting:

```bash
kill -HUP $(pidof fcp-call-manager)
```

---

## 7. Troubleshooting

### Split Brain

**Symptoms**: Two groups of nodes can't see each other; duplicate registrations.

**Causes**:
- Redis is unreachable from some nodes but not others
- Network partition between node groups

**Resolution**:
1. Check Redis connectivity from all nodes: `redis-cli -h <redis-host> ping`
2. Check inter-node HTTP: `curl http://other-node:8084/health`
3. If Redis is down, see "Redis Down" below
4. If network partition, fix network and nodes will self-heal within one heartbeat cycle

### Redis Down

**Symptoms**: Logs show `Failed to publish cluster heartbeat` and `Failed to discover cluster members`. Circuit breaker may open.

**Impact**:
- New call registrations won't be tracked in the cluster
- Inter-node call lookup fails (but local calls continue working)
- Node membership is frozen (no join/leave detection)

**Resolution**:
1. Restore Redis connectivity
2. If circuit breaker opened, it will automatically probe after the cooldown period
3. Nodes will resume heartbeats and rediscover each other within one cycle
4. No manual intervention needed — the system self-heals

### Heartbeat Storms

**Symptoms**: Redis `SCAN` commands taking longer than expected. High CPU on Redis.

**Causes**: Too many nodes with too-short heartbeat intervals.

**Resolution**:
1. Increase `heartbeat_interval_secs` (e.g., 5 → 10)
2. Increase `heartbeat_ttl_secs` proportionally (e.g., 15 → 30)
3. Consider using Redis Cluster for larger deployments (future feature)

### Clock Skew

**Symptoms**: Nodes intermittently appear and disappear. TTL-based expiry behaves unpredictably.

**Resolution**:
1. Verify NTP sync on all nodes: `chronyc tracking`
2. Ensure clock skew < 2 seconds
3. If skew is large, increase `heartbeat_ttl_secs` to compensate

### Cross-Node Operations Failing

**Symptoms**: `POST /internal/hangup` returns 401 or connection refused.

**Resolution**:
1. Verify all nodes use the same `cluster_secret`
2. Verify `http.bind_address` is reachable from other nodes (not `127.0.0.1`)
3. Check firewall rules for the HTTP port
4. Check logs for `traceparent` and `x-fcp-node-id` headers for request tracing

---

## 8. Circuit Breaker

### States

| State | Behavior |
|-------|----------|
| **Closed** | All Redis operations proceed normally. Failures are counted. |
| **Open** | All Redis operations return an error immediately. No traffic to Redis. |
| **Half-Open** | One probe request is allowed. Success → Closed. Failure → Open. |

### Configuration

```toml
[cluster]
redis_circuit_breaker_threshold = 5    # failures before opening
redis_circuit_breaker_cooldown_secs = 30  # seconds before probe
```

### Fallback Behavior When Open

- Cluster heartbeats fail silently (node membership frozen)
- Call registry lookups return errors (local calls continue)
- Registration store falls back to primary backend if configured
- The node continues serving local calls normally

### Recovery Procedure

The circuit breaker is fully automatic:

1. After `cooldown_secs`, the breaker transitions to Half-Open
2. The next Redis operation is a probe
3. If the probe succeeds, the breaker closes and normal operation resumes
4. If the probe fails, the breaker reopens for another cooldown period

No manual intervention is required. Monitor the breaker state via logs:
```
Redis circuit breaker opening (failures=5, threshold=5, cooldown_secs=30)
Redis circuit breaker transitioning to half-open
Redis circuit breaker closing (probe succeeded)
```

---

## 9. Capacity Planning

### Redis Memory

| Item | Keys per Node | Size per Key | Notes |
|------|---------------|--------------|-------|
| Heartbeat | 1 | ~500 bytes | JSON with node metadata |
| Call registry | 1 per active call | ~100 bytes | `node_id` string |
| Parking slots | 1 per parked call | ~200 bytes | JSON with parking metadata |
| Registrations | 1 per SIP registration | ~300 bytes | JSON with contact, AOR, expiry |

**Estimate**: For a 10-node cluster with 5,000 concurrent calls and 2,000 registrations:
- Heartbeats: 10 x 500B = 5 KB
- Call registry: 5,000 x 100B = 500 KB
- Registrations: 2,000 x 300B = 600 KB
- **Total: ~1.1 MB** (Redis memory overhead adds ~2x, so ~2.5 MB)

Redis memory is not a bottleneck for typical PBX deployments.

### Redis Operations

Each node generates approximately:
- 1 `SET` + 1 `EXPIRE` per heartbeat interval (pipeline)
- 1 `SCAN` per heartbeat interval (discovery)
- N `GET` per SCAN result (member deserialization)
- 1 `SET` + 1 `EXPIRE` per new call (registration)
- 1 `DEL` per call termination

For a 10-node cluster with 5s heartbeat interval:
- Heartbeats: 10 nodes x 2 ops / 5s = 4 ops/s
- Discovery: 10 nodes x 1 SCAN / 5s = 2 SCANs/s
- Calls: depends on call volume

### Network Bandwidth

Inter-node HTTP traffic is minimal:
- Cross-node operations (hangup, supervisor, unpark) are rare
- Each operation is a single HTTP POST (~1 KB)
- Redis traffic is also lightweight (< 10 KB/s per node)
