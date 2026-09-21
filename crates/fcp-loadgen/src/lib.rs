//! The load harness: many scripted SIP phones against a running node, and
//! what carrying them costs it (`docs/CAPACITY_AND_SURVIVABILITY.md` §8).
//!
//! A run is a [`profile`] — a register storm, a call ramp, a soak, or a
//! mixture — driven by [`fcp_testkit`]'s `Phone`, the same scripted
//! endpoint with real RTP that the test floor uses, run many at a time.
//! What it measures goes into a [`report::Report`]: what the callers saw
//! (latency percentiles per milestone, outcomes by status code, media
//! loss and jitter per leg) beside what the node said about itself (its
//! `/metrics`, its resident memory and CPU), so the two views of the same
//! minute are in one file.
//!
//! Nothing here runs in production. The crate is `publish = false`, and
//! the numbers a run produces are only as good as the hardware it ran on:
//! CI runs one small profile for behaviour, never for a rate.

pub mod budget;
pub mod profile;
pub mod report;
pub mod run;
pub mod sampler;
pub mod target;

pub use budget::Budget;
pub use profile::Profile;
pub use report::Report;
pub use run::{run, RunOptions};
pub use target::Target;
