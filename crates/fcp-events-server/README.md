# fcp-events-server - FCP Events Server

**Real-time WebSocket event streaming for FCP platform events**

## Overview

`fcp-events-server` is a stateful WebSocket service that provides real-time event streaming to clients. It subscribes to the FCP event bus and broadcasts events to connected clients based on their subscriptions and filters.

## Features

- **WebSocket Connections**: Full-duplex communication with heartbeat/timeout handling
- **Event Filtering**: Clients can subscribe to specific event types or call IDs
- **Tenant Isolation**: Multi-tenant support with per-tenant connection limits
- **Event Batching**: Efficient batch delivery with configurable size and timeout
- **Connection Management**: Automatic cleanup of stale connections
- **Backpressure Handling**: Graceful handling of slow consumers
- **Health Monitoring**: Health check and status endpoints

## Architecture

fcp-events-server is designed as a **stateful service** for sticky sessions:

```
┌─────────────┐
│  Clients    │  (Web apps, mobile apps, dashboards)
└──────┬──────┘
       │
   ┌───▼────────────┐
   │  Load Balancer │  (Sticky sessions required)
   └───┬────────────┘
       │
   ┌───▼──────────────┐
   │ fcp-events-server│  (Stateful - WebSocket connections)
   │ (Port 8081)      │
   └───┬──────────────┘
       │
   ┌───▼───────┐
   │ Event Bus │  (In-memory or distributed)
   └───────────┘
```

### Key Characteristics

- **Stateful**: Maintains active WebSocket connections
- **Sticky Sessions**: Clients must connect to same instance
- **Event Streaming**: Real-time broadcast of platform events
- **Tenant Aware**: Per-tenant connection limits
- **Horizontal Scaling**: Limited (requires sticky sessions)

## Installation

Build from source:

```bash
cargo build --release -p fcp-events-server
```

The binary will be located at `target/release/fcp-events-server`.

## Configuration

fcp-events-server uses a TOML configuration file. Create `fcp-events-server.toml`:

```toml
[service]
service_type = "events"
node_id = "events-01"

[websocket]
bind_address = "0.0.0.0:8081"
heartbeat_interval = 30  # seconds
client_timeout = 60      # seconds
max_message_size = 1048576  # 1MB
batch_size = 10
batch_timeout = 100      # milliseconds

[event_bus]
bus_type = "in-memory"
buffer_size = 1000

[limits]
max_connections = 10000
max_connections_per_tenant = 100

[logging]
level = "info"
format = "json"
```

See `fcp-events-server.toml.example` for comprehensive configuration options.

### Configuration Reference

#### [service]

- `service_type`: Service type (always "events")
- `node_id`: Unique node identifier (auto-generated if not specified)

#### [websocket]

- `bind_address`: WebSocket bind address (default: "0.0.0.0:8081")
- `heartbeat_interval`: Interval for ping frames in seconds (default: 30)
- `client_timeout`: Client timeout in seconds (default: 60)
- `max_message_size`: Maximum message size in bytes (default: 1MB)
- `batch_size`: Maximum events per batch (default: 10)
- `batch_timeout`: Batch timeout in milliseconds (default: 100)

#### [event_bus]

- `bus_type`: Event bus type (default: "in-memory")
- `buffer_size`: Buffer size for event streams (default: 1000)

#### [limits]

- `max_connections`: Maximum concurrent connections (default: 10000)
- `max_connections_per_tenant`: Maximum connections per tenant (default: 100)

#### [logging]

- `level`: Log level: "trace", "debug", "info", "warn", "error" (default: "info")
- `format`: Log format: "json", "pretty", "compact" (default: "json")

## Usage

### Basic Usage

```bash
# Run with default configuration (fcp-events-server.toml in current directory)
./fcp-events-server

# Run with custom configuration file
./fcp-events-server --config /etc/fcp/events.toml

# Override log level
./fcp-events-server --log-level debug

# Override bind address
./fcp-events-server --bind 0.0.0.0:9081

# Override with environment variables
FCP_EVENTS_LOG_LEVEL=debug ./fcp-events-server
```

### Command-Line Options

