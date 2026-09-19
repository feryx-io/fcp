//! Call manager service for B2BUA.
//!
//! The CallManager tracks all active calls and provides methods for call control.

use crate::call::{Call, CallDirection, CallId, CallState};
use anyhow::{Context, Result};
use dashmap::DashMap;
use fcp_events::{CallEvent, EventPublisher, PlatformEvent};
use std::sync::Arc;
use tracing::{debug, info};

/// What a [`CallManager`] tells of each call it ends or forgets.
pub type OnEnded = Arc<dyn Fn(&CallId) + Send + Sync>;

/// Call manager service.
///
/// This service manages the lifecycle of all active calls in the B2BUA.
/// It provides methods to create, retrieve, update, and terminate calls.
#[derive(Clone)]
pub struct CallManager {
    /// Active calls, indexed by CallId
    calls: Arc<DashMap<CallId, Call>>,

    /// Optional storage backend for CDR creation
    storage: Option<Arc<fcp_core::DynStorage>>,

    /// Optional event publisher for real-time events
    events: Option<Arc<dyn EventPublisher>>,

    /// Node ID for event publishing (identifies which call-manager node owns the call)
    node_id: Option<String>,

    /// Billing engine: rates and settles calls at CDR creation. Present
    /// whenever storage is; publishes billing events when events are too.
    billing: Option<Arc<fcp_billing::BillingEngine>>,

    /// Where a finished call's record goes (`docs/CDR_PIPELINE.md`). With
    /// storage but no spool configured, straight to the in-process
    /// processor; the binary swaps in a spooled writer.
    cdr: Option<fcp_cdr::CdrWriter>,

    /// The region this node advertises, stamped on every record.
    region: Option<String>,

    /// Told of every call this manager ends or forgets, after its record
    /// leaves: what the node counted against the call (a trunk's channels,
    /// `docs/TENANT_TRUNKS.md` §7) is released here, wherever the call ended.
    on_ended: Option<OnEnded>,

    /// Emergency calls' audit records as their inserts land: the record id
    /// once written, so the call's ending completes the right row however
    /// short the call was.
    emergency_records: Arc<DashMap<CallId, tokio::sync::watch::Receiver<Option<i64>>>>,
}

impl CallManager {
    /// Create a new call manager without storage (no CDRs).
    pub fn new() -> Self {
        Self {
            calls: Arc::new(DashMap::new()),
            storage: None,
            events: None,
            node_id: None,
            billing: None,
            cdr: None,
            region: None,
            on_ended: None,
            emergency_records: Arc::new(DashMap::new()),
        }
    }

    /// Create a new call manager with storage backend for CDR creation.
    pub fn with_storage(storage: Arc<fcp_core::DynStorage>) -> Self {
        let billing = Arc::new(fcp_billing::BillingEngine::new(Arc::clone(&storage)));
        let cdr = Self::direct_writer(&storage, &billing, None);
        Self {
            calls: Arc::new(DashMap::new()),
            storage: Some(storage),
            events: None,
            node_id: None,
            billing: Some(billing),
            cdr: Some(cdr),
            region: None,
            on_ended: None,
            emergency_records: Arc::new(DashMap::new()),
        }
    }

    /// A writer that hands records straight to an in-process processor over
    /// `storage`, rating them with `billing`: the MVP's durability, until
    /// the binary configures a spool.
    fn direct_writer(
        storage: &Arc<fcp_core::DynStorage>,
        billing: &Arc<fcp_billing::BillingEngine>,
        events: Option<Arc<dyn EventPublisher>>,
    ) -> fcp_cdr::CdrWriter {
        fcp_cdr::CdrWriter::direct(Arc::new(Self::processor(storage, billing, events)))
    }

    /// The in-process processor for this node's records: stores into
    /// `storage`, fills the call's direction from the tenant's numbers,
    /// rates and settles through `billing`, and announces `cdr.written` on
    /// `events` (`docs/CDR_PIPELINE.md` §4.3).
    pub fn processor(
        storage: &Arc<fcp_core::DynStorage>,
        billing: &Arc<fcp_billing::BillingEngine>,
        events: Option<Arc<dyn EventPublisher>>,
    ) -> fcp_cdr::Processor {
        let store: Arc<dyn fcp_core::storage::CdrStore> = storage.clone();
        let rater: Arc<dyn fcp_cdr::Rater> = billing.clone();
        let numbers: Arc<dyn fcp_core::storage::PhoneNumberStore> = storage.clone();
        let processor = fcp_cdr::Processor::new(store, Some(rater)).with_enricher(Arc::new(
            fcp_cdr::DirectionEnricher::new(numbers, std::time::Duration::from_secs(60)),
        ));
        match events {
            Some(events) => processor.with_events(events),
            None => processor,
        }
    }

