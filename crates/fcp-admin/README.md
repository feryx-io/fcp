# fcp-admin - FCP Admin API

**Management and Administration API for FCP**

## Overview

`fcp-admin` is a specialized API binary focused on management and administration operations. It provides RESTful endpoints for CRUD operations on:

- **Tenant Management**: Create, update, delete, and list tenants
- **User Administration**: User provisioning, password management, role assignment
- **Device Provisioning**: Register and manage SIP devices
- **Extension Configuration**: Create and manage phone extensions
- **Phone Number Management**: Assign and manage DIDs (Direct Inward Dialing numbers)
- **System Monitoring**: Health checks and system status

## Architecture

fcp-admin is designed to run **internally** behind an API gateway, not exposed directly to the internet. It focuses on storage-based CRUD operations and does not handle live SIP traffic or media processing.

```
┌──────────────┐
│ API Gateway  │  (TLS termination, rate limiting, caching)
└──────┬───────┘
       │
   ┌───▼────────┐
   │ fcp-admin  │  (Internal HTTP API on port 8080)
   └───┬────────┘
       │
   ┌───▼────────┐
   │  Database  │  (PostgreSQL/MySQL/MongoDB/SQLite)
   └────────────┘
```

### Key Characteristics

- **Stateless**: No session state, can be horizontally scaled
- **Storage-focused**: Primarily interacts with database
- **Internal only**: Not exposed to public internet
- **No SIP transport**: Doesn't listen for SIP messages
- **Event publishing**: Publishes changes to event bus

## Installation

Build from source:

```bash
cargo build --release -p fcp-admin
```

The binary will be located at `target/release/fcp-admin`.

## Configuration

fcp-admin uses a TOML configuration file. Create `fcp-admin.toml`:

```toml
[service]
service_type = "admin"
node_id = "admin-01"

[api]
bind_address = "0.0.0.0:8080"
auth_enabled = true
auth_token = "your-secret-token-here"

[storage]
storage_type = "postgres"
postgres_url = "postgres://fcp:password@localhost:5432/fcp"
max_connections = 50
min_connections = 10

[logging]
level = "info"
format = "json"
```

### Configuration Reference

#### [service]

- `service_type`: Always "admin" (default)
- `node_id`: Unique identifier for this admin instance (auto-generated if not provided)

#### [api]

- `bind_address`: API listen address (default: "0.0.0.0:8080")
- `auth_enabled`: Enable bearer token authentication (default: true)
- `auth_token`: Secret bearer token for API authentication

#### [storage]

- `storage_type`: Backend type: "sqlite", "postgres", "mysql", "mongodb" (default: "sqlite")
- `sqlite_path`: Path to SQLite database file (default: "fcp-admin.db")
- `postgres_url`: PostgreSQL connection string
- `mysql_url`: MySQL connection string
- `mongodb_url`: MongoDB connection string
- `max_connections`: Maximum database connections (default: 50)
- `min_connections`: Minimum database connections (default: 10)

#### [logging]

- `level`: Log level: "trace", "debug", "info", "warn", "error" (default: "info")
- `format`: Log format: "json", "pretty", "compact" (default: "json")

#### [conference]

- `server_url`: A conference server's HTTP API (e.g. "http://fcp-conference-server:8090"),
  used for live room controls in the admin UI — video layout, pin, spotlight,
  participant video on/off, dial-out — proxied under `/api/v1/conferences/live/{room}/…`.
  Any conference node will do: it forwards to the nodes hosting a room itself.
  Without it those endpoints answer 503.
- `auth_token`: Bearer token for that API when it has `http.auth_enabled`

## Usage

### Basic Usage

```bash
# Run with default configuration (fcp-admin.toml in current directory)
./fcp-admin

# Run with custom configuration file
./fcp-admin --config /etc/fcp/admin.toml

# Override log level
./fcp-admin --log-level debug

# Override bind address
./fcp-admin --bind 0.0.0.0:9090

# Provide auth token via environment variable
FCP_ADMIN_AUTH_TOKEN=secret ./fcp-admin
```

