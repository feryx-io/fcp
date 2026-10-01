//! Flow execution engine.
//!
//! Executes flow definitions step-by-step, managing state and variable interpolation.

use crate::schema::{BranchOperator, FlowDefinition, NodeType};
use crate::{FlowError, Result};
use fcp_core::CallId;
use fcp_media::TtsService;
use futures::future::BoxFuture;
use regex::Regex;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::RwLock;
use tracing::{debug, error, info, warn};

/// Flow execution engine.
pub struct FlowEngine {
    /// HTTP client for HTTP nodes
    http_client: reqwest::Client,

    /// TTS service for Say nodes (optional)
    tts_service: Option<Arc<TtsService>>,
}

impl FlowEngine {
    /// Create a new flow engine.
    pub fn new() -> Self {
        let http_client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .expect("Failed to build HTTP client");

        Self {
            http_client,
            tts_service: None,
        }
    }

    /// Create a new flow engine with TTS service.
    pub fn with_tts(tts_service: Arc<TtsService>) -> Self {
        let http_client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .expect("Failed to build HTTP client");

        Self {
            http_client,
            tts_service: Some(tts_service),
        }
    }

    /// Start executing a flow.
    pub async fn execute(
        &self,
        flow: Arc<FlowDefinition>,
        call_id: CallId,
        initial_variables: HashMap<String, String>,
    ) -> Result<FlowContext> {
        info!(
            call_id = %call_id,
            flow_id = %flow.id,
            flow_name = %flow.name,
            "Starting flow execution"
        );

        let context = FlowContext::new(flow.clone(), call_id, initial_variables);

        Ok(context)
    }