    /// Create a new call manager with storage and event publishing.
    pub fn with_storage_and_events(
        storage: Arc<fcp_core::DynStorage>,
        events: Arc<dyn EventPublisher>,
    ) -> Self {
        let billing = Arc::new(
            fcp_billing::BillingEngine::new(Arc::clone(&storage)).with_events(Arc::clone(&events)),
        );
        let cdr = Self::direct_writer(&storage, &billing, Some(Arc::clone(&events)));
        Self {
            calls: Arc::new(DashMap::new()),
            storage: Some(storage),
            events: Some(events),
            node_id: None,
            billing: Some(billing),
            cdr: Some(cdr),
            region: None,
            on_ended: None,
            emergency_records: Arc::new(DashMap::new()),
        }
    }

    /// Create a new call manager with only event publishing (no CDRs).
    pub fn with_events(events: Arc<dyn EventPublisher>) -> Self {
        Self {
            calls: Arc::new(DashMap::new()),
            storage: None,
            events: Some(events),
            node_id: None,
            cdr: None,
            region: None,
            on_ended: None,
            billing: None,
            emergency_records: Arc::new(DashMap::new()),
        }
    }

    /// Replace the billing engine (e.g. to add custom billing hooks).
    pub fn set_billing(&mut self, billing: Arc<fcp_billing::BillingEngine>) {
        self.billing = Some(billing);
    }

    /// Where platform events go, when a bus is attached.
    pub fn events(&self) -> Option<&Arc<dyn EventPublisher>> {
        self.events.as_ref()
    }

    /// The billing engine used for settlement, shared with the SIP handler
    /// for admission control.
    pub fn billing(&self) -> Option<&Arc<fcp_billing::BillingEngine>> {
        self.billing.as_ref()
    }

    /// Set the node ID for event publishing.
    ///
    /// This identifies which call-manager node owns the calls and is included
    /// in all published events for multi-node aggregation.
    pub fn set_node_id(&mut self, node_id: impl Into<String>) {
        self.node_id = Some(node_id.into());
    }

    /// Get the configured node ID.
    pub fn node_id(&self) -> Option<&str> {
        self.node_id.as_deref()
    }

    /// Be told of every call this manager ends or forgets.
    pub fn set_on_ended(&mut self, on_ended: OnEnded) {
        self.on_ended = Some(on_ended);
    }

    /// The region this node advertises; on every record it writes.
    pub fn set_region(&mut self, region: Option<String>) {
        self.region = region;
    }

    /// Where finished calls' records go. The binary sets a spooled writer
    /// here once it has opened the spool (`[cdr] spool_dir`).
    pub fn set_cdr_writer(&mut self, writer: fcp_cdr::CdrWriter) {
        self.cdr = Some(writer);
    }

    /// The writer in use, for status.
    pub fn cdr_writer(&self) -> Option<&fcp_cdr::CdrWriter> {
        self.cdr.as_ref()
    }

    /// The storage the manager was built with.
    pub fn storage(&self) -> Option<&Arc<fcp_core::DynStorage>> {
        self.storage.as_ref()
    }

    /// Keep the media session's final counters on the call, for its
    /// record. Called by the handler before it tears the session down.
    pub fn set_rtp_stats(&self, call_id: &CallId, stats: fcp_cdr::RtpStats) {
        let _ = self.update_call(call_id, |call| call.rtp_stats = Some(stats));
    }

    /// Forget a call without ending it or writing its record: for a call
    /// handed to a handler that keeps its own record of it (a conference
    /// node's leg).
    pub fn discard_call(&self, call_id: &CallId) {
        self.calls.remove(call_id);
        if let Some(on_ended) = &self.on_ended {
            on_ended(call_id);
        }
    }

    /// Record why a call is ending before the ending completes: the BYE
    /// handler knows who hung up, but the call is only terminated once the
    /// forwarded BYE completes, by a site that does not. A later
    /// `terminate_call` with no better reason uses this one.
    pub fn set_pending_termination(&self, call_id: &CallId, reason: fcp_cdr::TerminationReason) {
        let _ = self.update_call(call_id, |call| call.termination = Some(reason));
    }

    /// Publish `call.emergency` for an emergency call that was just placed.
    /// Best-effort and asynchronous like the other call events.
    pub fn publish_emergency(&self, call: &Call, notify_contacts: Vec<String>) {
        let Some(events) = &self.events else {
            return;
        };
        let Some(em) = &call.emergency else {
            return;
        };
        let event = PlatformEvent::Call(CallEvent::Emergency {
            call_id: fcp_core::CallId::from_uuid(*call.id.as_uuid()),
            tenant_id: call.tenant_id,
            from: call.a_leg.remote_uri.clone(),
            dialed: em.dialed.clone(),
            kind: em.kind.to_string(),
            callback_number: em.callback_number.clone(),
            location_source: em.location.as_ref().map(|l| l.source.to_string()),
            location_summary: em.location.as_ref().map(|l| l.summary()),
            client_address: em.location.as_ref().and_then(|l| l.client_address.clone()),
            trunk_name: call.trunk_name.clone(),
            notify_contacts,
            timestamp: std::time::SystemTime::now(),
            node_id: self.node_id.clone(),
        });
        let events = Arc::clone(events);
        tokio::spawn(async move {
            if let Err(e) = events.publish(event).await {
                tracing::warn!(error = %e, "Failed to publish call.emergency event");
            }
        });
    }

