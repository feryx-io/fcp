# FCP Deployment Guide

This guide covers deploying FCP (Ferrous Communications Platform) in various environments.

---

## Table of Contents

1. [Quick Start with Docker](#quick-start-with-docker)
2. [Production Deployment](#production-deployment)
3. [Kubernetes Deployment](#kubernetes-deployment)
4. [Configuration](#configuration)
5. [Security](#security)
6. [Monitoring](#monitoring)
7. [Troubleshooting](#troubleshooting)

---

## Quick Start with Docker

### Prerequisites

- Docker 20.10+
- Docker Compose 2.0+
- 2GB RAM minimum (4GB recommended)
- 10GB disk space

### Step 1: Clone Repository

```bash
git clone --recursive https://github.com/feryx-io/fcp.git
cd fcp
```

### Step 2: Start Services

```bash
# Start all services
docker-compose up -d

# Check status
docker-compose ps

# View logs
docker-compose logs -f fcp-server
```

### Step 3: Verify Installation

```bash
# Health check
curl http://localhost:8080/api/v1/health

# Expected response:
# {"status":"healthy","version":"0.9.0"}
```

### Step 4: Access Services

- **REST API**: http://localhost:8080
- **Grafana**: http://localhost:3000 (admin/admin)
- **Prometheus**: http://localhost:9090

### Step 5: Create First Tenant

```bash
curl -X POST http://localhost:8080/api/v1/tenants \
  -H "Content-Type: application/json" \
  -d '{
    "name": "My Company",
    "domain": "mycompany.local",
    "max_extensions": 100
  }'
```

### Step 6: Create First User

```bash
curl -X POST http://localhost:8080/api/v1/users \
  -H "Content-Type: application/json" \
  -d '{
    "username": "alice",
    "display_name": "Alice Smith",
    "email": "alice@mycompany.com",
    "password": "secure_password"
  }'
```

---

## Production Deployment

### System Requirements

#### Minimum (50 concurrent calls)
- CPU: 4 cores
- RAM: 8GB
- Disk: 100GB SSD
- Network: 100 Mbps

#### Recommended (500 concurrent calls)
- CPU: 16 cores
- RAM: 32GB
- Disk: 500GB SSD
- Network: 1 Gbps

### Preparation

#### 1. Update Configuration

Edit `config/fcp-docker.toml`:

```toml
[sip]
external_ip = "YOUR_PUBLIC_IP"
domain = "pbx.yourcompany.com"

[storage]
backend = "postgres"
connection_string = "postgresql://fcp:STRONG_PASSWORD@postgres:5432/fcp"

[security]
digest_auth_enabled = true
rate_limiting_enabled = true
cors_origins = ["https://admin.yourcompany.com"]
```

#### 2. Configure TLS

Generate TLS certificates:

```bash
# Using Let's Encrypt (recommended)
certbot certonly --standalone -d pbx.yourcompany.com

# Copy certificates
sudo cp /etc/letsencrypt/live/pbx.yourcompany.com/fullchain.pem config/certs/fcp.crt
sudo cp /etc/letsencrypt/live/pbx.yourcompany.com/privkey.pem config/certs/fcp.key

# Update config
[sip]
tls_enabled = true
tls_cert_path = "/etc/fcp/certs/fcp.crt"
tls_key_path = "/etc/fcp/certs/fcp.key"
```

#### 3. Firewall Configuration

```bash
# Allow SIP ports
sudo ufw allow 5060/udp
sudo ufw allow 5060/tcp
sudo ufw allow 5061/tcp  # SIP TLS

# Allow RTP media
sudo ufw allow 10000:20000/udp

# Allow API (restrict to admin network)
sudo ufw allow from 10.0.0.0/8 to any port 8080
```

#### 4. PostgreSQL Tuning

Edit docker-compose.yml PostgreSQL service:

```yaml
postgres:
  environment:
    - POSTGRES_INITDB_ARGS=--encoding=UTF-8 --lc-collate=C --lc-ctype=C
  command:
    - postgres
    - -c
    - max_connections=200
    - -c
    - shared_buffers=2GB
    - -c
    - effective_cache_size=6GB
    - -c
    - maintenance_work_mem=512MB
    - -c
    - checkpoint_completion_target=0.9
    - -c
    - wal_buffers=16MB
    - -c
    - default_statistics_target=100
    - -c
    - random_page_cost=1.1
    - -c
    - effective_io_concurrency=200
    - -c
    - work_mem=10MB
    - -c
    - min_wal_size=1GB
    - -c
    - max_wal_size=4GB
```

### Deployment

```bash
# Pull latest images
docker-compose pull

# Start with production config
docker-compose up -d

# Check health
docker-compose logs fcp-server | grep "Server listening"

# Run database migrations
docker-compose exec fcp-server fcp-server --migrate
```

### Post-Deployment

#### 1. Verify Services

```bash
# Check all containers
docker-compose ps

# Verify FCP
curl -k https://pbx.yourcompany.com:8080/api/v1/health

# Verify database
docker-compose exec postgres psql -U fcp -c "SELECT version();"
```

#### 2. Configure Monitoring

Access Grafana at http://localhost:3000:
1. Login with admin/admin
2. Change default password
3. Verify Prometheus datasource
4. Import FCP dashboard

#### 3. Configure Backup

```bash
# Create backup script
cat > /opt/fcp/backup.sh <<'EOF'
#!/bin/bash
DATE=$(date +%Y%m%d_%H%M%S)
BACKUP_DIR=/backup/fcp

# Backup PostgreSQL
docker-compose exec -T postgres pg_dump -U fcp fcp | gzip > $BACKUP_DIR/fcp_$DATE.sql.gz

# Backup voicemail
tar -czf $BACKUP_DIR/voicemail_$DATE.tar.gz -C /var/lib/docker/volumes/fcp_fcp-data/_data voicemail

# Cleanup old backups (keep 30 days)
find $BACKUP_DIR -name "*.gz" -mtime +30 -delete
EOF

chmod +x /opt/fcp/backup.sh

# Add to crontab (daily at 2 AM)
echo "0 2 * * * /opt/fcp/backup.sh" | crontab -
```

---

## Kubernetes Deployment

### Prerequisites

- Kubernetes 1.25+
- kubectl configured
- Helm 3+ (optional)

### Basic Deployment

```yaml
# fcp-deployment.yaml
apiVersion: v1
kind: Namespace
metadata:
  name: fcp

---
apiVersion: apps/v1
kind: Deployment
metadata:
  name: fcp-server
  namespace: fcp
spec:
  replicas: 3
  selector:
    matchLabels:
      app: fcp-server
  template:
    metadata:
      labels:
        app: fcp-server
    spec:
      containers:
      - name: fcp-server
        image: fcp:latest
        ports:
        - containerPort: 5060
          name: sip-udp
          protocol: UDP
        - containerPort: 5060
          name: sip-tcp
          protocol: TCP
        - containerPort: 8080
          name: http
        env:
        - name: RUST_LOG
          value: "info"
        - name: DATABASE_URL
          valueFrom:
            secretKeyRef:
              name: fcp-secrets
              key: database-url
        volumeMounts:
        - name: config
          mountPath: /etc/fcp
        - name: data
          mountPath: /var/lib/fcp
      volumes:
      - name: config
        configMap:
          name: fcp-config
      - name: data
        persistentVolumeClaim:
          claimName: fcp-data-pvc

---
apiVersion: v1
kind: Service
metadata:
  name: fcp-service
  namespace: fcp
spec:
  type: LoadBalancer
  selector:
    app: fcp-server
  ports:
  - name: sip-udp
    port: 5060
    protocol: UDP
  - name: sip-tcp
    port: 5060
    protocol: TCP
  - name: http
    port: 8080
```

Apply:

```bash
kubectl apply -f fcp-deployment.yaml
```

---

## Configuration

### Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `RUST_LOG` | `info` | Log level (error, warn, info, debug, trace) |
| `FCP_LOG_FORMAT` | `json` | Log format (json, pretty) |
| `FCP_CONFIG` | `/etc/fcp/fcp.toml` | Configuration file path |
| `DATABASE_URL` | - | Database connection string |
| `FCP_ENCRYPTION_PROVIDER` | `none` | Encryption at rest: `none`, `local`, or `aws-kms` (see `docs/ENCRYPTION.md`) |
| `FCP_MASTER_KEY` | - | `local` provider: base64 32-byte master key (`openssl rand -base64 32`) |
| `FCP_MASTER_KEY_FILE` | - | `local` provider: file containing the base64 master key |
| `FCP_AWS_KMS_KEY_ID` | - | `aws-kms` provider: KMS key id / ARN / alias |

### Configuration File Sections

#### [server]
- `bind_address`: API server bind address
- `log_level`: Logging verbosity
- `log_format`: Log output format

#### [sip]
- `bind_address`: SIP server bind address
- `external_ip`: Public IP for SDP
- `domain`: SIP domain

#### [storage]
- `backend`: Database backend (postgres, mysql, sqlite, mongodb)
- `connection_string`: Database connection

#### [media]
- `rtp_port_range`: RTP port range
- `recording_dir`: Call recording directory

#### [voicemail]
- `enabled`: Enable voicemail
- `recordings_dir`: Voicemail storage directory
- `max_message_duration`: Maximum message length in seconds

---

## Security

### Best Practices

1. **Change Default Passwords**
   - PostgreSQL
   - Grafana admin
   - API credentials

2. **Enable TLS**
   ```toml
   [sip]
   tls_enabled = true
   tls_cert_path = "/path/to/cert.pem"
   tls_key_path = "/path/to/key.pem"
   ```

3. **Restrict API Access**
   ```toml
   [security]
   cors_origins = ["https://admin.yourcompany.com"]
   rate_limiting_enabled = true
   ```

4. **Use Strong Passwords**
   - Minimum 12 characters
   - Mix of uppercase, lowercase, numbers, symbols
   - Hash SIP passwords with bcrypt

5. **Regular Updates**
   ```bash
   docker-compose pull
   docker-compose up -d
   ```

---

## Monitoring

### Metrics

FCP exports Prometheus metrics at `/api/v1/metrics`:

- `fcp_active_calls`: Current active calls
- `fcp_total_calls`: Total calls processed
- `fcp_failed_calls`: Total failed calls
- `fcp_active_registrations`: Active SIP registrations
- `fcp_http_requests_total`: HTTP requests by method/path
- `fcp_sip_transactions_total`: SIP transactions by state

### Grafana Dashboards

Pre-configured dashboards available in `deploy/grafana/dashboards/`:

1. **FCP Overview**: High-level metrics
2. **Call Statistics**: Detailed call analytics
3. **SIP Performance**: Transaction and dialog metrics
4. **System Resources**: CPU, memory, disk usage

### Alerts

Configure Prometheus alerts in `deploy/prometheus/alerts/`:

```yaml
# Example alert
groups:
- name: fcp
  rules:
  - alert: HighCallFailureRate
    expr: rate(fcp_failed_calls[5m]) > 0.1
    for: 5m
    labels:
      severity: warning
    annotations:
      summary: "High call failure rate detected"
```

---

## Troubleshooting

### Common Issues

#### 1. Container Won't Start

```bash
# Check logs
docker-compose logs fcp-server

# Common causes:
# - Invalid configuration
# - Port conflicts
# - Database connection failure
```

#### 2. SIP Registration Fails

```bash
# Check SIP bind
docker-compose exec fcp-server netstat -tulpn | grep 5060

# Verify firewall
sudo ufw status

# Check logs for auth errors
docker-compose logs fcp-server | grep -i "auth"
```

#### 3. No Audio (RTP Issues)

```bash
# Verify RTP ports are open
sudo ufw allow 10000:20000/udp

# Check external_ip setting
grep external_ip config/fcp-docker.toml

# Verify NAT traversal
# Set external_ip to public IP, not "auto"
```

#### 4. Database Connection Issues

```bash
# Check PostgreSQL is running
docker-compose ps postgres

# Test connection
docker-compose exec postgres psql -U fcp -d fcp -c "SELECT 1;"

# Verify credentials
grep connection_string config/fcp-docker.toml
```

#### 5. High CPU/Memory Usage

```bash
# Check resource usage
docker stats

# Tune connection pool
[storage]
max_connections = 50  # Reduce if high memory

# Enable query logging
RUST_LOG=debug,sqlx=debug
```

### Debug Mode

Enable debug logging:

```bash
# Edit docker-compose.yml
environment:
  - RUST_LOG=debug

# Restart
docker-compose restart fcp-server

# Follow logs
docker-compose logs -f fcp-server
```

### Support

- **Documentation**: https://fcp-docs.example.com
- **Issues**: https://github.com/feryx-io/fcp/issues
- **Community**: https://discord.gg/fcp

---

## Maintenance

### Updates

```bash
# Pull latest version
git pull origin main
git submodule update --recursive

# Rebuild
docker-compose build --no-cache

# Apply
docker-compose up -d

# Verify
curl http://localhost:8080/api/v1/health
```

### Log Rotation

Configure log rotation in `/etc/logrotate.d/fcp`:

```
/var/log/fcp/*.log {
    daily
    missingok
    rotate 14
    compress
    delaycompress
    notifempty
    create 0640 fcp fcp
    sharedscripts
    postrotate
        docker-compose restart fcp-server
    endscript
}
```

### Database Maintenance

```bash
# Vacuum (weekly)
docker-compose exec postgres psql -U fcp -d fcp -c "VACUUM ANALYZE;"

# Reindex (monthly)
docker-compose exec postgres psql -U fcp -d fcp -c "REINDEX DATABASE fcp;"

# Check size
docker-compose exec postgres psql -U fcp -d fcp -c "
  SELECT pg_size_pretty(pg_database_size('fcp'));"
```

---

## Capacity: what one node carries

Measured 2026-09-21 with `fcp-loadgen` (`docs/CAPACITY_AND_SURVIVABILITY.md`
§8) against a release-built call manager. **Read the machine before the
numbers**: a 12-core Azure VM with 2 GB of RAM running Postgres, the node
and the load generator at once. Every figure below is a floor — the run
ended before the node did.

| | Measured |
|---|---|
| Calls per second | **20/s sustained**, every rung from 2 to 20 held; the ramp ran out of time, not headroom |
| INVITE → `200 OK` | p50 **4 ms**, p99 **7 ms** at 20 calls/s |
| Media (B2BUA relay, 4419 legs) | **0.00 % loss**, 0.37 ms mean jitter |
| Memory per concurrent call | **≈0.3 MiB** (34 MiB idle → 64 MiB with 100 calls up) |
| CPU at 20 calls/s | 3 % of twelve cores |
| Registration storm | 300 devices, 1200 registrations, **all answered**, none refused |

What to take from it when sizing: a node's resident memory is roughly
`35 MiB + 0.3 MiB × concurrent calls` before recordings and conferences,
the signalling path costs single-digit milliseconds, and the relay does
not lose packets. What **not** to take from it: a ceiling. Nothing here
found one, and a real one needs a dedicated box with the load generator
on a different machine.

To measure your own hardware:

```bash
# On a machine that is not the node:
fcp-loadgen --profile call-ramp --sip <node>:5060 --realm <realm>   --users 50 --answer-extension-base 2000   --cps 5 --cps-step 5 --cps-max 200 --rung-seconds 30   --metrics-url http://<node>:8084/metrics --json ramp.json
```

Keep a single harness process under about 100 phones: past that the
generator becomes the bottleneck and measures itself (see the storm table
in §13 of the design document).

## Performance Tuning

### For High Call Volume

1. **Increase connection pool**
   ```toml
   [storage]
   max_connections = 200
   ```

2. **Expand RTP port range**
   ```toml
   [media]
   rtp_port_range = "10000-30000"
   ```

3. **Enable connection pooling**
   ```toml
   [storage]
   connection_timeout = 30
   min_connections = 10
   ```

4. **Add more workers**
   ```yaml
   # docker-compose.yml
   fcp-server:
     deploy:
       replicas: 3
   ```

---

**Last Updated**: January 2026