```
FCP Events Server - WebSocket event streaming

Usage: fcp-events-server [OPTIONS]

Options:
  -c, --config <FILE>         Path to TOML configuration file
  -l, --log-level <LEVEL>     Override log level (trace, debug, info, warn, error)
      --log-format <FORMAT>   Log format (json, pretty, compact)
  -b, --bind <ADDR>           Override WebSocket bind address
  -h, --help                  Print help
  -V, --version               Print version
```

### Environment Variables

- `FCP_EVENTS_CONFIG`: Path to configuration file
- `FCP_EVENTS_LOG_LEVEL`: Log level override
- `FCP_EVENTS_LOG_FORMAT`: Log format override
- `FCP_EVENTS_BIND_ADDR`: WebSocket bind address override

## API Endpoints

### Health and Status

- `GET /health` - Health check (returns 200 OK)
- `GET /status` - Server status with connection statistics

Status response:
```json
{
  "status": "operational",
  "service": "fcp-events-server",
  "version": "0.14.0",
  "node_id": "events-01",
  "connections": {
    "total": 42,
    "max": 10000
  },
  "config": {
    "heartbeat_interval_secs": 30,
    "client_timeout_secs": 60,
    "batch_size": 10
  }
}
```

### WebSocket Connection

- `GET /ws/:tenant_id` - WebSocket upgrade endpoint

Connect via WebSocket:
```javascript
const ws = new WebSocket('ws://localhost:8081/ws/tenant-abc123');
```

## WebSocket Protocol

### Connection

1. Client connects to `/ws/:tenant_id`
2. Server upgrades to WebSocket
3. Server sends connection confirmation:

```json
{
  "type": "connected",
  "connection_id": "conn-abc123",
  "tenant_id": "tenant-abc123"
}
```

### Subscribing to Events

Send a subscription message to filter events:

```json
{
  "type": "subscribe",
  "filters": [
    {
      "call_id": "call-abc123"
    },
    {
      "event_types": ["call", "media"]
    }
  ]
}
```

Filter options:
- `call_id`: Filter by specific call ID
- `event_types`: Array of event types: "call", "media", "ivr", "recording", "flow", "system"

### Receiving Events

Events are delivered in batches:

```json
{
  "type": "events",
  "count": 3,
  "events": [
    {
      "event_type": "call",
      "type": "initiated",
      "call_id": "call-abc123",
      "tenant_id": "tenant-abc123",
      "from": "alice@example.com",
      "to": "bob@example.com",
      "direction": "inbound",
      "timestamp": 1706284800
    },
    {
      "event_type": "call",
      "type": "ringing",
      "call_id": "call-abc123",
      "timestamp": 1706284801
    },
    {
      "event_type": "call",
      "type": "answered",
      "call_id": "call-abc123",
      "answer_time": 1706284803,
      "codec": "PCMU"
    }
  ]
}
```

### Heartbeat

Server sends ping frames every `heartbeat_interval` seconds. Client should respond with pong frames or send ping frames to keep connection alive.

Client can also send keepalive messages:

```json
{
  "type": "ping"
}
```

### Disconnection

Client can close the WebSocket connection normally. Server will automatically cleanup after `client_timeout` seconds of inactivity.

## Event Types

### Call Events

- `initiated`: Call started (INVITE received/sent)
- `ringing`: Call ringing (180/183)
- `progress`: Call progress (183)
- `answered`: Call answered (200 OK)
- `ended`: Call ended (BYE/timeout)
- `failed`: Call failed (4xx/5xx)

### Media Events

- `session_started`: RTP session started
- `session_ended`: RTP session ended
- `dtmf_received`: DTMF digit received
- `quality_warning`: Media quality degraded

### IVR Events

- `prompt_started`: Prompt playback started
- `prompt_completed`: Prompt completed
- `input_gathered`: Input received
- `gather_timeout`: Gather timed out

### Recording Events

- `started`: Recording started
- `completed`: Recording completed
- `failed`: Recording failed

### Flow Events

- `started`: Flow execution started
- `node_executed`: Flow node executed
- `waiting_for_input`: Waiting for user input
- `completed`: Flow completed
- `failed`: Flow failed

### System Events

- `started`: System started
- `stopping`: System stopping
- `health_check`: Health check result

## Client Examples

### JavaScript/Browser

