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
use tokio::sync::RwLock;
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
            NodeType::Answer { next } => {
                info!(call_id = %context.call_id, "Answering call");
                Ok(FlowExecutionResult::Continue(next.clone()))
            }

            NodeType::Play {
                url,
                loop_count,
                next,
                on_error,
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
            } => {
                info!(
                    call_id = %context.call_id,
                    num_digits = %num_digits,
                    timeout = %timeout,
                    has_prompt = prompt.is_some(),
                    "Gathering DTMF input"
                );

                // Handle prompt if present
                if let Some(gather_prompt) = prompt {
                    use crate::schema::GatherPrompt;

                    match gather_prompt {
                        GatherPrompt::Play { url } => {
                            // Store Play prompt information for flow handler
                            context.set_variable("__gather_prompt_type".to_string(), "play".to_string());
                            context.set_variable("__gather_prompt_url".to_string(), url.clone());
                        }
                        GatherPrompt::Say { text, voice, language } => {
                            // If TTS service is available, synthesize the prompt
                            if let Some(tts_service) = &self.tts_service {
                                let interpolated_text = self.interpolate_variables(text, context)?;

                                match tts_service
                                    .synthesize(&interpolated_text, Some(language), voice.as_deref())
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

                // Store gather configuration in context
                context.set_variable("__gather_num_digits".to_string(), num_digits.to_string());
                context.set_variable("__gather_timeout".to_string(), timeout.to_string());
                if let Some(key) = finish_on_key {
                    context.set_variable("__gather_finish_on_key".to_string(), key.to_string());
                }
                if let Some(timeout_node) = on_timeout {
                    context.set_variable("__gather_on_timeout".to_string(), timeout_node.clone());
                }

                // Return WaitingForInput with transition information
                Ok(FlowExecutionResult::WaitingForInput {
                    variable: variable.clone(),
                    transitions: transitions.clone(),
                    default: default.clone(),
                })
            }

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

                match self.execute_http_request(method, &interpolated_url, body.as_deref(), headers, *timeout, context).await {
                    Ok(response_body) => {
                        context.set_variable(response_variable.clone(), response_body);
                        Ok(FlowExecutionResult::Continue(next.clone()))
                    }
                    Err(e) => {
                        error!(error = %e, "HTTP request failed");
                        if let Some(error_node) = on_error {
                            Ok(FlowExecutionResult::Continue(error_node.clone()))
                        } else {
                            Err(e)
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
            } => {
                let var_value = context.get_variable(variable)?;
                let interpolated_compare_value = self.interpolate_variables(value, context)?;

                let condition_met = self.evaluate_condition(
                    &var_value,
                    *operator,
                    &interpolated_compare_value,
                );

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

            NodeType::SetVariable {
                variable,
                value,
                next,
            } => {
                let interpolated_value = self.interpolate_variables(value, context)?;
                debug!(
                    call_id = %context.call_id,
                    variable = %variable,
                    value = %interpolated_value,
                    "Setting variable"
                );
                context.set_variable(variable.clone(), interpolated_value);
                Ok(FlowExecutionResult::Continue(next.clone()))
            }

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
                    "Forking to multiple destinations"
                );

                // In real implementation, this would dial multiple destinations
                Ok(FlowExecutionResult::Continue(next.clone()))
            }

            NodeType::StartRecording {
                format,
                variable,
                next,
            } => {
                info!(
                    call_id = %context.call_id,
                    format = ?format,
                    "Starting recording"
                );

                // In real implementation, this would start media recording
                context.set_variable(variable.clone(), format!("recording://placeholder"));
                Ok(FlowExecutionResult::Continue(next.clone()))
            }

            NodeType::StopRecording { next } => {
                info!(call_id = %context.call_id, "Stopping recording");
                Ok(FlowExecutionResult::Continue(next.clone()))
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
                    crate::schema::LogLevel::Debug => debug!(call_id = %context.call_id, "{}", interpolated_message),
                    crate::schema::LogLevel::Info => info!(call_id = %context.call_id, "{}", interpolated_message),
                    crate::schema::LogLevel::Warn => warn!(call_id = %context.call_id, "{}", interpolated_message),
                    crate::schema::LogLevel::Error => error!(call_id = %context.call_id, "{}", interpolated_message),
                }
                Ok(FlowExecutionResult::Continue(next.clone()))
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
                context.set_variable("__voicemail_mailbox".to_string(), interpolated_mailbox.clone());
                context.set_variable("__voicemail_is_busy".to_string(), is_busy.to_string());
                context.set_variable("__voicemail_max_duration".to_string(), max_duration.to_string());
                context.set_variable("__voicemail_play_beep".to_string(), play_beep.to_string());
                context.set_variable("__voicemail_next_on_success".to_string(), next_on_success.clone());
                if let Some(error_node) = next_on_error {
                    context.set_variable("__voicemail_next_on_error".to_string(), error_node.clone());
                }

                // Return WaitingForVoicemail to let flow handler invoke voicemail service
                // The flow handler will need to integrate with VoicemailService
                info!(
                    call_id = %context.call_id,
                    "Voicemail node configured, waiting for voicemail recording to complete"
                );

                // For now, we'll just continue to next_on_success
                // In full implementation, this would return a new FlowExecutionResult variant
                // like WaitingForVoicemail
                Ok(FlowExecutionResult::Continue(next_on_success.clone()))
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

    /// Execute an HTTP request.
    async fn execute_http_request(
        &self,
        method: &crate::schema::HttpMethod,
        url: &str,
        body: Option<&str>,
        headers: &HashMap<String, String>,
        timeout: u32,
        context: &FlowContext,
    ) -> Result<String> {
        use crate::schema::HttpMethod;

        let mut request = match method {
            HttpMethod::Get => self.http_client.get(url),
            HttpMethod::Post => self.http_client.post(url),
            HttpMethod::Put => self.http_client.put(url),
            HttpMethod::Delete => self.http_client.delete(url),
            HttpMethod::Patch => self.http_client.patch(url),
        };

        // Add headers
        for (key, value) in headers {
            let interpolated_value = self.interpolate_variables(value, context)?;
            request = request.header(key, interpolated_value);
        }

        // Add body if present
        if let Some(body_str) = body {
            let interpolated_body = self.interpolate_variables(body_str, context)?;
            request = request.body(interpolated_body);
        }

        // Set timeout
        request = request.timeout(std::time::Duration::from_secs(timeout as u64));

        // Execute request
        let response = request
            .send()
            .await
            .map_err(|e| FlowError::HttpError(e.to_string()))?;

        let body_text = response
            .text()
            .await
            .map_err(|e| FlowError::HttpError(e.to_string()))?;

        Ok(body_text)
    }

    /// Evaluate a branch condition.
    fn evaluate_condition(&self, left: &str, operator: BranchOperator, right: &str) -> bool {
        match operator {
            BranchOperator::Equals => left == right,
            BranchOperator::NotEquals => left != right,
            BranchOperator::Contains => left.contains(right),
            BranchOperator::StartsWith => left.starts_with(right),
            BranchOperator::EndsWith => left.ends_with(right),
            BranchOperator::GreaterThan => {
                left.parse::<f64>()
                    .and_then(|l| right.parse::<f64>().map(|r| l > r))
                    .unwrap_or(false)
            }
            BranchOperator::LessThan => {
                left.parse::<f64>()
                    .and_then(|l| right.parse::<f64>().map(|r| l < r))
                    .unwrap_or(false)
            }
            BranchOperator::GreaterThanOrEqual => {
                left.parse::<f64>()
                    .and_then(|l| right.parse::<f64>().map(|r| l >= r))
                    .unwrap_or(false)
            }
            BranchOperator::LessThanOrEqual => {
                left.parse::<f64>()
                    .and_then(|l| right.parse::<f64>().map(|r| l <= r))
                    .unwrap_or(false)
            }
            BranchOperator::Exists => !left.is_empty(),
            BranchOperator::NotExists => left.is_empty(),
        }
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
pub type HangupCallback = Arc<dyn Fn(Option<String>) -> BoxFuture<'static, anyhow::Result<()>> + Send + Sync>;

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
        let mut variables = flow.variables.clone();
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
        let variables = self.variables.blocking_read();
        variables
            .get(name)
            .cloned()
            .ok_or_else(|| FlowError::VariableNotFound(name.to_string()))
    }

    /// Set a variable value.
    pub fn set_variable(&self, name: String, value: String) {
        let mut variables = self.variables.blocking_write();
        variables.insert(name, value);
    }

    /// Get all variables.
    pub async fn get_all_variables(&self) -> HashMap<String, String> {
        self.variables.read().await.clone()
    }
}

/// Result of executing a flow node.
#[derive(Debug, Clone)]
pub enum FlowExecutionResult {
    /// Continue to the next node
    Continue(String),

    /// Hangup the call
    Hangup(Option<String>),

    /// Waiting for user input (DTMF)
    WaitingForInput {
        variable: String,
        transitions: HashMap<String, String>,
        default: Option<String>,
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
            webhook_url: None,
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
        let result = engine
            .execute_node(&mut context, "hangup")
            .await
            .unwrap();
        assert!(matches!(result, FlowExecutionResult::Hangup(_)));
    }
}
