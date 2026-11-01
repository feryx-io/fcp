//! Starting and stopping a call's recording (`docs/COMPLIANCE_FOLLOWUPS.md`
//! §8.1).
//!
//! [`CallRecorder::start`] is the only way the call manager starts
//! recording a B2BUA call, whoever asks: the REST route, a flow's
//! `StartRecording` node, or the automatic sources (a user's
//! `record_all_calls`, a number's `recording_enabled`, the node's
//! `enable_recording`). Every start goes through the same steps:
//!
//! 1. the call's media session, from `media_session_id` — a call that is not
//!    anchored cannot be recorded;
//! 2. the tenant's consent plan: the prompt (or synthesised speech) played
//!    to both parties and **awaited**, so the first sample is written after
//!    the announcement ends;
//! 3. a failed announcement under `recording_consent_on_failure = skip`
//!    records nothing: the outcome is `refused`;
//! 4. the recorder, its `call_recordings` row, the call's `recording_path`
//!    and `recording_consent` (which its record carries), the notice tone
//!    when the policy asks for one, and a `recording.started` event.
//!
//! A call is recorded once: a second start while one runs is refused, so a
//! call recorded for several reasons has one recording. The recording stops
//! when the call ends ([`CallRecorder::call_ended`]).

use dashmap::mapref::entry::Entry;
use dashmap::DashMap;
use fcp_b2bua_core::{CallId, CallManager};
use fcp_core::compliance::{
    ConsentPlan, RecordingCounters, NOTICE_TONE_HZ, NOTICE_TONE_MS, RECORDING_CONSENT_TEXT,
};
use fcp_core::recording::{CallRecording, CallRecordingSource};
use fcp_core::{RecordingConsent, RecordingId, UserId};
use fcp_events::{PlatformEvent, RecordingEvent};
use fcp_media::{AudioTarget, MediaManager};
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

/// The longest a consent announcement may take before it counts as failed.
const ANNOUNCEMENT_LIMIT: Duration = Duration::from_secs(120);

/// How long synthesising the fallback sentence may take.
const SYNTHESIS_LIMIT: Duration = Duration::from_secs(10);

/// Why a recording did not start or stop.
#[derive(Debug, thiserror::Error)]
pub enum RecordingError {
    #[error("call not found")]
    CallNotFound,
    #[error("the call has ended")]
    CallEnded,
    #[error("the call's media is not anchored, so it cannot be recorded")]
    NotAnchored,
    #[error("this node has no media manager")]
    NoMedia,
    #[error("the call is already being recorded")]
    AlreadyRecording,
    #[error("the call is not being recorded")]
    NotRecording,
    #[error("the call's recording was not started on demand by this user")]
    NotStartedByUser,
    #[error(
        "the recording announcement could not be played and the tenant's policy does not \
         record without one"
    )]
    ConsentRefused,
    #[error("path must be a relative path without '..'")]
    InvalidPath,
    #[error("the media session could not record: {0}")]
    Media(String),
    #[error("recording not registered, so not kept: {0}")]
    NotRegistered(String),
}

/// A recording that has started.
#[derive(Debug, Clone, PartialEq)]
pub struct StartedRecording {
    pub recording_id: RecordingId,
    /// The file, as its row and the call's record name it.
    pub file: String,
    pub consent: RecordingConsent,
}

/// Where a new recording is written.
#[derive(Debug, Clone)]
pub enum RecordingPath {
    /// `<tenant>/<yyyy>/<mm>/<dd>/<call-id>.<extension>` under the node's
    /// recording directory.
    Dated { extension: &'static str },
    /// A path the API's caller supplied, placed under the tenant's
    /// directory.
    Supplied(String),
}

impl RecordingPath {
    /// The default: a WAV file named for the call.
    pub fn wav() -> Self {
        RecordingPath::Dated { extension: "wav" }
    }
}

/// A recording the recorder holds for a call. `recording` is `None` while
/// its start is still in its consent step.
struct Active {
    recording: Option<RecordingId>,
    source: CallRecordingSource,
    /// The party who asked, for an `on_demand` recording.
    started_by: Option<UserId>,
    session_id: fcp_core::CallId,
    notice: Option<JoinHandle<()>>,
}

/// Starts, stops and ends the recordings of a node's calls.
pub struct CallRecorder {
    call_manager: Arc<CallManager>,
    media_manager: Option<Arc<MediaManager>>,
    storage: Option<Arc<fcp_core::DynStorage>>,
    active: DashMap<CallId, Active>,
}

impl CallRecorder {
    pub fn new(
        call_manager: Arc<CallManager>,
        media_manager: Option<Arc<MediaManager>>,
        storage: Option<Arc<fcp_core::DynStorage>>,
    ) -> Self {
        Self {
            call_manager,
            media_manager,
            storage,
            active: DashMap::new(),
        }
    }

