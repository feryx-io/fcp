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

        /// How many times the audio is played; 0 and 1 are once (flows
        /// written before the count was honoured carry 0).
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

    /// Gather input from the caller: digits, speech, or either
    /// (`docs/DEVELOPER_SURFACE.md` phase 7).
    Gather {
        /// Optional prompt to play/say before gathering
        #[serde(skip_serializing_if = "Option::is_none")]
        prompt: Option<Box<GatherPrompt>>,

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

        /// Allow input during prompt playback (default: true): a digit, or
        /// the caller starting to speak, stops the prompt.
        #[serde(default = "default_barge_in")]
        barge_in: bool,

        /// What is listened for: `dtmf` (the default), `speech`, or
        /// `dtmf_speech` (whichever comes first).
        #[serde(default, skip_serializing_if = "GatherInput::is_dtmf")]
        input: GatherInput,

        /// The silence, in milliseconds, that ends the caller's utterance;
        /// the recognizer's default (800) when unset.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        speech_timeout_ms: Option<u32>,

        /// Words or phrases the caller is likely to say, to bias
        /// recognition.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        hints: Vec<String>,

        /// The language spoken (BCP 47, `en-US`); the recognizer's default
        /// when unset.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        language: Option<String>,

        /// Spoken transitions: a phrase or keyword to the node it leads
        /// to. A phrase matches when the caller's words contain it, case
        /// and punctuation aside; the longest matching phrase wins. What
        /// was said is in `speech`, the recognizer's confidence in
        /// `speech_confidence`.
        #[serde(default, skip_serializing_if = "HashMap::is_empty")]
        speech_transitions: HashMap<String, String>,

        /// How many times the caller is asked before the menu gives up:
        /// nothing heard, or a choice the menu does not have, plays the
        /// matching retry prompt and the menu again, until this many
        /// attempts are spent. Unset, or 1, asks once: the timeout and
        /// default branches are taken at once, as they always were.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_attempts: Option<u32>,

        /// Played before the menu is asked again when nothing was heard
        /// ("Sorry, I didn't hear anything.").
        #[serde(default, skip_serializing_if = "Option::is_none")]
        no_input_prompt: Option<Box<GatherPrompt>>,

        /// Played before the menu is asked again after a choice it does
        /// not have ("That is not a valid choice.").
        #[serde(default, skip_serializing_if = "Option::is_none")]
        invalid_prompt: Option<Box<GatherPrompt>>,

        /// Where the call goes when the attempts are spent; unset, the
        /// timeout branch for nothing heard and the default branch for an
        /// invalid choice, as with one attempt.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        on_max_attempts: Option<String>,
    },

    /// Send a call's live transcript as `call.transcript` events
    /// (`docs/DEVELOPER_SURFACE.md` phase 7), or stop sending it. The
    /// transcript runs until stopped, the flow ends, or the call does.
    Transcribe {
        /// `start` (the default) or `stop`.
        #[serde(default)]
        action: TranscribeAction,

        /// Whose speech: `caller` (the default), `callee`, or `both`.
        #[serde(default)]
        leg: TranscribeLeg,

        /// The language spoken; the recognizer's default when unset.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        language: Option<String>,

        /// Words or phrases to bias recognition.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        hints: Vec<String>,

        /// Send partial transcripts as well as finals.
        #[serde(default)]
        interim_results: bool,

        /// Next node to execute.
        next: String,

        /// Node to execute when the transcript cannot start (no
        /// recognizer configured, no media).
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

        /// Send the flow's variables as a JSON object, in place of `body`
        /// (`Content-Type: application/json` unless a header says
        /// otherwise).
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        send_variables: bool,

        /// Variable to store the response body in. A JSON response is also
        /// unpacked into dotted variables under it (`http_response.name`,
        /// `http_response.items.0.id`), and the status is stored in
        /// `<response_variable>_status`.
        #[serde(default = "default_http_response_var")]
        response_variable: String,

        /// Next node after request completes
        next: String,

        /// Node to execute on error: no response, or a status of 400 or
        /// above. Without it a request that could not be made fails the
        /// flow, and an error status goes on to `next`.
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

    /// Branch on a schedule (`docs/ENTERPRISE_CALLING.md` §3.5): open,
    /// closed, or a holiday (the closed branch when there is no holiday
    /// branch). Read by the call manager; a runner without the tenant's
    /// schedules takes the open branch.
    Schedule {
        /// The schedule's id.
        schedule_id: String,
        /// Node when the schedule is open.
        open: String,
        /// Node when it is closed.
        closed: String,
        /// Node on a holiday.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        holiday: Option<String>,
    },

    /// Log a user in on a hoteling host (`docs/ENTERPRISE_CALLING.md`
    /// §3.8): their extension then their PIN, by DTMF. Run by the call
    /// manager for `*88`; `hoteling_login_status` is `ok` or `failed`.
    HotelingLogin {
        /// The host device.
        device_id: String,

        /// Attempts before giving up.
        #[serde(default = "default_pin_attempts")]
        max_attempts: u32,

        /// Node after a login.
        next: String,

        /// Node when every attempt failed; `next` when unset.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        on_failure: Option<String>,
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

    /// Route call to an ACD queue
    Queue {
        /// Queue name or extension to route the call to
        queue_name: String,

        /// Priority for this caller in the queue (0 = normal, lower = higher priority)
        #[serde(default)]
        priority: u32,

        /// Node to execute when the call is answered by an agent
        next_on_answer: String,

        /// Node to execute if the queue times out or overflows
        #[serde(skip_serializing_if = "Option::is_none")]
        next_on_timeout: Option<String>,

        /// Node to execute on error (queue not found, etc.)
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

/// What a Gather listens for.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GatherInput {
    /// Digits only.
    #[default]
    Dtmf,
    /// Speech only.
    Speech,
    /// Digits or speech, whichever comes first.
    DtmfSpeech,
}

impl GatherInput {
    /// Whether digits are listened for.
    pub fn digits(self) -> bool {
        matches!(self, Self::Dtmf | Self::DtmfSpeech)
    }

    /// Whether speech is listened for.
    pub fn speech(self) -> bool {
        matches!(self, Self::Speech | Self::DtmfSpeech)
    }

    fn is_dtmf(&self) -> bool {
        *self == Self::Dtmf
    }
}

/// Start or stop a call's transcript.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TranscribeAction {
    #[default]
    Start,
    Stop,
}

/// Whose speech a transcript carries.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TranscribeLeg {
    /// The party the flow answered.
    #[default]
    Caller,
    /// The party a `Dial` reached.
    Callee,
    /// Both, each transcript naming its leg.
    Both,
}

