//! A call's quality as a phone measured it (`docs/CALL_DIAGNOSTICS.md` §2):
//! the final figures an app sends when a call ends — packets, loss, jitter,
//! the round trip, what the far end reported over RTCP and an estimated
//! score (E-model MOS) each way — kept beside the call's record, one report
//! per phone on the call.

use crate::{CallId, TenantId, UserId};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::time::SystemTime;

/// A call scored under this, either way, is poor: `call.quality_poor`.
pub const POOR_MOS: f32 = 3.1;

/// What one phone reported of one call.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CallQuality {
    pub tenant_id: TenantId,
    pub call_id: CallId,
    pub user_id: UserId,
    /// The phone, when the report came from an app's session.
    pub device_id: Option<String>,
    /// `incoming` or `outgoing`, from the phone's side.
    pub direction: String,
    pub codec: String,
    pub srtp: bool,
    /// Talk time, as the phone counted it.
    pub duration_seconds: Option<u32>,
    pub packets_sent: u64,
    pub packets_received: u64,
    /// The far end's packets its sequence numbers say were sent, and those
    /// that never arrived.
    pub packets_expected: u64,
    pub packets_lost: u64,
    /// `packets_lost` of `packets_expected`, in percent.
    pub loss_pct: f32,
    /// The phone's jitter estimate on what it received, at the end.
    pub jitter_ms: f32,
    /// The round trip from RTCP, when it was measured.
    pub rtt_ms: Option<f32>,
    /// What the far end last reported of the phone's audio, when it did:
    /// its loss in its last interval, its jitter, its losses in all.
    pub far_loss_pct: Option<f32>,
    pub far_jitter_ms: Option<f32>,
    pub far_lost: Option<i64>,
    /// The estimated score of what the user heard, and of what the far end
    /// heard once it reported.
    pub mos: f32,
    pub far_mos: Option<f32>,
    /// Where media flowed between, as the phone saw it.
    pub local_rtp: Option<String>,
    pub remote_rtp: Option<String>,
    pub reported_at: SystemTime,
}

impl CallQuality {
    /// The lower of the two scores: how the call went.
    pub fn worst_mos(&self) -> f32 {
        self.far_mos.map_or(self.mos, |far| far.min(self.mos))
    }

    pub fn is_poor(&self) -> bool {
        self.worst_mos() < POOR_MOS
    }
}

#[async_trait]
pub trait CallQualityStore: Send + Sync {
    /// Keep a report; a phone's second report of the same call replaces
    /// its first.
    async fn put_call_quality(&self, report: &CallQuality) -> anyhow::Result<()>;
    /// Every phone's report of a call.
    async fn call_quality(
        &self,
        tenant: &TenantId,
        call: &CallId,
    ) -> anyhow::Result<Vec<CallQuality>>;
    /// A user's own reports of these calls.
    async fn call_qualities_of_user(
        &self,
        user: &UserId,
        calls: &[CallId],
    ) -> anyhow::Result<Vec<CallQuality>>;
}