    /// Whether a recording of `call_id` is running or starting.
    pub fn is_recording(&self, call_id: &CallId) -> bool {
        self.active.contains_key(call_id)
    }

    /// Record `call_id` for `source`, after the tenant's consent step.
    pub async fn start(
        &self,
        call_id: &CallId,
        source: CallRecordingSource,
        path: RecordingPath,
    ) -> Result<StartedRecording, RecordingError> {
        self.start_as(call_id, source, path, None).await
    }

    /// Record `call_id` because `user`, one of its parties, asked
    /// (`docs/RECORD_AND_CONTACTS.md` §2). The caller has checked that they
    /// may.
    pub async fn start_on_demand(
        &self,
        call_id: &CallId,
        user: UserId,
    ) -> Result<StartedRecording, RecordingError> {
        self.start_as(
            call_id,
            CallRecordingSource::OnDemand,
            RecordingPath::wav(),
            Some(user),
        )
        .await
    }

    async fn start_as(
        &self,
        call_id: &CallId,
        source: CallRecordingSource,
        path: RecordingPath,
        started_by: Option<UserId>,
    ) -> Result<StartedRecording, RecordingError> {
        let media = self.media_manager.clone().ok_or(RecordingError::NoMedia)?;
        let call = self
            .call_manager
            .get_call(call_id)
            .ok_or(RecordingError::CallNotFound)?;
        if call.state.is_terminal() {
            return Err(RecordingError::CallEnded);
        }
        let session_id = match (call.media_anchored, call.media_session_id) {
            (true, Some(session)) => session,
            _ => return Err(RecordingError::NotAnchored),
        };
        let tenant = call.tenant_id;

        // Claim the call before the first await: a second start while this
        // one announces is refused, and a start abandoned midway releases it.
        match self.active.entry(*call_id) {
            Entry::Occupied(_) => return Err(RecordingError::AlreadyRecording),
            Entry::Vacant(slot) => {
                slot.insert(Active {
                    recording: None,
                    source,
                    started_by,
                    session_id,
                    notice: None,
                });
            }
        }
        let mut claim = Claim {
            active: &self.active,
            call_id: *call_id,
            held: true,
        };

        let relative = match path {
            RecordingPath::Supplied(supplied) => {
                fcp_core::recording::tenant_recording_path(&tenant, &supplied)
                    .ok_or(RecordingError::InvalidPath)?
                    .to_string_lossy()
                    .into_owned()
            }
            RecordingPath::Dated { extension } => {
                dated_path(&media, &tenant, call_id, extension).await
            }
        };

        let plan = ConsentPlan::resolve(
            self.storage
                .as_ref()
                .and_then(|s| s.compliance_store())
                .map(|s| s.as_ref()),
            self.storage
                .as_ref()
                .and_then(|s| s.prompt_store())
                .map(|s| s.as_ref()),
            &tenant,
        )
        .await;
        let announced = plan.announce && announce(&media, &session_id, &plan, call_id).await;

        // The parties may have hung up while they were being told.
        if self
            .call_manager
            .get_call(call_id)
            .is_none_or(|c| c.state.is_terminal())
        {
            return Err(RecordingError::CallEnded);
        }
        let consent = plan.outcome(announced);
        RecordingCounters::global().note(source.as_str(), consent);
        let _ = self
            .call_manager
            .update_call(call_id, |c| c.recording_consent = Some(consent));
        let core_call_id = fcp_core::CallId::from_uuid(*call_id.as_uuid());

        if consent == RecordingConsent::Refused {
            warn!(
                call_id = %call_id,
                tenant_id = %tenant,
                source = source.as_str(),
                "Recording not started: the consent announcement failed and the policy says skip"
            );
            self.publish(PlatformEvent::Recording(RecordingEvent::Failed {
                recording_id: String::new(),
                call_id: core_call_id,
                tenant_id: tenant,
                source: source.as_str().to_string(),
                reason: "consent_refused".to_string(),
                timestamp: SystemTime::now(),
            }));
            return Err(RecordingError::ConsentRefused);
        }

        media
            .start_recording(&session_id, relative.clone())
            .await
            .map_err(|e| RecordingError::Media(e.to_string()))?;
        let file = media.recording_file(&relative);

        // A recording with no row cannot be found by retention, a hold or an
        // erasure: without one, it does not go ahead.
        let mut row =
            CallRecording::starting(tenant, core_call_id.to_string(), source, file.clone());
        row.consent = consent;
        row.node_id = self.call_manager.node_id().map(str::to_string);
        if let Some(store) = self.storage.as_ref().and_then(|s| s.call_recording_store()) {
            if let Err(e) = store.create_call_recording(&row).await {
                let _ = media.stop_recording(&session_id).await;
                let _ = tokio::fs::remove_file(&file).await;
                return Err(RecordingError::NotRegistered(e.to_string()));
            }
        }
        let recorded = file.clone();
        let _ = self
            .call_manager
            .update_call(call_id, |c| c.recording_path = Some(recorded));

        let notice = plan.notice_interval.map(|every| {
            spawn_notice(
                Arc::clone(&self.call_manager),
                Arc::clone(&media),
                *call_id,
                session_id,
                every,
            )
        });
        if let Some(mut active) = self.active.get_mut(call_id) {
            active.recording = Some(row.id);
            active.notice = notice;
        }
        claim.held = false;

        // A call that ended while the recorder started found nothing to stop
        // then: stop it now.
        if self
            .call_manager
            .get_call(call_id)
            .is_none_or(|c| c.state.is_terminal())
        {
            let _ = self.stop(call_id).await;
            return Err(RecordingError::CallEnded);
        }

        info!(
            call_id = %call_id,
            tenant_id = %tenant,
            recording = %row.id,
            source = source.as_str(),
            consent = consent.as_str(),
            path = %file,
            "Call recording started"
        );
        self.publish(PlatformEvent::Recording(RecordingEvent::Started {
            recording_id: row.id.to_string(),
            call_id: core_call_id,
            tenant_id: tenant,
            source: source.as_str().to_string(),
            consent: consent.as_str().to_string(),
            started_by,
            timestamp: SystemTime::now(),
        }));
        Ok(StartedRecording {
            recording_id: row.id,
            file,
            consent,
        })
    }

