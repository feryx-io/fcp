//! The record and how a node builds one.
//!
//! [`TerminationReason`] is what the node knows about why a call ended;
//! [`RecordBuilder`] turns it, and everything else the node knows at
//! hangup, into a [`Record`]. Building is pure: no storage, no clock other
//! than the times the caller gives it (and `now` for a missing end).

use std::collections::HashMap;
use std::time::SystemTime;

use fcp_core::storage::{
    CallDetailRecord, CallDisposition, CdrKind, HangupCause, HangupParty, CDR_SCHEMA_VERSION,
};
use fcp_core::{CallId, TenantId};
use serde::{Deserialize, Serialize};

/// Why the platform ended a call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SystemCause {
    /// RFC 4028 session timer ran out.
    SessionExpired,
    /// The tenant's balance covered no more talk time.
    BalanceExhausted,
    /// An operator or the API hung it up.
    Admin,
    /// A parked call waited too long and its return failed.
    ParkExpired,
    /// A participant was moved to another conference node.
    Migration,
    /// The node was draining and the call was still up at the deadline
    /// (`docs/CAPACITY_AND_SURVIVABILITY.md` §6).
    NodeDraining,
    /// Anything else the platform decided.
    Other,
}

impl SystemCause {
    fn hangup_cause(self) -> HangupCause {
        match self {
            SystemCause::SessionExpired => HangupCause::Timeout,
            _ => HangupCause::NormalClearing,
        }
    }
}

/// Why a call ended, as the node that owned it knows.
///
/// The free-text failure strings of the MVP still arrive through
/// [`From<Option<String>>`], which reads a leading SIP status code and
/// nothing else; the sites that know better pass a variant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TerminationReason {
    /// A BYE on an answered call. `by` is `None` when the node could not
    /// tell which party sent it.
    Hangup { by: Option<HangupParty> },
    /// The caller cancelled before the call was answered.
    Cancelled,
    /// The callee side answered the INVITE with a final failure.
    Rejected { status: u16, reason: String },
    /// Nobody answered within the time allowed.
    NoAnswer,
    /// The platform ended the call.
    System { cause: SystemCause, detail: String },
    /// The platform could not set the call up.
    Failed { status: Option<u16>, detail: String },
    /// A feature-code call that did what it was dialed for.
    Completed { detail: String },
}

impl TerminationReason {
    /// A hangup by a known party.
    pub fn hangup_by(party: HangupParty) -> Self {
        TerminationReason::Hangup { by: Some(party) }
    }

    /// A rejection with the status the callee side sent.
    pub fn rejected(status: u16, reason: impl Into<String>) -> Self {
        TerminationReason::Rejected {
            status,
            reason: reason.into(),
        }
    }

    /// The platform ended the call for `cause`.
    pub fn system(cause: SystemCause, detail: impl Into<String>) -> Self {
        TerminationReason::System {
            cause,
            detail: detail.into(),
        }
    }

    /// The platform could not set the call up.
    pub fn failed(detail: impl Into<String>) -> Self {
        TerminationReason::Failed {
            status: None,
            detail: detail.into(),
        }
    }

    /// Whether the call should be marked failed rather than terminated when
    /// it ends for this reason: a setup that never completed, or a platform
    /// decision on a call that was never answered.
    pub fn is_failure(&self, answered: bool) -> bool {
        match self {
            TerminationReason::Hangup { .. } | TerminationReason::Completed { .. } => false,
            TerminationReason::Cancelled
            | TerminationReason::Rejected { .. }
            | TerminationReason::NoAnswer
            | TerminationReason::Failed { .. } => true,
            TerminationReason::System { .. } => !answered,
        }
    }

