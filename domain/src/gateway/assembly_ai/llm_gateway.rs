//! AssemblyAI LLM Gateway client for structured chat completions.

use std::time::Duration;

use async_trait::async_trait;
use log::*;
use meeting_ai::traits::analysis;
use meeting_ai::types::analysis::{Completion, Request, Usage};
use reqwest::StatusCode;
use serde::Deserialize;
use serde_json::json;

use crate::error::{DomainErrorKind, Error, InternalErrorKind};

const PROVIDER_ID: &str = "assembly_ai_llm_gateway";
const DEFAULT_RETRY_AFTER_SECONDS: u64 = 30;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(180);
const TRUNCATED_FINISH_REASONS: [&str; 2] = ["max_tokens", "length"];

/// AssemblyAI LLM Gateway client. Built once at startup and shared.
#[derive(Clone)]
pub struct Provider {
    client: reqwest::Client,
    base_url: String,
    model: String,
}

#[derive(Debug, Deserialize)]
struct ChatResponse {
    request_id: Option<String>,
    model: Option<String>,
    #[serde(default)]
    choices: Vec<Choice>,
    #[serde(default)]
    usage: UsageBody,
}

#[derive(Debug, Deserialize)]
struct Choice {
    finish_reason: Option<String>,
    message: Option<Message>,
}

#[derive(Debug, Deserialize)]
struct Message {
    content: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct UsageBody {
    input_tokens: Option<u32>,
    prompt_tokens: Option<u32>,
    output_tokens: Option<u32>,
    completion_tokens: Option<u32>,
}

impl From<UsageBody> for Usage {
    fn from(body: UsageBody) -> Self {
        Usage {
            input_tokens: body.input_tokens.or(body.prompt_tokens).unwrap_or(0),
            output_tokens: body.output_tokens.or(body.completion_tokens).unwrap_or(0),
        }
    }
}

/// Error body shapes the gateway may return.
#[derive(Debug, Deserialize)]
struct ErrorBody {
    message: Option<String>,
    error: Option<ErrorDetail>,
}

#[derive(Debug, Deserialize)]
struct ErrorDetail {
    message: Option<String>,
}

impl ErrorBody {
    fn into_message(self) -> Option<String> {
        self.error.and_then(|e| e.message).or(self.message)
    }
}

impl Provider {
    /// Builds a client that authenticates with the raw API key.
    pub fn new(api_key: &str, base_url: &str, model: &str) -> Result<Self, Error> {
        let mut auth_value = reqwest::header::HeaderValue::from_str(api_key).map_err(|e| {
            warn!(
                "Failed to create AssemblyAI LLM Gateway auth header: {:?}",
                e
            );
            Error {
                source: Some(Box::new(e)),
                error_kind: DomainErrorKind::Internal(InternalErrorKind::Other(
                    "Invalid AssemblyAI API key format".to_string(),
                )),
            }
        })?;
        auth_value.set_sensitive(true);

        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(reqwest::header::AUTHORIZATION, auth_value);

        let client = reqwest::Client::builder()
            .use_rustls_tls()
            .default_headers(headers)
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
            .build()?;

        Ok(Self {
            client,
            base_url: base_url.strip_suffix('/').unwrap_or(base_url).to_string(),
            model: model.to_string(),
        })
    }

    fn request_body(&self, request: &Request) -> serde_json::Value {
        json!({
            "model": self.model,
            "max_tokens": request.max_output_tokens,
            "messages": [
                {"role": "system", "content": request.instructions},
                {"role": "user", "content": request.content}
            ],
            "response_format": {
                "type": "json_schema",
                "json_schema": {
                    "name": request.schema_name,
                    "strict": true,
                    "schema": request.output_schema
                }
            }
        })
    }

    fn completion_from(&self, response: ChatResponse) -> Result<Completion, meeting_ai::Error> {
        let choice = response.choices.into_iter().next().ok_or_else(|| {
            meeting_ai::Error::Deserialization("LLM Gateway response has no choices".to_string())
        })?;

        if choice
            .finish_reason
            .as_deref()
            .is_some_and(|reason| TRUNCATED_FINISH_REASONS.contains(&reason))
        {
            return Err(meeting_ai::Error::Provider(
                "LLM Gateway output was truncated at the max token limit".to_string(),
            ));
        }

        let content = choice.message.and_then(|m| m.content).ok_or_else(|| {
            meeting_ai::Error::Deserialization(
                "LLM Gateway response has no message content".to_string(),
            )
        })?;

        let output = serde_json::from_str(&content).map_err(|_| {
            meeting_ai::Error::Deserialization(
                "LLM Gateway message content is not valid JSON".to_string(),
            )
        })?;

        Ok(Completion {
            output,
            model: response.model.unwrap_or_else(|| self.model.clone()),
            usage: response.usage.into(),
            request_id: response.request_id,
        })
    }
}

/// Maps a transport failure to a timeout or network error.
fn transport_error(err: reqwest::Error) -> meeting_ai::Error {
    if err.is_timeout() {
        meeting_ai::Error::Timeout("LLM Gateway request timed out".to_string())
    } else {
        meeting_ai::Error::Network(format!("LLM Gateway request failed: {err}"))
    }
}

/// Maps a non-2xx response to the matching error variant.
async fn status_error(response: reqwest::Response) -> meeting_ai::Error {
    let status = response.status();
    let retry_after_seconds = response
        .headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or(DEFAULT_RETRY_AFTER_SECONDS);
    let detail = response
        .json::<ErrorBody>()
        .await
        .ok()
        .and_then(ErrorBody::into_message)
        .map_or_else(
            || format!("LLM Gateway returned {status}"),
            |m| format!("LLM Gateway returned {status}: {m}"),
        );

    match status {
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
            meeting_ai::Error::Authentication(detail)
        }
        StatusCode::TOO_MANY_REQUESTS => meeting_ai::Error::RateLimited {
            retry_after_seconds,
        },
        StatusCode::BAD_REQUEST | StatusCode::UNPROCESSABLE_ENTITY => {
            meeting_ai::Error::Configuration(detail)
        }
        _ => meeting_ai::Error::Provider(detail),
    }
}

#[async_trait]
impl analysis::Provider for Provider {
    async fn complete(&self, request: Request) -> Result<Completion, meeting_ai::Error> {
        let url = format!("{}/chat/completions", self.base_url);

        let response = self
            .client
            .post(&url)
            .json(&self.request_body(&request))
            .send()
            .await
            .map_err(transport_error)?;

        let status = response.status();
        if !status.is_success() {
            warn!("LLM Gateway completion failed with status {status}");
            return Err(status_error(response).await);
        }

        let body = response.bytes().await.map_err(transport_error)?;
        let parsed: ChatResponse = serde_json::from_slice(&body).map_err(|_| {
            meeting_ai::Error::Deserialization(
                "LLM Gateway response is not the expected JSON".to_string(),
            )
        })?;

        let completion = self.completion_from(parsed)?;
        info!(
            "LLM Gateway completion succeeded: request_id={:?}, model={}, input_tokens={}, output_tokens={}",
            completion.request_id,
            completion.model,
            completion.usage.input_tokens,
            completion.usage.output_tokens
        );
        Ok(completion)
    }

    fn provider_id(&self) -> &str {
        PROVIDER_ID
    }
}

#[cfg(test)]
#[path = "llm_gateway_tests.rs"]
mod tests;
