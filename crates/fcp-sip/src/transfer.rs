//! Transfers the B2BUA carries out itself (`docs/ENTERPRISE_CALLING.md`
//! §3.2). A phone's REFER names a target; instead of relaying it to the
//! far phone, the node dials the target on the transferred party's behalf
//! — back into itself, so the target is routed as any call is (a user's
//! plan, a trunk, a flow) — and, when it answers, moves the transferred
//! party onto the new leg, reports the outcome to the referrer and drops
//! the referrer's leg. A Refer-To with `Replaces` (an attended transfer)
//! carries the header on that INVITE, and the node's own handling of an
//! INVITE with `Replaces` joins the two calls.

use crate::uac::SipUserAgentClient;
use anyhow::{anyhow, Result};
use fcp_b2bua_core::{B2buaConfig, CallId, CallManager, LegSide};
use sip_dialog::session_timer_manager::SessionTimerManager;
use sip_dialog::{Dialog, DialogManager};
use smol_str::SmolStr;
use std::sync::Arc;
use std::time::Duration;
use tracing::{info, warn};

/// What a Refer-To asks for: the user part to dial, and the dialog an
/// attended transfer replaces (the `Replaces` URI header, decoded).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReferTarget {
    pub user: String,
    pub replaces: Option<String>,
}

/// Parse a Refer-To value: `<sip:1003@host;params?Replaces=…>` or a bare
/// URI. `None` when there is no user part to dial.
pub fn parse_refer_to(raw: &str) -> Option<ReferTarget> {
    let raw = raw.trim();
    let uri = match (raw.find('<'), raw.find('>')) {
        (Some(start), Some(end)) if start < end => &raw[start + 1..end],
        _ => raw.split(';').next().unwrap_or(raw),
    };
    let (uri, headers) = match uri.split_once('?') {
        Some((uri, headers)) => (uri, Some(headers)),
        None => (uri, None),
    };
    let rest = uri
        .strip_prefix("sips:")
        .or_else(|| uri.strip_prefix("sip:"))
        .or_else(|| uri.strip_prefix("tel:"))?;
    let user = match rest.split_once('@') {
        Some((user, _)) => user,
        None if uri.starts_with("tel:") => rest.split(';').next().unwrap_or(rest),
        None => return None,
    };
    let user = percent_decode(user.split(';').next().unwrap_or(user));
    if user.is_empty() {
        return None;
    }
    let replaces = headers.and_then(|headers| {
        headers.split('&').find_map(|pair| {
            let (name, value) = pair.split_once('=')?;
            name.eq_ignore_ascii_case("Replaces")
                .then(|| percent_decode(value))
        })
    });
    Some(ReferTarget { user, replaces })
}

/// `%XX` decoded; anything else as it is.
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// How long the target may take to answer: it rings, and its own plan
/// may forward it.
const TRANSFER_INVITE_TIMEOUT: Duration = Duration::from_secs(90);
/// How long a re-INVITE, a NOTIFY or a BYE in the transfer may take.
const TRANSFER_STEP_TIMEOUT: Duration = Duration::from_secs(10);

/// One transfer in flight.
pub(crate) struct TransferRun {
    pub uac: Arc<SipUserAgentClient>,
    /// What the NOTIFY is signed as.
    pub local_uri: sip_core::SipUri,
    pub call_manager: Arc<CallManager>,
    pub dialog_manager: Arc<DialogManager>,
    pub session_timer_manager: Arc<SessionTimerManager>,
    pub media_manager: Option<Arc<fcp_media::MediaManager>>,
    pub sip_call_id_map: Arc<dashmap::DashMap<String, String>>,
    pub in_flight: Arc<dashmap::DashMap<CallId, ()>>,
    pub b2bua_config: B2buaConfig,
    pub local_addr: String,
    pub call_id: CallId,
    /// The referrer's slot; the transferred party is in the other.
    pub referrer: LegSide,
    /// The referrer's dialog: where the NOTIFY goes, and what is dropped.
    /// `None` for a parked call sent back to its parker, whose slot is
    /// empty: nobody to tell, and a failure ends the call.
    pub referrer_dialog: Option<Dialog>,
    /// Where the new leg is dialed: the target at this node.
    pub target: String,
    /// Extra headers on that INVITE (the call it came from, `Replaces`).
    pub headers: Vec<(String, String)>,
    /// Who the target sees calling: the transferred party.
    pub caller: Option<(Option<String>, sip_core::SipUri)>,
}