    /// The text kept on the call as its failure reason, and on the record
    /// as `sip_reason`.
    pub fn detail(&self) -> Option<String> {
        match self {
            TerminationReason::Hangup { .. } => None,
            TerminationReason::Cancelled => Some("Cancelled".to_string()),
            TerminationReason::Rejected { status, reason } => Some(format!("{status} {reason}")),
            TerminationReason::NoAnswer => Some("No answer".to_string()),
            TerminationReason::System { detail, .. }
            | TerminationReason::Failed { detail, .. }
            | TerminationReason::Completed { detail } => Some(detail.clone()),
        }
    }

    /// What the record says about this ending on a call that was or was
    /// not answered.
    pub fn outcome(&self, answered: bool) -> Outcome {
        match self {
            TerminationReason::Hangup { by } => Outcome {
                disposition: if answered {
                    CallDisposition::Answered
                } else {
                    CallDisposition::Cancelled
                },
                cause: HangupCause::NormalClearing,
                party: *by,
                status: None,
            },
            TerminationReason::Cancelled => Outcome {
                disposition: CallDisposition::Cancelled,
                cause: HangupCause::NormalClearing,
                party: Some(HangupParty::Caller),
                status: Some(487),
            },
            TerminationReason::Rejected { status, .. } => classify_status(*status, true),
            TerminationReason::NoAnswer => Outcome {
                disposition: CallDisposition::NoAnswer,
                cause: HangupCause::NoAnswer,
                party: Some(HangupParty::System),
                status: Some(480),
            },
            TerminationReason::System { cause, .. } => Outcome {
                disposition: if answered {
                    CallDisposition::Answered
                } else {
                    CallDisposition::Failed
                },
                cause: cause.hangup_cause(),
                party: Some(HangupParty::System),
                status: None,
            },
            TerminationReason::Failed { status, .. } => match status {
                Some(code) => classify_status(*code, false),
                None => Outcome {
                    disposition: CallDisposition::Failed,
                    cause: HangupCause::InternalError,
                    party: Some(HangupParty::System),
                    status: None,
                },
            },
            TerminationReason::Completed { .. } => Outcome {
                disposition: CallDisposition::Answered,
                cause: HangupCause::NormalClearing,
                party: Some(HangupParty::System),
                status: None,
            },
        }
    }
}

/// What a final SIP status says about a call that never connected.
/// `from_callee` says whether the callee side sent it (a rejection) or the
/// platform produced it (a setup failure).
fn classify_status(status: u16, from_callee: bool) -> Outcome {
    let callee_or_system = if from_callee {
        HangupParty::Callee
    } else {
        HangupParty::System
    };
    let (disposition, cause, party) = match status {
        486 | 600 => (
            CallDisposition::Busy,
            HangupCause::UserBusy,
            callee_or_system,
        ),
        480 | 408 => (
            CallDisposition::NoAnswer,
            HangupCause::NoAnswer,
            callee_or_system,
        ),
        487 => (
            CallDisposition::Cancelled,
            HangupCause::NormalClearing,
            HangupParty::Caller,
        ),
        403 | 603 => (
            CallDisposition::Failed,
            HangupCause::CallRejected,
            callee_or_system,
        ),
        500..=599 => (
            CallDisposition::Failed,
            HangupCause::InternalError,
            HangupParty::System,
        ),
        600..=699 => (
            CallDisposition::Failed,
            HangupCause::CallRejected,
            callee_or_system,
        ),
        _ => (
            CallDisposition::Failed,
            HangupCause::Other(status),
            HangupParty::System,
        ),
    };
    Outcome {
        disposition,
        cause,
        party: Some(party),
        status: Some(status),
    }
}

/// The record's view of an ending.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Outcome {
    pub disposition: CallDisposition,
    pub cause: HangupCause,
    pub party: Option<HangupParty>,
    pub status: Option<u16>,
}

impl From<Option<String>> for TerminationReason {
    /// The MVP's contract: `None` was a normal ending, `Some(text)` a
    /// failure. A leading SIP status code in the text is honoured; nothing
    /// else in it is interpreted.
    fn from(reason: Option<String>) -> Self {
        match reason {
            None => TerminationReason::Hangup { by: None },
            Some(detail) => TerminationReason::Failed {
                status: leading_status_code(&detail),
                detail,
            },
        }
    }
}

