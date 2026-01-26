# fcp-api-gateway - FCP API Gateway

**Public-facing API with caching, rate limiting, and request routing**

## Overview

`fcp-api-gateway` is the public-facing API gateway for FCP. It sits between clients and backend services, providing:

- **Request Routing**: Routes requests to appropriate backend services (admin, events, jobs)
- **Response Caching**: Redis-backed caching for GET requests
- **Rate Limiting**: Token bucket algorithm to prevent abuse
- **CORS Handling**: Configurable cross-origin resource sharing
- **Compression**: Gzip compression for responses
- **Health Checks**: Monitor backend service health

## Architecture

fcp-api-gateway is designed as a **stateless reverse proxy** that can be horizontally scaled:

```
┌─────────────┐
│   Clients   │  (Web apps, mobile apps, integrations)
└──────┬──────┘
       │
   ┌───▼────────────┐
   │  Load Balancer │  (HAProxy/nginx for TLS termination)
   └───┬────────────┘
       │
   ┌───▼──────────────┐
   │ fcp-api-gateway  │  (3+ instances, stateless)
   │ (Port 8000)      │
   └───┬──────────────┘
       │
       ├──────────┬──────────┬──────────┐
       │          │          │          │
   ┌───▼───┐  ┌──▼───┐  ┌───▼────┐  ┌─▼─────┐
   │ Redis │  │Admin │  │Events  │  │ Jobs  │
   │       │  │API   │  │API     │  │API    │
   └───────┘  └──────┘  └────────┘  └───────┘
```

### Key Characteristics

- **Stateless**: No local state, can be scaled horizontally
- **Caching**: Shared Redis cache across all gateway instances
- **Rate Limiting**: Per-gateway instance (distributed rate limiting via Redis in future)
- **Request Routing**: Path-based routing to backend services
- **Health Monitoring**: Periodic health checks of backends

## Features

### 1. Request Routing

Routes requests to appropriate backend services based on URL path:

- `/api/v1/*` → Admin API (fcp-admin)
- `/metrics` → Admin API metrics endpoint
- Future: `/api/v1/events/*` → Events API
- Future: `/api/v1/jobs/*` → Jobs API

### 2. Response Caching

- **Redis-backed**: Shared cache across all gateway instances
- **GET only**: Only GET requests are cached (configurable)
- **Configurable TTL**: Default 60 seconds, adjustable per deployment
- **Cache headers**: `X-Cache: HIT` or `X-Cache: MISS` in responses
- **Automatic invalidation**: Future enhancement

### 3. Rate Limiting

- **Token bucket algorithm**: Uses `governor` crate
- **Configurable limits**: Default 1000 requests/minute with 50 burst
- **Per-gateway**: Each gateway instance has its own limiter
- **HTTP 429**: Returns `429 Too Many Requests` when exceeded
- Future: Distributed rate limiting via Redis

### 4. CORS

- **Configurable**: Enable/disable via configuration
- **Allow all origins**: For development
- **Specific origins**: For production security
- **Credentials support**: Optional

### 5. Compression

- **Gzip compression**: Automatic for supported clients
- **Configurable**: Can be disabled if needed
- **Transparent**: Clients see compressed responses

## Installation

Build from source:

```bash
cargo build --release -p fcp-api-gateway
```

The binary will be located at `target/release/fcp-api-gateway`.

## Configuration

fcp-api-gateway uses a TOML configuration file. Create `fcp-api-gateway.toml`:

```toml
[gateway]
bind_address = "0.0.0.0:8000"
request_timeout = 30
compression_enabled = true

[backends.admin]
url = "http://localhost:8080"
health_check_path = "/api/v1/system/health"
health_check_interval = 10
connection_timeout = 5

[redis]
url = "redis://127.0.0.1:6379"
pool_size = 100
connection_timeout = 5

[rate_limit]
enabled = true
default_requests_per_minute = 1000
burst_size = 50

[cache]
enabled = true
default_ttl = 60
get_only = true

[cors]
enabled = true
allow_all_origins = true

[logging]
level = "info"
format = "json"
```

See `fcp-api-gateway.toml.example` for comprehensive configuration options.