    /// Execute a single node in the flow.
    pub async fn execute_node(
        &self,
        context: &mut FlowContext,
        node_id: &str,
    ) -> Result<FlowExecutionResult> {
        let flow = &context.flow;
        let node = flow
            .nodes
            .get(node_id)
            .ok_or_else(|| FlowError::NodeNotFound(node_id.to_string()))?;

        debug!(
            call_id = %context.call_id,
            node_id = %node_id,
            node_type = ?node.node_type,
            "Executing node"
        );

        context.current_node = node_id.to_string();
        context.execution_path.push(node_id.to_string());

        let result = match &node.node_type {
            NodeType::Answer { next, on_error: _ } => {
                info!(call_id = %context.call_id, "Answering call");
                // Answer errors are handled by the flow handler (SIP layer),
                // not here. The on_error field is passed through to the handler.
                Ok(FlowExecutionResult::Continue(next.clone()))
            }

            NodeType::Play {
                url,
                loop_count,
                next,
                on_error: _,
            } => {
                let interpolated_url = self.interpolate_variables(url, context)?;
                info!(
                    call_id = %context.call_id,
                    url = %interpolated_url,
                    loop_count = %loop_count,
                    "Playing audio"
                );

                // In real implementation, this would trigger media playback
                // For now, we just continue to next node
                Ok(FlowExecutionResult::Continue(next.clone()))
            }

            NodeType::Say {
                text,
                voice,
                language,
                next,
                on_error,
            } => {
                let interpolated_text = self.interpolate_variables(text, context)?;

                // If TTS service is available, synthesize the text
                if let Some(tts_service) = &self.tts_service {
                    info!(
                        call_id = %context.call_id,
                        text = %interpolated_text,
                        voice = ?voice,
                        language = %language,
                        "Synthesizing text with TTS"
                    );

                    match tts_service
                        .synthesize(&interpolated_text, Some(language), voice.as_deref())
                        .await
                    {
                        Ok(audio_path) => {
                            // Store audio path in context for flow handler
                            context.set_variable(
                                "__tts_audio_path".to_string(),
                                audio_path.display().to_string(),
                            );
                            context.set_variable("__tts_next_node".to_string(), next.clone());

                            info!(
                                call_id = %context.call_id,
                                audio_path = %audio_path.display(),
                                "TTS synthesis complete"
                            );

                            Ok(FlowExecutionResult::WaitingForMedia)
                        }
                        Err(e) => {
                            error!(
                                call_id = %context.call_id,
                                error = %e,
                                "TTS synthesis failed"
                            );

                            if let Some(error_node) = on_error {
                                Ok(FlowExecutionResult::Continue(error_node.clone()))
                            } else {
                                Err(FlowError::TtsError(e.to_string()))
                            }
                        }
                    }
                } else {
                    warn!(
                        call_id = %context.call_id,
                        "TTS service not available, skipping Say node"
                    );
                    Ok(FlowExecutionResult::Continue(next.clone()))
                }
            }

            NodeType::Gather {
                prompt,
                num_digits,
                timeout,
                finish_on_key,
                variable,
                transitions,
                default,
                on_timeout,
                on_error,
                barge_in,
                input,
                speech_timeout_ms,
                hints,
                language,
                speech_transitions,
            } => {
                info!(
                    call_id = %context.call_id,
                    num_digits = %num_digits,
                    timeout = %timeout,
                    input = ?input,
                    has_prompt = prompt.is_some(),
                    "Gathering input"
                );
                // A prompt is played afresh each time: a previous Gather's
                // must not stand in for one this node does not have.
                context.remove_variable("__gather_prompt_type");
                context.remove_variable("__gather_prompt_url");
                context.remove_variable("__gather_finish_on_key");
                context.remove_variable("__gather_on_timeout");

                // Handle prompt if present
                if let Some(gather_prompt) = prompt {
                    use crate::schema::GatherPrompt;

                    match gather_prompt {
                        GatherPrompt::Play { url } => {
                            // Store Play prompt information for flow handler
                            context.set_variable(
                                "__gather_prompt_type".to_string(),
                                "play".to_string(),
                            );
                            context.set_variable("__gather_prompt_url".to_string(), url.clone());
                        }
                        GatherPrompt::Say {
                            text,
                            voice,
                            language,
                        } => {
                            // If TTS service is available, synthesize the prompt
                            if let Some(tts_service) = &self.tts_service {
                                let interpolated_text =
                                    self.interpolate_variables(text, context)?;

                                match tts_service
                                    .synthesize(
                                        &interpolated_text,
                                        Some(language),
                                        voice.as_deref(),
                                    )
                                    .await
                                {
                                    Ok(audio_path) => {
                                        // Store synthesized audio path
                                        context.set_variable(
                                            "__gather_prompt_type".to_string(),
                                            "play".to_string(),
                                        );
                                        context.set_variable(
                                            "__gather_prompt_url".to_string(),
                                            audio_path.display().to_string(),
                                        );
                                    }
                                    Err(e) => {
                                        error!(
                                            call_id = %context.call_id,
                                            error = %e,
                                            "TTS synthesis failed for Gather prompt"
                                        );
                                        // Continue anyway - will gather without prompt
                                    }
                                }
                            } else {
                                warn!(
                                    call_id = %context.call_id,
                                    "TTS service not available for Gather prompt"
                                );
                            }
                        }
                    }
                }

                // When num_digits is 0 (variable-length) and there is no
                // finish_on_key, infer from the transition keys so the
                // collector completes as soon as enough digits are received
                // instead of always waiting for the full timeout.
                let effective_num_digits = if *num_digits == 0
                    && finish_on_key.is_none()
                    && !transitions.is_empty()
                {
                    let max_key_len = transitions.keys().map(|k| k.len()).max().unwrap_or(0);
                    if max_key_len > 0 {
                        info!(
                            call_id = %context.call_id,
                            inferred_num_digits = max_key_len,
                            "Inferred num_digits from transition keys (num_digits was 0 with no finish_on_key)"
                        );
                        max_key_len as u32
                    } else {
                        *num_digits
                    }
                } else {
                    *num_digits
                };

                // Store gather configuration in context
                context.set_variable(
                    "__gather_num_digits".to_string(),
                    effective_num_digits.to_string(),
                );
                context.set_variable("__gather_timeout".to_string(), timeout.to_string());
                if let Some(key) = finish_on_key {
                    context.set_variable("__gather_finish_on_key".to_string(), key.to_string());
                }
                if let Some(timeout_node) = on_timeout {
                    context.set_variable("__gather_on_timeout".to_string(), timeout_node.clone());
                }
                context.set_variable("__gather_barge_in".to_string(), barge_in.to_string());

                // Return WaitingForInput with transition information
                Ok(FlowExecutionResult::WaitingForInput {
                    variable: variable.clone(),
                    transitions: transitions.clone(),
                    default: default.clone(),
                    // The flow builder writes an unset field empty.
                    speech: SpeechGather {
                        input: *input,
                        speech_timeout_ms: speech_timeout_ms.filter(|ms| *ms > 0),
                        hints: hints.clone(),
                        language: language.clone().filter(|l| !l.is_empty()),
                        transitions: speech_transitions.clone(),
                    },
                    on_error: on_error.clone(),
                })
            }

            NodeType::Transcribe {
                action,
                leg,
                language,
                hints,
                interim_results,
                next,
                on_error,
            } => Ok(FlowExecutionResult::Transcribe {
                action: *action,
                leg: *leg,
                language: language.clone().filter(|l| !l.is_empty()),
                hints: hints.clone(),
                interim_results: *interim_results,
                next: next.clone(),
                on_error: on_error.clone(),
            }),

            NodeType::Dial {
                destination,
                timeout,
                caller_id,
                max_duration,
                next,
                on_busy,
                on_no_answer,
                on_error,
            } => {
                // Interpolate variables in destination
                let interpolated_dest = self.interpolate_variables(destination, context)?;

                // Interpolate caller ID if provided
                let interpolated_cid = if let Some(cid) = caller_id {
                    Some(self.interpolate_variables(cid, context)?)
                } else {
                    None
                };

                info!(
                    call_id = %context.call_id,
                    destination = %interpolated_dest,
                    timeout = %timeout,
                    max_duration = %max_duration,
                    caller_id = ?interpolated_cid,
                    "Dialing destination"
                );

                // Store dial configuration in context
                context.set_variable("__dial_destination".to_string(), interpolated_dest.clone());
                context.set_variable("__dial_timeout".to_string(), timeout.to_string());
                if let Some(cid) = &interpolated_cid {
                    context.set_variable("__dial_caller_id".to_string(), cid.clone());
                }
                context.set_variable("__dial_max_duration".to_string(), max_duration.to_string());

                // Return WaitingForDial to let flow handler create the outbound call
                Ok(FlowExecutionResult::WaitingForDial {
                    destination: interpolated_dest,
                    timeout_secs: *timeout,
                    caller_id: interpolated_cid,
                    max_duration_secs: *max_duration,
                    on_busy: on_busy.clone(),
                    on_no_answer: on_no_answer.clone(),
                    on_error: on_error.clone(),
                    next: next.clone(),
                })
            }

            NodeType::Hangup { reason } => {
                info!(
                    call_id = %context.call_id,
                    reason = ?reason,
                    "Hanging up call"
                );
                Ok(FlowExecutionResult::Hangup(reason.clone()))
            }

            NodeType::Http {
                method,
                url,
                body,
                headers,
                timeout,
                send_variables,
                response_variable,
                next,
                on_error,
            } => {
                let interpolated_url = self.interpolate_variables(url, context)?;

                info!(
                    call_id = %context.call_id,
                    method = ?method,
                    url = %interpolated_url,
                    "Making HTTP request"
                );

                match self
                    .execute_http_request(
                        method,
                        &interpolated_url,
                        body.as_deref(),
                        *send_variables,
                        headers,
                        *timeout,
                        context,
                    )
                    .await
                {
                    Ok((status, response_body)) => {
                        store_http_response(context, response_variable, status, &response_body);
                        if status >= 400 {
                            warn!(
                                call_id = %context.call_id,
                                status,
                                url = %interpolated_url,
                                "HTTP request answered with an error status"
                            );
                        }
                        // The flow builder writes an unset node as "".
                        let node = match on_error.as_ref().filter(|n| !n.is_empty()) {
                            Some(error_node) if status >= 400 => error_node,
                            _ => next,
                        };
                        Ok(FlowExecutionResult::Continue(node.clone()))
                    }
                    Err(e) => {
                        error!(error = %e, "HTTP request failed");
                        match on_error.as_ref().filter(|n| !n.is_empty()) {
                            Some(error_node) => {
                                Ok(FlowExecutionResult::Continue(error_node.clone()))
                            }
                            None => Err(e),
                        }
                    }
                }
            }

            NodeType::Branch {
                variable,
                operator,
                value,
                if_true,
                if_false,
                on_error,
            } => match context.get_variable(variable) {
                Ok(var_value) => {
                    let interpolated_compare_value = self.interpolate_variables(value, context)?;

                    let condition_met =
                        self.evaluate_condition(&var_value, *operator, &interpolated_compare_value);

                    debug!(
                        call_id = %context.call_id,
                        variable = %variable,
                        value = %var_value,
                        operator = ?operator,
                        compare_value = %interpolated_compare_value,
                        result = %condition_met,
                        "Evaluating branch condition"
                    );

                    if condition_met {
                        Ok(FlowExecutionResult::Continue(if_true.clone()))
                    } else {
                        Ok(FlowExecutionResult::Continue(if_false.clone()))
                    }
                }
                Err(e) => {
                    warn!(
                        call_id = %context.call_id,
                        variable = %variable,
                        error = %e,
                        "Branch variable lookup failed"
                    );
                    if let Some(error_node) = on_error {
                        Ok(FlowExecutionResult::Continue(error_node.clone()))
                    } else {
                        Err(e)
                    }
                }
            },

            NodeType::SetVariable {
                variable,
                value,
                next,
                on_error,
            } => match self.interpolate_variables(value, context) {
                Ok(interpolated_value) => {
                    debug!(
                        call_id = %context.call_id,
                        variable = %variable,
                        value = %interpolated_value,
                        "Setting variable"
                    );
                    context.set_variable(variable.clone(), interpolated_value);
                    Ok(FlowExecutionResult::Continue(next.clone()))
                }
                Err(e) => {
                    warn!(
                        call_id = %context.call_id,
                        variable = %variable,
                        error = %e,
                        "SetVariable interpolation failed"
                    );
                    if let Some(error_node) = on_error {
                        Ok(FlowExecutionResult::Continue(error_node.clone()))
                    } else {
                        Err(e)
                    }
                }
            },

            NodeType::Fork {
                destinations,
                strategy,
                timeout,
                next,
                on_error,
            } => {
                info!(
                    call_id = %context.call_id,
                    destinations = ?destinations,
                    strategy = ?strategy,
                    timeout = timeout,
                    "Forking to multiple destinations"
                );

                // Store fork configuration in context
                context.set_variable(
                    "__fork_destinations".to_string(),
                    serde_json::to_string(destinations).unwrap_or_default(),
                );
                context.set_variable("__fork_strategy".to_string(), format!("{:?}", strategy));
                context.set_variable("__fork_timeout".to_string(), timeout.to_string());

                Ok(FlowExecutionResult::WaitingForFork {
                    destinations: destinations.clone(),
                    strategy: *strategy,
                    timeout_secs: *timeout,
                    next: next.clone(),
                    on_error: on_error.clone(),
                })
            }

            NodeType::StartRecording {
                format,
                variable,
                next,
                on_error,
            } => {
                info!(
                    call_id = %context.call_id,
                    format = ?format,
                    "Starting recording"
                );

                Ok(FlowExecutionResult::WaitingForRecording {
                    format: *format,
                    variable: variable.clone(),
                    next: next.clone(),
                    on_error: on_error.clone(),
                })
            }

            NodeType::StopRecording { next, on_error } => {
                info!(call_id = %context.call_id, "Stopping recording");
                Ok(FlowExecutionResult::StoppingRecording {
                    next: next.clone(),
                    on_error: on_error.clone(),
                })
            }

            NodeType::Pause { duration, next } => {
                info!(call_id = %context.call_id, duration = %duration, "Pausing");
                tokio::time::sleep(std::time::Duration::from_secs(*duration as u64)).await;
                Ok(FlowExecutionResult::Continue(next.clone()))
            }

            NodeType::Log {
                message,
                level,
                next,
            } => {
                let interpolated_message = self.interpolate_variables(message, context)?;
                match level {
                    crate::schema::LogLevel::Debug => {
                        debug!(call_id = %context.call_id, "{}", interpolated_message)
                    }
                    crate::schema::LogLevel::Info => {
                        info!(call_id = %context.call_id, "{}", interpolated_message)
                    }
                    crate::schema::LogLevel::Warn => {
                        warn!(call_id = %context.call_id, "{}", interpolated_message)
                    }
                    crate::schema::LogLevel::Error => {
                        error!(call_id = %context.call_id, "{}", interpolated_message)
                    }
                }
                Ok(FlowExecutionResult::Continue(next.clone()))
            }

            NodeType::Schedule { open, .. } => {
                warn!(
                    call_id = %context.call_id,
                    "A Schedule node is read by the call manager; this runner takes the open branch"
                );
                Ok(FlowExecutionResult::Continue(open.clone()))
            }

            NodeType::HotelingLogin {
                device_id,
                max_attempts,
                next,
                on_failure,
            } => Ok(FlowExecutionResult::WaitingForHotelingLogin {
                device_id: device_id.clone(),
                max_attempts: *max_attempts,
                next: next.clone(),
                on_failure: on_failure.clone(),
            }),

            NodeType::VoicemailAccess {
                mailbox,
                max_pin_attempts,
                next,
                on_auth_failure,
            } => {
                let interpolated_mailbox = self.interpolate_variables(mailbox, context)?;

                info!(
                    call_id = %context.call_id,
                    mailbox = %interpolated_mailbox,
                    max_pin_attempts = %max_pin_attempts,
                    "Starting voicemail access IVR"
                );

                Ok(FlowExecutionResult::WaitingForVoicemailAccess {
                    mailbox: interpolated_mailbox,
                    max_pin_attempts: *max_pin_attempts,
                    next: next.clone(),
                    on_auth_failure: on_auth_failure.clone(),
                })
            }

            NodeType::Voicemail {
                mailbox,
                is_busy,
                max_duration,
                play_beep,
                next_on_success,
                next_on_error,
            } => {
                let interpolated_mailbox = self.interpolate_variables(mailbox, context)?;

                info!(
                    call_id = %context.call_id,
                    mailbox = %interpolated_mailbox,
                    is_busy = %is_busy,
                    max_duration = %max_duration,
                    "Recording voicemail message"
                );

                // Store voicemail configuration in context
                context.set_variable(
                    "__voicemail_mailbox".to_string(),
                    interpolated_mailbox.clone(),
                );
                context.set_variable("__voicemail_is_busy".to_string(), is_busy.to_string());
                context.set_variable(
                    "__voicemail_max_duration".to_string(),
                    max_duration.to_string(),
                );
                context.set_variable("__voicemail_play_beep".to_string(), play_beep.to_string());
                context.set_variable(
                    "__voicemail_next_on_success".to_string(),
                    next_on_success.clone(),
                );
                if let Some(error_node) = next_on_error {
                    context
                        .set_variable("__voicemail_next_on_error".to_string(), error_node.clone());
                }

                // Return WaitingForVoicemail to let flow handler invoke voicemail service
                Ok(FlowExecutionResult::WaitingForVoicemail {
                    mailbox: interpolated_mailbox,
                    is_busy: *is_busy,
                    max_duration: *max_duration,
                    play_beep: *play_beep,
                    next_on_success: next_on_success.clone(),
                    next_on_error: next_on_error.clone(),
                })
            }

            NodeType::Queue {
                queue_name,
                priority,
                next_on_answer,
                next_on_timeout,
                next_on_error,
            } => {
                let interpolated_queue = self.interpolate_variables(queue_name, context)?;

                info!(
                    call_id = %context.call_id,
                    queue = %interpolated_queue,
                    priority = %priority,
                    "Routing call to ACD queue"
                );

                // Store queue routing configuration in context for the flow handler
                context.set_variable("__queue_name".to_string(), interpolated_queue.clone());
                context.set_variable("__queue_priority".to_string(), priority.to_string());
                context.set_variable("__queue_next_on_answer".to_string(), next_on_answer.clone());
                if let Some(timeout_node) = next_on_timeout {
                    context
                        .set_variable("__queue_next_on_timeout".to_string(), timeout_node.clone());
                }
                if let Some(error_node) = next_on_error {
                    context.set_variable("__queue_next_on_error".to_string(), error_node.clone());
                }

                // Continue to the answer node; the flow handler will manage queue routing
                Ok(FlowExecutionResult::Continue(next_on_answer.clone()))
            }
        };

        match &result {
            Ok(FlowExecutionResult::Continue(next_node)) => {
                debug!(
                    call_id = %context.call_id,
                    current_node = %node_id,
                    next_node = %next_node,
                    "Node execution completed, continuing to next node"
                );
            }
            Ok(FlowExecutionResult::Hangup(reason)) => {
                info!(
                    call_id = %context.call_id,
                    reason = ?reason,
                    "Flow execution ending with hangup"
                );
            }
            Ok(FlowExecutionResult::Transcribe { action, .. }) => {
                debug!(
                    call_id = %context.call_id,
                    action = ?action,
                    "Flow execution paused, starting or stopping a transcript"
                );
            }
            Ok(FlowExecutionResult::WaitingForInput { .. }) => {
                debug!(
                    call_id = %context.call_id,
                    "Flow execution paused, waiting for input"
                );
            }
            Ok(FlowExecutionResult::WaitingForMedia) => {
                debug!(
                    call_id = %context.call_id,
                    "Flow execution paused, waiting for media playback"
                );
            }
            Ok(FlowExecutionResult::WaitingForDial { destination, .. }) => {
                debug!(
                    call_id = %context.call_id,
                    destination = %destination,
                    "Flow execution paused, waiting for outbound call"
                );
            }
            Ok(FlowExecutionResult::WaitingForVoicemail { mailbox, .. }) => {
                debug!(
                    call_id = %context.call_id,
                    mailbox = %mailbox,
                    "Flow execution paused, waiting for voicemail recording"
                );
            }
            Ok(FlowExecutionResult::WaitingForVoicemailAccess { mailbox, .. }) => {
                debug!(
                    call_id = %context.call_id,
                    mailbox = %mailbox,
                    "Flow execution paused, waiting for voicemail access IVR"
                );
            }
            Ok(FlowExecutionResult::WaitingForHotelingLogin { device_id, .. }) => {
                debug!(
                    call_id = %context.call_id,
                    device_id = %device_id,
                    "Flow execution paused, waiting for a hoteling login"
                );
            }
            Ok(FlowExecutionResult::WaitingForFork { destinations, .. }) => {
                debug!(
                    call_id = %context.call_id,
                    num_destinations = destinations.len(),
                    "Flow execution paused, waiting for parallel fork"
                );
            }
            Ok(FlowExecutionResult::WaitingForRecording { .. }) => {
                debug!(
                    call_id = %context.call_id,
                    "Flow execution paused, starting recording"
                );
            }
            Ok(FlowExecutionResult::StoppingRecording { .. }) => {
                debug!(
                    call_id = %context.call_id,
                    "Flow execution paused, stopping recording"
                );
            }
            Err(e) => {
                error!(
                    call_id = %context.call_id,
                    node_id = %node_id,
                    error = %e,
                    "Node execution failed"
                );
            }
        }

        result
    }