    /// Stop the call's recording for `user`: only one they started on
    /// demand. Any other recording (the user's own record-all among them)
    /// goes on.
    pub async fn stop_on_demand(
        &self,
        call_id: &CallId,
        user: UserId,
    ) -> Result<(), RecordingError> {
        let theirs = match self.active.get(call_id) {
            None => return Err(RecordingError::NotRecording),
            Some(active) => {
                active.source == CallRecordingSource::OnDemand && active.started_by == Some(user)
            }
        };
        if !theirs {
            return Err(RecordingError::NotStartedByUser);
        }
        self.stop(call_id).await
    }

    /// Stop the call's recording and close its row.
    pub async fn stop(&self, call_id: &CallId) -> Result<(), RecordingError> {
        // A start still announcing is not a recording yet: leave it.
        let taken = self
            .active
            .remove_if(call_id, |_, active| active.recording.is_some());
        let Some((_, active)) = taken else {
            return Err(RecordingError::NotRecording);
        };
        if let Some(notice) = &active.notice {
            notice.abort();
        }
        let media = self.media_manager.as_ref().ok_or(RecordingError::NoMedia)?;
        if let Err(e) = media.stop_recording(&active.session_id).await {
            debug!(call_id = %call_id, error = %e, "The media session had no recording to stop");
        }
        let ended_at = SystemTime::now();
        let Some(recording_id) = active.recording else {
            return Ok(());
        };
        let Some(store) = self.storage.as_ref().and_then(|s| s.call_recording_store()) else {
            return Ok(());
        };
        if let Err(e) = store.finish_call_recording(&recording_id, ended_at).await {
            warn!(call_id = %call_id, recording = %recording_id, error = %e, "Could not close the recording's row");
        }
        if let Ok(Some(row)) = store.get_call_recording(&recording_id).await {
            let size_bytes = tokio::fs::metadata(&row.path)
                .await
                .map(|m| m.len())
                .unwrap_or(0);
            let duration_ms = ended_at
                .duration_since(row.started_at)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);
            info!(call_id = %call_id, recording = %recording_id, duration_ms, size_bytes, "Call recording stopped");
            self.publish(PlatformEvent::Recording(RecordingEvent::Completed {
                recording_id: recording_id.to_string(),
                call_id: fcp_core::CallId::from_uuid(*call_id.as_uuid()),
                tenant_id: row.tenant_id,
                duration_ms,
                size_bytes,
                url: row.path,
                timestamp: ended_at,
            }));
        }
        Ok(())
    }