```javascript
const ws = new WebSocket('ws://localhost:8081/ws/tenant-abc123');

ws.onopen = () => {
  console.log('Connected');

  // Subscribe to call events only
  ws.send(JSON.stringify({
    type: 'subscribe',
    filters: [{
      event_types: ['call']
    }]
  }));
};

ws.onmessage = (event) => {
  const data = JSON.parse(event.data);

  if (data.type === 'connected') {
    console.log('Connection ID:', data.connection_id);
  } else if (data.type === 'events') {
    console.log('Received', data.count, 'events:', data.events);
  }
};

ws.onerror = (error) => {
  console.error('WebSocket error:', error);
};

ws.onclose = () => {
  console.log('Disconnected');
};
```

### Python

```python
import websocket
import json

def on_message(ws, message):
    data = json.loads(message)
    if data['type'] == 'events':
        print(f"Received {data['count']} events")
        for event in data['events']:
            print(f"  {event['event_type']}: {event['type']}")

def on_open(ws):
    print("Connected")
    # Subscribe to specific call
    ws.send(json.dumps({
        'type': 'subscribe',
        'filters': [{'call_id': 'call-abc123'}]
    }))

ws = websocket.WebSocketApp(
    'ws://localhost:8081/ws/tenant-abc123',
    on_open=on_open,
    on_message=on_message
)

ws.run_forever()
```

### Rust

```rust
use tokio_tungstenite::connect_async;
use futures_util::{SinkExt, StreamExt};

#[tokio::main]
async fn main() {
    let (ws_stream, _) = connect_async("ws://localhost:8081/ws/tenant-abc123")
        .await
        .expect("Failed to connect");

    let (mut write, mut read) = ws_stream.split();

    // Subscribe to all events
    let subscribe_msg = serde_json::json!({
        "type": "subscribe",
        "filters": []
    });
    write.send(subscribe_msg.to_string().into()).await.unwrap();

    while let Some(msg) = read.next().await {
        let msg = msg.unwrap();
        if let Ok(text) = msg.to_text() {
            println!("Received: {}", text);
        }
    }
}
```

## Deployment

### Docker

```dockerfile
FROM rust:1.75 as builder
WORKDIR /app
COPY . .
RUN cargo build --release -p fcp-events-server

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/fcp-events-server /usr/local/bin/
ENTRYPOINT ["fcp-events-server"]
```

### Kubernetes

```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: fcp-events-server
spec:
  replicas: 3
  selector:
    matchLabels:
      app: fcp-events-server
  template:
    metadata:
      labels:
        app: fcp-events-server
    spec:
      containers:
      - name: fcp-events-server
        image: fcp-events-server:latest
        ports:
        - containerPort: 8081
        env:
        - name: FCP_EVENTS_LOG_FORMAT
          value: "json"
        volumeMounts:
        - name: config
          mountPath: /etc/fcp
        livenessProbe:
          httpGet:
            path: /health
            port: 8081
          initialDelaySeconds: 10
          periodSeconds: 10
      volumes:
      - name: config
        configMap:
          name: fcp-events-config
---
apiVersion: v1
kind: Service
metadata:
  name: fcp-events-server
spec:
  selector:
    app: fcp-events-server
  ports:
  - port: 8081
    targetPort: 8081
  sessionAffinity: ClientIP  # Sticky sessions
  sessionAffinityConfig:
    clientIP:
      timeoutSeconds: 10800  # 3 hours
```

**IMPORTANT:** WebSocket connections require sticky sessions. Use `sessionAffinity: ClientIP` in Kubernetes or session persistence in your load balancer.

## Scaling

### Horizontal Scaling Strategy

fcp-events-server requires sticky sessions for WebSocket connections:

1. **Stateful**: Each instance maintains WebSocket connections
2. **Sticky Sessions**: Load balancer must route client to same instance
3. **Connection Limits**: Configure per-instance connection limits
4. **Health Checks**: Use `GET /health` for load balancer health checks

### Recommended Configuration

**Small deployment** (< 1000 concurrent connections):
- 2-3 events server instances
- In-memory event bus
- Simple sticky session load balancing

**Medium deployment** (1000-10000 concurrent connections):
- 5-10 events server instances
- Distributed event bus (Redis Streams/NATS)
- Load balancer with session persistence

**Large deployment** (> 10000 concurrent connections):
- 10+ events server instances
- Distributed event bus with clustering
- Multiple load balancers
- Connection pooling and rate limiting

