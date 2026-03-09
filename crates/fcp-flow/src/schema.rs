//! Flow definition schema for JSON-based call flows.
//!
//! Defines the structure of call flows, including nodes, transitions, and variables.
//! Inspired by Twilio Studio but designed for self-hosted deployment.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A complete call flow definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowDefinition {
    /// Unique identifier for this flow
    pub id: String,

    /// Human-readable name
    pub name: String,

    /// Optional description
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Version number (for flow versioning)
    pub version: u32,

    /// Initial node to execute when flow starts
    pub initial_node: String,

    /// All nodes in the flow, indexed by node ID
    pub nodes: HashMap<String, FlowNode>,

    /// Global variables and their default values
    #[serde(default)]
    pub variables: HashMap<String, serde_json::Value>,

    /// Tenant ID this flow belongs to
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tenant_id: Option<String>,

    /// Whether this flow is enabled
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Optional webhook URL for flow events
    #[serde(skip_serializing_if = "Option::is_none")]
    pub webhook_url: Option<String>,

    /// Timestamp when flow was created
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,

    /// Timestamp when flow was last updated
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<chrono::DateTime<chrono::Utc>>,
}

fn default_true() -> bool {
    true
}

/// A node in the call flow.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowNode {
    /// Unique identifier for this node
    #[serde(default)]
    pub id: String,

    /// Human-readable name for this node
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    /// Node type and configuration
    #[serde(flatten)]
    pub node_type: NodeType,
}