impl From<String> for TerminationReason {
    fn from(detail: String) -> Self {
        TerminationReason::from(Some(detail))
    }
}

impl From<&str> for TerminationReason {
    fn from(detail: &str) -> Self {
        TerminationReason::from(Some(detail.to_string()))
    }
}

/// The SIP status a failure text starts with, e.g. `"486 Busy Here"`.
pub fn leading_status_code(text: &str) -> Option<u16> {
    text.split_whitespace()
        .next()
        .and_then(|code| code.parse::<u16>().ok())
        .filter(|code| (100..700).contains(code))
}

/// RTP counters as the media session saw them at the end of the call,
/// totals over both legs.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct RtpStats {
    pub packets_sent: u64,
    pub packets_received: u64,
    pub bytes_sent: u64,
    pub bytes_received: u64,
    pub packets_lost: u64,
    pub jitter_ms: Option<f64>,
}

/// Builds a [`CallDetailRecord`] from what a node knows at the end of a
/// call. Every setter is optional; `build` derives the rest.
#[derive(Debug, Clone)]
pub struct RecordBuilder {
    kind: CdrKind,
    call_id: CallId,
    tenant_id: TenantId,
    sip_call_id: Option<String>,
    from_uri: String,
    from_display_name: Option<String>,
    to_uri: String,
    to_display_name: Option<String>,
    start_time: Option<SystemTime>,
    answer_time: Option<SystemTime>,
    end_time: Option<SystemTime>,
    reason: Option<TerminationReason>,
    media_anchored: bool,
    codec_a_leg: Option<String>,
    codec_b_leg: Option<String>,
    rtp: Option<RtpStats>,
    rtp_packets: Option<(u64, u64)>,
    trunk_name: Option<String>,
    trunk_id: Option<fcp_core::TrunkId>,
    gateway_id: Option<String>,
    route_id: Option<String>,
    node_id: Option<String>,
    region: Option<String>,
    correlation_id: Option<CallId>,
    recording_url: Option<String>,
    recording_consent: Option<fcp_core::RecordingConsent>,
    custom_data: Option<serde_json::Value>,
}

impl RecordBuilder {
    /// A record of `kind` for `call_id` in `tenant_id`.
    pub fn new(kind: CdrKind, call_id: CallId, tenant_id: TenantId) -> Self {
        Self {
            kind,
            call_id,
            tenant_id,
            sip_call_id: None,
            from_uri: String::new(),
            from_display_name: None,
            to_uri: String::new(),
            to_display_name: None,
            start_time: None,
            answer_time: None,
            end_time: None,
            reason: None,
            media_anchored: false,
            codec_a_leg: None,
            codec_b_leg: None,
            rtp: None,
            rtp_packets: None,
            trunk_name: None,
            trunk_id: None,
            gateway_id: None,
            route_id: None,
            node_id: None,
            region: None,
            correlation_id: None,
            recording_url: None,
            recording_consent: None,
            custom_data: None,
        }
    }

    pub fn sip_call_id(mut self, id: impl Into<String>) -> Self {
        self.sip_call_id = Some(id.into());
        self
    }

    pub fn from(mut self, uri: impl Into<String>, display_name: Option<String>) -> Self {
        self.from_uri = uri.into();
        self.from_display_name = display_name;
        self
    }

    pub fn to(mut self, uri: impl Into<String>, display_name: Option<String>) -> Self {
        self.to_uri = uri.into();
        self.to_display_name = display_name;
        self
    }

    pub fn started_at(mut self, at: SystemTime) -> Self {
        self.start_time = Some(at);
        self
    }

    pub fn answered_at(mut self, at: Option<SystemTime>) -> Self {
        self.answer_time = at;
        self
    }

    pub fn ended_at(mut self, at: Option<SystemTime>) -> Self {
        self.end_time = at;
        self
    }