    /// Interpolate variables in a string (e.g., "Hello {{name}}")
    fn interpolate_variables(&self, template: &str, context: &FlowContext) -> Result<String> {
        let re = Regex::new(r"\{\{([^}]+)\}\}").unwrap();
        let mut result = template.to_string();

        for cap in re.captures_iter(template) {
            let var_name = cap[1].trim();
            let value = context.get_variable(var_name)?;
            result = result.replace(&cap[0], &value);
        }

        Ok(result)
    }

    /// Make an HTTP request: its status and body (at most
    /// [`HTTP_RESPONSE_LIMIT`] bytes, as text). An error only when no
    /// response came.
    #[allow(clippy::too_many_arguments)]
    async fn execute_http_request(
        &self,
        method: &crate::schema::HttpMethod,
        url: &str,
        body: Option<&str>,
        send_variables: bool,
        headers: &HashMap<String, String>,
        timeout: u32,
        context: &FlowContext,
    ) -> Result<(u16, String)> {
        use crate::schema::HttpMethod;

        let mut request = match method {
            HttpMethod::Get => self.http_client.get(url),
            HttpMethod::Post => self.http_client.post(url),
            HttpMethod::Put => self.http_client.put(url),
            HttpMethod::Delete => self.http_client.delete(url),
            HttpMethod::Patch => self.http_client.patch(url),
        };

        let mut content_type_set = false;
        for (key, value) in headers {
            let interpolated_value = self.interpolate_variables(value, context)?;
            content_type_set |= key.eq_ignore_ascii_case("content-type");
            request = request.header(key, interpolated_value);
        }

        if send_variables {
            let variables: serde_json::Map<String, serde_json::Value> = context
                .variables_snapshot()
                .into_iter()
                // The runner's own bookkeeping is not the flow's.
                .filter(|(name, _)| !name.starts_with("__"))
                .map(|(name, value)| (name, serde_json::Value::String(value)))
                .collect();
            if !content_type_set {
                request = request.header("content-type", "application/json");
            }
            request = request.body(serde_json::Value::Object(variables).to_string());
        } else if let Some(body_str) = body {
            let interpolated_body = self.interpolate_variables(body_str, context)?;
            request = request.body(interpolated_body);
        }

        request = request.timeout(std::time::Duration::from_secs(timeout as u64));

        let mut response = request
            .send()
            .await
            .map_err(|e| FlowError::HttpError(e.to_string()))?;
        let status = response.status().as_u16();
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|e| FlowError::HttpError(e.to_string()))?
        {
            if bytes.len() + chunk.len() > HTTP_RESPONSE_LIMIT {
                return Err(FlowError::HttpError(format!(
                    "the response is larger than {HTTP_RESPONSE_LIMIT} bytes"
                )));
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok((status, String::from_utf8_lossy(&bytes).into_owned()))
    }

    /// Evaluate a branch condition.
    fn evaluate_condition(&self, left: &str, operator: BranchOperator, right: &str) -> bool {
        match operator {
            BranchOperator::Equals => left == right,
            BranchOperator::NotEquals => left != right,
            BranchOperator::Contains => left.contains(right),
            BranchOperator::StartsWith => left.starts_with(right),
            BranchOperator::EndsWith => left.ends_with(right),
            BranchOperator::GreaterThan => left
                .parse::<f64>()
                .and_then(|l| right.parse::<f64>().map(|r| l > r))
                .unwrap_or(false),
            BranchOperator::LessThan => left
                .parse::<f64>()
                .and_then(|l| right.parse::<f64>().map(|r| l < r))
                .unwrap_or(false),
            BranchOperator::GreaterThanOrEqual => left
                .parse::<f64>()
                .and_then(|l| right.parse::<f64>().map(|r| l >= r))
                .unwrap_or(false),
            BranchOperator::LessThanOrEqual => left
                .parse::<f64>()
                .and_then(|l| right.parse::<f64>().map(|r| l <= r))
                .unwrap_or(false),
            BranchOperator::Exists => !left.is_empty(),
            BranchOperator::NotExists => left.is_empty(),
        }
    }
}

/// The largest HTTP response a flow reads.
pub const HTTP_RESPONSE_LIMIT: usize = 1024 * 1024;

/// The most variables one JSON response is unpacked into.
const HTTP_JSON_MAX_VARIABLES: usize = 256;

/// How deep a JSON response is unpacked.
const HTTP_JSON_MAX_DEPTH: usize = 16;

/// Store an HTTP response: the body under `name`, the status under
/// `<name>_status`, and a JSON body's values under `<name>.<path>`, the
/// previous response's dotted values cleared first.
fn store_http_response(context: &FlowContext, name: &str, status: u16, body: &str) {
    let prefix = format!("{name}.");
    context.remove_variables_with_prefix(&prefix);
    context.set_variable(name.to_string(), body.to_string());
    context.set_variable(format!("{name}_status"), status.to_string());
    let Ok(json) = serde_json::from_str::<serde_json::Value>(body) else {
        return;
    };
    let mut values = Vec::new();
    flatten_json(&json, name, 0, &mut values);
    for (path, value) in values {
        context.set_variable(path, value);
    }
}

/// A JSON value's leaves as dotted paths under `path`: object keys and
/// array indexes joined by `.`, strings as they are, other scalars as
/// JSON, null as empty. At most [`HTTP_JSON_MAX_VARIABLES`] leaves,
/// [`HTTP_JSON_MAX_DEPTH`] deep.
pub(crate) fn flatten_json(
    value: &serde_json::Value,
    path: &str,
    depth: usize,
    out: &mut Vec<(String, String)>,
) {
    if out.len() >= HTTP_JSON_MAX_VARIABLES || depth > HTTP_JSON_MAX_DEPTH {
        return;
    }
    match value {
        serde_json::Value::Object(map) => {
            for (key, child) in map {
                flatten_json(child, &format!("{path}.{key}"), depth + 1, out);
            }
        }
        serde_json::Value::Array(items) => {
            for (index, child) in items.iter().enumerate() {
                flatten_json(child, &format!("{path}.{index}"), depth + 1, out);
            }
        }
        // The whole body is already stored under the name itself.
        _ if depth == 0 => {}
        serde_json::Value::String(text) => out.push((path.to_string(), text.clone())),
        serde_json::Value::Null => out.push((path.to_string(), String::new())),
        other => out.push((path.to_string(), other.to_string())),
    }
}

impl Default for FlowEngine {
    fn default() -> Self {
        Self::new()
    }
}

/// Callback for answering a call (sends 200 OK)
pub type AnswerCallback = Arc<dyn Fn() -> BoxFuture<'static, anyhow::Result<()>> + Send + Sync>;

/// Callback for hanging up a call (sends BYE)
pub type HangupCallback =
    Arc<dyn Fn(Option<String>) -> BoxFuture<'static, anyhow::Result<()>> + Send + Sync>;

/// Flow execution context.
///
/// Holds the state of a flow execution including variables and execution history.
pub struct FlowContext {
    /// The flow being executed
    pub flow: Arc<FlowDefinition>,

