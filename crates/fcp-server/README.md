# FCP Server

The main binary for running the Ferrous Communications Platform (FCP) server.

## Features

- **Multi-transport support**: UDP, TCP, TLS, WebSocket, WSS
- **Configuration**: TOML-based configuration with CLI overrides
- **Graceful shutdown**: Proper signal handling (Ctrl+C, SIGTERM)
- **Observability**: Structured logging with multiple output formats
- **Service management**: Automatic lifecycle management of all FCP services

## Installation

```bash
cargo install --path .
```

Or build from workspace root:

```bash
cargo build --release -p fcp-server
```

## Quick Start

1. **Create configuration file**:

```bash
cp fcp.toml.example fcp.toml
# Edit fcp.toml with your settings
```

2. **Run the server**:

```bash
fcp-server
```

Or with custom configuration:

```bash
fcp-server --config /path/to/config.toml
```

## Usage

```
FCP Server - Ferrous Communications Platform

Usage: fcp-server [OPTIONS]

Options:
  -c, --config <FILE>
          Path to TOML configuration file [env: FCP_CONFIG=]

  -l, --log-level <LEVEL>
          Override log level (trace, debug, info, warn, error) [env: FCP_LOG_LEVEL=]

      --log-format <FORMAT>
          Log format (json, pretty, compact) [env: FCP_LOG_FORMAT=]

  -n, --node-name <NAME>
          Override node name from config [env: FCP_NODE_NAME=]

  -b, --bind <ADDR>
          Override SIP bind address (e.g., 0.0.0.0:5060) [env: FCP_BIND_ADDR=]

  -h, --help
          Print help (see a summary with '-h')

  -V, --version
          Print version
```

## Configuration

### Configuration Priority

1. Command-line arguments (highest priority)
2. Environment variables (prefixed with `FCP_`)
3. Configuration file (`--config` or `./fcp.toml`)
4. Default values (lowest priority)

### Example Configuration

See [fcp.toml.example](./fcp.toml.example) for a complete example with all options.

### Environment Variables

All configuration options can be set via environment variables:

```bash
export FCP_CONFIG=/etc/fcp/config.toml
export FCP_LOG_LEVEL=debug
export FCP_LOG_FORMAT=json
export FCP_NODE_NAME=my-node
export FCP_BIND_ADDR=0.0.0.0:5060
```

## Development

Run in development mode with debug logging:

```bash
cargo run -p fcp-server -- --log-level debug --log-format pretty
```

Run tests:

```bash
cargo test -p fcp-server
```

## Graceful Shutdown

The server handles shutdown signals gracefully:

- **Ctrl+C** (SIGINT)
- **SIGTERM**

On receiving a shutdown signal, the server will:
1. Stop accepting new connections
2. Complete in-flight requests
3. Shutdown all services
4. Exit cleanly

## Observability

### Logging

Three log formats are available:

- **compact**: Human-readable, single-line format (default)
- **pretty**: Multi-line format with colors (good for development)
- **json**: Structured JSON format (good for log aggregation)

### Log Levels

- **trace**: Very verbose, includes internal details
- **debug**: Detailed information for debugging
- **info**: General information (default)
- **warn**: Warning messages
- **error**: Error messages

### Example: JSON Logging

```bash
fcp-server --log-format json | jq
```

## Architecture

The server follows a layered architecture:

```
fcp-server (binary)
    ↓
fcp-runtime (service coordination)
    ↓
fcp-sip (SIP integration)
    ↓
fcp-core (core types & traits)
    ↓
siphon-rs (SIP stack)
```

## License

MIT OR Apache-2.0