    pub fn reason(mut self, reason: TerminationReason) -> Self {
        self.reason = Some(reason);
        self
    }

    pub fn media_anchored(mut self, anchored: bool) -> Self {
        self.media_anchored = anchored;
        self
    }

    /// The codecs by name, for a caller that knows them.
    pub fn codecs(mut self, a_leg: Option<String>, b_leg: Option<String>) -> Self {
        self.codec_a_leg = a_leg;
        self.codec_b_leg = b_leg;
        self
    }

    /// The codecs from each leg's SDP: the first audio format offered,
    /// parsed with `sip-sdp`.
    pub fn sdp(mut self, a_leg: Option<&str>, b_leg: Option<&str>) -> Self {
        self.codec_a_leg = a_leg.and_then(codec_from_sdp);
        self.codec_b_leg = b_leg.and_then(codec_from_sdp);
        self
    }

    pub fn rtp(mut self, stats: Option<RtpStats>) -> Self {
        self.rtp = stats;
        self
    }

    /// Packet counts alone, `(sent, received)`, for a leg whose media path
    /// counts nothing else (a conference participant).
    pub fn rtp_packets(mut self, sent: u64, received: u64) -> Self {
        self.rtp_packets = Some((sent, received));
        self
    }

    pub fn trunk(mut self, trunk_name: Option<String>) -> Self {
        self.trunk_name = trunk_name;
        self
    }

    /// Which trunk carried the call (`docs/TENANT_TRUNKS.md` §4). Two
    /// tenants may both have a "carrier-a", so the record keeps the id.
    pub fn trunk_id(mut self, trunk_id: Option<fcp_core::TrunkId>) -> Self {
        self.trunk_id = trunk_id;
        self
    }

    pub fn gateway(mut self, gateway_id: Option<String>) -> Self {
        self.gateway_id = gateway_id;
        self
    }

    pub fn route(mut self, route_id: Option<String>) -> Self {
        self.route_id = route_id;
        self
    }

    pub fn node(mut self, node_id: Option<String>, region: Option<String>) -> Self {
        self.node_id = node_id;
        self.region = region;
        self
    }

    pub fn correlation(mut self, call_id: Option<CallId>) -> Self {
        self.correlation_id = call_id;
        self
    }

    pub fn recording(mut self, url: Option<String>) -> Self {
        self.recording_url = url;
        self
    }

    /// What the parties were told about the call's recording.
    pub fn recording_consent(mut self, consent: Option<fcp_core::RecordingConsent>) -> Self {
        self.recording_consent = consent;
        self
    }

    pub fn custom_data(mut self, data: Option<serde_json::Value>) -> Self {
        self.custom_data = data;
        self
    }

