//! Flows that listen (`docs/DEVELOPER_SURFACE.md` phase 7): a Gather
//! waiting for digits, speech or either, and a call's live transcript.
//!
//! The caller is the session's A leg (the party the flow answered); a
//! `Dial` adds the callee as its B leg.

use fcp_b2bua_core::CallId;
use fcp_events::{CallEvent, EventPublisher, PlatformEvent};
use fcp_flow::schema::TranscribeLeg;
use fcp_flow::SpeechGather;
use fcp_media::stt::{self, SttOptions, SttStream, Transcript};
use fcp_media::{CollectionResult, DtmfCollector, MediaLeg, MediaManager};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::Instant;
use tracing::{debug, info, warn};

/// How long an utterance that has begun may run before the Gather gives
/// up on it: the endpointer's longest utterance and a margin for the
/// recognizer to answer.
const UTTERANCE_GRACE: Duration = Duration::from_secs(35);

/// Voiced 20 ms frames in a row that count as the caller starting to
/// speak, for barge-in: 200 ms, so a click or a cough does not stop the
/// prompt.
const ONSET_FRAMES: usize = 10;

/// After the end-of-utterance silence, how long a recognizer may take to
/// answer before the caller is taken to have said nothing (a cough heard
/// as speech, recognised as no words).
const RECOGNITION_GRACE: Duration = Duration::from_secs(3);

/// The digit collector's own limit, which never comes: a Gather's deadline
/// runs from its prompt's end, and is kept by [`Listening::wait`].
const UNTIMED: Duration = Duration::from_secs(24 * 60 * 60);

/// What the watch on the caller's leg hears.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Voice {
    /// The caller began to speak.
    Began,
    /// The caller has been quiet long enough that their utterance, if it
    /// was one, should have been recognised.
    Quiet,
}

/// How a Gather ended.
#[derive(Debug)]
pub(crate) enum GatherOutcome {
    /// Digits, collection complete.
    Digits(String),
    /// Digits pressed before the timeout, collection incomplete.
    PartialDigits(String),
    /// What the caller said.
    Speech(Transcript),
    /// Nothing, or nothing usable, before the timeout.
    Timeout,
    /// The collection was cancelled (the call ended).
    Cancelled,
    /// Speech was asked for alone and cannot be listened for.
    SpeechUnavailable,
}

/// What one Gather listens with, set up before its prompt plays so input
/// during the prompt is heard.
pub(crate) struct Listening {
    collector: Option<DtmfCollector>,
    speech: Option<SttStream>,
    onset: Option<mpsc::Receiver<Voice>>,
    speech_unavailable: bool,
}

impl Listening {
    /// Start listening on `session`: a digit collector when digits are
    /// asked for, a recognition of the caller's leg when speech is. When
    /// `barge_in` is off nothing heard before [`Listening::wait`] counts,
    /// so the recognition starts there instead.
    pub(crate) fn start(
        media: &MediaManager,
        session: &fcp_core::CallId,
        num_digits: usize,
        finish_on_key: Option<char>,
        speech: &SpeechGather,
    ) -> Self {
        let collector = speech
            .input
            .digits()
            .then(|| media.create_dtmf_collector(session, num_digits, finish_on_key));
        Self {
            collector,
            speech: None,
            onset: None,
            speech_unavailable: false,
        }
    }

    /// Where a digit that arrives some other way (SIP INFO) is fed in.
    pub(crate) fn digit_sender(&self) -> Option<mpsc::UnboundedSender<char>> {
        self.collector.as_ref().map(DtmfCollector::get_sender)
    }

    /// Begin recognising the caller's speech, and watching for its onset:
    /// a caller who has begun to speak is waited for past the timeout, and
    /// stops a prompt playing over them.
    pub(crate) async fn listen_for_speech(
        &mut self,
        media: &MediaManager,
        session: &fcp_core::CallId,
        speech: &SpeechGather,
    ) {
        if !speech.input.speech() || self.speech.is_some() {
            return;
        }
        let options = SttOptions {
            language: speech.language.clone(),
            hints: speech.hints.clone(),
            // Partials tell a streaming recognizer's caller has begun.
            interim_results: true,
            speech_timeout_ms: speech.speech_timeout_ms,
        };
        match media.recognize(session, MediaLeg::A, &options).await {
            Ok(stream) => self.speech = Some(stream),
            Err(e) => {
                warn!(session = %session, error = %e, "A Gather cannot listen for speech");
                self.speech_unavailable = true;
                return;
            }
        }
        let quiet_after = Duration::from_millis(u64::from(
            speech
                .speech_timeout_ms
                .unwrap_or(stt::DEFAULT_SPEECH_TIMEOUT_MS),
        )) + RECOGNITION_GRACE;
        match media.listen(session, MediaLeg::A).await {
            Ok(frames) => self.onset = Some(watch_voice(frames, quiet_after)),
            Err(e) => debug!(session = %session, error = %e, "No watch for the onset of speech"),
        }
    }

