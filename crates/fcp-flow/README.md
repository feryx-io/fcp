# fcp-flow - Flow Engine for Programmable Voice

A JSON-based DSL (Domain-Specific Language) for defining programmable call flows, similar to Twilio Studio but self-hosted.

## Overview

The fcp-flow crate provides a complete flow execution engine for building IVR (Interactive Voice Response) systems, call routing, and other programmable voice applications. Flows are defined in JSON and executed step-by-step by the flow engine.

## Features

- **JSON-based flow definitions** - Easy to create, edit, and version control
- **16 node types** - Answer, Play, Say, Gather, Dial, Hangup, HTTP, Branch, SetVariable, Fork, Recording, Pause, Log
- **Variable interpolation** - Use `{{variable}}` syntax in text fields
- **Conditional branching** - Route calls based on variables and user input
- **HTTP integration** - Call external APIs during flow execution
- **Error handling** - On-error transitions for graceful failure handling
- **Version control** - Built-in flow versioning
- **Multi-tenant** - Tenant-aware storage and execution

## Quick Start

### Define a Flow

Create a flow definition in JSON:

```json
{
  "id": "welcome-flow",
  "name": "Welcome Flow",
  "description": "Simple welcome message",
  "version": 1,
  "initial_node": "answer",
  "enabled": true,
  "nodes": {
    "answer": {
      "id": "answer",
      "type": "answer",
      "next": "welcome"
    },
    "welcome": {
      "id": "welcome",
      "type": "say",
      "text": "Welcome to our service!",
      "language": "en-US",
      "next": "hangup"
    },
    "hangup": {
      "id": "hangup",
      "type": "hangup"
    }
  },
  "variables": {}
}
```

### Execute a Flow

```rust
use fcp_flow::{FlowEngine, FlowDefinition};
use fcp_core::CallId;
use std::collections::HashMap;
use std::sync::Arc;

#[tokio::main]
async fn main() {
    // Load flow definition
    let flow_json = std::fs::read_to_string("my-flow.json").unwrap();
    let flow: FlowDefinition = serde_json::from_str(&flow_json).unwrap();
    let flow = Arc::new(flow);

    // Create flow engine
    let engine = FlowEngine::new();

    // Start execution
    let call_id = CallId::new();
    let variables = HashMap::new();

    let mut context = engine.execute(flow, call_id, variables).await.unwrap();

    // Execute nodes
    let mut current_node = context.flow.initial_node.clone();
    loop {
        let result = engine.execute_node(&mut context, &current_node).await.unwrap();

        match result {
            FlowExecutionResult::Continue(next) => {
                current_node = next;
            },
            FlowExecutionResult::Hangup(_) => {
                println!("Flow completed - hanging up");
                break;
            },
            FlowExecutionResult::WaitingForInput { .. } => {
                println!("Waiting for user input");
                break;
            }
        }
    }
}
```

## Node Types

### Answer
Answers the incoming call.

```json
{
  "type": "answer",
  "next": "welcome"
}
```

### Say (TTS)
Speaks text using Text-to-Speech.

```json
{
  "type": "say",
  "text": "Hello {{caller_name}}, welcome to {{company}}!",
  "language": "en-US",
  "voice": "alice",
  "next": "menu",
  "on_error": "error_handler"
}
```

### Play
Plays an audio file.

```json
{
  "type": "play",
  "url": "https://example.com/music.wav",
  "loop_count": 0,
  "next": "menu",
  "on_error": "error_handler"
}
```

### Gather
Collects DTMF input from the caller.

```json
{
  "type": "gather",
  "prompt": {
    "type": "say",
    "text": "Press 1 for Sales, 2 for Support",
    "language": "en-US"
  },
  "num_digits": 1,
  "timeout": 5,
  "finish_on_key": "#",
  "variable": "menu_choice",
  "transitions": {
    "1": "sales",
    "2": "support"
  },
  "default": "invalid",
  "on_timeout": "timeout_handler"
}
```

### Dial
Dials/bridges to another destination.

```json
{
  "type": "dial",
  "destination": "sip:alice@example.com",
  "timeout": 30,
  "caller_id": "+15551234567",
  "max_duration": 3600,
  "next": "after_call",
  "on_busy": "busy_handler",
  "on_no_answer": "voicemail",
  "on_error": "error_handler"
}
```

### HTTP
Makes an HTTP request.