    /// Write an emergency call's audit record (best-effort, off the call
    /// path) and remember it, so the call's ending completes it.
    pub fn record_emergency(&self, call_id: CallId, record: fcp_core::e911::EmergencyCallRecord) {
        let Some(store) = self
            .storage
            .as_ref()
            .and_then(|s| s.emergency_store())
            .cloned()
        else {
            return;
        };
        let (tx, rx) = tokio::sync::watch::channel(None);
        self.emergency_records.insert(call_id, rx);
        tokio::spawn(async move {
            match store.insert_emergency_call(&record).await {
                Ok(id) => {
                    let _ = tx.send(Some(id));
                }
                Err(e) => {
                    tracing::error!(error = %e, call_id = %record.call_id, "Failed to write emergency call record");
                }
            }
        });
    }

    /// Complete an ended emergency call's record and publish
    /// `call.emergency_completed`.
    fn complete_emergency(&self, call: &Call) {
        let Some(em) = &call.emergency else {
            return;
        };
        let answered = call.established_at.is_some();
        let reason = call
            .termination
            .clone()
            .unwrap_or(fcp_cdr::TerminationReason::Hangup { by: None });
        let outcome = reason.outcome(answered);
        let sip_status = if answered {
            Some(200)
        } else {
            outcome.status.or_else(|| {
                call.failure_reason
                    .as_deref()
                    .and_then(extract_sip_status_code)
            })
        };
        let disposition = serde_json::to_value(outcome.disposition)
            .ok()
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_else(|| "failed".to_string());
        let ended_at = call.ended_at.unwrap_or_else(std::time::SystemTime::now);
        let completion = match &em.leg {
            // A leg placed apart from the call (a flow's Dial node) has its
            // own outcome; the call's answer was the flow's.
            Some(leg) => fcp_core::e911::EmergencyCompletion {
                trunk_name: call.trunk_name.clone(),
                trunk_id: call.trunk_id.clone(),
                outcome: leg.outcome.clone(),
                sip_status: leg.sip_status,
                answered_at: leg.answered_at,
                ended_at,
                delivery: em.delivery(),
                geolocation_error: em.geolocation_error.clone(),
            },
            None => fcp_core::e911::EmergencyCompletion {
                trunk_name: call.trunk_name.clone(),
                trunk_id: call.trunk_id.clone(),
                outcome: disposition,
                sip_status,
                answered_at: call.established_at,
                ended_at,
                delivery: em.delivery(),
                geolocation_error: em.geolocation_error.clone(),
            },
        };
        fcp_core::e911::EmergencyCounters::global().placed(
            em.kind,
            em.location.as_ref(),
            completion.delivery.as_deref(),
        );
        if let Some(error) = &completion.geolocation_error {
            fcp_core::e911::EmergencyCounters::global().location_refused(error);
        }
        info!(
            call_id = %call.id,
            outcome = %completion.outcome,
            sip_status = ?completion.sip_status,
            trunk = ?completion.trunk_name,
            "Emergency call completed"
        );

        if let Some(events) = &self.events {
            let event = PlatformEvent::Call(CallEvent::EmergencyCompleted {
                call_id: fcp_core::CallId::from_uuid(*call.id.as_uuid()),
                tenant_id: call.tenant_id,
                dialed: em.dialed.clone(),
                kind: em.kind.to_string(),
                outcome: completion.outcome.clone(),
                sip_status: completion.sip_status,
                trunk_name: completion.trunk_name.clone(),
                trunk_id: completion.trunk_id.as_ref().map(|t| t.to_string()),
                delivery: completion.delivery.clone(),
                duration_ms: completion
                    .answered_at
                    .and_then(|a| ended_at.duration_since(a).ok())
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0),
                timestamp: ended_at,
                node_id: self.node_id.clone(),
            });
            let events = Arc::clone(events);
            tokio::spawn(async move {
                if let Err(e) = events.publish(event).await {
                    tracing::warn!(error = %e, "Failed to publish call.emergency_completed event");
                }
            });
        }

