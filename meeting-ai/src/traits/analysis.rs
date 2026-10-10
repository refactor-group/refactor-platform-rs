//! Analysis provider trait.

use crate::types::analysis::{Completion, Request};
use crate::Error;
use async_trait::async_trait;

/// Abstraction for LLM services that return structured JSON output.
///
/// Implementations send instructions plus content and return output conforming to
/// the request's JSON schema, along with token usage.
#[async_trait]
pub trait Provider: Send + Sync {
    /// Runs one structured completion and returns the parsed output.
    async fn complete(&self, request: Request) -> std::result::Result<Completion, Error>;

    /// Lowercase identifier, e.g. "assembly_ai_llm_gateway".
    fn provider_id(&self) -> &str;
}
