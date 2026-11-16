//! fcp-webrtc-gw — the WebRTC-to-SIP gateway, as a library so its parts
//! can be tested in-process. The binary in `main.rs` wires them together.
//!
//! - [`session`]: browser sessions — a room join or a call — each owning a
//!   WebRTC peer connection and a SIP leg towards the conference server or
//!   a SIP destination
//! - [`relay`]: the media relay between the two legs (audio in the codec
//!   both sides negotiated, video packets, RTCP feedback)
//! - [`placement`]: which conference node a room join goes to
//! - [`inbound`]: the SIP messages the gateway receives (responses to its
//!   INVITEs, BYE from the far end)
//! - [`http_api`]: the `/v1/rooms` and `/v1/calls` routes
//! - [`drain`]: leaving service — what this node is carrying, and where
//!   its browsers are told to go

pub mod config;
pub mod drain;
pub mod forwarding;
pub mod http_api;
pub mod inbound;
pub mod placement;
pub mod relay;
pub mod session;
pub mod sip_client;