    /// Call ID this flow is executing for
    pub call_id: CallId,

    /// Media session ID (for media operations like Play/Say)
    pub media_session_id: Option<CallId>,

    /// Current node being executed
    pub current_node: String,

    /// Variables and their values
    variables: Arc<RwLock<HashMap<String, String>>>,

    /// Execution path (list of visited nodes)
    pub execution_path: Vec<String>,

    /// Timestamp when execution started
    pub started_at: chrono::DateTime<chrono::Utc>,

    /// Callback to answer the call (send 200 OK)
    pub answer_callback: Option<AnswerCallback>,

    /// Callback to hangup the call (send BYE)
    pub hangup_callback: Option<HangupCallback>,
}

impl FlowContext {
    /// Create a new flow context.
    pub fn new(
        flow: Arc<FlowDefinition>,
        call_id: CallId,
        initial_variables: HashMap<String, String>,
    ) -> Self {
        // Extract string variables from flow definition (skip non-string/internal entries)
        let mut variables: HashMap<String, String> = flow
            .variables
            .iter()
            .filter(|(k, _)| !k.starts_with('_'))
            .filter_map(|(k, v)| match v {
                serde_json::Value::String(s) => Some((k.clone(), s.clone())),
                _ => None,
            })
            .collect();
        variables.extend(initial_variables);

        Self {
            flow: flow.clone(),
            call_id,
            media_session_id: None,
            current_node: flow.initial_node.clone(),
            variables: Arc::new(RwLock::new(variables)),
            execution_path: Vec::new(),
            started_at: chrono::Utc::now(),
            answer_callback: None,
            hangup_callback: None,
        }
    }