### Command-Line Options

```
FCP Admin - Management and administration endpoints

Usage: fcp-admin [OPTIONS]

Options:
  -c, --config <FILE>          Path to TOML configuration file
  -l, --log-level <LEVEL>      Override log level (trace, debug, info, warn, error)
      --log-format <FORMAT>    Log format (json, pretty, compact)
  -b, --bind <ADDR>            Override API bind address (e.g., 0.0.0.0:8080)
      --auth-token <TOKEN>     Bearer authentication token
  -h, --help                   Print help
  -V, --version                Print version
```

### Environment Variables

- `FCP_ADMIN_CONFIG`: Path to configuration file
- `FCP_ADMIN_LOG_LEVEL`: Log level override
- `FCP_ADMIN_LOG_FORMAT`: Log format override
- `FCP_ADMIN_BIND_ADDR`: API bind address override
- `FCP_ADMIN_AUTH_TOKEN`: Bearer authentication token

## API Endpoints

fcp-admin uses the shared fcp-api-core library and provides all FCP v1 API endpoints:

### Tenants
- `GET /api/v1/tenants` - List tenants
- `POST /api/v1/tenants` - Create tenant
- `GET /api/v1/tenants/{id}` - Get tenant details
- `PUT /api/v1/tenants/{id}` - Update tenant
- `DELETE /api/v1/tenants/{id}` - Delete tenant

### Users
- `GET /api/v1/users` - List users
- `POST /api/v1/users` - Create user
- `GET /api/v1/users/{id}` - Get user details
- `PUT /api/v1/users/{id}` - Update user
- `DELETE /api/v1/users/{id}` - Delete user

### Devices
- `GET /api/v1/devices` - List devices
- `POST /api/v1/devices` - Register device
- `GET /api/v1/devices/{id}` - Get device details
- `PUT /api/v1/devices/{id}` - Update device
- `DELETE /api/v1/devices/{id}` - Deregister device

### Extensions
- `GET /api/v1/extensions` - List extensions
- `POST /api/v1/extensions` - Create extension
- `GET /api/v1/extensions/{id}` - Get extension details
- `PUT /api/v1/extensions/{id}` - Update extension
- `DELETE /api/v1/extensions/{id}` - Delete extension

### Phone Numbers
- `GET /api/v1/phone-numbers` - List phone numbers
- `POST /api/v1/phone-numbers` - Assign phone number
- `GET /api/v1/phone-numbers/{id}` - Get phone number details
- `PUT /api/v1/phone-numbers/{id}` - Update phone number
- `DELETE /api/v1/phone-numbers/{id}` - Release phone number

### CDRs (Call Detail Records)
- `GET /api/v1/cdrs` - List CDRs
- `GET /api/v1/cdrs/{id}` - Get CDR details

### System
- `GET /api/v1/health` - Health check
- `GET /api/v1/system/version` - Version information

## Authentication

fcp-admin uses Bearer token authentication when `auth_enabled = true`:

```bash
curl -H "Authorization: Bearer your-secret-token-here" \
  http://localhost:8080/api/v1/tenants
```

## Deployment

### Docker

```dockerfile
FROM rust:1.75 as builder
WORKDIR /app
COPY . .
RUN cargo build --release -p fcp-admin

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/fcp-admin /usr/local/bin/
ENTRYPOINT ["fcp-admin"]
```

### Kubernetes

```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: fcp-admin
spec:
  replicas: 3
  selector:
    matchLabels:
      app: fcp-admin
  template:
    metadata:
      labels:
        app: fcp-admin
    spec:
      containers:
      - name: fcp-admin
        image: fcp-admin:latest
        ports:
        - containerPort: 8080
        env:
        - name: FCP_ADMIN_AUTH_TOKEN
          valueFrom:
            secretKeyRef:
              name: fcp-secrets
              key: admin-api-token
        - name: FCP_ADMIN_LOG_FORMAT
          value: "json"
        volumeMounts:
        - name: config
          mountPath: /etc/fcp
      volumes:
      - name: config
        configMap:
          name: fcp-admin-config
```

