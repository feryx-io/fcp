# fcp-events

Real-time event system for the Ferrous Communications Platform (FCP).

## Overview

The `fcp-events` crate provides a pub/sub event bus architecture for streaming platform events in real-time. It enables applications to subscribe to call lifecycle, media, IVR, recording, and system events as they occur.

## Features

- **Type-safe events**: Strongly-typed event hierarchy with full serde support
- **Flexible filtering**: Subscribe to specific events by tenant, call, or type
- **Async streaming**: Built on tokio with non-blocking event delivery
- **In-memory bus**: Production-ready InMemoryEventBus with DashMap
- **Extensible**: Trait-based design allows custom event bus implementations

## Event Types

### PlatformEvent

Top-level event enum with the following categories:

- **Call**: Call lifecycle events (Initiated, Ringing, Answered, Ended, Failed)
- **Media**: RTP session, DTMF, quality warnings
- **IVR**: Prompt playback, input gathering
- **Recording**: Recording start/stop/completion
- **System**: Node health, startup/shutdown

### Call Events

```rust
use fcp_events::{PlatformEvent, CallEvent, CallDirection};
use std::time::SystemTime;

let event = PlatformEvent::Call(CallEvent::Initiated {
    call_id: CallId::new(),
    tenant_id: TenantId::new(),
    from: "alice@example.com".to_string(),
    to: "bob@example.com".to_string(),
    direction: CallDirection::Inbound,
    timestamp: SystemTime::now(),
});
```

## Event Bus

### Publishing Events

```rust
use fcp_events::{InMemoryEventBus, EventPublisher};

let bus = InMemoryEventBus::new();

// Publish an event
bus.publish(event).await?;
```

### Subscribing to Events

```rust
use fcp_events::{EventFilter, EventSubscriber};

// Subscribe to all events
let mut stream = bus.subscribe(EventFilter::new()).await?;

// Subscribe to events for a specific call
let filter = EventFilter::new().call(call_id);
let mut stream = bus.subscribe(filter).await?;

// Subscribe to specific event types
let filter = EventFilter::new()
    .types(vec![EventType::Call, EventType::Media]);
let mut stream = bus.subscribe(filter).await?;

// Receive events
while let Some(event) = stream.next().await {
    println!("Received event: {:?}", event);
}
```

## Event Filtering

Filter events by:

- **tenant_id**: Receive events for a specific tenant
- **call_id**: Receive events for a specific call
- **event_types**: Receive only certain event categories

### Example: Multi-tenant Dashboard

```rust
// Subscribe to all call events for a specific tenant
let filter = EventFilter::new()
    .tenant(tenant_id)
    .types(vec![EventType::Call]);

let mut stream = bus.subscribe(filter).await?;

while let Some(event) = stream.next().await {
    // Update dashboard with call events for this tenant only
    update_dashboard(event);
}
```

## WebSocket Streaming

The `fcp-server` exposes events via WebSocket at `/api/v1/events`.

### JavaScript Client Example

```javascript
// Connect to event stream
const ws = new WebSocket('ws://localhost:8080/api/v1/events');

// With filtering
const ws = new WebSocket(
    'ws://localhost:8080/api/v1/events?event_types=call,media&call_id=call-123...'
);

ws.onmessage = (event) => {
    const platformEvent = JSON.parse(event.data);
    console.log('Event received:', platformEvent);
};
```

### Query Parameters

- `tenant_id`: Filter by tenant UUID (e.g., `tenant-123e4567-...`)
- `call_id`: Filter by call UUID (e.g., `call-123e4567-...`)
- `event_types`: Comma-separated types (e.g., `call,media,system`)

## Event Serialization

Events are serialized to JSON with `serde`:

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

**Note**: `SystemTime` is serialized as Unix timestamp (seconds since epoch).

## Architecture

### InMemoryEventBus

The default implementation uses:

- **DashMap**: Lock-free concurrent hash map for subscriptions
- **tokio::mpsc**: Buffered channels (1000 events) for event delivery
- **parking_lot::Mutex**: For subscription ID generation

### Event Delivery

- Events are cloned and sent to all matching subscribers
- Delivery is **non-blocking** via `try_send()`
- If a subscriber's channel is full, the event is dropped (logged as warning)
- Closed subscriptions are automatically cleaned up

### Performance Characteristics

- **Publish latency**: O(subscribers) with DashMap iteration
- **Memory**: O(subscribers × buffer_size) for channels
- **Subscription**: O(1) insertion with atomic ID generation
- **Filtering**: O(1) per event (simple field comparisons)

## Use Cases

### Real-time Dashboard

```rust
// Subscribe to all call events
let filter = EventFilter::new().types(vec![EventType::Call]);
let mut stream = bus.subscribe(filter).await?;

while let Some(event) = stream.next().await {
    match event {
        PlatformEvent::Call(CallEvent::Initiated { .. }) => {
            dashboard.increment_active_calls();
        }
        PlatformEvent::Call(CallEvent::Ended { .. }) => {
            dashboard.decrement_active_calls();
        }
        _ => {}
    }
}
```

### Webhook Dispatcher

```rust
// Forward events to external webhook
let mut stream = bus.subscribe(EventFilter::new()).await?;

while let Some(event) = stream.next().await {
    let json = serde_json::to_string(&event)?;
    http_client.post("https://example.com/webhook")
        .body(json)
        .send()
        .await?;
}
```

### Call Recording Trigger

```rust
// Start recording when call is answered
let filter = EventFilter::new().types(vec![EventType::Call]);
let mut stream = bus.subscribe(filter).await?;

while let Some(event) = stream.next().await {
    if let PlatformEvent::Call(CallEvent::Answered { call_id, .. }) = event {
        recording_service.start_recording(call_id).await?;
    }
}
```

## Future Enhancements

Planned for future releases:

- **Redis backend**: Distributed event bus with Redis Streams
- **NATS backend**: High-performance streaming with NATS JetStream
- **Event persistence**: Store events for replay/analytics
- **Dead letter queue**: Failed event delivery handling
- **Backpressure control**: Dynamic buffer sizing
- **Event TTL**: Automatic event expiration

## Integration with FCP

The event system is automatically integrated into FCP:

1. **CallManager** publishes events at each call lifecycle stage
2. **RuntimeBuilder** creates the event bus on startup
3. **ApiState** exposes the bus to HTTP/WebSocket handlers
4. **WebSocket endpoint** streams events to connected clients

No additional setup required - events start flowing automatically when the FCP server starts.

## Examples

See `/home/siphon/fcp/examples/websocket-events-client.html` for a complete WebSocket client example with filtering and real-time display.

## Testing

```bash
# Run unit tests
cargo test -p fcp-events

# Run integration tests
cargo test -p fcp-events --test '*'

# Check event serialization
cargo test -p fcp-events test_event_serialization
```

## License

Licensed under the same terms as the FCP project.