/// Types of nodes available in a flow.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum NodeType {
    /// Answer the incoming call
    Answer {
        /// Next node to execute
        next: String,

        /// Node to execute if answering fails
        #[serde(skip_serializing_if = "Option::is_none")]
        on_error: Option<String>,
    },

    /// Play audio to the caller
    Play {
        /// URL or path to audio file
        url: String,

        /// Number of times to loop (0 = infinite)
        #[serde(default)]
        loop_count: u32,

        /// Next node after playback completes
        next: String,

        /// Node to execute on error
        #[serde(skip_serializing_if = "Option::is_none")]
        on_error: Option<String>,
    },

    /// Say text using TTS (Text-to-Speech)
    Say {
        /// Text to speak (supports variable interpolation)
        text: String,

        /// Voice to use (e.g., "alice", "man", "woman")
        #[serde(skip_serializing_if = "Option::is_none")]
        voice: Option<String>,

        /// Language code (e.g., "en-US", "es-ES")
        #[serde(default = "default_language")]
        language: String,

        /// Next node after TTS completes
        next: String,

        /// Node to execute on error
        #[serde(skip_serializing_if = "Option::is_none")]
        on_error: Option<String>,
    },

    /// Gather DTMF input from caller
    Gather {
        /// Optional prompt to play/say before gathering
        #[serde(skip_serializing_if = "Option::is_none")]
        prompt: Option<GatherPrompt>,

        /// Number of digits to gather (0 = variable length)
        #[serde(default)]
        num_digits: u32,

        /// Timeout in seconds for first digit
        #[serde(default = "default_timeout")]
        timeout: u32,

        /// Finish on key (e.g., "#")
        #[serde(skip_serializing_if = "Option::is_none")]
        finish_on_key: Option<char>,

        /// Variable name to store gathered digits
        #[serde(default = "default_gather_var")]
        variable: String,

        /// Transitions based on gathered input
        transitions: HashMap<String, String>,

        /// Default transition if no match
        #[serde(skip_serializing_if = "Option::is_none")]
        default: Option<String>,

        /// Node to execute on timeout
        #[serde(skip_serializing_if = "Option::is_none")]
        on_timeout: Option<String>,

        /// Node to execute on error (e.g., prompt synthesis failure)
        #[serde(skip_serializing_if = "Option::is_none")]
        on_error: Option<String>,
    },

    /// Dial/bridge to another destination
    Dial {
        /// Destination to dial (SIP URI or phone number)
        destination: String,

        /// Timeout in seconds
        #[serde(default = "default_dial_timeout")]
        timeout: u32,

        /// Caller ID to present
        #[serde(skip_serializing_if = "Option::is_none")]
        caller_id: Option<String>,

        /// Maximum call duration in seconds (0 = unlimited)
        #[serde(default)]
        max_duration: u32,

        /// Next node after call ends
        next: String,

        /// Node to execute if dial fails
        #[serde(skip_serializing_if = "Option::is_none")]
        on_busy: Option<String>,

        /// Node to execute if no answer
        #[serde(skip_serializing_if = "Option::is_none")]
        on_no_answer: Option<String>,

        /// Node to execute on other errors
        #[serde(skip_serializing_if = "Option::is_none")]
        on_error: Option<String>,
    },

    /// Hangup the call
    Hangup {
        /// Optional reason for hangup
        #[serde(skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },

    /// Make an HTTP request
    Http {
        /// HTTP method (GET, POST, PUT, DELETE)
        method: HttpMethod,

        /// URL to request
        url: String,

        /// Optional request body (supports variable interpolation)
        #[serde(skip_serializing_if = "Option::is_none")]
        body: Option<String>,

        /// Request headers
        #[serde(default)]
        headers: HashMap<String, String>,

        /// Timeout in seconds
        #[serde(default = "default_http_timeout")]
        timeout: u32,

        /// Variable name to store response body
        #[serde(default = "default_http_response_var")]
        response_variable: String,

        /// Next node after request completes
        next: String,

        /// Node to execute on error
        #[serde(skip_serializing_if = "Option::is_none")]
        on_error: Option<String>,
    },

    /// Conditional branch based on variables
    Branch {
        /// Variable to evaluate
        variable: String,

        /// Operator for comparison
        operator: BranchOperator,

        /// Value to compare against
        value: String,

        /// Node to execute if condition is true
        if_true: String,

        /// Node to execute if condition is false
        if_false: String,

        /// Node to execute on error (e.g., variable not found)
        #[serde(skip_serializing_if = "Option::is_none")]
        on_error: Option<String>,
    },

    /// Set a variable value
    SetVariable {
        /// Variable name
        variable: String,

        /// Value (supports interpolation)
        value: String,

        /// Next node to execute
        next: String,

        /// Node to execute on error (e.g., interpolation failure)
        #[serde(skip_serializing_if = "Option::is_none")]
        on_error: Option<String>,
    },

    /// Fork execution to multiple destinations
    Fork {
        /// Destinations to dial in parallel
        destinations: Vec<String>,

        /// Strategy (first, all)
        #[serde(default = "default_fork_strategy")]
        strategy: ForkStrategy,

        /// Timeout in seconds
        #[serde(default = "default_dial_timeout")]
        timeout: u32,

        /// Next node after fork completes
        next: String,

        /// Node to execute if all fail
        #[serde(skip_serializing_if = "Option::is_none")]
        on_error: Option<String>,
    },

    /// Start call recording
    StartRecording {
        /// Format (wav, mp3)
        #[serde(default = "default_recording_format")]
        format: RecordingFormat,

        /// Variable to store recording URL
        #[serde(default = "default_recording_var")]
        variable: String,

        /// Next node to execute
        next: String,

        /// Node to execute on error (e.g., media manager unavailable)
        #[serde(skip_serializing_if = "Option::is_none")]
        on_error: Option<String>,
    },

    /// Stop call recording
    StopRecording {
        /// Next node to execute
        next: String,

        /// Node to execute on error
        #[serde(skip_serializing_if = "Option::is_none")]
        on_error: Option<String>,
    },

    /// Pause execution for a duration
    Pause {
        /// Duration in seconds
        duration: u32,

        /// Next node to execute
        next: String,
    },

    /// Log a message (for debugging)
    Log {
        /// Message to log (supports interpolation)
        message: String,

        /// Log level
        #[serde(default = "default_log_level")]
        level: LogLevel,

        /// Next node to execute
        next: String,
    },

    /// Access voicemail IVR (PIN entry, message playback, DTMF menu)
    VoicemailAccess {
        /// Target mailbox (variable-interpolated). Empty = derive from caller.
        mailbox: String,

        /// Max PIN entry attempts before hanging up
        #[serde(default = "default_pin_attempts")]
        max_pin_attempts: u32,

        /// Node to go to after the user hangs up or exits the menu
        next: String,

        /// Node to go to on auth failure (too many wrong PINs)
        #[serde(skip_serializing_if = "Option::is_none")]
        on_auth_failure: Option<String>,
    },

    /// Leave a voicemail message
    Voicemail {
        /// Extension or mailbox ID to leave message for
        /// Supports variable interpolation (e.g., "{{dialedNumber}}")
        mailbox: String,

        /// Play busy greeting instead of unavailable greeting
        #[serde(default)]
        is_busy: bool,

        /// Maximum recording duration in seconds
        #[serde(default = "default_voicemail_duration")]
        max_duration: u32,

        /// Play beep before recording
        #[serde(default = "default_true")]
        play_beep: bool,

        /// Node to execute after message is successfully recorded
        next_on_success: String,

        /// Node to execute on error (mailbox full, disabled, etc.)
        #[serde(skip_serializing_if = "Option::is_none")]
        next_on_error: Option<String>,
    },
}