        let Some((_, mut written)) = self.emergency_records.remove(&call.id) else {
            return;
        };
        let Some(store) = self
            .storage
            .as_ref()
            .and_then(|s| s.emergency_store())
            .cloned()
        else {
            return;
        };
        let call_id = call.id;
        tokio::spawn(async move {
            // The insert may still be on its way; a failed one drops its
            // sender and there is nothing to complete.
            let id = match tokio::time::timeout(
                std::time::Duration::from_secs(30),
                written.wait_for(Option::is_some),
            )
            .await
            {
                Ok(Ok(id)) => id.unwrap_or_default(),
                _ => {
                    tracing::error!(call_id = %call_id, "Emergency call record never written; not completed");
                    return;
                }
            };
            if let Err(e) = store.complete_emergency_call(id, &completion).await {
                tracing::error!(error = %e, call_id = %call_id, "Failed to complete emergency call record");
            }
        });
    }

    /// Create a new call and store it.
    ///
    /// Returns the CallId for the new call.
    pub fn create_call(&self, call: Call) -> CallId {
        let call_id = call.id;
        info!(
            call_id = %call_id,
            caller = %call.caller(),
            callee = ?call.callee(),
            "Creating new call"
        );

        // Publish CallEvent::Initiated
        if let Some(events) = &self.events {
            let event = PlatformEvent::Call(CallEvent::Initiated {
                call_id: fcp_core::CallId::from_uuid(*call_id.as_uuid()),
                tenant_id: call.tenant_id,
                from: call.a_leg.remote_uri.clone(),
                to: call.callee().unwrap_or("unknown").to_string(),
                direction: match call.a_leg.direction {
                    CallDirection::Inbound => fcp_events::CallDirection::Inbound,
                    CallDirection::Outbound => fcp_events::CallDirection::Outbound,
                },
                timestamp: call.created_at,
                trunk_name: call.trunk_name.clone(),
                trunk_id: call.trunk_id.as_ref().map(|t| t.to_string()),
                node_id: self.node_id.clone(),
            });

            let events = Arc::clone(events);
            tokio::spawn(async move {
                if let Err(e) = events.publish(event).await {
                    tracing::error!(error = %e, "Failed to publish CallEvent::Initiated");
                }
            });
        }

        self.calls.insert(call_id, call);
        call_id
    }

    /// Get a call by ID.
    ///
    /// Returns a clone of the call if it exists.
    pub fn get_call(&self, call_id: &CallId) -> Option<Call> {
        self.calls.get(call_id).map(|entry| entry.value().clone())
    }

    /// Update a call.
    ///
    /// The provided function receives a mutable reference to the call
    /// and can modify it. Returns the updated call.
    pub fn update_call<F>(&self, call_id: &CallId, update_fn: F) -> Result<Call>
    where
        F: FnOnce(&mut Call),
    {
        let mut entry = self.calls.get_mut(call_id).context("Call not found")?;

        // Capture old state for comparison
        let old_state = entry.state;

        update_fn(entry.value_mut());

        let new_state = entry.state;
        let updated_call = entry.value().clone();

        debug!(
            call_id = %call_id,
            old_state = ?old_state,
            new_state = ?new_state,
            "Updated call"
        );

        // Publish state change events
        if let Some(events) = &self.events {
            let node_id = self.node_id.clone();
            let should_publish = match (old_state, new_state) {
                // Ringing: transition to Proceeding
                (_, CallState::Proceeding) if old_state != CallState::Proceeding => {
                    Some(PlatformEvent::Call(CallEvent::Ringing {
                        call_id: fcp_core::CallId::from_uuid(*call_id.as_uuid()),
                        timestamp: std::time::SystemTime::now(),
                        node_id: node_id.clone(),
                    }))
                }
                // Answered: transition to Established
                (_, CallState::Established) if old_state != CallState::Established => {
                    // Extract codec from SDP if available
                    let codec = updated_call
                        .a_leg
                        .sdp
                        .as_deref()
                        .and_then(fcp_cdr::record::codec_from_sdp)
                        .unwrap_or_else(|| "unknown".to_string());

                    Some(PlatformEvent::Call(CallEvent::Answered {
                        call_id: fcp_core::CallId::from_uuid(*call_id.as_uuid()),
                        answer_time: updated_call
                            .established_at
                            .unwrap_or_else(std::time::SystemTime::now),
                        codec,
                        a_leg_sdp: updated_call.a_leg.sdp.clone(),
                        b_leg_sdp: updated_call.b_leg.as_ref().and_then(|b| b.sdp.clone()),
                        node_id: node_id.clone(),
                    }))
                }
                _ => None,
            };

            if let Some(event) = should_publish {
                let events = Arc::clone(events);
                tokio::spawn(async move {
                    if let Err(e) = events.publish(event).await {
                        tracing::error!(error = %e, "Failed to publish state change event");
                    }
                });
            }
        }

        Ok(updated_call)
    }

    /// Terminate a call.
    ///
    /// The MVP's contract: `None` is a normal ending, `Some(text)` a
    /// failure, with a leading SIP status code in the text honoured. A site
    /// that knows why the call ended calls [`terminate_call_with`].
    ///
    /// [`terminate_call_with`]: CallManager::terminate_call_with
    pub fn terminate_call(&self, call_id: &CallId, reason: Option<String>) -> Result<()> {
        self.terminate_call_with(call_id, fcp_cdr::TerminationReason::from(reason))
    }

    /// Terminate a call for `reason`: mark it ended or failed, publish the
    /// termination event, write its record, and drop it from the active
    /// calls. Synchronous and cheap; the record leaves on the writer.
    pub fn terminate_call_with(
        &self,
        call_id: &CallId,
        reason: fcp_cdr::TerminationReason,
    ) -> Result<()> {
        let call = self.update_call(call_id, |call| {
            // A reason set while the ending was under way (the BYE handler
            // knows who hung up; the completion that ends the call does not)
            // wins over an unspecific one.
            let reason = match (&reason, &call.termination) {
                (fcp_cdr::TerminationReason::Hangup { by: None }, Some(pending)) => pending.clone(),
                _ => reason.clone(),
            };
            let answered = call.established_at.is_some();
            if reason.is_failure(answered) {
                call.fail(reason.detail().unwrap_or_else(|| "Call failed".to_string()));
            } else {
                call.terminate();
                call.failure_reason = reason.detail();
            }
            call.termination = Some(reason);
        })?;
        let reason = call
            .termination
            .clone()
            .unwrap_or(fcp_cdr::TerminationReason::Hangup { by: None });
        let hangup_party = reason
            .outcome(call.established_at.is_some())
            .party
            .map(|p| format!("{p:?}").to_lowercase());

        info!(
            call_id = %call_id,
            caller = %call.caller(),
            callee = ?call.callee(),
            duration = ?call.duration(),
            state = ?call.state,
            "Call terminated"
        );

        // Publish termination event
        if let Some(events) = &self.events {
            let node_id = self.node_id.clone();
            let event = match call.state {
                CallState::Terminated => {
                    // Normal termination
                    PlatformEvent::Call(CallEvent::Ended {
                        call_id: fcp_core::CallId::from_uuid(*call_id.as_uuid()),
                        end_time: call.ended_at.unwrap_or_else(std::time::SystemTime::now),
                        duration_ms: call.duration().map(|d| d.as_millis() as u64).unwrap_or(0),
                        hangup_cause: "Normal Clearing".to_string(),
                        hangup_party,
                        trunk_name: call.trunk_name.clone(),
                        trunk_id: call.trunk_id.as_ref().map(|t| t.to_string()),
                        node_id: node_id.clone(),
                    })
                }
                CallState::Failed => {
                    // Failed call
                    let (reason, sip_code) = call
                        .failure_reason
                        .as_ref()
                        .map(|r| {
                            let code = extract_sip_status_code(r);
                            (r.clone(), code)
                        })
                        .unwrap_or_else(|| ("Unknown error".to_string(), None));

                    PlatformEvent::Call(CallEvent::Failed {
                        call_id: fcp_core::CallId::from_uuid(*call_id.as_uuid()),
                        timestamp: call.ended_at.unwrap_or_else(std::time::SystemTime::now),
                        reason,
                        sip_code,
                        trunk_name: call.trunk_name.clone(),
                        trunk_id: call.trunk_id.as_ref().map(|t| t.to_string()),
                        node_id: node_id.clone(),
                    })
                }
                _ => {
                    // Unexpected state - log warning but don't publish event
                    tracing::warn!(
                        call_id = %call_id,
                        state = ?call.state,
                        "Call terminated in unexpected state"
                    );
                    // Publish as Failed with generic reason
                    PlatformEvent::Call(CallEvent::Failed {
                        call_id: fcp_core::CallId::from_uuid(*call_id.as_uuid()),
                        timestamp: std::time::SystemTime::now(),
                        reason: format!("Terminated in unexpected state: {:?}", call.state),
                        sip_code: None,
                        trunk_name: call.trunk_name.clone(),
                        trunk_id: call.trunk_id.as_ref().map(|t| t.to_string()),
                        node_id: node_id.clone(),
                    })
                }
            };

            let events = Arc::clone(events);
            tokio::spawn(async move {
                if let Err(e) = events.publish(event).await {
                    tracing::error!(error = %e, "Failed to publish termination event");
                }
            });
        }

        self.complete_emergency(&call);

        // The call's record, off the call path through the writer.
        if let Some(writer) = &self.cdr {
            writer.write(self.record_for(&call));
        }

        // Remove from active calls
        self.calls.remove(call_id);
        if let Some(on_ended) = &self.on_ended {
            on_ended(call_id);
        }

        Ok(())
    }

    /// The record of a finished call, from everything the manager knows.
    pub fn record_for(&self, call: &Call) -> fcp_cdr::Record {
        let sip_call_id = call
            .original_invite
            .as_ref()
            .and_then(|invite| sip_parse::header(invite.headers(), "Call-ID"))
            .map(|cid| cid.as_str().to_string());

        // The callee from the B-leg, or the To header when no B-leg was made.
        let (to_uri, to_display_name) = match &call.b_leg {
            Some(b_leg) => (b_leg.remote_uri.clone(), b_leg.remote_display_name.clone()),
            None => (
                call.original_invite
                    .as_ref()
                    .and_then(|invite| sip_parse::header(invite.headers(), "To"))
                    .map(|to| to.as_str().to_string())
                    .unwrap_or_else(|| "unknown".to_string()),
                None,
            ),
        };

        let mut builder = fcp_cdr::RecordBuilder::new(
            fcp_cdr::CdrKind::Call,
            fcp_core::CallId::from_uuid(*call.id.as_uuid()),
            call.tenant_id,
        )
        .from(
            call.a_leg.remote_uri.clone(),
            call.a_leg.remote_display_name.clone(),
        )
        .to(to_uri, to_display_name)
        .started_at(call.created_at)
        .answered_at(call.established_at)
        .ended_at(call.ended_at)
        .reason(
            call.termination
                .clone()
                .unwrap_or(fcp_cdr::TerminationReason::Hangup { by: None }),
        )
        .media_anchored(call.media_anchored)
        .sdp(
            call.a_leg.sdp.as_deref(),
            call.b_leg.as_ref().and_then(|b| b.sdp.as_deref()),
        )
        .rtp(call.rtp_stats)
        .correlation(call.correlation_id)
        .recording(call.recording_path.clone())
        .recording_consent(call.recording_consent)
        .trunk(call.trunk_name.clone())
        .trunk_id(call.trunk_id.clone())
        .node(self.node_id.clone(), self.region.clone())
        .custom_data(call.custom_data.clone());
        if let Some(id) = sip_call_id {
            builder = builder.sip_call_id(id);
        }
        builder.build()
    }

    /// Get all active calls.
    ///
    /// Returns a vector of all calls that are not in a terminal state.
    pub fn active_calls(&self) -> Vec<Call> {
        self.calls
            .iter()
            .filter(|entry| !entry.value().state.is_terminal())
            .map(|entry| entry.value().clone())
            .collect()
    }

    /// Get the count of active calls.
    pub fn active_call_count(&self) -> usize {
        self.calls
            .iter()
            .filter(|entry| !entry.value().state.is_terminal())
            .count()
    }

    /// Get all calls (including terminated).
    pub fn all_calls(&self) -> Vec<Call> {
        self.calls
            .iter()
            .map(|entry| entry.value().clone())
            .collect()
    }

    /// Find a call by the SIP Call-ID of the original INVITE.
    pub fn find_by_invite_call_id(&self, sip_call_id: &str) -> Option<Call> {
        self.calls
            .iter()
            .find(|entry| {
                entry
                    .value()
                    .original_invite
                    .as_ref()
                    .and_then(|inv| sip_parse::header(inv.headers(), "Call-ID"))
                    .map(|cid| cid.as_str() == sip_call_id)
                    .unwrap_or(false)
            })
            .map(|entry| entry.value().clone())
    }

    /// Get the total count of calls (including terminated).
    pub fn total_call_count(&self) -> usize {
        self.calls.len()
    }

    /// Find a call by dialog Call-ID.
    ///
    /// This is useful when receiving an in-dialog request (ACK, BYE) where
    /// we need to find the corresponding call from the SIP Call-ID header.
    pub fn find_by_dialog_call_id(&self, dialog_call_id: &str) -> Option<Call> {
        debug!(
            dialog_call_id = dialog_call_id,
            total_calls = self.calls.len(),
            "Looking up call by dialog Call-ID"
        );

        let result = self.calls
            .iter()
            .find(|entry| {
                let call = entry.value();

                debug!(
                    call_id = %call.id,
                    has_a_leg_dialog = call.a_leg.dialog.is_some(),
                    has_b_leg = call.b_leg.is_some(),
                    has_b_leg_dialog = call.b_leg.as_ref().and_then(|b| b.dialog.as_ref()).is_some(),
                    "Checking call"
                );

                // Check A-leg dialog
                if let Some(dialog) = &call.a_leg.dialog {
                    debug!(
                        call_id = %call.id,
                        a_leg_call_id = %dialog.id().call_id(),
                        searching_for = dialog_call_id,
                        "Checking A-leg dialog"
                    );
                    if dialog.id().call_id() == dialog_call_id {
                        debug!(call_id = %call.id, "Found call via A-leg dialog");
                        return true;
                    }
                }

                // Check B-leg dialog
                if let Some(b_leg) = &call.b_leg {
                    if let Some(dialog) = &b_leg.dialog {
                        debug!(
                            call_id = %call.id,
                            b_leg_call_id = %dialog.id().call_id(),
                            searching_for = dialog_call_id,
                            "Checking B-leg dialog"
                        );
                        if dialog.id().call_id() == dialog_call_id {
                            debug!(call_id = %call.id, "Found call via B-leg dialog");
                            return true;
                        }
                    } else {
                        debug!(call_id = %call.id, "B-leg exists but has no dialog");
                    }
                } else {
                    debug!(call_id = %call.id, "Call has no B-leg");
                }

                false
            })
            .map(|entry| entry.value().clone());

        if result.is_none() {
            debug!(
                dialog_call_id = dialog_call_id,
                "No call found for dialog Call-ID"
            );
        }

        result
    }

    /// Find an established call where the A-leg remote_uri user-part matches the given extension.
    ///
    /// This is used by call parking to find the active call for a user who dials *70.
    pub fn find_established_by_caller(&self, extension: &str) -> Option<Call> {
        self.calls
            .iter()
            .find(|entry| {
                let call = entry.value();
                call.state == CallState::Established
                    && extract_user_from_uri(&call.a_leg.remote_uri)
                        .map(|user| user == extension)
                        .unwrap_or(false)
            })
            .map(|entry| entry.value().clone())
    }

    /// Clean up terminated calls older than the specified duration.
    ///
    /// This should be called periodically to prevent memory leaks.
    pub fn cleanup_old_calls(&self, max_age: std::time::Duration) -> usize {
        let now = std::time::SystemTime::now();
        let mut removed = 0;

        self.calls.retain(|call_id, call| {
            if call.state.is_terminal() {
                if let Some(ended_at) = call.ended_at {
                    if let Ok(age) = now.duration_since(ended_at) {
                        if age > max_age {
                            debug!(call_id = %call_id, "Cleaning up old terminated call");
                            removed += 1;
                            return false;
                        }
                    }
                }
            }
            true
        });

        if removed > 0 {
            info!(removed = removed, "Cleaned up old terminated calls");
        }

        removed
    }

    /// Get statistics about calls.
    pub fn stats(&self) -> CallStats {
        let all_calls: Vec<_> = self.calls.iter().collect();

        let total = all_calls.len();
        let mut active = 0;
        let mut established = 0;
        let mut terminated = 0;
        let mut failed = 0;

        for entry in all_calls {
            let state = entry.value().state;
            match state {
                CallState::Established | CallState::Parked => {
                    active += 1;
                    established += 1;
                }
                CallState::Terminated => terminated += 1,
                CallState::Failed => failed += 1,
                _ => active += 1,
            }
        }

        CallStats {
            total,
            active,
            established,
            terminated,
            failed,
        }
    }
}