    /// Wait for input, stopping `prompt` (a playback on `session`) as soon
    /// as the caller presses a digit or begins to speak. `timeout` is how
    /// long the caller has to begin once the prompt is over — it used to
    /// run from the prompt's start, so a menu whose prompt was as long as
    /// its timeout said goodbye as it finished asking (`docs/DEEP_TESTING.md`
    /// D8). Once they have begun speaking, their utterance is waited for.
    pub(crate) async fn wait(
        mut self,
        media: &MediaManager,
        session: &fcp_core::CallId,
        timeout: Duration,
        mut prompt: Option<String>,
    ) -> GatherOutcome {
        if self.collector.is_none() && self.speech.is_none() {
            if let Some(pid) = prompt.take() {
                let _ = media.stop_playback(session, pid).await;
            }
            return if self.speech_unavailable {
                GatherOutcome::SpeechUnavailable
            } else {
                GatherOutcome::Timeout
            };
        }
        let collector = self.collector.take();
        // The collector keeps no time: the deadline below does, from the
        // prompt's end.
        let digits = async {
            match &collector {
                Some(collector) => collector.collect(UNTIMED).await,
                None => std::future::pending().await,
            }
        };
        tokio::pin!(digits);
        let mut digits_done = collector.is_none();
        let mut speech = self.speech.take();
        let mut onset = self.onset.take();
        let playing = prompt.clone();
        let prompt_done = async {
            match &playing {
                Some(pid) => {
                    let _ = media.wait_playback_completion(pid.clone()).await;
                }
                None => std::future::pending().await,
            }
        };
        tokio::pin!(prompt_done);
        let mut begin_by = prompt.is_none().then(|| Instant::now() + timeout);
        let mut speaking = false;

        let outcome = loop {
            let deadline = begin_by.map(|at| if speaking { at + UTTERANCE_GRACE } else { at });
            tokio::select! {
                result = &mut digits, if !digits_done => match result {
                    CollectionResult::Complete(d) => break GatherOutcome::Digits(d),
                    CollectionResult::Timeout(d) if !d.is_empty() => {
                        break GatherOutcome::PartialDigits(d)
                    }
                    CollectionResult::Timeout(_) => {
                        digits_done = true;
                        if speech.is_none() || !speaking {
                            break GatherOutcome::Timeout;
                        }
                    }
                    CollectionResult::Cancelled => break GatherOutcome::Cancelled,
                },
                () = &mut prompt_done, if begin_by.is_none() => {
                    prompt = None;
                    begin_by = Some(Instant::now() + timeout);
                }
                _ = sleep_until_some(deadline) => {
                    let pressed = match &collector {
                        Some(collector) if !digits_done => {
                            collector.cancel().await;
                            collector.get_digits().await
                        }
                        _ => String::new(),
                    };
                    break if pressed.is_empty() {
                        GatherOutcome::Timeout
                    } else {
                        GatherOutcome::PartialDigits(pressed)
                    };
                }
                transcript = next_transcript(&mut speech) => match transcript {
                    // An utterance recognised as no words: not speech.
                    Some(t) if t.text.trim().is_empty() => {
                        if t.is_final {
                            speaking = false;
                        }
                    }
                    Some(t) if t.is_final => break GatherOutcome::Speech(t),
                    Some(t) => {
                        debug!(partial = %t.text, "The caller is speaking");
                        speaking = true;
                        if let Some(pid) = prompt.take() {
                            let _ = media.stop_playback(session, pid).await;
                        }
                        begin_by.get_or_insert_with(|| Instant::now() + timeout);
                    }
                    None => {
                        // The recognizer has gone; digits may still come.
                        warn!(session = %session, "Speech recognition ended during a Gather");
                        speech = None;
                        speaking = false;
                        if digits_done {
                            break GatherOutcome::Timeout;
                        }
                    }
                },
                Some(voice) = next_voice(&mut onset) => match voice {
                    Voice::Began => {
                        debug!(session = %session, "The caller began to speak");
                        speaking = true;
                        if let Some(pid) = prompt.take() {
                            let _ = media.stop_playback(session, pid).await;
                        }
                        begin_by.get_or_insert_with(|| Instant::now() + timeout);
                    }
                    // Nothing came of it: the timeout counts again.
                    Voice::Quiet => speaking = false,
                },
            }
        };
        if let Some(pid) = prompt {
            let _ = media.stop_playback(session, pid).await;
        }
        outcome
    }
}