/// Prompt configuration for Gather node
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum GatherPrompt {
    /// Play audio file
    Play { url: String },

    /// Say text with TTS
    Say {
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        voice: Option<String>,
        #[serde(default = "default_language")]
        language: String,
    },
}

/// HTTP methods
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum HttpMethod {
    Get,
    Post,
    Put,
    Delete,
    Patch,
}

/// Branch operators for conditional logic
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BranchOperator {
    Equals,
    NotEquals,
    Contains,
    StartsWith,
    EndsWith,
    GreaterThan,
    LessThan,
    GreaterThanOrEqual,
    LessThanOrEqual,
    Exists,
    NotExists,
}

/// Fork strategy
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ForkStrategy {
    /// Connect to first destination that answers
    First,

    /// Dial all destinations simultaneously
    All,
}

/// Recording format
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RecordingFormat {
    Wav,
    Mp3,
}

/// Log level
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

// Default value functions
fn default_language() -> String {
    "en-US".to_string()
}

fn default_timeout() -> u32 {
    5
}

fn default_dial_timeout() -> u32 {
    30
}

fn default_http_timeout() -> u32 {
    10
}

fn default_gather_var() -> String {
    "digits".to_string()
}

fn default_http_response_var() -> String {
    "http_response".to_string()
}

fn default_fork_strategy() -> ForkStrategy {
    ForkStrategy::First
}

fn default_recording_format() -> RecordingFormat {
    RecordingFormat::Wav
}

fn default_recording_var() -> String {
    "recording_url".to_string()
}

fn default_log_level() -> LogLevel {
    LogLevel::Info
}

fn default_voicemail_duration() -> u32 {
    300 // 5 minutes
}

fn default_pin_attempts() -> u32 {
    3
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_flow_serialization() {
        let mut nodes = HashMap::new();
        nodes.insert(
            "start".to_string(),
            FlowNode {
                id: "start".to_string(),
                name: Some("Answer Call".to_string()),
                node_type: NodeType::Answer {
                    next: "welcome".to_string(),
                    on_error: None,
                },
            },
        );

        nodes.insert(
            "welcome".to_string(),
            FlowNode {
                id: "welcome".to_string(),
                name: Some("Welcome Message".to_string()),
                node_type: NodeType::Say {
                    text: "Welcome to our service".to_string(),
                    voice: None,
                    language: "en-US".to_string(),
                    next: "menu".to_string(),
                    on_error: None,
                },
            },
        );

        let flow = FlowDefinition {
            id: "test-flow".to_string(),
            name: "Test Flow".to_string(),
            description: Some("A test flow".to_string()),
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

        let json = serde_json::to_string_pretty(&flow).unwrap();
        println!("Flow JSON:\n{}", json);

        let deserialized: FlowDefinition = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.id, "test-flow");
        assert_eq!(deserialized.nodes.len(), 2);
    }
}
