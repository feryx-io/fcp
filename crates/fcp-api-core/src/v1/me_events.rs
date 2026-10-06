//! A user's own live events (`docs/SOFTPHONE.md` §2.7, phase F4b):
//! `GET /me/events`, a WebSocket carrying their calls, their mailbox,
//! changes to their account and settings, and the tenant's presence while
//! the tenant shows its directory.
//!
//! It authenticates as `/events` does, with the first message, but takes a
//! signed-in user (a session, an app's session or a personal token) and
//! needs no permission: everything on it is the user's own.

use crate::auth::{self, AuthIdentity};
use crate::state::ApiState;
use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    response::IntoResponse,
};
use fcp_core::legal_hold::{self, SubjectIdentifier};
use fcp_core::storage::{CredentialStore, TenantStore, VoicemailStore};
use fcp_core::subject::SubjectMatcher;
use fcp_core::CallId;
use fcp_events::{CallEvent, EventFilter, PlatformEvent, PresenceEvent};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use std::collections::HashSet;
use std::time::Duration;
use tracing::{debug, warn};

/// Which events are a user's: decided per event, remembering the calls it
/// has let through so their later events, which name only the call, follow.
pub struct UserStream {
    user_id: String,
    mailbox_id: Option<String>,
    matcher: SubjectMatcher,
    presence: bool,
    calls: HashSet<CallId>,
}

impl UserStream {
    pub fn new(
        user_id: String,
        mailbox_id: Option<String>,
        matcher: SubjectMatcher,
        presence: bool,
    ) -> Self {
        Self {
            user_id,
            mailbox_id,
            matcher,
            presence,
            calls: HashSet::new(),
        }
    }

    /// Whether `event` goes to the user.
    pub fn admit(&mut self, event: &PlatformEvent) -> bool {
        match event {
            // A call is theirs when they are one of its parties; its later
            // events name only the call.
            PlatformEvent::Call(CallEvent::Initiated {
                call_id, from, to, ..
            }) => {
                if self.matcher.matches(from) || self.matcher.matches(to) {
                    self.calls.insert(*call_id);
                    true
                } else {
                    false
                }
            }
            PlatformEvent::Call(CallEvent::Ended { call_id, .. })
            | PlatformEvent::Call(CallEvent::Failed { call_id, .. }) => self.calls.remove(call_id),
            PlatformEvent::Voicemail(v) => {
                let mailbox = serde_json::to_value(v).ok().and_then(|v| {
                    v.get("mailbox_id")
                        .and_then(|m| m.as_str())
                        .map(String::from)
                });
                mailbox.is_some() && mailbox == self.mailbox_id
            }
            // Their own account: settings changed elsewhere, renamed,
            // suspended. Not a failed sign-in, which names no user.
            PlatformEvent::User(u) => serde_json::to_value(u).ok().is_some_and(|v| {
                v.get("user_id").and_then(|id| id.as_str()) == Some(self.user_id.as_str())
            }),
            PlatformEvent::Presence(
                PresenceEvent::StatusChanged { .. } | PresenceEvent::DialogChanged { .. },
            ) => self.presence,
            other => other.call_id().is_some_and(|c| self.calls.contains(c)),
        }
    }
}

#[derive(Debug, Deserialize)]
struct AuthMessage {
    #[serde(rename = "type")]
    msg_type: String,
    token: Option<String>,
}

/// GET /me/events — the user's live events as a WebSocket. The first
/// message is `{"type":"auth","token":"…"}`; the answer `auth_ok`, then
/// `connected`, then events as `/events` sends them.
#[utoipa::path(
    get,
    path = "/api/v1/me/events",
    tag = "self_service",
    operation_id = "self_service_my_events",
    responses(
        (status = 101, description = "Switching to the user's event WebSocket; authenticate with the first message"),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn my_events(ws: WebSocketUpgrade, State(state): State<ApiState>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle(socket, state))
}

async fn refuse(sender: &mut futures_util::stream::SplitSink<WebSocket, Message>, message: &str) {
    let _ = sender
        .send(Message::Text(
            serde_json::json!({"type": "auth_error", "message": message}).to_string(),
        ))
        .await;
}

