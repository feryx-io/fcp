//! Test-support helpers shared by FCP crates.
//!
//! Nothing here is used at runtime; binaries and libraries pull this crate
//! in as a dev-dependency only.
//!
//! - [`smoke`] drives a built binary as a subprocess: start it on free
//!   ports, wait for its health endpoint, send SIGTERM, and check that it
//!   exits cleanly. Each binary crate has a `tests/smoke.rs` built on it.
//! - [`db`] creates and drops throwaway PostgreSQL databases, gated on
//!   `FCP_TEST_DATABASE_URL` exactly like the storage and API test suites.

pub mod db;
pub mod smoke;

pub use db::TestDatabase;
pub use smoke::{free_port, Smoke, SmokeReport};
