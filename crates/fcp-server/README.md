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

## Storage Backends

FCP Server supports multiple storage backends for user credentials, SIP registrations, and call detail records (CDRs). The storage backend is configured via the `[storage]` section in the configuration file.

### Available Backends

- **SQLite** (default): File-based database, great for development and single-node deployments
- **PostgreSQL**: Production-ready relational database with ACID guarantees
- **MySQL**: Popular relational database, compatible with MariaDB
- **MongoDB**: NoSQL document database for large-scale deployments

### Configuration

Storage backends are configured in the `[storage]` section of `fcp.toml`:

```toml
[storage]
backend = "sqlite"  # Options: "sqlite", "postgres", "mysql", "mongodb"
connection_string = "sqlite://./data/fcp.db"
max_connections = 10
connection_timeout = "30s"
```

### SQLite Configuration

Default backend, requires no additional setup:

```toml
[storage]
backend = "sqlite"
connection_string = "sqlite://./data/fcp.db"
max_connections = 10
connection_timeout = "30s"
```

**Build with SQLite** (enabled by default):
```bash
cargo build --release -p fcp-server
```

### PostgreSQL Configuration

Requires a running PostgreSQL instance:

```toml
[storage]
backend = "postgres"
connection_string = "postgresql://username:password@localhost:5432/fcp"
max_connections = 20
connection_timeout = "30s"
```

**Build with PostgreSQL**:
```bash
cargo build --release -p fcp-server --features storage-postgres
```

**Setup PostgreSQL**:
```bash
# Create database
createdb fcp

# Run migrations
psql fcp < crates/fcp-storage-postgres/migrations/001_initial_schema.sql
```

### MySQL Configuration

Requires a running MySQL or MariaDB instance:

```toml
[storage]
backend = "mysql"
connection_string = "mysql://username:password@localhost:3306/fcp"
max_connections = 20
connection_timeout = "30s"
```

**Build with MySQL**:
```bash
cargo build --release -p fcp-server --features storage-mysql
```

**Setup MySQL**:
```bash
# Create database
mysql -u root -p -e "CREATE DATABASE fcp CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci;"

# Run migrations
mysql -u root -p fcp < crates/fcp-storage-mysql/migrations/001_initial_schema.sql
```

### MongoDB Configuration

Requires a running MongoDB instance:

```toml
[storage]
backend = "mongodb"
connection_string = "mongodb://username:password@localhost:27017/fcp"
max_connections = 20
connection_timeout = "30s"
```

**Build with MongoDB**:
```bash
cargo build --release -p fcp-server --features storage-mongodb
```

MongoDB doesn't require explicit migrations - collections and indexes are created automatically on first startup.

### Building with Multiple Backends

You can build with support for all storage backends:

```bash
cargo build --release -p fcp-server --features storage-all
```

This allows you to switch backends at runtime by changing the configuration file.

### Storage Backend Features

| Feature | SQLite | PostgreSQL | MySQL | MongoDB |
|---------|--------|------------|-------|---------|
| **User Management** | ✓ | ✓ | ✓ | ✓ |
| **SIP Registrations** | ✓ | ✓ | ✓ | ✓ |
| **CDRs** | ✗ | ✓ | ✓ | ✓ |
| **Connection Pooling** | ✓ | ✓ | ✓ | ✓ |
| **Transactions** | ✓ | ✓ | ✓ | ✓ |
| **High Availability** | ✗ | ✓ | ✓ | ✓ |
| **Horizontal Scaling** | ✗ | ✗ | ✗ | ✓ |

**Note**: SQLite's FileBackedStorage doesn't persist CDRs to optimize for simple deployments. For production CDR storage, use PostgreSQL, MySQL, or MongoDB.

### Choosing a Storage Backend

**Development**: Use SQLite (default) for quick setup with no external dependencies.

**Production - Single Node**: Use PostgreSQL or MySQL for ACID guarantees and full CDR support.

**Production - High Availability**: Use PostgreSQL or MySQL with replication/clustering.

**Production - Large Scale**: Use MongoDB for horizontal scalability across multiple nodes.

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