### Configuration Reference

#### [gateway]

- `bind_address`: Public bind address (default: "0.0.0.0:8000")
- `request_timeout`: Request timeout in seconds (default: 30)
- `compression_enabled`: Enable gzip compression (default: true)

#### [backends.admin]

- `url`: Backend service URL (required)
- `health_check_path`: Health check endpoint (default: "/api/v1/system/health")
- `health_check_interval`: Check interval in seconds (default: 10)
- `connection_timeout`: Connection timeout in seconds (default: 5)

#### [redis]

- `url`: Redis connection URL (default: "redis://127.0.0.1:6379")
- `pool_size`: Connection pool size (default: 100)
- `connection_timeout`: Connection timeout in seconds (default: 5)

#### [rate_limit]

- `enabled`: Enable rate limiting (default: true)
- `default_requests_per_minute`: Requests per minute limit (default: 1000)
- `burst_size`: Maximum burst size (default: 50)

#### [cache]

- `enabled`: Enable caching (default: true)
- `default_ttl`: Cache TTL in seconds (default: 60)
- `get_only`: Cache only GET requests (default: true)

#### [cors]

- `enabled`: Enable CORS (default: true)
- `allow_all_origins`: Allow all origins (default: true)
- `allowed_origins`: List of allowed origins (when allow_all_origins = false)
- `allow_credentials`: Allow credentials (default: false)

#### [logging]

- `level`: Log level (default: "info")
- `format`: Log format: "json", "pretty", "compact" (default: "json")

## Usage

### Basic Usage

```bash
# Run with default configuration (fcp-api-gateway.toml in current directory)
./fcp-api-gateway

# Run with custom configuration file
./fcp-api-gateway --config /etc/fcp/gateway.toml

# Override log level
./fcp-api-gateway --log-level debug

# Override bind address
./fcp-api-gateway --bind 0.0.0.0:9000

# Override Redis URL
FCP_GATEWAY_REDIS_URL=redis://redis-cluster:6379 ./fcp-api-gateway
```

### Command-Line Options

```
FCP API Gateway - Public-facing API with caching and rate limiting

Usage: fcp-api-gateway [OPTIONS]

Options:
  -c, --config <FILE>         Path to TOML configuration file
  -l, --log-level <LEVEL>     Override log level (trace, debug, info, warn, error)
      --log-format <FORMAT>   Log format (json, pretty, compact)
  -b, --bind <ADDR>           Override gateway bind address
      --redis-url <URL>       Override Redis connection URL
  -h, --help                  Print help
  -V, --version               Print version
```

### Environment Variables

- `FCP_GATEWAY_CONFIG`: Path to configuration file
- `FCP_GATEWAY_LOG_LEVEL`: Log level override
- `FCP_GATEWAY_LOG_FORMAT`: Log format override
- `FCP_GATEWAY_BIND_ADDR`: Gateway bind address override
- `FCP_GATEWAY_REDIS_URL`: Redis URL override

## API Endpoints

### Gateway Endpoints

- `GET /health` - Gateway health check
- `GET /gateway/status` - Gateway status with backend health

Response:
```json
{
  "status": "operational",
  "service": "fcp-api-gateway",
  "version": "0.11.0",
  "backends": {
    "admin": {
      "url": "http://localhost:8080",
      "status": "unknown"
    }
  }
}
```

### Proxied Endpoints

All requests to `/api/v1/*` and `/metrics` are proxied to backend services.

## Caching Behavior

### Cache Keys

Cache keys are generated from the request path and query string:

```
gateway:cache:/api/v1/tenants
gateway:cache:/api/v1/users?limit=10
```

### Cache Headers

Responses include cache status headers:

```bash
# Cache miss (first request)
curl -I http://localhost:8000/api/v1/tenants
X-Cache: MISS

# Cache hit (subsequent request within TTL)
curl -I http://localhost:8000/api/v1/tenants
X-Cache: HIT
```

### Cache TTL

Default TTL is 60 seconds. Adjust based on data freshness requirements:

- **User profiles**: 300 seconds (5 minutes)
- **Tenant list**: 600 seconds (10 minutes)
- **CDRs**: 3600 seconds (1 hour)
- **System status**: 10 seconds

