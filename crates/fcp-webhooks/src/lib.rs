//! General-purpose webhook dispatcher for FCP platform events.
//!
//! This crate provides a webhook delivery system that subscribes to the FCP event bus,
//! matches events against registered endpoint filters, signs payloads with HMAC-SHA256,
//! and delivers them via HTTP POST with exponential-backoff retries.

pub mod consumer;
pub mod dispatcher;
pub mod payload;
pub mod target;

pub use consumer::spawn_webhook_consumer;
pub use dispatcher::{DeliveryOutcome, WebhookDispatcher, WebhookDispatcherConfig};
pub use payload::{sign, signature_header, WebhookPayload, SIGNATURE_HEADER};