async fn handle(socket: WebSocket, state: ApiState) {
    let (mut sender, mut receiver) = socket.split();
    let Some(auth_config) = state.auth_config.clone() else {
        refuse(
            &mut sender,
            "Authentication is off: there is no user to stream for",
        )
        .await;
        return;
    };
    let identity = match tokio::time::timeout(Duration::from_secs(5), receiver.next()).await {
        Ok(Some(Ok(Message::Text(text)))) => match serde_json::from_str::<AuthMessage>(&text) {
            Ok(m) if m.msg_type == "auth" => match m.token {
                Some(token) => auth::validate_token(&token, &auth_config).await,
                None => None,
            },
            _ => None,
        },
        _ => None,
    };
    let (username, tenant_id) = match &identity {
        Some(AuthIdentity::Session {
            username,
            tenant_id: Some(tenant_id),
            ..
        })
        | Some(AuthIdentity::Personal {
            username,
            tenant_id,
            ..
        }) => (username.clone(), *tenant_id),
        Some(_) => {
            refuse(&mut sender, "A signed-in user's token is needed").await;
            return;
        }
        None => {
            refuse(&mut sender, "Invalid or missing authentication token").await;
            return;
        }
    };
    let Some(storage) = state.storage.clone() else {
        refuse(&mut sender, "Storage is not configured").await;
        return;
    };
    let stream = async {
        let user = storage
            .get_user_in_tenant(&username, &tenant_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("no such user"))?;
        let expansion = legal_hold::expand(
            &storage,
            &tenant_id,
            &[SubjectIdentifier::User {
                username: user.username.clone(),
            }],
        )
        .await?;
        let mailbox = storage
            .get_mailbox_by_user(&user.id)
            .await?
            .map(|m| m.id.to_string());
        let presence = storage
            .get_tenant(&tenant_id)
            .await?
            .is_some_and(|t| t.directory.visible_to_users);
        anyhow::Ok(UserStream::new(
            user.id.to_string(),
            mailbox,
            expansion.matcher(),
            presence,
        ))
    }
    .await;
    let mut stream = match stream {
        Ok(s) => s,
        Err(e) => {
            debug!(error = %e, "No event stream for the user");
            refuse(&mut sender, "The user is not there").await;
            return;
        }
    };
    let mut events = match state
        .event_bus
        .subscribe(EventFilter::new().tenant(tenant_id))
        .await
    {
        Ok(events) => events,
        Err(e) => {
            warn!(error = %e, "Could not subscribe a user's event stream");
            refuse(&mut sender, "The event bus is unavailable").await;
            return;
        }
    };
    for message in [
        serde_json::json!({"type": "auth_ok"}),
        serde_json::json!({"type": "connected", "message": "Subscribed to your events"}),
    ] {
        if sender
            .send(Message::Text(message.to_string()))
            .await
            .is_err()
        {
            return;
        }
    }

    let mut recv = tokio::spawn(async move {
        while let Some(Ok(message)) = receiver.next().await {
            if matches!(message, Message::Close(_)) {
                break;
            }
        }
    });
    let mut send = tokio::spawn(async move {
        while let Some(event) = events.next().await {
            if !stream.admit(&event) {
                continue;
            }
            let Ok(json) = serde_json::to_string(&event) else {
                continue;
            };
            if sender.send(Message::Text(json)).await.is_err() {
                break;
            }
        }
    });
    tokio::select! {
        _ = &mut send => recv.abort(),
        _ = &mut recv => send.abort(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fcp_core::TenantId;
    use std::time::SystemTime;

    fn initiated(call: CallId, from: &str, to: &str) -> PlatformEvent {
        PlatformEvent::Call(CallEvent::Initiated {
            call_id: call,
            tenant_id: TenantId::nil(),
            from: from.into(),
            to: to.into(),
            direction: fcp_events::CallDirection::Inbound,
            timestamp: SystemTime::now(),
            trunk_name: None,
            trunk_id: None,
            node_id: None,
        })
    }

    fn ringing(call: CallId) -> PlatformEvent {
        PlatformEvent::Call(CallEvent::Ringing {
            tenant_id: None,
            call_id: call,
            timestamp: SystemTime::now(),
            node_id: None,
        })
    }

    fn ended(call: CallId) -> PlatformEvent {
        PlatformEvent::Call(CallEvent::Ended {
            tenant_id: None,
            call_id: call,
            end_time: SystemTime::now(),
            duration_ms: 0,
            hangup_cause: "normal".into(),
            hangup_party: None,
            trunk_name: None,
            trunk_id: None,
            node_id: None,
        })
    }

    #[test]
    fn a_users_calls_follow_from_their_start_to_their_end() {
        let mut s = UserStream::new(
            "u1".into(),
            None,
            SubjectMatcher::new(["carol", "carol@a.test", "2101"]),
            false,
        );
        let (mine, theirs) = (CallId::new(), CallId::new());
        assert!(s.admit(&initiated(mine, "sip:dan@a.test", "sip:2101@a.test")));
        assert!(!s.admit(&initiated(theirs, "sip:dan@a.test", "sip:erin@a.test")));
        assert!(s.admit(&ringing(mine)));
        assert!(!s.admit(&ringing(theirs)));
        assert!(s.admit(&ended(mine)));
        // Forgotten once ended.
        assert!(!s.admit(&ringing(mine)));
    }

    #[test]
    fn their_mailbox_account_and_presence_reach_them() {
        let mut s = UserStream::new(
            "u1".into(),
            Some("mb1".into()),
            SubjectMatcher::new(["carol"]),
            true,
        );
        let mwi = |mailbox: &str| {
            PlatformEvent::Voicemail(fcp_events::VoicemailEvent::MailboxDeleted {
                tenant_id: None,
                mailbox_id: mailbox.into(),
                timestamp: SystemTime::now(),
            })
        };
        assert!(s.admit(&mwi("mb1")));
        assert!(!s.admit(&mwi("mb2")));
        let calling = |user: &str| {
            PlatformEvent::User(fcp_events::UserEvent::CallingChanged {
                user_id: user.into(),
                username: "x".into(),
                tenant_id: TenantId::nil(),
                timestamp: SystemTime::now(),
            })
        };
        assert!(s.admit(&calling("u1")));
        assert!(!s.admit(&calling("u2")));
        let presence = PlatformEvent::Presence(PresenceEvent::StatusChanged {
            aor: "dan@a.test".into(),
            tenant_id: TenantId::nil(),
            old_state: "closed".into(),
            new_state: "open".into(),
            timestamp: SystemTime::now(),
        });
        assert!(s.admit(&presence));
        let mut hidden = UserStream::new("u1".into(), None, SubjectMatcher::new(["carol"]), false);
        assert!(!hidden.admit(&presence));
    }
}