    /// Set the media session ID for this flow context.
    pub fn set_media_session_id(&mut self, session_id: CallId) {
        self.media_session_id = Some(session_id);
    }

    /// Set the answer callback.
    pub fn set_answer_callback(&mut self, callback: AnswerCallback) {
        self.answer_callback = Some(callback);
    }

    /// Set the hangup callback.
    pub fn set_hangup_callback(&mut self, callback: HangupCallback) {
        self.hangup_callback = Some(callback);
    }

    /// Get a variable value.
    pub fn get_variable(&self, name: &str) -> Result<String> {
        let variables = self.variables.read().unwrap();
        variables
            .get(name)
            .cloned()
            .ok_or_else(|| FlowError::VariableNotFound(name.to_string()))
    }

    /// Set a variable value.
    pub fn set_variable(&self, name: String, value: String) {
        let mut variables = self.variables.write().unwrap();
        variables.insert(name, value);
    }

    /// Remove a variable.
    pub fn remove_variable(&self, name: &str) {
        self.variables.write().unwrap().remove(name);
    }

    /// Remove every variable whose name starts with `prefix`.
    pub fn remove_variables_with_prefix(&self, prefix: &str) {
        self.variables
            .write()
            .unwrap()
            .retain(|name, _| !name.starts_with(prefix));
    }