/// Why a transfer did not happen: what the referrer is told, and why.
type TransferFailure = (u16, &'static str, String);

impl TransferRun {
    pub async fn run(self) {
        let call_id = self.call_id;
        let outcome = self.transfer().await;
        self.in_flight.remove(&call_id);
        match outcome {
            Ok(()) => {
                info!(call_id = %call_id, target = %self.target, "Transfer completed");
                self.report(200, "OK").await;
                if let Some(referrer) = self.referrer_dialog.clone() {
                    let uac = Arc::clone(&self.uac);
                    // The referrer's own BYE may cross ours; each side
                    // answers 481 to a dialog it no longer holds.
                    tokio::spawn(async move {
                        let _ =
                            tokio::time::timeout(TRANSFER_STEP_TIMEOUT, uac.bye(&referrer)).await;
                    });
                }
            }
            Err((code, reason, why)) if self.referrer_dialog.is_some() => {
                warn!(call_id = %call_id, code, error = %why, "Transfer failed; the referrer keeps the call");
                self.report(code, reason).await;
            }
            Err((code, _, why)) => {
                // Nobody to hand the call back to: it ends.
                warn!(call_id = %call_id, code, error = %why, "Sending the parked call back failed; ending it");
                if let Some(call) = self.call_manager.get_call(&call_id) {
                    let party = match self.referrer.other() {
                        LegSide::A => call.a_leg.dialog.clone(),
                        LegSide::B => call.b_leg.as_ref().and_then(|b| b.dialog.clone()),
                    };
                    if let Some(dialog) = party {
                        let _ = tokio::time::timeout(TRANSFER_STEP_TIMEOUT, self.uac.bye(&dialog))
                            .await;
                        self.dialog_manager.remove(dialog.id());
                    }
                    let _ = self.call_manager.terminate_call_with(
                        &call_id,
                        fcp_cdr::TerminationReason::failed(format!("Park timed out: {why}")),
                    );
                }
            }
        }
    }

    async fn transfer(&self) -> std::result::Result<(), TransferFailure> {
        let gone = || {
            (
                481,
                "Call/Transaction Does Not Exist",
                "call ended".to_string(),
            )
        };
        let call = self.call_manager.get_call(&self.call_id).ok_or_else(gone)?;
        let party = self.referrer.other();
        let party_leg = match party {
            LegSide::A => Some(call.a_leg.clone()),
            LegSide::B => call.b_leg.clone(),
        }
        .ok_or_else(gone)?;
        let mut party_dialog = party_leg.dialog.clone().ok_or_else(gone)?;
        let party_sdp = party_leg
            .sdp
            .clone()
            .or_else(|| {
                (party == LegSide::A)
                    .then(|| {
                        call.original_invite
                            .as_ref()
                            .and_then(crate::body::request_sdp)
                    })
                    .flatten()
            })
            .ok_or_else(|| {
                (
                    488,
                    "Not Acceptable Here",
                    "no SDP for the transferred party".to_string(),
                )
            })?;
        let anchored = call
            .media_anchored
            .then_some(call.media_session_id)
            .flatten()
            .zip(self.media_manager.clone());
        let rtp_port = match &anchored {
            Some((session_id, media_manager)) => Some(
                media_manager
                    .get_session(session_id)
                    .await
                    .map_err(|e| (500, "Server Internal Error", format!("media session: {e}")))?
                    .ports()
                    .0,
            ),
            None => None,
        };

        // The offer: the transferred party's media (the node's own when
        // anchored), sending both ways whatever hold it is in.
        let offer = crate::hold::with_direction(
            &match rtp_port {
                Some(port) => {
                    fcp_media::rewrite_sdp_for_anchoring(&party_sdp, &self.local_addr, port, None)
                }
                None => party_sdp.clone(),
            },
            "sendrecv",
        );

        let mut headers = sip_core::Headers::new();
        for (name, value) in &self.headers {
            headers
                .push(SmolStr::new(name), SmolStr::new(value))
                .map_err(|e| (500, "Server Internal Error", format!("header {name}: {e}")))?;
        }
        // The target sees the transferred party calling. The UAC's identity
        // is shared, as for a forward (`route_cfna`): set, send, clear.
        if let Some((display, uri)) = &self.caller {
            self.uac.inner().set_display_name(display.clone()).await;
            self.uac.inner().set_from_uri(Some(uri.clone())).await;
        }
        let sent = tokio::time::timeout(
            Duration::from_secs(5),
            self.uac
                .invite_with_headers(&self.target, Some(&offer), headers),
        )
        .await;
        if self.caller.is_some() {
            self.uac.inner().set_display_name(None).await;
            self.uac.inner().set_from_uri(None).await;
        }
        let handle = sent
            .map_err(|_| {
                (
                    503,
                    "Service Unavailable",
                    "INVITE could not be sent".to_string(),
                )
            })?
            .map_err(|e| (503, "Service Unavailable", format!("INVITE failed: {e}")))?;
        let handle = Arc::new(handle);
        let final_response = tokio::time::timeout(TRANSFER_INVITE_TIMEOUT, handle.await_final())
            .await
            .map_err(|_| {
                (
                    408,
                    "Request Timeout",
                    "the target did not answer".to_string(),
                )
            })?
            .map_err(|e| (503, "Service Unavailable", format!("INVITE failed: {e}")))?;
        let code = final_response.start_line().code();
        if !(200..300).contains(&code) {
            return Err((
                code,
                reason_for(code),
                format!("the target answered {code}"),
            ));
        }
        let new_dialog = handle.dialog.read().await.clone();
        let answer = String::from_utf8_lossy(final_response.body()).to_string();
        if answer.is_empty() {
            let _ = self.uac.bye(&new_dialog).await;
            return Err((
                488,
                "Not Acceptable Here",
                "the target answered without SDP".to_string(),
            ));
        }
        if self.call_manager.get_call(&self.call_id).is_none() {
            let _ = self.uac.bye(&new_dialog).await;
            return Err(gone());
        }

        // Point the transferred party at the target.
        let mut party_sdp_now = party_sdp.clone();
        match (&anchored, rtp_port) {
            (Some((session_id, media_manager)), Some(port)) => {
                let updated = match self.referrer {
                    LegSide::A => media_manager
                        .update_session(session_id, answer.clone())
                        .await
                        .map(|_| ()),
                    LegSide::B => {
                        media_manager
                            .update_session_b_leg(session_id, answer.clone())
                            .await
                    }
                };
                if let Err(e) = updated {
                    let _ = self.uac.bye(&new_dialog).await;
                    return Err((500, "Server Internal Error", format!("media session: {e}")));
                }
                // Held by the referrer: taken off hold, by the node.
                if call.held == Some(party) {
                    let resume = crate::hold::with_direction(
                        &fcp_media::rewrite_sdp_for_anchoring(
                            &party_sdp,
                            &self.local_addr,
                            port,
                            None,
                        ),
                        "sendrecv",
                    );
                    self.reinvite(&mut party_dialog, &resume).await;
                }
            }
            _ => {
                let offer_to_party = crate::hold::with_direction(&answer, "sendrecv");
                match self.reinvite(&mut party_dialog, &offer_to_party).await {
                    Some(answered) => {
                        if let Some(body) = answered {
                            party_sdp_now = body;
                        }
                    }
                    None => {
                        let _ = self.uac.bye(&new_dialog).await;
                        return Err((
                            500,
                            "Server Internal Error",
                            "the transferred party refused the new media".to_string(),
                        ));
                    }
                }
                // A party that moved its media in its answer: tell the
                // target, which was offered the old endpoint.
                if crate::conference_migration::media_endpoint_changed(&party_sdp, &party_sdp_now) {
                    let mut target_dialog = new_dialog.clone();
                    self.reinvite(&mut target_dialog, &party_sdp_now).await;
                }
            }
        }

        // Swap the referrer's slot for the new leg.
        let referrer = self.referrer;
        let target = self.target.clone();
        let new_dialog_for_call = new_dialog.clone();
        let party_dialog_for_call = party_dialog.clone();
        let party_sdp_for_call = party_sdp_now.clone();
        let swapped = self.call_manager.update_call(&self.call_id, |c| {
            let mut leg = fcp_b2bua_core::CallLeg::new_outbound(target.clone(), None);
            leg.dialog = Some(new_dialog_for_call.clone());
            leg.sdp = Some(answer.clone());
            leg.outbound_handle = Some(Arc::clone(&handle));
            match referrer {
                LegSide::A => {
                    c.a_leg = leg;
                    if let Some(b) = c.b_leg.as_mut() {
                        b.dialog = Some(party_dialog_for_call.clone());
                        b.sdp = Some(party_sdp_for_call.clone());
                    }
                }
                LegSide::B => {
                    c.b_leg = Some(leg);
                    c.a_leg.dialog = Some(party_dialog_for_call.clone());
                    c.a_leg.sdp = Some(party_sdp_for_call.clone());
                }
            }
            c.held = None;
            // The referrer has left the call; the target counts on the
            // call the node dialed it on.
            match referrer {
                LegSide::A => c.caller_user_id = None,
                LegSide::B => c.callee_user_id = None,
            }
            if c.state == fcp_b2bua_core::CallState::Parked {
                c.unpark();
            }
        });
        if swapped.is_err() {
            let _ = self.uac.bye(&new_dialog).await;
            return Err(gone());
        }
        let _ = self.dialog_manager.insert(new_dialog.clone());
        let _ = self.dialog_manager.insert(party_dialog.clone());
        self.sip_call_id_map.insert(
            new_dialog.id().call_id().to_string(),
            self.call_id.to_string(),
        );
        if let Some(referrer) = &self.referrer_dialog {
            self.sip_call_id_map.remove(referrer.id().call_id());
            self.session_timer_manager.stop_timer(referrer.id());
            self.dialog_manager.remove(referrer.id());
        }
        crate::handler::start_session_timer_for(
            &self.session_timer_manager,
            &self.b2bua_config,
            &new_dialog,
        );
        Ok(())
    }

    /// A re-INVITE on `dialog`: `Some(answer body)` on success.
    async fn reinvite(&self, dialog: &mut Dialog, sdp: &str) -> Option<Option<String>> {
        match tokio::time::timeout(
            TRANSFER_STEP_TIMEOUT,
            self.uac.send_reinvite(dialog, Some(sdp)),
        )
        .await
        {
            Ok(Ok(r)) if (200..300).contains(&r.start_line().code()) => {
                let _ = self.dialog_manager.insert(dialog.clone());
                Some((!r.body().is_empty()).then(|| String::from_utf8_lossy(r.body()).to_string()))
            }
            Ok(Ok(r)) => {
                warn!(call_id = %self.call_id, status = r.start_line().code(), "A re-INVITE in the transfer was refused");
                None
            }
            Ok(Err(e)) => {
                warn!(call_id = %self.call_id, error = %e, "A re-INVITE in the transfer failed");
                None
            }
            Err(_) => {
                warn!(call_id = %self.call_id, "A re-INVITE in the transfer timed out");
                None
            }
        }
    }

    /// The REFER's implicit subscription, reported once (RFC 3515 §2.4.5).
    async fn report(&self, code: u16, reason: &str) {
        let Some(mut dialog) = self.referrer_dialog.clone() else {
            return;
        };
        if let Err(e) = crate::conference_migration::report_refer_outcome(
            &self.uac,
            &self.local_uri,
            &mut dialog,
            code,
            reason,
        )
        .await
        {
            warn!(call_id = %self.call_id, error = %e, "The transfer's NOTIFY was not delivered");
        }
    }
}

/// A reason phrase for a code the referrer is told.
fn reason_for(code: u16) -> &'static str {
    match code {
        404 => "Not Found",
        408 => "Request Timeout",
        480 => "Temporarily Unavailable",
        486 => "Busy Here",
        487 => "Request Terminated",
        488 => "Not Acceptable Here",
        503 => "Service Unavailable",
        603 => "Decline",
        _ => "Transfer Failed",
    }
}

