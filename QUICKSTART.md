# FCP Quick Start Guide

## 🚀 Getting Started

### Prerequisites
- Rust 1.75+ (check: `rustc --version`)
- Git
- Cargo

### Build & Run

```bash
# Build the server
cargo build --release --bin fcp-server

# Run the server (API-only mode currently)
./target/release/fcp-server

# In another terminal, test the API
curl http://localhost:8080/api/v1/users
```

### Create Your First User

```bash
curl -X POST http://localhost:8080/api/v1/users \
  -H 'Content-Type: application/json' \
  -d '{"username":"alice","password":"secret123","display_name":"Alice","enabled":true}'
```

## 📚 Key Documents

- **README.md** - Project overview
- **ARCHITECTURE.md** - System design
- **Phase1 plan.md** - Original 16-week plan
- **PHASE1_COMPLETION.md** - Remaining work to finish Phase 1 (← **START HERE**)
- **TESTING.md** - Testing guide with current status
- **SESSION_SUMMARY.md** - Latest session notes

## 📂 Project Structure

```
fcp/
├── crates/
│   ├── fcp-core/       # Core types, storage, config
│   ├── fcp-routing/    # Policy-based routing (BONUS!)
│   ├── fcp-sip/        # SIP integration layer
│   ├── fcp-runtime/    # Runtime & service management
│   ├── fcp-server/     # Main server binary + REST API
│   └── siphon-rs/      # SIP stack (submodule)
├── examples/
│   ├── routing.yaml    # Policy configuration
│   └── users.json      # Pre-configured users
├── fcp.toml           # Server configuration
└── target/release/
    └── fcp-server     # Compiled binary
```

## 🎯 Current Status (75% Phase 1 Complete)

### ✅ Working
- REST API (user management)
- Policy-based routing engine
- Multi-tenant isolation
- Storage layer (SQLite)
- Configuration management

### ⚠️ Known Issues
- Socket binding fails (SIP transport disabled)

### 🔲 TODO for Phase 1
- B2BUA call handling (INVITE/BYE)
- RTP media relay
- Fix socket binding
- Docker packaging
- Integration testing

## 🛠️ Common Tasks

### Run Tests
```bash
# All tests
cargo test --workspace

# Specific test
cargo test --test multi_tenant_test

# With output
cargo test -- --nocapture
```

### Check Code
```bash
# Format
cargo fmt

# Lint
cargo clippy

# Build
cargo build
```

### View Logs
```bash
# Run with debug logging
RUST_LOG=debug ./target/release/fcp-server

# API requests are logged automatically
```

### Clean Build
```bash
cargo clean
rm -rf target/
rm fcp.db data/
```

## 🐛 Troubleshooting

### "Address already in use"
**Workaround**: Transport is disabled in `fcp.toml`. Server runs in API-only mode.

### "Cannot connect to API"
**Check**: Server is running on http://localhost:8080
```bash
curl http://localhost:8080/api/v1/users
```

### "cargo: command not found"
**Install Rust**: https://rustup.rs/
```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

## 📖 Learn More

### Phase 1 Completion
See `PHASE1_COMPLETION.md` for detailed task breakdown:
- Estimated time: 25-37 hours (3-4 weeks)
- Tasks organized by sprint
- Clear acceptance criteria

### API Reference
All endpoints under `/api/v1/`:
- `GET /users` - List users
- `GET /users/:username` - Get user
- `POST /users` - Create user
- `PUT /users/:username` - Update user
- `PUT /users/:username/password` - Change password
- `DELETE /users/:username` - Delete user

### Configuration
Edit `fcp.toml` to configure:
- Node settings (domain, name)
- Transport (UDP/TCP ports)
- Registrar (auth, expiry)
- API (bind address)
- Routing (policy file)
- Logging (level, format)

## 🚧 Next Steps

1. **Read** `PHASE1_COMPLETION.md` for detailed plan
2. **Choose** next task:
   - Option A: Debug socket binding issue
   - Option B: Implement B2BUA call handling
   - Option C: Add Docker packaging
3. **Test** as you go with `cargo test`
4. **Update** docs when complete

## 💬 Need Help?

- Check `ARCHITECTURE.md` for system design
- Review `Phase1 plan.md` for original vision
- Read `SESSION_SUMMARY.md` for latest notes
- Look at test files for examples

## 🎉 Success Criteria

Phase 1 is complete when:
- [ ] SIP registration works
- [ ] Calls can be placed
- [ ] Audio flows in both directions
- [ ] Policies enforce correctly
- [ ] Docker deployment works
- [ ] All docs complete

**Current**: 75% → **Target**: 100%

---

Happy coding! 🚀

*Last Updated: 2025-12-01*