    /// Every variable, as it is now.
    pub fn variables_snapshot(&self) -> HashMap<String, String> {
        self.variables.read().unwrap().clone()
    }

    /// Get all variables.
    pub async fn get_all_variables(&self) -> HashMap<String, String> {
        self.variables.read().unwrap().clone()
    }
}

/// A Gather's speech settings, as the runner needs them.
#[derive(Debug, Clone, Default)]
pub struct SpeechGather {
    /// What is listened for.
    pub input: crate::schema::GatherInput,
    /// The end-of-utterance silence.
    pub speech_timeout_ms: Option<u32>,
    /// Phrases to bias recognition.
    pub hints: Vec<String>,
    /// The language spoken.
    pub language: Option<String>,
    /// Phrase or keyword to node.
    pub transitions: HashMap<String, String>,
}

/// Result of executing a flow node.
#[derive(Debug, Clone)]
pub enum FlowExecutionResult {
    /// Continue to the next node
    Continue(String),

    /// Hangup the call
    Hangup(Option<String>),

    /// Waiting for the caller's input: digits, speech or either.
    WaitingForInput {
        variable: String,
        transitions: HashMap<String, String>,
        default: Option<String>,
        /// What is listened for, and how speech is recognised and routed.
        speech: SpeechGather,
        /// Node when speech cannot be listened for.
        on_error: Option<String>,
    },

    /// Start or stop a call's live transcript (Transcribe node).
    Transcribe {
        action: crate::schema::TranscribeAction,
        leg: crate::schema::TranscribeLeg,
        language: Option<String>,
        hints: Vec<String>,
        interim_results: bool,
        next: String,
        on_error: Option<String>,
    },

    /// Waiting for media playback to complete (Play/Say nodes)
    WaitingForMedia,

    /// Waiting for outbound call to complete (Dial node)
    WaitingForDial {
        destination: String,
        timeout_secs: u32,
        caller_id: Option<String>,
        max_duration_secs: u32,
        on_busy: Option<String>,
        on_no_answer: Option<String>,
        on_error: Option<String>,
        next: String,
    },

    /// Waiting for voicemail recording to complete (Voicemail node)
    WaitingForVoicemail {
        mailbox: String,
        is_busy: bool,
        max_duration: u32,
        play_beep: bool,
        next_on_success: String,
        next_on_error: Option<String>,
    },

    /// Waiting for fork (parallel dial) to complete
    WaitingForFork {
        destinations: Vec<String>,
        strategy: crate::schema::ForkStrategy,
        timeout_secs: u32,
        next: String,
        on_error: Option<String>,
    },

    /// Waiting for recording to start
    WaitingForRecording {
        format: crate::schema::RecordingFormat,
        variable: String,
        next: String,
        on_error: Option<String>,
    },

    /// Recording stop requested
    StoppingRecording {
        next: String,
        on_error: Option<String>,
    },

    /// Waiting for voicemail access IVR to complete
    WaitingForVoicemailAccess {
        mailbox: String,
        max_pin_attempts: u32,
        next: String,
        on_auth_failure: Option<String>,
    },