    /// The record. A missing start is `now`; a missing end is `now`; a
    /// missing reason is a normal ending by nobody in particular.
    pub fn build(self) -> CallDetailRecord {
        let now = SystemTime::now();
        let start_time = self.start_time.unwrap_or(now);
        let end_time = Some(self.end_time.unwrap_or(now));
        let answered = self.answer_time.is_some();
        let reason = self
            .reason
            .unwrap_or(TerminationReason::Hangup { by: None });
        let outcome = reason.outcome(answered);
        let (setup_duration_ms, call_duration_ms, total_duration_ms) =
            durations(start_time, self.answer_time, end_time);

        CallDetailRecord {
            cdr_id: uuid::Uuid::new_v4(),
            kind: self.kind,
            schema_version: CDR_SCHEMA_VERSION,
            call_id: self.call_id,
            tenant_id: self.tenant_id,
            sip_call_id: self
                .sip_call_id
                .unwrap_or_else(|| format!("unknown-{}", self.call_id)),
            from_uri: self.from_uri,
            from_display_name: self.from_display_name,
            to_uri: self.to_uri,
            to_display_name: self.to_display_name,
            start_time,
            answer_time: self.answer_time,
            end_time,
            setup_duration_ms,
            call_duration_ms,
            total_duration_ms,
            disposition: outcome.disposition,
            hangup_cause: outcome.cause,
            hangup_party: outcome.party,
            sip_status_code: outcome.status,
            sip_reason: reason.detail(),
            direction: None,
            node_id: self.node_id,
            site_id: None,
            region: self.region,
            correlation_id: self.correlation_id,
            trunk_name: self.trunk_name.clone(),
            trunk_id: self.trunk_id,
            route_id: self.route_id,
            gateway_id: self.gateway_id.or(self.trunk_name),
            media_anchored: self.media_anchored,
            codec_a_leg: self.codec_a_leg,
            codec_b_leg: self.codec_b_leg,
            rtp_packets_sent: self
                .rtp
                .map(|s| s.packets_sent)
                .or(self.rtp_packets.map(|p| p.0)),
            rtp_packets_received: self
                .rtp
                .map(|s| s.packets_received)
                .or(self.rtp_packets.map(|p| p.1)),
            rtp_bytes_sent: self.rtp.map(|s| s.bytes_sent),
            rtp_bytes_received: self.rtp.map(|s| s.bytes_received),
            rtp_packets_lost: self.rtp.map(|s| s.packets_lost),
            rtp_jitter_ms: self.rtp.and_then(|s| s.jitter_ms),
            recording_url: self.recording_url,
            recording_consent: self.recording_consent,
            cost: None,
            currency: None,
            rate_plan_id: None,
            matched_prefix: None,
            billable_seconds: None,
            rated_at: None,
            created_at: None,
            custom_data: self.custom_data,
        }
    }
}

/// `(setup, talk, total)` in milliseconds. A clock that went backwards
/// yields zero, not nothing: the durations are always there when the
/// times are.
pub fn durations(
    start: SystemTime,
    answer: Option<SystemTime>,
    end: Option<SystemTime>,
) -> (Option<u64>, Option<u64>, Option<u64>) {
    let millis = |from: SystemTime, to: SystemTime| -> u64 {
        to.duration_since(from).unwrap_or_default().as_millis() as u64
    };
    let setup = answer.map(|a| millis(start, a));
    let talk = answer.and_then(|a| end.map(|e| millis(a, e)));
    let total = end.map(|e| millis(start, e));
    (setup, talk, total)
}

/// The codec of the first audio section of an SDP body: the first format
/// listed, named by its `a=rtpmap` or, for a static payload type, by RFC
/// 3551. `None` for a body that does not parse or has no audio.
pub fn codec_from_sdp(sdp: &str) -> Option<String> {
    let session = sip_sdp::SessionDescription::parse(sdp).ok()?;
    let audio = session
        .media
        .iter()
        .find(|m| m.media_type == sip_sdp::MediaType::Audio)?;
    let rtpmaps: HashMap<u8, String> = audio
        .rtpmaps_iter()
        .map(|m| (m.payload_type, m.encoding_name.to_string()))
        .collect();
    for format in &audio.formats {
        let Ok(pt) = format.parse::<u8>() else {
            continue;
        };
        if let Some(name) = rtpmaps.get(&pt) {
            return Some(name.to_uppercase());
        }
        if let Some(name) = static_payload_name(pt) {
            return Some(name.to_string());
        }
    }
    None
}