impl Default for CallManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Extract user-part from a SIP URI (e.g., "sip:alice@example.com" → "alice").
fn extract_user_from_uri(uri: &str) -> Option<&str> {
    // Strip angle brackets if present
    let uri = uri.trim();
    let uri = if let Some(start) = uri.find('<') {
        if let Some(end) = uri.find('>') {
            &uri[start + 1..end]
        } else {
            uri
        }
    } else {
        uri
    };

    // Strip sip: or sips: prefix
    let after_scheme = uri
        .strip_prefix("sip:")
        .or_else(|| uri.strip_prefix("sips:"))
        .unwrap_or(uri);

    // Take everything before @
    after_scheme.split('@').next()
}

/// Extract SIP status code from a failure reason string (e.g., "486 Busy Here" -> Some(486)).
fn extract_sip_status_code(reason: &str) -> Option<u16> {
    fcp_cdr::record::leading_status_code(reason)
}

/// Call statistics.
#[derive(Debug, Clone, Copy)]
pub struct CallStats {
    /// Total number of calls (including terminated)
    pub total: usize,

    /// Number of active calls (not in terminal state)
    pub active: usize,

    /// Number of established calls (200 OK received/sent)
    pub established: usize,

    /// Number of terminated calls
    pub terminated: usize,

