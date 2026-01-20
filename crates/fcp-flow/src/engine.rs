//! Flow execution engine.
//!
//! Executes flow definitions step-by-step, managing state and variable interpolation.

use crate::schema::{BranchOperator, FlowDefinition, NodeType};
use crate::{FlowError, Result};
use fcp_core::CallId;
use regex::Regex;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

/// Flow execution engine.
pub struct FlowEngine {
    /// HTTP client for HTTP nodes
    http_client: reqwest::Client,
}

impl FlowEngine {
    /// Create a new flow engine.
    pub fn new() -> Self {
        let http_client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .expect("Failed to build HTTP client");

        Self { http_client }
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
                info!(
                    call_id = %context.call_id,
                    text = %interpolated_text,
                    voice = ?voice,
                    language = %language,
                    "Speaking text (TTS)"
                );

                // In real implementation, this would trigger TTS
                Ok(FlowExecutionResult::Continue(next.clone()))
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
                    "Gathering DTMF input"
                );

                // In real implementation, this would wait for DTMF input
                // For now, we return WaitingForInput to signal the caller
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
                let interpolated_dest = self.interpolate_variables(destination, context)?;
                info!(
                    call_id = %context.call_id,
                    destination = %interpolated_dest,
                    timeout = %timeout,
                    "Dialing destination"
                );

                // In real implementation, this would create a B-leg
                Ok(FlowExecutionResult::Continue(next.clone()))
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

/// Flow execution context.
///
/// Holds the state of a flow execution including variables and execution history.
pub struct FlowContext {
    /// The flow being executed
    pub flow: Arc<FlowDefinition>,

    /// Call ID this flow is executing for
    pub call_id: CallId,

    /// Current node being executed
    pub current_node: String,

    /// Variables and their values
    variables: Arc<RwLock<HashMap<String, String>>>,

    /// Execution path (list of visited nodes)
    pub execution_path: Vec<String>,

    /// Timestamp when execution started
    pub started_at: chrono::DateTime<chrono::Utc>,
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
            current_node: flow.initial_node.clone(),
            variables: Arc::new(RwLock::new(variables)),
            execution_path: Vec::new(),
            started_at: chrono::Utc::now(),
        }
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