    /// Waiting for a hoteling login (extension and PIN by DTMF).
    WaitingForHotelingLogin {
        device_id: String,
        max_attempts: u32,
        next: String,
        on_failure: Option<String>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{FlowNode, NodeType};

    #[tokio::test]
    async fn test_simple_flow_execution() {
        let mut nodes = HashMap::new();
        nodes.insert(
            "start".to_string(),
            FlowNode {
                id: "start".to_string(),
                name: Some("Start".to_string()),
                node_type: NodeType::Answer {
                    next: "say".to_string(),
                    on_error: None,
                },
            },
        );

        nodes.insert(
            "say".to_string(),
            FlowNode {
                id: "say".to_string(),
                name: Some("Say Hello".to_string()),
                node_type: NodeType::Say {
                    text: "Hello, world!".to_string(),
                    voice: None,
                    language: "en-US".to_string(),
                    next: "hangup".to_string(),
                    on_error: None,
                },
            },
        );

        nodes.insert(
            "hangup".to_string(),
            FlowNode {
                id: "hangup".to_string(),
                name: Some("Hangup".to_string()),
                node_type: NodeType::Hangup { reason: None },
            },
        );

        let flow = FlowDefinition {
            id: "test-flow".to_string(),
            name: "Test Flow".to_string(),
            description: None,
            version: 1,
            initial_node: "start".to_string(),
            nodes,
            variables: HashMap::new(),
            tenant_id: None,
            enabled: true,
            created_at: None,
            updated_at: None,
        };

        let engine = FlowEngine::new();
        let mut context = engine
            .execute(Arc::new(flow), CallId::new(), HashMap::new())
            .await
            .unwrap();

        // Execute Answer node
        let result = engine.execute_node(&mut context, "start").await.unwrap();
        assert!(matches!(result, FlowExecutionResult::Continue(_)));

        // Execute Say node
        let result = engine.execute_node(&mut context, "say").await.unwrap();
        assert!(matches!(result, FlowExecutionResult::Continue(_)));

        // Execute Hangup node
        let result = engine.execute_node(&mut context, "hangup").await.unwrap();
        assert!(matches!(result, FlowExecutionResult::Hangup(_)));
    }

    // ---- Helpers ----

    fn test_engine() -> FlowEngine {
        FlowEngine::new()
    }

    fn test_context_with(vars: HashMap<String, String>) -> FlowContext {
        let flow = FlowDefinition {
            id: "test".to_string(),
            name: "Test".to_string(),
            description: None,
            version: 1,
            initial_node: "start".to_string(),
            nodes: HashMap::new(),
            variables: HashMap::new(),
            tenant_id: None,
            enabled: true,
            created_at: None,
            updated_at: None,
        };
        FlowContext::new(Arc::new(flow), CallId::new(), vars)
    }

    // ---- evaluate_condition tests ----

    #[test]
    fn test_condition_equals() {
        let engine = test_engine();
        assert!(engine.evaluate_condition("hello", BranchOperator::Equals, "hello"));
        assert!(!engine.evaluate_condition("hello", BranchOperator::Equals, "world"));
    }

    #[test]
    fn test_condition_not_equals() {
        let engine = test_engine();
        assert!(engine.evaluate_condition("hello", BranchOperator::NotEquals, "world"));
        assert!(!engine.evaluate_condition("hello", BranchOperator::NotEquals, "hello"));
    }

    #[test]
    fn test_condition_contains() {
        let engine = test_engine();
        assert!(engine.evaluate_condition("hello world", BranchOperator::Contains, "world"));
        assert!(!engine.evaluate_condition("hello", BranchOperator::Contains, "xyz"));
    }

    #[test]
    fn test_condition_starts_with() {
        let engine = test_engine();
        assert!(engine.evaluate_condition("hello world", BranchOperator::StartsWith, "hello"));
        assert!(!engine.evaluate_condition("hello world", BranchOperator::StartsWith, "world"));
    }

    #[test]
    fn test_condition_ends_with() {
        let engine = test_engine();
        assert!(engine.evaluate_condition("hello world", BranchOperator::EndsWith, "world"));
        assert!(!engine.evaluate_condition("hello world", BranchOperator::EndsWith, "hello"));
    }

    #[test]
    fn test_condition_greater_than() {
        let engine = test_engine();
        assert!(engine.evaluate_condition("10", BranchOperator::GreaterThan, "5"));
        assert!(!engine.evaluate_condition("5", BranchOperator::GreaterThan, "10"));
    }

    #[test]
    fn test_condition_less_than() {
        let engine = test_engine();
        assert!(engine.evaluate_condition("5", BranchOperator::LessThan, "10"));
        assert!(!engine.evaluate_condition("10", BranchOperator::LessThan, "5"));
    }

    #[test]
    fn test_condition_greater_than_or_equal() {
        let engine = test_engine();
        assert!(engine.evaluate_condition("10", BranchOperator::GreaterThanOrEqual, "10"));
        assert!(engine.evaluate_condition("11", BranchOperator::GreaterThanOrEqual, "10"));
        assert!(!engine.evaluate_condition("9", BranchOperator::GreaterThanOrEqual, "10"));
    }

    #[test]
    fn test_condition_less_than_or_equal() {
        let engine = test_engine();
        assert!(engine.evaluate_condition("10", BranchOperator::LessThanOrEqual, "10"));
        assert!(engine.evaluate_condition("9", BranchOperator::LessThanOrEqual, "10"));
        assert!(!engine.evaluate_condition("11", BranchOperator::LessThanOrEqual, "10"));
    }

    #[test]
    fn test_condition_exists() {
        let engine = test_engine();
        assert!(engine.evaluate_condition("something", BranchOperator::Exists, ""));
        assert!(!engine.evaluate_condition("", BranchOperator::Exists, ""));
    }

    #[test]
    fn test_condition_not_exists() {
        let engine = test_engine();
        assert!(engine.evaluate_condition("", BranchOperator::NotExists, ""));
        assert!(!engine.evaluate_condition("something", BranchOperator::NotExists, ""));
    }

    #[test]
    fn test_condition_numeric_non_parseable() {
        let engine = test_engine();
        // Non-numeric strings default to false for numeric operators
        assert!(!engine.evaluate_condition("abc", BranchOperator::GreaterThan, "def"));
        assert!(!engine.evaluate_condition("abc", BranchOperator::LessThan, "def"));
    }

    #[test]
    fn test_condition_float_comparison() {
        let engine = test_engine();
        assert!(engine.evaluate_condition("3.14", BranchOperator::GreaterThan, "2.71"));
        assert!(!engine.evaluate_condition("2.71", BranchOperator::GreaterThan, "3.14"));
    }

    #[test]
    fn test_condition_equal_numbers_not_greater() {
        let engine = test_engine();
        assert!(!engine.evaluate_condition("5", BranchOperator::GreaterThan, "5"));
        assert!(!engine.evaluate_condition("5", BranchOperator::LessThan, "5"));
    }

    // ---- interpolate_variables tests ----

    #[test]
    fn test_interpolate_simple() {
        let engine = test_engine();
        let mut vars = HashMap::new();
        vars.insert("name".to_string(), "World".to_string());
        let ctx = test_context_with(vars);

        let result = engine
            .interpolate_variables("Hello {{name}}", &ctx)
            .unwrap();
        assert_eq!(result, "Hello World");
    }

    #[test]
    fn test_interpolate_multiple_vars() {
        let engine = test_engine();
        let mut vars = HashMap::new();
        vars.insert("first".to_string(), "John".to_string());
        vars.insert("last".to_string(), "Doe".to_string());
        let ctx = test_context_with(vars);

        let result = engine
            .interpolate_variables("{{first}} {{last}}", &ctx)
            .unwrap();
        assert_eq!(result, "John Doe");
    }

    #[test]
    fn test_interpolate_missing_variable_errors() {
        let engine = test_engine();
        let ctx = test_context_with(HashMap::new());

        let result = engine.interpolate_variables("Hello {{missing}}", &ctx);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            FlowError::VariableNotFound(_)
        ));
    }

    #[test]
    fn test_interpolate_no_placeholders_unchanged() {
        let engine = test_engine();
        let ctx = test_context_with(HashMap::new());

        let result = engine
            .interpolate_variables("No placeholders here", &ctx)
            .unwrap();
        assert_eq!(result, "No placeholders here");
    }

    // ---- FlowContext tests ----

    #[test]
    fn test_context_set_get_variable() {
        let ctx = test_context_with(HashMap::new());
        ctx.set_variable("key".to_string(), "value".to_string());
        assert_eq!(ctx.get_variable("key").unwrap(), "value");
    }