/// The node a caller's words lead to: the longest phrase of
/// `transitions` their words contain, case and punctuation aside.
pub fn match_speech<'a>(
    transitions: &'a HashMap<String, String>,
    said: &str,
) -> Option<&'a String> {
    let said = normalize_words(said);
    let padded = format!(" {said} ");
    transitions
        .iter()
        .filter_map(|(phrase, node)| {
            let phrase = normalize_words(phrase);
            (!phrase.is_empty() && padded.contains(&format!(" {phrase} "))).then_some((
                phrase.len(),
                phrase,
                node,
            ))
        })
        // The longest phrase, then the first by name, so a tie is stable.
        .max_by(|a, b| a.0.cmp(&b.0).then_with(|| b.1.cmp(&a.1)))
        .map(|(_, _, node)| node)
}

/// Lowercase words separated by single spaces, punctuation dropped.
fn normalize_words(text: &str) -> String {
    text.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '\'' {
                c.to_lowercase().next().unwrap_or(c)
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
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

fn default_barge_in() -> bool {
    true
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
    fn spoken_transitions_match_whole_words_and_the_longest_phrase() {
        let transitions: HashMap<String, String> = [
            ("sales", "to_sales"),
            ("billing", "to_billing"),
            ("billing question", "to_billing_question"),
            ("Customer Service!", "to_service"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
        let node = |said| match_speech(&transitions, said).map(String::as_str);
        assert_eq!(node("Sales, please."), Some("to_sales"));
        assert_eq!(
            node("I have a billing question"),
            Some("to_billing_question")
        );
        assert_eq!(node("billing"), Some("to_billing"));
        assert_eq!(node("customer service"), Some("to_service"));
        // A word inside another word is not that word.
        assert_eq!(node("wholesales"), None);
        assert_eq!(node(""), None);
    }

    #[test]
    fn a_gather_written_before_speech_reads_as_digits() {
        let node: FlowNode = serde_json::from_value(serde_json::json!({
            "type": "gather",
            "transitions": {"1": "a"},
        }))
        .unwrap();
        let NodeType::Gather { input, hints, .. } = &node.node_type else {
            panic!("a gather");
        };
        assert_eq!(*input, GatherInput::Dtmf);
        assert!(hints.is_empty());
        // And it is written back without the new fields.
        let json = serde_json::to_value(&node).unwrap();
        assert!(json.get("input").is_none());
        assert!(json.get("speech_transitions").is_none());
    }

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