    /// Number of failed calls
    pub failed: usize,
}

#[cfg(test)]
#[allow(unused_must_use, unused_imports)]
mod tests {
    use super::*;
    use crate::call::Call;
    use std::net::SocketAddr;

    fn create_test_call() -> Call {
        use bytes::Bytes;
        use sip_core::{Headers, Method, RequestLine};
        use smol_str::SmolStr;

        let peer: SocketAddr = "127.0.0.1:5060".parse().unwrap();

        let mut headers = Headers::new();
        headers
            .push(
                SmolStr::new("From"),
                SmolStr::new("Alice <sip:alice@example.com>"),
            )
            .unwrap();
        headers
            .push(
                SmolStr::new("To"),
                SmolStr::new("Bob <sip:bob@example.com>"),
            )
            .unwrap();
        headers
            .push(SmolStr::new("Call-ID"), SmolStr::new("test-call-id"))
            .unwrap();

        let request = sip_core::Request::new(
            RequestLine::new(
                Method::Invite,
                sip_core::SipUri::parse("sip:bob@example.com").unwrap(),
            ),
            headers,
            Bytes::new(),
        )
        .expect("Failed to create request");

        Call::from_invite(request, peer, fcp_core::TenantId::nil())
    }

    #[test]
    fn test_create_and_get_call() {
        let manager = CallManager::new();
        let call = create_test_call();
        let call_id = call.id;

        manager.create_call(call.clone());

        let retrieved = manager.get_call(&call_id);
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().id, call_id);
    }

    #[test]
    fn test_update_call() {
        let manager = CallManager::new();
        let call = create_test_call();
        let call_id = call.id;

        manager.create_call(call);

        let result = manager.update_call(&call_id, |call| {
            call.establish();
        });

        assert!(result.is_ok());
        let updated = manager.get_call(&call_id).unwrap();
        assert_eq!(updated.state, CallState::Established);
    }

    #[test]
    fn test_terminate_call() {
        let manager = CallManager::new();
        let call = create_test_call();
        let call_id = call.id;

        manager.create_call(call);

        let result = manager.terminate_call(&call_id, None);
        assert!(result.is_ok());

        // Call should be removed
        assert!(manager.get_call(&call_id).is_none());
    }

    #[test]
    fn test_active_calls() {
        let manager = CallManager::new();

        // Create active call
        let mut call1 = create_test_call();
        call1.state = CallState::Established;
        manager.create_call(call1);

        // Create terminated call
        let mut call2 = create_test_call();
        call2.terminate();
        manager.create_call(call2);

        assert_eq!(manager.active_call_count(), 1);
        assert_eq!(manager.total_call_count(), 2);
    }

    #[test]
    fn test_stats() {
        let manager = CallManager::new();

        let mut call1 = create_test_call();
        call1.establish();
        manager.create_call(call1);

        let mut call2 = create_test_call();
        call2.terminate();
        manager.create_call(call2);

        let stats = manager.stats();
        assert_eq!(stats.total, 2);
        assert_eq!(stats.established, 1);
        assert_eq!(stats.terminated, 1);
    }

    // ============================================================================
    // find_established_by_caller Tests
    // ============================================================================

    #[test]
    fn test_find_established_by_caller() {
        let manager = CallManager::new();

        let mut call = create_test_call();
        call.establish();
        let call_id = call.id;
        manager.create_call(call);

        // "alice" is the From user in the test call
        let found = manager.find_established_by_caller("alice");
        assert!(found.is_some());
        assert_eq!(found.unwrap().id, call_id);

        // Non-existent extension
        let not_found = manager.find_established_by_caller("bob");
        assert!(not_found.is_none());
    }

    #[test]
    fn test_find_established_by_caller_ignores_non_established() {
        let manager = CallManager::new();

        // Initiated call (not established)
        let call = create_test_call();
        manager.create_call(call);

        let found = manager.find_established_by_caller("alice");
        assert!(found.is_none());
    }

    #[test]
    fn test_extract_sip_status_code_valid() {
        assert_eq!(extract_sip_status_code("486 Busy Here"), Some(486));
        assert_eq!(extract_sip_status_code("200 OK"), Some(200));
        assert_eq!(extract_sip_status_code("404 Not Found"), Some(404));
        assert_eq!(
            extract_sip_status_code("503 Service Unavailable"),
            Some(503)
        );
    }

    #[test]
    fn test_extract_sip_status_code_invalid() {
        // Out of range
        assert_eq!(extract_sip_status_code("99 Invalid"), None);
        assert_eq!(extract_sip_status_code("700 Out of Range"), None);

        // Not a number
        assert_eq!(extract_sip_status_code("ABC Invalid"), None);
        assert_eq!(extract_sip_status_code("Unknown error"), None);

        // Empty
        assert_eq!(extract_sip_status_code(""), None);
    }

    // ============================================================================

    #[test]
    fn test_call_routing_context_fields_default_none() {
        let call = create_test_call();
        assert!(call.line_binding_id.is_none());
        assert!(call.original_callee.is_none());
        assert!(call.caller_number.is_none());
    }

    #[test]
    fn test_call_routing_context_fields_update() {
        let manager = CallManager::new();
        let call = create_test_call();
        let call_id = call.id;
        manager.create_call(call);

        let lb_id = fcp_core::LineBindingId::new();
        let result = manager.update_call(&call_id, |call| {
            call.line_binding_id = Some(lb_id);
            call.original_callee = Some("sip:+15551234567@example.com".to_string());
            call.caller_number = Some("sip:alice@example.com".to_string());
        });
        assert!(result.is_ok());

        let updated = manager.get_call(&call_id).unwrap();
        assert_eq!(updated.line_binding_id, Some(lb_id));
        assert_eq!(
            updated.original_callee.as_deref(),
            Some("sip:+15551234567@example.com")
        );
        assert_eq!(
            updated.caller_number.as_deref(),
            Some("sip:alice@example.com")
        );
    }

    #[test]
    fn test_originated_call_routing_context_none() {
        let call = crate::call::Call::new_originated(
            "sip:alice@example.com".to_string(),
            None,
            "sip:bob@example.com".to_string(),
            fcp_core::TenantId::nil(),
        );
        assert!(call.line_binding_id.is_none());
        assert!(call.original_callee.is_none());
        assert!(call.caller_number.is_none());
    }
}