```json
{
  "type": "http",
  "method": "POST",
  "url": "https://api.example.com/lookup",
  "body": "{\"caller\": \"{{from_number}}\"}",
  "headers": {
    "Content-Type": "application/json",
    "Authorization": "Bearer {{api_token}}"
  },
  "timeout": 10,
  "response_variable": "api_response",
  "next": "process_response",
  "on_error": "api_error"
}
```

### Branch
Conditional branching based on variables.

```json
{
  "type": "branch",
  "variable": "customer_tier",
  "operator": "equals",
  "value": "premium",
  "if_true": "premium_queue",
  "if_false": "standard_queue"
}
```

**Supported operators**: `equals`, `not_equals`, `contains`, `starts_with`, `ends_with`, `greater_than`, `less_than`, `greater_than_or_equal`, `less_than_or_equal`, `exists`, `not_exists`

### SetVariable
Sets a variable value.

```json
{
  "type": "set_variable",
  "variable": "call_priority",
  "value": "high",
  "next": "route_call"
}
```

### Fork
Dials multiple destinations in parallel.

```json
{
  "type": "fork",
  "destinations": [
    "sip:alice@example.com",
    "sip:bob@example.com",
    "sip:charlie@example.com"
  ],
  "strategy": "first",
  "timeout": 30,
  "next": "after_fork",
  "on_error": "all_busy"
}
```

**Strategies**: `first` (first to answer wins), `all` (all ring simultaneously)

### StartRecording / StopRecording
Control call recording.

```json
{
  "type": "start_recording",
  "format": "wav",
  "variable": "recording_url",
  "next": "continue_call"
}
```

```json
{
  "type": "stop_recording",
  "next": "goodbye"
}
```

### Pause
Pauses execution for a duration.

```json
{
  "type": "pause",
  "duration": 2,
  "next": "continue"
}
```

### Log
Logs a message (for debugging).

```json
{
  "type": "log",
  "message": "Call from {{from_number}} reached menu",
  "level": "info",
  "next": "continue"
}
```

**Log levels**: `debug`, `info`, `warn`, `error`

### Hangup
Ends the call.

```json
{
  "type": "hangup",
  "reason": "completed"
}
```

## Variable Interpolation

Use `{{variable}}` syntax to insert variable values into text fields:

```json
{
  "type": "say",
  "text": "Hello {{caller_name}}, your account balance is {{balance}} dollars.",
  "next": "menu"
}
```

Variables can come from:
- Flow-level `variables` field (defaults)
- `initial_variables` passed to `execute()`
- Set during execution via `SetVariable` node
- HTTP response via `response_variable`
- User input via `Gather` node

## REST API

The flow engine exposes REST API endpoints for managing flows:

### Create Flow
```bash
POST /api/v1/flows
Content-Type: application/json

{
  "id": "my-flow",
  "name": "My Flow",
  "description": "A test flow",
  "initial_node": "answer",
  "nodes": { ... },
  "variables": {},
  "enabled": true
}
```

### List Flows
```bash
GET /api/v1/flows?enabled=true&limit=100&offset=0
```

### Get Flow
```bash
GET /api/v1/flows/my-flow
```

### Update Flow
```bash
PUT /api/v1/flows/my-flow
Content-Type: application/json

{
  "name": "Updated Flow",
  "enabled": false
}
```

### Delete Flow
```bash
DELETE /api/v1/flows/my-flow
```

### List Flow Versions
```bash
GET /api/v1/flows/my-flow/versions
```

## Storage

Flow definitions are stored in the database. Migrations are provided for:

- **SQLite**: `migrations/005_add_flow_engine.sql`
- **PostgreSQL**: `migrations/005_add_flow_engine.sql`
- **MySQL**: `migrations/005_add_flow_engine.sql`

For development and testing, an in-memory storage implementation (`InMemoryFlowStore`) is available.

## Examples

See the `examples/` directory for complete flow examples:

- `simple-ivr.json` - Basic IVR menu with DTMF input
- More examples coming soon!

## Architecture

```
┌─────────────────┐
│  FlowEngine     │  - Executes flows step-by-step
└────────┬────────┘
         │
         ├── FlowContext     - Execution state and variables
         ├── FlowStore       - Storage interface
         └── FlowExecutor    - Lifecycle management
```

## Future Enhancements

- **Visual Flow Editor**: Web-based drag-and-drop flow designer
- **Flow Testing**: Unit testing framework for flows
- **Flow Analytics**: Execution metrics and reporting
- **Subflows**: Reusable flow components
- **A/B Testing**: Run multiple flow variants
- **Real-time Events**: WebSocket streaming of flow execution events
- **Integration with CallManager**: Automatic flow execution on inbound calls

## License

MIT OR Apache-2.0