/// Until `deadline`, or for ever without one.
async fn sleep_until_some(deadline: Option<Instant>) {
    match deadline {
        Some(at) => tokio::time::sleep_until(at).await,
        None => std::future::pending().await,
    }
}

async fn next_transcript(stream: &mut Option<SttStream>) -> Option<Transcript> {
    match stream {
        Some(stream) => stream.next().await,
        None => std::future::pending().await,
    }
}

async fn next_voice(voice: &mut Option<mpsc::Receiver<Voice>>) -> Option<Voice> {
    match voice {
        Some(rx) => rx.recv().await,
        None => std::future::pending().await,
    }
}

/// [`Voice::Began`] when `frames` carry [`ONSET_FRAMES`] voiced frames in
/// a row, then [`Voice::Quiet`] once they have been silent for
/// `quiet_after`, and so on.
fn watch_voice(
    mut frames: mpsc::Receiver<Vec<i16>>,
    quiet_after: Duration,
) -> mpsc::Receiver<Voice> {
    let (tx, rx) = mpsc::channel(4);
    let quiet_samples = (quiet_after.as_millis() as usize * stt::STT_SAMPLE_RATE as usize) / 1000;
    tokio::spawn(async move {
        let mut run = 0usize;
        let mut silent = 0usize;
        let mut speaking = false;
        loop {
            tokio::select! {
                frame = frames.recv() => match frame {
                    Some(frame) => {
                        if stt::is_voiced(&frame) {
                            run += 1;
                            silent = 0;
                        } else {
                            run = 0;
                            silent += frame.len();
                        }
                        let heard = if !speaking && run >= ONSET_FRAMES {
                            speaking = true;
                            Some(Voice::Began)
                        } else if speaking && silent >= quiet_samples {
                            speaking = false;
                            Some(Voice::Quiet)
                        } else {
                            None
                        };
                        if let Some(voice) = heard {
                            if tx.send(voice).await.is_err() {
                                return;
                            }
                        }
                    }
                    None => return,
                },
                // Nobody waits any more: let the listener go.
                _ = tx.closed() => return,
            }
        }
    });
    rx
}

/// A call's live transcript: one recognition per leg asked for, each
/// transcript published as `call.transcript`.
pub(crate) struct TranscriptRequest {
    pub leg: TranscribeLeg,
    pub language: Option<String>,
    pub hints: Vec<String>,
    pub interim_results: bool,
}

/// Start the transcript; the tasks end when the session does, or when
/// they are aborted.
pub(crate) async fn start_transcript(
    media: &MediaManager,
    session: &fcp_core::CallId,
    call_id: CallId,
    tenant_id: fcp_core::TenantId,
    events: Option<Arc<dyn EventPublisher>>,
    request: &TranscriptRequest,
) -> fcp_media::Result<Vec<tokio::task::JoinHandle<()>>> {
    let legs: &[(MediaLeg, &str)] = match request.leg {
        TranscribeLeg::Caller => &[(MediaLeg::A, "caller")],
        TranscribeLeg::Callee => &[(MediaLeg::B, "callee")],
        TranscribeLeg::Both => &[(MediaLeg::A, "caller"), (MediaLeg::B, "callee")],
    };
    let options = SttOptions {
        language: request.language.clone(),
        hints: request.hints.clone(),
        interim_results: request.interim_results,
        speech_timeout_ms: None,
    };
    let mut streams = Vec::with_capacity(legs.len());
    for (leg, name) in legs {
        streams.push((media.recognize(session, *leg, &options).await?, *name));
    }
    let call_id = fcp_core::CallId::from_uuid(*call_id.as_uuid());
    let interim = request.interim_results;
    Ok(streams
        .into_iter()
        .map(|(mut stream, leg)| {
            let events = events.clone();
            tokio::spawn(async move {
                info!(call_id = %call_id, leg, "Transcribing the call");
                while let Some(t) = stream.next().await {
                    if t.text.trim().is_empty() || (!t.is_final && !interim) {
                        continue;
                    }
                    let Some(events) = &events else { continue };
                    let event = PlatformEvent::Call(CallEvent::Transcript {
                        call_id,
                        tenant_id,
                        leg: leg.to_string(),
                        text: t.text,
                        confidence: t.confidence,
                        is_final: t.is_final,
                        timestamp: std::time::SystemTime::now(),
                    });
                    if let Err(e) = events.publish(event).await {
                        warn!(error = %e, "Failed to publish call.transcript");
                    }
                }
                debug!(call_id = %call_id, leg, "The call's transcript ended");
            })
        })
        .collect())
}