    #[test]
    fn test_context_get_missing_variable_error() {
        let ctx = test_context_with(HashMap::new());
        let result = ctx.get_variable("nonexistent");
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            FlowError::VariableNotFound(_)
        ));
    }

    /// A one-request HTTP server answering `status` and `body`; the
    /// request it read (head and body) comes back on the receiver.
    async fn answer_once(
        status: u16,
        body: &'static str,
    ) -> (String, tokio::sync::oneshot::Receiver<String>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/hook", listener.local_addr().unwrap());
        let (tx, rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut buf = [0u8; 4096];
            loop {
                let n = stream.read(&mut buf).await.unwrap();
                request.extend_from_slice(&buf[..n]);
                let text = String::from_utf8_lossy(&request).to_string();
                if let Some(head_end) = text.find("\r\n\r\n") {
                    let length = text[..head_end]
                        .lines()
                        .find_map(|l| {
                            l.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(|v| v.trim().parse::<usize>().unwrap())
                        })
                        .unwrap_or(0);
                    if request.len() >= head_end + 4 + length || n == 0 {
                        break;
                    }
                }
            }
            let response = format!(
                "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).await.unwrap();
            let _ = tx.send(String::from_utf8_lossy(&request).to_string());
        });
        (url, rx)
    }

    fn http_flow(node: serde_json::Value) -> Arc<FlowDefinition> {
        Arc::new(
            serde_json::from_value(serde_json::json!({
                "id": "http", "name": "http", "version": 1, "initial_node": "call",
                "nodes": {
                    "call": node,
                    "ok": {"type": "hangup"},
                    "failed": {"type": "hangup"}
                }
            }))
            .unwrap(),
        )
    }

    async fn run_http(
        node: serde_json::Value,
        variables: &[(&str, &str)],
    ) -> (FlowContext, FlowExecutionResult) {
        let engine = FlowEngine::new();
        let flow = http_flow(node);
        let initial = variables
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        let mut context = engine.execute(flow, CallId::new(), initial).await.unwrap();
        let result = engine.execute_node(&mut context, "call").await.unwrap();
        (context, result)
    }

    fn went_to(result: &FlowExecutionResult) -> &str {
        match result {
            FlowExecutionResult::Continue(node) => node,
            other => panic!("not a continue: {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_json_response_is_unpacked_into_dotted_variables() {
        let (url, _) = answer_once(
            200,
            r#"{"customer": {"name": "Ada", "vip": true, "balance": 12.5, "note": null},
                "items": [{"id": "a1"}, {"id": "b2"}]}"#,
        )
        .await;
        let (context, result) = run_http(
            serde_json::json!({"type": "http", "method": "GET", "url": url,
                "next": "ok", "on_error": "failed"}),
            &[],
        )
        .await;
        assert_eq!(went_to(&result), "ok");
        let var = |name: &str| context.get_variable(name).unwrap();
        assert_eq!(var("http_response_status"), "200");
        assert_eq!(var("http_response.customer.name"), "Ada");
        assert_eq!(var("http_response.customer.vip"), "true");
        assert_eq!(var("http_response.customer.balance"), "12.5");
        assert_eq!(var("http_response.customer.note"), "");
        assert_eq!(var("http_response.items.1.id"), "b2");
        assert!(var("http_response").contains("\"Ada\""), "the raw body too");
    }

    #[tokio::test]
    async fn an_error_status_takes_on_error_and_without_it_goes_on() {
        let (url, _) = answer_once(503, r#"{"error": "busy"}"#).await;
        let (context, result) = run_http(
            serde_json::json!({"type": "http", "method": "GET", "url": url,
                "next": "ok", "on_error": "failed"}),
            &[],
        )
        .await;
        assert_eq!(went_to(&result), "failed");
        assert_eq!(context.get_variable("http_response_status").unwrap(), "503");
        assert_eq!(context.get_variable("http_response.error").unwrap(), "busy");

        let (url, _) = answer_once(404, "missing").await;
        let (_, result) = run_http(
            serde_json::json!({"type": "http", "method": "GET", "url": url, "next": "ok"}),
            &[],
        )
        .await;
        assert_eq!(went_to(&result), "ok", "no on_error: on to next, as before");

        // The flow builder's unset on_error is "", and is no node.
        let (url, _) = answer_once(500, "broken").await;
        let (_, result) = run_http(
            serde_json::json!({"type": "http", "method": "GET", "url": url,
                "next": "ok", "on_error": ""}),
            &[],
        )
        .await;
        assert_eq!(went_to(&result), "ok");
    }

    #[tokio::test]
    async fn send_variables_posts_the_flows_variables_as_json() {
        let (url, request) = answer_once(200, "{}").await;
        let (_, result) = run_http(
            serde_json::json!({"type": "http", "method": "POST", "url": url,
                "send_variables": true, "body": "ignored", "next": "ok"}),
            &[("caller", "+15125550100"), ("__tts_next_node", "x")],
        )
        .await;
        assert_eq!(went_to(&result), "ok");
        let request = request.await.unwrap();
        assert!(
            request
                .to_ascii_lowercase()
                .contains("content-type: application/json"),
            "{request}"
        );
        let body = &request[request.find("\r\n\r\n").unwrap() + 4..];
        let sent: serde_json::Value = serde_json::from_str(body).unwrap();
        assert_eq!(sent["caller"], "+15125550100");
        assert!(
            sent.get("__tts_next_node").is_none(),
            "the runner's own are not sent"
        );
    }

    #[tokio::test]
    async fn a_second_response_clears_the_first_ones_values() {
        let (url, _) = answer_once(200, r#"{"a": "1", "b": "2"}"#).await;
        let engine = FlowEngine::new();
        let node = serde_json::json!({"type": "http", "method": "GET", "url": url, "next": "ok"});
        let mut context = engine
            .execute(http_flow(node), CallId::new(), HashMap::new())
            .await
            .unwrap();
        engine.execute_node(&mut context, "call").await.unwrap();
        assert_eq!(context.get_variable("http_response.b").unwrap(), "2");

        let (url, _) = answer_once(200, r#"{"a": "3"}"#).await;
        let node = serde_json::json!({"type": "http", "method": "GET", "url": url, "next": "ok"});
        let mut context2 = engine
            .execute(http_flow(node), CallId::new(), context.variables_snapshot())
            .await
            .unwrap();
        engine.execute_node(&mut context2, "call").await.unwrap();
        assert_eq!(context2.get_variable("http_response.a").unwrap(), "3");
        assert!(
            context2.get_variable("http_response.b").is_err(),
            "a stale value"
        );
    }

    #[test]
    fn unpacking_stops_at_its_limits() {
        let wide = serde_json::Value::Array((0..1000).map(serde_json::Value::from).collect());
        let mut out = Vec::new();
        flatten_json(&wide, "r", 0, &mut out);
        assert_eq!(out.len(), HTTP_JSON_MAX_VARIABLES);

        let mut deep = serde_json::json!("bottom");
        for _ in 0..40 {
            deep = serde_json::json!({ "x": deep });
        }
        let mut out = Vec::new();
        flatten_json(&deep, "r", 0, &mut out);
        assert!(out.is_empty(), "too deep to reach");
    }
}
