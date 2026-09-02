# Event Streaming API

Real-time event streaming via WebSocket.

## Overview

The FCP Event Streaming API provides a WebSocket endpoint that streams platform events in real-time. Clients can subscribe to events and optionally filter by tenant, call, or event type.

## Endpoint

```
ws://localhost:8080/api/v1/events
```

## Connection

Upgrade to WebSocket using standard WebSocket handshake:

```http
GET /api/v1/events HTTP/1.1
Host: localhost:8080
Upgrade: websocket
Connection: Upgrade
Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==
Sec-WebSocket-Version: 13
```

## Query Parameters

All query parameters are optional. If no filters are specified, all events are streamed.

### tenant_id

Filter events for a specific tenant.

- **Type**: UUID string
- **Format**: `tenant-{uuid}` or raw UUID
- **Example**: `tenant-123e4567-e89b-12d3-a456-426614174000`

### call_id

Filter events for a specific call.

- **Type**: UUID string
- **Format**: `call-{uuid}` or raw UUID
- **Example**: `call-123e4567-e89b-12d3-a456-426614174000`

### event_types

Filter by event type categories.

- **Type**: Comma-separated string
- **Values**: `call`, `media`, `ivr`, `recording`, `system`
- **Example**: `call,media`

## Examples

### Subscribe to All Events

```javascript
const ws = new WebSocket('ws://localhost:8080/api/v1/events');
```

### Subscribe to Call Events Only

```javascript
const ws = new WebSocket(
    'ws://localhost:8080/api/v1/events?event_types=call'
);
```

### Subscribe to Events for a Specific Call

```javascript
const callId = 'call-123e4567-e89b-12d3-a456-426614174000';
const ws = new WebSocket(
    `ws://localhost:8080/api/v1/events?call_id=${callId}`
);
```

### Subscribe with Multiple Filters

```javascript
const ws = new WebSocket(
    'ws://localhost:8080/api/v1/events?event_types=call,media&tenant_id=tenant-123...'
);
```

## Message Format

All messages are JSON-encoded `PlatformEvent` objects.

### Connection Confirmation

Upon successful connection, the server sends a confirmation message:

```json
{
  "type": "connected",
  "message": "Successfully subscribed to event stream"
}
```

### Event Messages

Events follow this structure:

```json
{
  "event_type": "call|media|ivr|recording|system",
  "{event_type}": {
    "type": "...",
    // Event-specific fields
  }
}
```

## Event Types

### Call Events

#### Initiated

Sent when a call is created.

```json
{
  "event_type": "call",
  "call": {
    "type": "initiated",
    "call_id": "call-123e4567-e89b-12d3-a456-426614174000",
    "tenant_id": "tenant-123e4567-e89b-12d3-a456-426614174000",
    "from": "alice@example.com",
    "to": "bob@example.com",
    "direction": "inbound",
    "timestamp": 1705584000
  }
}
```

#### Ringing

Sent when a call starts ringing (180/183 received).

```json
{
  "event_type": "call",
  "call": {
    "type": "ringing",
    "call_id": "call-123e4567-e89b-12d3-a456-426614174000",
    "timestamp": 1705584001
  }
}
```

#### Answered

Sent when a call is answered (200 OK).

```json
{
  "event_type": "call",
  "call": {
    "type": "answered",
    "call_id": "call-123e4567-e89b-12d3-a456-426614174000",
    "answer_time": 1705584002,
    "codec": "PCMU"
  }
}
```

#### Ended

Sent when a call ends normally.

```json
{
  "event_type": "call",
  "call": {
    "type": "ended",
    "call_id": "call-123e4567-e89b-12d3-a456-426614174000",
    "end_time": 1705584120,
    "duration_ms": 118000,
    "hangup_cause": "Normal Clearing",
    "hangup_party": null
  }
}
```

#### Failed

Sent when a call fails (4xx/5xx response or timeout).

```json
{
  "event_type": "call",
  "call": {
    "type": "failed",
    "call_id": "call-123e4567-e89b-12d3-a456-426614174000",
    "timestamp": 1705584003,
    "reason": "486 Busy Here",
    "sip_code": 486
  }
}
```

### Media Events

#### Session Started

```json
{
  "event_type": "media",
  "media": {
    "type": "session_started",
    "session_id": "sess-123e4567-e89b-12d3-a456-426614174000",
    "call_id": "call-123e4567-e89b-12d3-a456-426614174000",
    "codec": "PCMU",
    "remote_addr": "192.168.1.100:10000",
    "timestamp": 1705584002
  }
}
```

#### DTMF Received

```json
{
  "event_type": "media",
  "media": {
    "type": "dtmf_received",
    "call_id": "call-123e4567-e89b-12d3-a456-426614174000",
    "digit": "5",
    "timestamp": 1705584010
  }
}
```

### System Events

#### Started

```json
{
  "event_type": "system",
  "system": {
    "type": "started",
    "node_id": "node-123e4567-e89b-12d3-a456-426614174000",
    "timestamp": 1705584000
  }
}
```

### Billing Events

Emitted by the billing engine; subscribe a webhook to `billing.*` to mirror
the ledger into an external billing system. See [BILLING.md](BILLING.md).

#### Call Rated

```json
{
  "event_type": "billing",
  "billing": {
    "type": "call_rated",
    "tenant_id": "tenant-123e4567-e89b-12d3-a456-426614174000",
    "call_id": "call-123e4567-e89b-12d3-a456-426614174000",
    "cost": 0.06,
    "currency": "USD",
    "matched_prefix": "44",
    "billable_secs": 120,
    "destination": "442079460000",
    "balance_after": 24.94,
    "timestamp": 1705584060
  }
}
```

#### Balance Changed

```json
{
  "event_type": "billing",
  "billing": {
    "type": "balance_changed",
    "tenant_id": "tenant-123e4567-e89b-12d3-a456-426614174000",
    "transaction_id": "7e1c…",
    "kind": "top_up",
    "amount": 25.0,
    "balance_after": 25.0,
    "currency": "USD",
    "call_id": null,
    "description": "card payment",
    "external_ref": "pay_01H…",
    "timestamp": 1705584000
  }
}
```

Other billing types: `balance_low` (`balance`, `threshold`, `currency`),
`balance_depleted` (`balance`, `credit_limit`, `currency`),
`call_rejected` (`destination`, `available`, `required`, `currency`) and
`call_cut_off` (`call_id`, `max_duration_secs`).

## Client Handling

### Receiving Events

```javascript
ws.onmessage = (event) => {
    const platformEvent = JSON.parse(event.data);

    if (platformEvent.type === 'connected') {
        console.log('Connected to event stream');
        return;
    }

    // Handle platform events
    switch (platformEvent.event_type) {
        case 'call':
            handleCallEvent(platformEvent.call);
            break;
        case 'media':
            handleMediaEvent(platformEvent.media);
            break;
        // ... other types
    }
};
```

### Error Handling

```javascript
ws.onerror = (error) => {
    console.error('WebSocket error:', error);
};

