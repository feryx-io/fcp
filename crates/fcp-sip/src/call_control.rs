//! Live call control from the API (`docs/DEVELOPER_SURFACE.md` phase 8):
//! what the node does when it is asked to hold, resume, transfer, redirect,
//! play into or hang up one of its calls. The node's HTTP API calls the
//! [`crate::handler::SipMessageHandler`] methods built on this; the admin
//! API forwards to the node holding the call.

use dashmap::DashMap;
use fcp_b2bua_core::LegSide;
use fcp_core::TenantId;
use std::time::{Duration, Instant};

/// Why a control did not happen, with the HTTP status it answers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallControlError {
    pub status: u16,
    pub message: String,
}

impl CallControlError {
    pub fn new(status: u16, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }

    /// The call is not on this node (the admin API then asks the next).
    pub fn not_found(call_id: impl std::fmt::Display) -> Self {
        Self::new(404, format!("Call not found: {call_id}"))
    }

    /// The call is not in a state that allows it.
    pub fn conflict(message: impl Into<String>) -> Self {
        Self::new(409, message)
    }

    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new(400, message)
    }
}

impl std::fmt::Display for CallControlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for CallControlError {}

/// A party of a call, as the API names it: `caller` (the A leg) or
/// `callee` (the B leg); `a` and `b` are accepted too.
pub fn parse_party(name: &str) -> Result<LegSide, CallControlError> {
    match name.trim().to_ascii_lowercase().as_str() {
        "caller" | "a" => Ok(LegSide::A),
        "callee" | "b" => Ok(LegSide::B),
        other => Err(CallControlError::bad_request(format!(
            "a party is `caller` or `callee`, not `{other}`"
        ))),
    }
}

/// The request user a redirect's dial-back carries: `flow~<ticket>`.
pub const REDIRECT_PREFIX: &str = "flow~";

/// How long a redirect's ticket waits for its dial-back, which the node
/// sends at once.
const TICKET_TTL: Duration = Duration::from_secs(60);

/// The flows calls are being redirected into: a one-time ticket per
/// redirect, named in the dial-back's request URI, so the node starts that
/// flow for its own dial-back and for nothing an outsider sends.
#[derive(Default)]
pub struct RedirectTickets {
    tickets: DashMap<String, Ticket>,
}

struct Ticket {
    flow_id: String,
    tenant_id: TenantId,
    issued: Instant,
}

/// What a ticket redeems for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Redirect {
    pub flow_id: String,
    pub tenant_id: TenantId,
}

impl RedirectTickets {
    /// A ticket for `flow_id` of `tenant_id`: the user part to dial.
    pub fn issue(&self, flow_id: &str, tenant_id: TenantId) -> String {
        self.tickets
            .retain(|_, ticket| ticket.issued.elapsed() < TICKET_TTL);
        let ticket = uuid::Uuid::new_v4().simple().to_string();
        self.tickets.insert(
            ticket.clone(),
            Ticket {
                flow_id: flow_id.to_string(),
                tenant_id,
                issued: Instant::now(),
            },
        );
        format!("{REDIRECT_PREFIX}{ticket}")
    }

    /// The redirect a dialed user names, once: `None` for any user that is
    /// not a live ticket.
    pub fn redeem(&self, user: &str) -> Option<Redirect> {
        let ticket = user.strip_prefix(REDIRECT_PREFIX)?;
        let (_, ticket) = self.tickets.remove(ticket)?;
        (ticket.issued.elapsed() < TICKET_TTL).then_some(Redirect {
            flow_id: ticket.flow_id,
            tenant_id: ticket.tenant_id,
        })
    }

    /// Forget a ticket whose transfer did not start.
    pub fn cancel(&self, user: &str) {
        if let Some(ticket) = user.strip_prefix(REDIRECT_PREFIX) {
            self.tickets.remove(ticket);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_ticket_redeems_once_and_nothing_else_redeems() {
        let tickets = RedirectTickets::default();
        let tenant = TenantId::new();
        let user = tickets.issue("support", tenant);
        assert!(user.starts_with(REDIRECT_PREFIX));
        assert_eq!(tickets.redeem("flow~guessed"), None);
        assert_eq!(tickets.redeem("1001"), None);
        assert_eq!(
            tickets.redeem(&user),
            Some(Redirect {
                flow_id: "support".into(),
                tenant_id: tenant
            })
        );
        assert_eq!(tickets.redeem(&user), None, "once");
        let other = tickets.issue("sales", tenant);
        tickets.cancel(&other);
        assert_eq!(tickets.redeem(&other), None);
    }

    #[test]
    fn parties_are_named_caller_and_callee() {
        assert_eq!(parse_party("caller"), Ok(LegSide::A));
        assert_eq!(parse_party("B"), Ok(LegSide::B));
        assert_eq!(parse_party("agent").unwrap_err().status, 400);
    }
}