## Rate Limiting

### Limits

Default: 1000 requests/minute with 50 burst

### Response

When rate limit is exceeded:

```bash
HTTP/1.1 429 Too Many Requests
Content-Length: 0
```

### Tuning

Adjust limits based on backend capacity:

```toml
[rate_limit]
enabled = true
default_requests_per_minute = 10000  # High traffic
burst_size = 100
```

## Deployment

### Docker

```dockerfile
FROM rust:1.75 as builder
WORKDIR /app
COPY . .
RUN cargo build --release -p fcp-api-gateway

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/fcp-api-gateway /usr/local/bin/
ENTRYPOINT ["fcp-api-gateway"]
```

### Kubernetes

```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: fcp-api-gateway
spec:
  replicas: 3
  selector:
    matchLabels:
      app: fcp-api-gateway
  template:
    metadata:
      labels:
        app: fcp-api-gateway
    spec:
      containers:
      - name: fcp-api-gateway
        image: fcp-api-gateway:latest
        ports:
        - containerPort: 8000
        env:
        - name: FCP_GATEWAY_REDIS_URL
          value: "redis://redis-service:6379"
        - name: FCP_GATEWAY_LOG_FORMAT
          value: "json"
        volumeMounts:
        - name: config
          mountPath: /etc/fcp
        livenessProbe:
          httpGet:
            path: /health
            port: 8000
          initialDelaySeconds: 10
          periodSeconds: 10
        readinessProbe:
          httpGet:
            path: /health
            port: 8000
          initialDelaySeconds: 5
          periodSeconds: 5
      volumes:
      - name: config
        configMap:
          name: fcp-gateway-config
---
apiVersion: v1
kind: Service
metadata:
  name: fcp-api-gateway
spec:
  selector:
    app: fcp-api-gateway
  ports:
  - port: 80
    targetPort: 8000
  type: LoadBalancer
```

### HAProxy (TLS Termination)

```
frontend https_front
    bind *:443 ssl crt /etc/ssl/certs/fcp.pem
    default_backend api_gateway

backend api_gateway
    balance roundrobin
    option httpchk GET /health
    http-check expect status 200
    server gateway1 10.0.1.1:8000 check
    server gateway2 10.0.1.2:8000 check
    server gateway3 10.0.1.3:8000 check
```

## Scaling

### Horizontal Scaling Strategy

fcp-api-gateway is designed for unlimited horizontal scaling:

1. **Stateless**: No local state, safe to add/remove instances
2. **Shared Cache**: Redis provides shared cache across instances
3. **Load Balancing**: Use round-robin or least-connections
4. **Health Checks**: `GET /health` for load balancer health checks

### Recommended Configuration

**Small deployment** (< 1000 RPS):
- 2-3 gateway instances
- Single Redis instance
- 1-2 admin API instances

**Medium deployment** (1000-10000 RPS):
- 5-10 gateway instances
- Redis cluster (3 masters, 3 replicas)
- 3-5 admin API instances

**Large deployment** (> 10000 RPS):
- 10+ gateway instances
- Redis cluster with sharding
- 10+ admin API instances
- Multiple Redis pools

### Performance Tuning

1. **Connection pooling**: Increase Redis pool size for high traffic
2. **Cache TTL**: Longer TTL reduces backend load
3. **Rate limiting**: Adjust based on backend capacity
4. **Compression**: Disable if backends are on fast local network

## Monitoring

### Metrics

Key metrics to monitor:

- **Request rate**: Requests per second
- **Cache hit rate**: Percentage of cached responses
- **Rate limit hits**: Number of 429 responses
- **Backend latency**: Time to backend response
- **Error rate**: 5xx responses from backends

### Logging

Structured JSON logging for production:

```json
{
  "timestamp": "2026-01-26T12:34:56.789Z",
  "level": "INFO",
  "target": "fcp_api_gateway::proxy",
  "fields": {
    "message": "Proxying request",
    "method": "GET",
    "path": "/api/v1/tenants",
    "target": "http://localhost:8080/api/v1/tenants"
  }
}
```