ws.onclose = (event) => {
    console.log('Connection closed:', event.code, event.reason);
    // Implement reconnection logic
};
```

### Connection Management

```javascript
let ws;
let reconnectInterval = 5000; // 5 seconds

function connect() {
    ws = new WebSocket('ws://localhost:8080/api/v1/events');

    ws.onopen = () => {
        console.log('Connected');
        reconnectInterval = 5000; // Reset backoff
    };

    ws.onclose = () => {
        console.log('Disconnected, reconnecting in', reconnectInterval, 'ms');
        setTimeout(connect, reconnectInterval);
        reconnectInterval = Math.min(reconnectInterval * 2, 60000); // Exponential backoff
    };
}

connect();
```

## Rate Limiting

Currently, no rate limiting is enforced on WebSocket connections. Future releases may implement:

- Maximum connections per client
- Maximum subscriptions per connection
- Event delivery rate limiting

## Performance

- **Event Buffer**: Each subscription has a 1000-event buffer
- **Dropped Events**: If buffer is full, events are dropped (logged server-side)
- **Latency**: Typical event delivery latency < 1ms for in-memory bus

## Security

**Note**: The current implementation does not include authentication. In production:

- Use TLS/WSS (wss://) for encrypted connections
- Implement authentication (JWT, API keys, OAuth)
- Validate tenant_id against authenticated user
- Apply rate limiting per user/tenant

## Examples

### Real-time Call Dashboard

See `/home/siphon/fcp/examples/websocket-events-client.html` for a complete example with:

- WebSocket connection management
- Event filtering UI
- Real-time statistics
- Event display with syntax highlighting

### Python Client

```python
import websocket
import json

def on_message(ws, message):
    event = json.loads(message)
    print(f"Received event: {event['event_type']}")

def on_error(ws, error):
    print(f"Error: {error}")

def on_close(ws, close_status_code, close_msg):
    print("Connection closed")

def on_open(ws):
    print("Connected to event stream")

ws = websocket.WebSocketApp(
    "ws://localhost:8080/api/v1/events?event_types=call",
    on_open=on_open,
    on_message=on_message,
    on_error=on_error,
    on_close=on_close
)

ws.run_forever()
```

### Node.js Client

```javascript
const WebSocket = require('ws');

const ws = new WebSocket('ws://localhost:8080/api/v1/events');

ws.on('open', () => {
    console.log('Connected to event stream');
});

ws.on('message', (data) => {
    const event = JSON.parse(data);
    console.log('Event received:', event);
});

ws.on('close', () => {
    console.log('Disconnected');
});

ws.on('error', (error) => {
    console.error('WebSocket error:', error);
});
```

## Troubleshooting

### Connection Refused

- Verify fcp-server is running
- Check bind address in configuration
- Ensure port 8080 is accessible

### No Events Received

- Verify filters are correct (UUIDs must match exactly)
- Check that calls are being created/processed
- Enable debug logging: `RUST_LOG=debug`

### Events Dropped

- Increase buffer size in InMemoryEventBus
- Implement backpressure handling in client
- Consider persistent event storage

## Future Enhancements

- **Authentication**: JWT/OAuth2 support
- **Event History**: Replay past events
- **Event Filtering**: More advanced filter expressions
- **Binary Protocol**: More efficient than JSON
- **Compression**: WebSocket compression support