    /// The call is over: stop its recording, if it has one, before its media
    /// session goes. Quiet when there is nothing to stop.
    pub async fn call_ended(&self, call_id: &CallId) {
        if self.active.contains_key(call_id) {
            match self.stop(call_id).await {
                Ok(()) | Err(RecordingError::NotRecording) => {}
                Err(e) => {
                    warn!(call_id = %call_id, error = %e, "Recording not stopped at the end of the call")
                }
            }
        }
    }

    fn publish(&self, event: PlatformEvent) {
        if let Some(events) = self.call_manager.events() {
            let events = Arc::clone(events);
            tokio::spawn(async move {
                if let Err(e) = events.publish(event).await {
                    warn!(error = %e, "Failed to publish a recording event");
                }
            });
        }
    }
}

/// Releases a claimed call if its start does not finish.
struct Claim<'a> {
    active: &'a DashMap<CallId, Active>,
    call_id: CallId,
    held: bool,
}

impl Drop for Claim<'_> {
    fn drop(&mut self) {
        if self.held {
            self.active
                .remove_if(&self.call_id, |_, active| active.recording.is_none());
        }
    }
}

/// The dated path for a new recording of `call_id`: named for the call, and
/// for the call and a count when an earlier recording of it is there.
async fn dated_path(
    media: &MediaManager,
    tenant: &fcp_core::TenantId,
    call_id: &CallId,
    extension: &str,
) -> String {
    let dir = fcp_core::recording::dated_recording_dir(tenant, SystemTime::now());
    let mut n = 1;
    loop {
        let name = if n == 1 {
            format!("{call_id}.{extension}")
        } else {
            format!("{call_id}-{n}.{extension}")
        };
        let relative = dir.join(name).to_string_lossy().into_owned();
        let taken = tokio::fs::try_exists(media.recording_file(&relative))
            .await
            .unwrap_or(false);
        if !taken {
            return relative;
        }
        n += 1;
    }
}

/// Play the consent announcement to both parties and wait for it to end.
/// True when it was heard to the end.
async fn announce(
    media: &MediaManager,
    session_id: &fcp_core::CallId,
    plan: &ConsentPlan,
    call_id: &CallId,
) -> bool {
    let audio = match &plan.prompt_audio {
        Some(path) => Some(path.to_string_lossy().into_owned()),
        None => match media.tts_service() {
            Some(tts) => {
                match tokio::time::timeout(
                    SYNTHESIS_LIMIT,
                    tts.synthesize(RECORDING_CONSENT_TEXT, None, None),
                )
                .await
                {
                    Ok(Ok(path)) => Some(path.to_string_lossy().into_owned()),
                    Ok(Err(e)) => {
                        warn!(call_id = %call_id, error = %e, "Recording-consent speech not synthesised");
                        None
                    }
                    Err(_) => {
                        warn!(call_id = %call_id, "Recording-consent speech timed out");
                        None
                    }
                }
            }
            None => None,
        },
    };
    let Some(audio) = audio else {
        warn!(call_id = %call_id, "The tenant asks for a recording announcement but there is no prompt audio and no TTS");
        return false;
    };
    let playback = match media
        .play_file(session_id, audio.clone(), AudioTarget::Both)
        .await
    {
        Ok(id) => id,
        Err(e) => {
            warn!(call_id = %call_id, error = %e, "Recording announcement not played");
            return false;
        }
    };
    match tokio::time::timeout(
        ANNOUNCEMENT_LIMIT,
        media.wait_playback_completion(playback.clone()),
    )
    .await
    {
        Ok(Ok(())) => {
            info!(call_id = %call_id, audio = %audio, "Recording announcement played");
            true
        }
        Ok(Err(e)) => {
            warn!(call_id = %call_id, error = %e, "Recording announcement failed");
            false
        }
        Err(_) => {
            let _ = media.stop_playback(session_id, playback).await;
            warn!(call_id = %call_id, "Recording announcement did not finish in time");
            false
        }
    }
}

/// Beep to both parties every `every` while the call lasts.
fn spawn_notice(
    call_manager: Arc<CallManager>,
    media: Arc<MediaManager>,
    call_id: CallId,
    session_id: fcp_core::CallId,
    every: Duration,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval_at(tokio::time::Instant::now() + every, every);
        loop {
            tick.tick().await;
            if call_manager
                .get_call(&call_id)
                .is_none_or(|c| c.state.is_terminal())
            {
                return;
            }
            if let Err(e) = media
                .play_tone(
                    &session_id,
                    NOTICE_TONE_HZ,
                    NOTICE_TONE_MS,
                    0.25,
                    AudioTarget::Both,
                )
                .await
            {
                debug!(call_id = %call_id, error = %e, "Recording notice not played; stopping it");
                return;
            }
        }
    })
}
