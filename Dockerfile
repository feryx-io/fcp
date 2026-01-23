# Multi-stage Dockerfile for FCP (Ferrous Communications Platform)
# Stage 1: Build stage
FROM rust:1.74-slim as builder

# Install build dependencies
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    libsqlite3-dev \
    ca-certificates \
    git \
    && rm -rf /var/lib/apt/lists/*

# Create app directory
WORKDIR /app

# Copy dependency files first for better caching
COPY Cargo.toml Cargo.lock ./
COPY crates/ ./crates/
COPY external/ ./external/

# Build dependencies first (cached layer)
RUN cargo build --release --bin fcp-server

# Build the application
RUN cargo build --release --bin fcp-server --bin fcp-cli

# Stage 2: Runtime stage
FROM debian:bookworm-slim

# Install runtime dependencies
RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    libsqlite3-0 \
    && rm -rf /var/lib/apt/lists/*

# Create non-root user
RUN useradd -m -u 1000 -s /bin/bash fcp

# Create necessary directories
RUN mkdir -p /var/lib/fcp/recordings \
    /var/lib/fcp/voicemail \
    /var/lib/fcp/audio \
    /etc/fcp \
    /var/log/fcp && \
    chown -R fcp:fcp /var/lib/fcp /etc/fcp /var/log/fcp

# Copy binaries from builder
COPY --from=builder /app/target/release/fcp-server /usr/local/bin/
COPY --from=builder /app/target/release/fcp-cli /usr/local/bin/

# Copy default configuration (if exists)
COPY config/fcp.toml.example /etc/fcp/fcp.toml.example

# Switch to non-root user
USER fcp

# Set working directory
WORKDIR /home/fcp

# Expose ports
# 5060: SIP UDP/TCP
# 5061: SIP TLS
# 8080: REST API
# 10000-20000: RTP media
EXPOSE 5060/udp 5060/tcp 5061/tcp 8080/tcp
EXPOSE 10000-20000/udp

# Health check
HEALTHCHECK --interval=30s --timeout=10s --start-period=40s --retries=3 \
    CMD curl -f http://localhost:8080/api/v1/health || exit 1

# Environment variables
ENV RUST_LOG=info \
    FCP_LOG_FORMAT=json \
    FCP_CONFIG=/etc/fcp/fcp.toml

# Volumes for persistent data
VOLUME ["/var/lib/fcp", "/etc/fcp", "/var/log/fcp"]

# Default command
CMD ["fcp-server"]