/// RFC 3551 §6 static audio payload types.
fn static_payload_name(pt: u8) -> Option<&'static str> {
    Some(match pt {
        0 => "PCMU",
        3 => "GSM",
        4 => "G723",
        8 => "PCMA",
        9 => "G722",
        18 => "G729",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn builder() -> RecordBuilder {
        RecordBuilder::new(CdrKind::Call, CallId::new(), TenantId::new())
            .from("sip:alice@example.com", None)
            .to("sip:bob@example.com", None)
    }

    #[test]
    fn a_hangup_by_the_callee_on_an_answered_call_is_answered_normal_clearing() {
        let start = SystemTime::now() - Duration::from_secs(10);
        let record = builder()
            .started_at(start)
            .answered_at(Some(start + Duration::from_secs(2)))
            .ended_at(Some(start + Duration::from_secs(9)))
            .reason(TerminationReason::hangup_by(HangupParty::Callee))
            .build();
        assert_eq!(record.disposition, CallDisposition::Answered);
        assert_eq!(record.hangup_cause, HangupCause::NormalClearing);
        assert_eq!(record.hangup_party, Some(HangupParty::Callee));
        assert_eq!(record.sip_status_code, None);
        assert_eq!(record.setup_duration_ms, Some(2000));
        assert_eq!(record.call_duration_ms, Some(7000));
        assert_eq!(record.total_duration_ms, Some(9000));
        assert_eq!(record.kind, CdrKind::Call);
        assert_eq!(record.schema_version, CDR_SCHEMA_VERSION);
        assert!(!record.cdr_id.is_nil());
    }

    #[test]
    fn the_reasons_map_to_the_dispositions_the_docs_promise() {
        type Case = (
            TerminationReason,
            bool,
            CallDisposition,
            HangupCause,
            Option<HangupParty>,
            Option<u16>,
        );
        let cases: Vec<Case> = vec![
            (
                TerminationReason::Cancelled,
                false,
                CallDisposition::Cancelled,
                HangupCause::NormalClearing,
                Some(HangupParty::Caller),
                Some(487),
            ),
            (
                TerminationReason::rejected(486, "Busy Here"),
                false,
                CallDisposition::Busy,
                HangupCause::UserBusy,
                Some(HangupParty::Callee),
                Some(486),
            ),
            (
                TerminationReason::rejected(603, "Decline"),
                false,
                CallDisposition::Failed,
                HangupCause::CallRejected,
                Some(HangupParty::Callee),
                Some(603),
            ),
            (
                TerminationReason::rejected(480, "Temporarily Unavailable"),
                false,
                CallDisposition::NoAnswer,
                HangupCause::NoAnswer,
                Some(HangupParty::Callee),
                Some(480),
            ),
            (
                TerminationReason::rejected(503, "Service Unavailable"),
                false,
                CallDisposition::Failed,
                HangupCause::InternalError,
                Some(HangupParty::System),
                Some(503),
            ),
            (
                TerminationReason::rejected(404, "Not Found"),
                false,
                CallDisposition::Failed,
                HangupCause::Other(404),
                Some(HangupParty::System),
                Some(404),
            ),
            (
                TerminationReason::NoAnswer,
                false,
                CallDisposition::NoAnswer,
                HangupCause::NoAnswer,
                Some(HangupParty::System),
                Some(480),
            ),
            (
                TerminationReason::system(SystemCause::SessionExpired, "Session expired"),
                true,
                CallDisposition::Answered,
                HangupCause::Timeout,
                Some(HangupParty::System),
                None,
            ),
            (
                TerminationReason::system(SystemCause::BalanceExhausted, "cut off"),
                true,
                CallDisposition::Answered,
                HangupCause::NormalClearing,
                Some(HangupParty::System),
                None,
            ),
            (
                TerminationReason::system(SystemCause::Admin, "admin"),
                false,
                CallDisposition::Failed,
                HangupCause::NormalClearing,
                Some(HangupParty::System),
                None,
            ),
            (
                TerminationReason::failed("No devices registered"),
                false,
                CallDisposition::Failed,
                HangupCause::InternalError,
                Some(HangupParty::System),
                None,
            ),
            (
                TerminationReason::from(Some("486 Busy Here".to_string())),
                false,
                CallDisposition::Busy,
                HangupCause::UserBusy,
                Some(HangupParty::System),
                Some(486),
            ),
            (
                TerminationReason::from(None),
                true,
                CallDisposition::Answered,
                HangupCause::NormalClearing,
                None,
                None,
            ),
            (
                TerminationReason::from(None),
                false,
                CallDisposition::Cancelled,
                HangupCause::NormalClearing,
                None,
                None,
            ),
            (
                TerminationReason::Completed {
                    detail: "Park completed".into(),
                },
                true,
                CallDisposition::Answered,
                HangupCause::NormalClearing,
                Some(HangupParty::System),
                None,
            ),
        ];
        for (reason, answered, disposition, cause, party, status) in cases {
            let outcome = reason.outcome(answered);
            assert_eq!(outcome.disposition, disposition, "{reason:?}");
            assert_eq!(outcome.cause, cause, "{reason:?}");
            assert_eq!(outcome.party, party, "{reason:?}");
            assert_eq!(outcome.status, status, "{reason:?}");
        }
    }

    #[test]
    fn a_free_text_failure_keeps_its_text_and_only_a_leading_code() {
        let reason = TerminationReason::from(Some("busy line, try later".to_string()));
        assert_eq!(
            reason,
            TerminationReason::Failed {
                status: None,
                detail: "busy line, try later".into()
            }
        );
        assert!(reason.is_failure(true));
        assert_eq!(reason.detail().as_deref(), Some("busy line, try later"));
        assert!(!TerminationReason::from(None).is_failure(true));
        assert!(!TerminationReason::system(SystemCause::Admin, "x").is_failure(true));
        assert!(TerminationReason::system(SystemCause::Admin, "x").is_failure(false));
    }

    #[test]
    fn durations_survive_a_clock_that_went_backwards() {
        let start = SystemTime::now();
        let (setup, talk, total) = durations(
            start,
            Some(start - Duration::from_secs(1)),
            Some(start - Duration::from_secs(2)),
        );
        assert_eq!((setup, talk, total), (Some(0), Some(0), Some(0)));
    }

    #[test]
    fn the_codec_is_the_first_audio_format_named_by_rtpmap_or_the_static_table() {
        let dynamic = "v=0\r\no=- 1 1 IN IP4 10.0.0.1\r\ns=-\r\nc=IN IP4 10.0.0.1\r\nt=0 0\r\nm=audio 4000 RTP/AVP 111 0\r\na=rtpmap:111 opus/48000/2\r\na=rtpmap:0 PCMU/8000\r\n";
        assert_eq!(codec_from_sdp(dynamic).as_deref(), Some("OPUS"));
        let static_only = "v=0\r\no=- 1 1 IN IP4 10.0.0.1\r\ns=-\r\nc=IN IP4 10.0.0.1\r\nt=0 0\r\nm=audio 4000 RTP/AVP 8 0\r\n";
        assert_eq!(codec_from_sdp(static_only).as_deref(), Some("PCMA"));
        let video_first = "v=0\r\no=- 1 1 IN IP4 10.0.0.1\r\ns=-\r\nc=IN IP4 10.0.0.1\r\nt=0 0\r\nm=video 4002 RTP/AVP 96\r\na=rtpmap:96 H264/90000\r\nm=audio 4000 RTP/AVP 9\r\na=rtpmap:9 G722/8000\r\n";
        assert_eq!(codec_from_sdp(video_first).as_deref(), Some("G722"));
        assert_eq!(codec_from_sdp("not sdp"), None);
        let no_audio = "v=0\r\no=- 1 1 IN IP4 10.0.0.1\r\ns=-\r\nc=IN IP4 10.0.0.1\r\nt=0 0\r\nm=video 4002 RTP/AVP 96\r\na=rtpmap:96 H264/90000\r\n";
        assert_eq!(codec_from_sdp(no_audio), None);
    }

    #[test]
    fn the_builder_fills_what_it_is_not_told() {
        let record = builder().build();
        assert!(record.sip_call_id.starts_with("unknown-"));
        assert!(record.end_time.is_some());
        assert_eq!(record.total_duration_ms, Some(0));
        assert_eq!(record.disposition, CallDisposition::Cancelled);
        assert_eq!(record.hangup_party, None);
        assert_eq!(record.cost, None);
        assert_eq!(record.created_at, None);
    }
}
