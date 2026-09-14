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

//! - [`phone`] is a scripted SIP endpoint with a media port — a UAC and a
//!   UAS on one dialog manager over an ephemeral loopback socket, an RTP
//!   pair that plays and hears tones and DTMF — for driving the binaries
//!   over real SIP and RTP (`docs/TEST_FLOOR.md`). [`seed`] writes the
//!   users, extensions and rooms such a test needs; [`bins`] finds another
//!   crate's binary.

pub mod bins;
pub mod certs;
pub mod db;
pub mod media;
pub mod phone;
pub mod seed;
pub mod service;
pub mod smoke;

pub use db::TestDatabase;
pub use phone::{Call, IncomingCall, Phone, PhoneEvent};
pub use seed::Seed;
pub use service::Service;
pub use smoke::{free_port, Smoke, SmokeReport};
