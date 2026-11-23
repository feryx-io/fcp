# Contributing to FCP

Thank you for your interest in contributing to the Ferrous Communications Platform! This document provides guidelines and information for contributors.

---

## Table of Contents

1. [Code of Conduct](#code-of-conduct)
2. [Getting Started](#getting-started)
3. [Development Workflow](#development-workflow)
4. [Pull Request Process](#pull-request-process)
5. [Coding Standards](#coding-standards)
6. [Testing Guidelines](#testing-guidelines)
7. [Documentation](#documentation)
8. [Community](#community)

---

## Code of Conduct

### Our Pledge

We pledge to make participation in our project a harassment-free experience for everyone, regardless of age, body size, disability, ethnicity, gender identity and expression, level of experience, nationality, personal appearance, race, religion, or sexual identity and orientation.

### Our Standards

**Positive behavior includes:**
- Using welcoming and inclusive language
- Being respectful of differing viewpoints
- Gracefully accepting constructive criticism
- Focusing on what is best for the community
- Showing empathy towards other community members

**Unacceptable behavior includes:**
- Trolling, insulting/derogatory comments, and personal or political attacks
- Public or private harassment
- Publishing others' private information without explicit permission
- Other conduct which could reasonably be considered inappropriate in a professional setting

### Enforcement

Instances of abusive, harassing, or otherwise unacceptable behavior may be reported to the project team at conduct@example.com. All complaints will be reviewed and investigated promptly and fairly.

---

## Getting Started

### Prerequisites

- **Rust 1.74 or later**: Install from [rust-lang.org](https://www.rust-lang.org/)
- **Git**: For version control
- **Docker** (optional): For containerized development
- **SQLite, PostgreSQL, MySQL, or MongoDB**: For storage backend

### Fork and Clone

1. Fork the repository on GitHub
2. Clone your fork locally:

```bash
git clone --recursive https://github.com/YOUR_USERNAME/fcp.git
cd fcp
```

3. Add upstream remote:

```bash
git remote add upstream https://github.com/feryx-io/fcp.git
```

4. Create a branch:

```bash
git checkout -b feature/your-feature-name
```

### Building

```bash
# Build all crates
cargo build

# Build in release mode
cargo build --release

# Build specific crate
cargo build -p fcp-server
```

### Running Tests

```bash
# Run all tests
cargo test --all

# Run tests for specific crate
cargo test -p fcp-core

# Run with output
cargo test -- --nocapture
```

---

## Development Workflow

### 1. Check for Existing Issues

Before starting work, check if there's an existing issue:
- Browse [open issues](https://github.com/feryx-io/fcp/issues)
- If none exists, create one describing the problem or feature
- Wait for maintainer feedback before starting major work

### 2. Branch Naming

Use descriptive branch names:
- `feature/add-voicemail-transcription`
- `fix/registration-memory-leak`
- `docs/update-api-reference`
- `refactor/simplify-routing-engine`

### 3. Commit Messages

Follow conventional commits format:

```
<type>(<scope>): <short summary>

<detailed description>

<footer>
```

**Types:**
- `feat`: New feature
- `fix`: Bug fix
- `docs`: Documentation only
- `style`: Code style changes (formatting, etc.)
- `refactor`: Code refactoring
- `test`: Adding or updating tests
- `chore`: Maintenance tasks

**Examples:**
```
feat(voicemail): add transcription support

Implements Deepgram integration for voicemail transcription.
Adds configuration options and REST API endpoints.

Closes #123
```

```
fix(b2bua): resolve To-tag mismatch in parallel forking

Reuses To-tag from provisional responses in 200 OK to ensure
RFC 3261 compliance and prevent dialog establishment failures.

Fixes #456
```

### 4. Keep Your Fork Updated

```bash
# Fetch upstream changes
git fetch upstream

# Merge into your branch
git merge upstream/main

# Or rebase
git rebase upstream/main
```

---

## Pull Request Process

### Before Submitting

1. **Run tests**: Ensure all tests pass
   ```bash
   cargo test --all
   ```

2. **Run clippy**: Fix all clippy warnings
   ```bash
   cargo clippy --all -- -D warnings
   ```

3. **Format code**: Use rustfmt
   ```bash
   cargo fmt --all
   ```

4. **Update documentation**: Add/update docs for new features
5. **Add tests**: Include tests for new functionality
6. **Update CHANGELOG**: Add entry for notable changes

### Submitting

1. Push your branch to your fork:
   ```bash
   git push origin feature/your-feature-name
   ```

2. Open a Pull Request on GitHub

3. Fill out the PR template:
   - Describe the changes
   - Link related issues
   - List any breaking changes
   - Add screenshots/examples if applicable

### PR Review Process

1. **Automated Checks**: CI will run tests and linters
2. **Maintainer Review**: A maintainer will review your code
3. **Revisions**: Address any requested changes
4. **Approval**: Once approved, a maintainer will merge

### PR Requirements

- All tests must pass
- Code coverage should not decrease
- No clippy warnings
- Properly formatted with rustfmt
- Documentation updated
- Commits squashed if requested

---

## Coding Standards

### Rust Style Guide

Follow the [Rust Style Guide](https://doc.rust-lang.org/stable/style-guide/):

- Use `rustfmt` with default settings
- Maximum line length: 100 characters
- Use explicit type annotations where it improves readability
- Prefer explicit error types over `anyhow` in library crates
- Use `tracing` for logging, not `log` crate

### Error Handling

**Library Crates:**
```rust
use fcp_core::{Error, Result};

pub fn parse_sip_uri(input: &str) -> Result<Uri> {
    // Return explicit Error enum variants
    Err(Error::Parse(ParseError::InvalidUri(input.to_string())))
}
```

**Application Crates:**
```rust
use anyhow::{Context, Result};

async fn main() -> Result<()> {
    let uri = parse_sip_uri(input)
        .context("Failed to parse SIP URI")?;
    Ok(())
}
```

### Naming Conventions

- **Crates**: `fcp-routing`, `fcp-media`
- **Modules**: `snake_case`
- **Types**: `PascalCase`
- **Functions**: `snake_case`
- **Constants**: `SCREAMING_SNAKE_CASE`
- **Type Parameters**: Single uppercase letter or `PascalCase`

### Documentation

- Public items must have doc comments
- Use `///` for item documentation
- Use `//!` for module documentation
- Include examples in doc comments
- Document panics, errors, and safety

**Example:**
```rust
/// Parses a SIP URI from a string.
///
/// # Arguments
///
/// * `input` - The string to parse
///
/// # Returns
///
/// Returns a `Uri` on success, or an error if the input is invalid.
///
/// # Errors
///
/// Returns `Error::Parse` if the input is not a valid SIP URI.
///
/// # Examples
///
/// ```
/// use fcp_core::Uri;
///
/// let uri = Uri::parse("sip:alice@example.com")?;
/// assert_eq!(uri.user(), Some("alice"));
/// ```
pub fn parse(input: &str) -> Result<Uri> {
    // Implementation
}
```

### Async/Await

- Use `tokio` runtime for all async code
- Prefer `async fn` over `impl Future`
- Use `#[tokio::test]` for async tests
- Avoid blocking operations in async contexts
- Use `spawn_blocking` for CPU-intensive work

---

## Testing Guidelines

### Unit Tests

Place unit tests in the same file as the code:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_uri_parsing() {
        let uri = Uri::parse("sip:alice@example.com").unwrap();
        assert_eq!(uri.user(), Some("alice"));
    }

    #[tokio::test]
    async fn test_async_operation() {
        let result = async_operation().await;
        assert!(result.is_ok());
    }
}
```

### Integration Tests

Place integration tests in `tests/` directory:

```rust
// tests/sip_registration.rs
use fcp_server::Server;

#[tokio::test]
async fn test_sip_registration() {
    let server = Server::new(config).await.unwrap();
    // Test registration flow
}
```

### Test Coverage

- Aim for >80% code coverage
- Test happy paths and error cases
- Use property-based testing where appropriate
- Mock external dependencies

### Performance Tests

- Use `criterion` for benchmarks
- Place benchmarks in `benches/` directory
- Run before submitting performance changes

---

## Documentation

### Types of Documentation

1. **API Documentation**: Doc comments in code
2. **User Guide**: Markdown files in `docs/`
3. **Architecture Docs**: Design documents
4. **Examples**: Working code examples

### Documentation Standards

- Use clear, concise language
- Include code examples
- Explain *why*, not just *what*
- Keep documentation up-to-date
- Link to relevant RFCs and specifications

### Building Documentation

```bash
# Build API docs
cargo doc --no-deps --open

# Check for broken links
cargo doc --no-deps 2>&1 | grep warning
```

---

## Community

### Communication Channels

- **GitHub Issues**: Bug reports and feature requests
- **GitHub Discussions**: General questions and ideas
- **Discord**: Real-time chat (https://discord.gg/fcp)
- **Email**: security@example.com (security issues only)

### Getting Help

- Check the [documentation](https://fcp-docs.example.com)
- Search [existing issues](https://github.com/feryx-io/fcp/issues)
- Ask in [GitHub Discussions](https://github.com/feryx-io/fcp/discussions)
- Join our [Discord server](https://discord.gg/fcp)

### Recognition

Contributors are recognized in:
- CONTRIBUTORS.md file
- Release notes
- Project README

---

## Project Structure

```
fcp/
├── crates/           # Rust crates
│   ├── fcp-core/    # Core types and traits
│   ├── fcp-sip/     # SIP integration
│   ├── fcp-media/   # Media handling
│   └── ...
├── external/
│   └── siphon-rs/   # SIP stack (submodule)
├── docs/            # Documentation
├── tests/           # Integration tests
├── deploy/          # Deployment configs
└── config/          # Configuration examples
```

---

## License

By contributing to FCP, you agree that your contributions will be licensed under the same license as the project (MIT OR Apache-2.0).

---

## Questions?

If you have questions about contributing, please:
1. Check this guide
2. Search existing issues
3. Ask in GitHub Discussions
4. Join our Discord

Thank you for contributing to FCP! 🦀