### Performance Tuning

1. **Batch Size**: Larger batches reduce overhead but increase latency
2. **Batch Timeout**: Lower timeout improves real-time delivery
3. **Heartbeat Interval**: Balance between overhead and responsiveness
4. **Buffer Size**: Increase for high-throughput scenarios

## Monitoring

### Metrics to Monitor

- **Active Connections**: Total concurrent connections
- **Connection Rate**: New connections per second
- **Event Throughput**: Events published per second
- **Batch Efficiency**: Average events per batch
- **Timeout Rate**: Connections timing out
- **Memory Usage**: Per-connection memory footprint

### Logging

Structured JSON logging for production:

```json
{
  "timestamp": "2026-01-26T12:34:56.789Z",
  "level": "INFO",
  "target": "fcp_events_server::connection",
  "fields": {
    "message": "WebSocket connection established",
    "connection_id": "conn-abc123",
    "tenant_id": "tenant-abc123"
  }
}
```

### Health Checks

```bash
# Server health
curl http://localhost:8081/health

# Server status with statistics
curl http://localhost:8081/status
```

## Security

### Production Checklist

- [ ] **TLS**: Use WSS (wss://) for encrypted connections
- [ ] **Authentication**: Implement JWT/OAuth token validation
- [ ] **Rate Limiting**: Limit connection attempts per IP
- [ ] **Connection Limits**: Configure per-tenant limits
- [ ] **Firewall**: Restrict access to event bus
- [ ] **Monitoring**: Alert on unusual connection patterns

### Best Practices

1. **Token Validation**: Validate authentication tokens on connection
2. **TLS Only**: Always use WSS in production (wss://)
3. **Tenant Isolation**: Enforce tenant boundaries
4. **Connection Limits**: Prevent resource exhaustion
5. **Input Validation**: Validate all client messages

## Troubleshooting

### High Connection Count

```bash
# Check connection statistics
curl http://localhost:8081/status | jq '.connections'

# Monitor connection rate in logs
grep "WebSocket connection established" fcp-events.log | wc -l
```

### Events Not Received

1. Check event bus connectivity
2. Verify subscription filters
3. Check client WebSocket connection state
4. Monitor batch delivery in logs

### Connection Timeouts

```bash
# Check timeout configuration
grep "client_timeout" fcp-events-server.toml

# Monitor timeout events
grep "timed out" fcp-events.log
```

### High Memory Usage

- Review connection limits
- Check event buffer sizes
- Monitor batch sizes
- Analyze per-connection memory

## Development

### Building

```bash
cargo build -p fcp-events-server
```

### Running Locally

```bash
# Create config
cat > fcp-events-server.toml <<EOF
[websocket]
bind_address = "127.0.0.1:8081"

[logging]
level = "debug"
format = "pretty"
EOF

# Run server
cargo run -p fcp-events-server
```

### Testing

```bash
# Run unit tests
cargo test -p fcp-events-server

# Test WebSocket connection
websocat ws://localhost:8081/ws/tenant-test

# Test with wscat (Node.js)
npm install -g wscat
wscat -c ws://localhost:8081/ws/tenant-test
```

## Comparison

| Feature | fcp-events-server | fcp-admin | fcp-api-gateway |
|---------|-------------------|-----------|------------------|
| Purpose | Event streaming | Admin CRUD | Public gateway |
| Protocol | WebSocket | HTTP | HTTP |
| State | Stateful | Stateless | Stateless |
| Sticky Sessions | ✅ Required | ❌ No | ❌ No |
| Real-time | ✅ Yes | ❌ No | ❌ No |
| Horizontal Scaling | ⚠️ Limited | ✅ Unlimited | ✅ Unlimited |

## License

Licensed under MIT OR Apache-2.0

## See Also

- [fcp-events](../fcp-events/README.md) - Event bus and types
- [fcp-admin](../fcp-admin/README.md) - Admin API backend
- [fcp-api-gateway](../fcp-api-gateway/README.md) - API gateway
- [API Architecture](../../docs/API_TIER_ARCHITECTURE.md) - Multi-tier API design
- [Deployment Guide](../../docs/DEPLOYMENT_BINARIES.md) - Binary deployment strategies
