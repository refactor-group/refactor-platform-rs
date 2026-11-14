//! Types for structured LLM completions.

/// One structured completion request.
#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    /// Sent as the system message.
    pub instructions: String,
    /// Sent as the user message.
    pub content: String,
    /// Name of the output JSON schema.
    pub schema_name: String,
    /// JSON schema the output must conform to.
    pub output_schema: serde_json::Value,
    /// Upper bound on generated tokens.
    pub max_output_tokens: u32,
}

/// The model's structured answer plus what it cost.
#[derive(Debug, Clone, PartialEq)]
pub struct Completion {
    /// Parsed JSON the model returned.
    pub output: serde_json::Value,
    /// Model the provider reports having used.
    pub model: String,
    /// Token usage for this completion.
    pub usage: Usage,
    /// Provider request id, when reported.
    pub request_id: Option<String>,
}

/// Token counts for one completion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Usage {
    pub input_tokens: u32,
    pub output_tokens: u32,
}