/// The dialog a `Replaces` value names, if the node holds it: the call,
/// and which leg (`docs/ENTERPRISE_CALLING.md` §3.2). The tags are matched
/// as a pair in either order: a phone names the dialog as it sees it.
pub fn replaced_leg(
    call_manager: &CallManager,
    replaces: &str,
) -> Result<(fcp_b2bua_core::Call, LegSide, Dialog)> {
    let header =
        sip_core::ReplacesHeader::parse(replaces).map_err(|e| anyhow!("Replaces: {e:?}"))?;
    let call = call_manager
        .find_by_dialog_call_id(header.call_id())
        .ok_or_else(|| anyhow!("no call holds dialog {}", header.call_id()))?;
    let tags_match = |d: &Dialog| {
        let (local, remote) = (d.id().local_tag(), d.id().remote_tag());
        (local == header.to_tag() && remote == header.from_tag())
            || (local == header.from_tag() && remote == header.to_tag())
    };
    if let Some(d) = call.a_leg.dialog.as_ref() {
        if d.id().call_id() == header.call_id() && tags_match(d) {
            let d = d.clone();
            return Ok((call, LegSide::A, d));
        }
    }
    if let Some(d) = call.b_leg.as_ref().and_then(|b| b.dialog.as_ref()) {
        if d.id().call_id() == header.call_id() && tags_match(d) {
            let d = d.clone();
            return Ok((call, LegSide::B, d));
        }
    }
    Err(anyhow!("the tags do not name a leg of that call"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_blind_target_is_its_user() {
        assert_eq!(
            parse_refer_to("<sip:1003@10.0.0.1:5060>"),
            Some(ReferTarget {
                user: "1003".into(),
                replaces: None
            })
        );
        assert_eq!(
            parse_refer_to("sip:*70@pbx;transport=udp").unwrap().user,
            "*70"
        );
        assert_eq!(parse_refer_to("<sip:%2A70@pbx>").unwrap().user, "*70");
        assert_eq!(
            parse_refer_to("<tel:+15551234567>").unwrap().user,
            "+15551234567"
        );
        assert_eq!(parse_refer_to("<sip:pbx.example.com>"), None);
    }

    #[test]
    fn an_attended_target_carries_its_replaces() {
        let t = parse_refer_to(
            "<sip:1003@10.0.0.1?Replaces=abc%40host%3Bto-tag%3D123%3Bfrom-tag%3D456&X-Other=1>",
        )
        .unwrap();
        assert_eq!(t.user, "1003");
        assert_eq!(
            t.replaces.as_deref(),
            Some("abc@host;to-tag=123;from-tag=456")
        );
    }
}