## Scaling

fcp-admin is designed for horizontal scaling:

- **Stateless**: No session state, safe to add/remove instances
- **Connection pooling**: Each instance has its own database connection pool
- **Load balancing**: Use round-robin or least-connections
- **Health checks**: `GET /api/v1/health`

### Recommended Scaling Strategy

- **2-3 instances minimum**: For high availability
- **Monitor**: Database connection usage, response times, error rates
- **Scale triggers**: CPU > 70%, DB connections > 80% of max
- **Database**: Use connection pooler (PgBouncer) for PostgreSQL

## Monitoring

### Metrics

While fcp-admin doesn't expose a `/metrics` endpoint by default (to avoid conflicting with dedicated metrics services), you can enable it if needed by modifying the configuration.

### Health Checks

```bash
# Health check
curl http://localhost:8080/api/v1/health

# Expected response
{"status":"healthy","version":"0.11.0"}
```

### Logging

fcp-admin logs in structured JSON format by default:

```json
{
  "timestamp": "2026-01-26T12:34:56.789Z",
  "level": "INFO",
  "target": "fcp_admin",
  "fields": {
    "message": "Admin API server started",
    "bind_address": "0.0.0.0:8080",
    "auth_enabled": true
  }
}
```

## Security Considerations

1. **Internal Only**: Never expose fcp-admin directly to the internet
2. **API Gateway**: Always deploy behind an API gateway
3. **TLS**: Use TLS at the gateway level
4. **Authentication**: Always enable `auth_enabled = true` in production
5. **Token Security**: Use strong, randomly generated auth tokens
6. **Database Access**: Use restricted database credentials
7. **Network Segmentation**: Deploy in internal network, isolated from DMZ

## Troubleshooting

### Database Connection Issues

```bash
# Check database connectivity
psql "postgres://fcp:password@localhost:5432/fcp" -c "SELECT 1"

# View connection pool status (check logs)
grep "connection_pool" fcp-admin.log
```

### Authentication Failures

```bash
# Verify token
curl -v -H "Authorization: Bearer your-token" http://localhost:8080/api/v1/health

# Check logs for auth errors
grep "auth" fcp-admin.log
```

### High Latency

```bash
# Check database query performance
# Enable query logging in PostgreSQL
ALTER SYSTEM SET log_statement = 'all';

# Monitor slow queries
grep "duration:" postgresql.log | awk '$4 > 1000'
```

## Development

### Building

```bash
cargo build -p fcp-admin
```

### Running Tests

```bash
cargo test -p fcp-admin
```

### Running Locally

```bash
# With SQLite (easiest for development)
cat > fcp-admin.toml <<EOF
[api]
bind_address = "127.0.0.1:8080"
auth_enabled = false

[storage]
storage_type = "sqlite"
sqlite_path = "test-admin.db"

[logging]
level = "debug"
format = "pretty"
EOF

cargo run -p fcp-admin
```

## Differences from fcp-server

| Feature | fcp-server | fcp-admin |
|---------|------------|-----------|
| SIP Transport | ✅ Yes (UDP/TCP/TLS) | ❌ No |
| API Endpoints | ✅ All endpoints | ✅ All endpoints |
| Call Processing | ✅ B2BUA, routing | ❌ No |
| Media Handling | ✅ RTP/SRTP | ❌ No |
| Registrar | ✅ Yes | ❌ No (read-only access) |
| Use Case | All-in-one server | Admin operations only |
| Deployment | Edge/Internal | Internal only |
| Scaling | Vertical (limited) | Horizontal (unlimited) |

## License

Licensed under MIT OR Apache-2.0

## See Also

- [fcp-server](../fcp-server/README.md) - All-in-one FCP server
- [fcp-api-core](../fcp-api-core/README.md) - Shared API implementation
- [API Documentation](../../docs/API_TIER_ARCHITECTURE.md) - Multi-tier API architecture
- [Deployment Guide](../../docs/DEPLOYMENT_BINARIES.md) - Binary deployment strategies