### Health Checks

```bash
# Gateway health
curl http://localhost:8000/health

# Gateway status with backend info
curl http://localhost:8000/gateway/status
```

## Security

### Production Checklist

- [ ] **TLS**: Terminate TLS at load balancer (HAProxy/nginx)
- [ ] **CORS**: Set `allow_all_origins = false` and configure specific origins
- [ ] **Rate Limiting**: Enable with appropriate limits
- [ ] **Authentication**: Backend services should validate auth tokens
- [ ] **Network**: Deploy in DMZ, backend services internal only
- [ ] **Redis**: Secure Redis with password, firewall rules
- [ ] **Monitoring**: Set up alerts for high error rates, rate limiting

### Best Practices

1. **No Sensitive Data**: Gateway doesn't handle authentication directly
2. **Token Validation**: Backends validate bearer tokens, not gateway
3. **HTTPS Only**: Always use TLS in production
4. **Firewall Rules**: Gateway can access backends, backends can't be accessed directly
5. **DDoS Protection**: Use Cloudflare/AWS Shield in front of gateway

## Troubleshooting

### High Cache Miss Rate

```bash
# Check Redis connectivity
redis-cli -h localhost -p 6379 ping

# Check cache TTL
redis-cli -h localhost ttl "gateway:cache:/api/v1/tenants"

# Monitor cache hits/misses in logs
grep "Cache hit" fcp-gateway.log | wc -l
grep "Cache miss" fcp-gateway.log | wc -l
```

### Backend Connection Failures

```bash
# Check backend health directly
curl http://localhost:8080/api/v1/system/health

# Check gateway logs for backend errors
grep "Failed to proxy request" fcp-gateway.log

# Increase connection timeout if needed
```

### Rate Limiting Too Aggressive

```bash
# Check rate limit hits in logs
grep "Rate limit exceeded" fcp-gateway.log

# Adjust limits in config
[rate_limit]
default_requests_per_minute = 2000  # Increase limit
burst_size = 100
```

## Development

### Building

```bash
cargo build -p fcp-api-gateway
```

### Running Locally

```bash
# Start Redis
docker run -d -p 6379:6379 redis:7

# Start fcp-admin (backend)
cd ../fcp-admin
cargo run

# Start gateway
cat > fcp-api-gateway.toml <<EOF
[gateway]
bind_address = "127.0.0.1:8000"

[backends.admin]
url = "http://localhost:8080"

[redis]
url = "redis://127.0.0.1:6379"

[logging]
level = "debug"
format = "pretty"
EOF

cargo run -p fcp-api-gateway
```

### Testing

```bash
# Test routing
curl http://localhost:8000/api/v1/system/health

# Test caching
curl -I http://localhost:8000/api/v1/system/health
# First request: X-Cache: MISS
curl -I http://localhost:8000/api/v1/system/health
# Second request: X-Cache: HIT

# Test rate limiting
for i in {1..100}; do curl http://localhost:8000/api/v1/system/health; done
```

## Comparison

| Feature | fcp-api-gateway | fcp-admin | fcp-server |
|---------|-----------------|-----------|------------|
| Purpose | Public gateway | Admin CRUD | All-in-one |
| Caching | ✅ Redis | ❌ No | ❌ No |
| Rate Limiting | ✅ Yes | ❌ No | ❌ No |
| Request Routing | ✅ Yes | ❌ No | ❌ No |
| Direct DB Access | ❌ No | ✅ Yes | ✅ Yes |
| SIP Handling | ❌ No | ❌ No | ✅ Yes |
| Deployment | Public/DMZ | Internal | Edge/Internal |
| Horizontal Scaling | ✅ Unlimited | ✅ Unlimited | ⚠️ Limited |

## License

Licensed under MIT OR Apache-2.0

## See Also

- [fcp-admin](../fcp-admin/README.md) - Admin API backend
- [fcp-api-core](../fcp-api-core/README.md) - Shared API implementation
- [API Architecture](../../docs/API_TIER_ARCHITECTURE.md) - Multi-tier API design
- [Deployment Guide](../../docs/DEPLOYMENT_BINARIES.md) - Binary deployment strategies
