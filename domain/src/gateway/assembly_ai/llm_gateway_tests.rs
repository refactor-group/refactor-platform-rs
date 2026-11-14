//! LLM Gateway request shape, response parsing, and error mapping.

use meeting_ai::traits::analysis::Provider as _;
use mockito::{Matcher, Server, ServerGuard};
use serde_json::json;

use super::*;

const MODEL: &str = "claude-sonnet-4-6";

fn request() -> Request {
    Request {
        instructions: "Extract the follow-up.".to_string(),
        content: "Ana: I will send the draft.".to_string(),
        schema_name: "follow_up".to_string(),
        output_schema: json!({
            "type": "object",
            "properties": {"who": {"type": "string"}, "what": {"type": "string"}},
            "required": ["who", "what"],
            "additionalProperties": false
        }),
        max_output_tokens: 512,
    }
}

fn provider(server: &ServerGuard) -> Provider {
    Provider::new("test-key", &server.url(), MODEL).unwrap()
}

fn success_body(content: &str, finish_reason: &str) -> String {
    json!({
        "request_id": "dcb24c2f-0000-4000-8000-000000000000",
        "model": MODEL,
        "choices": [{
            "index": 0,
            "finish_reason": finish_reason,
            "message": {"role": "assistant", "content": content}
        }],
        "usage": {
            "input_tokens": 189,
            "prompt_tokens": 189,
            "output_tokens": 19,
            "completion_tokens": 19,
            "total_tokens": 208
        }
    })
    .to_string()
}

async fn complete_with(status: usize, body: &str) -> Result<Completion, meeting_ai::Error> {
    let mut server = Server::new_async().await;
    let _mock = server
        .mock("POST", "/chat/completions")
        .with_status(status)
        .with_header("content-type", "application/json")
        .with_body(body)
        .create_async()
        .await;
    provider(&server).complete(request()).await
}

#[tokio::test]
async fn posts_the_expected_request_shape() {
    let mut server = Server::new_async().await;
    let req = request();
    let mock = server
        .mock("POST", "/chat/completions")
        .match_header("authorization", "test-key")
        .match_header(
            "content-type",
            Matcher::Regex("application/json".to_string()),
        )
        .match_body(Matcher::PartialJson(json!({
            "model": MODEL,
            "max_tokens": 512,
            "messages": [
                {"role": "system", "content": req.instructions},
                {"role": "user", "content": req.content}
            ],
            "response_format": {
                "type": "json_schema",
                "json_schema": {
                    "name": "follow_up",
                    "strict": true,
                    "schema": req.output_schema
                }
            }
        })))
        .with_status(200)
        .with_body(success_body(
            r#"{"who":"Ana","what":"Send the draft"}"#,
            "end_turn",
        ))
        .expect(1)
        .create_async()
        .await;

    provider(&server).complete(request()).await.unwrap();

    mock.assert_async().await;
}

#[tokio::test]
async fn request_has_no_temperature() {
    let mut server = Server::new_async().await;
    let mock = server
        .mock("POST", "/chat/completions")
        .match_request(|req| {
            req.body()
                .ok()
                .and_then(|b| serde_json::from_slice::<serde_json::Value>(b).ok())
                .and_then(|v| v.as_object().map(|o| !o.contains_key("temperature")))
                .unwrap_or(false)
        })
        .with_status(200)
        .with_body(success_body(
            r#"{"who":"Ana","what":"Send the draft"}"#,
            "end_turn",
        ))
        .expect(1)
        .create_async()
        .await;

    let result = provider(&server).complete(request()).await;

    assert!(result.is_ok(), "request body carried a temperature field");
    mock.assert_async().await;
}

#[tokio::test]
async fn parses_a_successful_completion() {
    let completion = complete_with(
        200,
        &success_body(r#"{"who":"Ben","what":"Book the venue"}"#, "end_turn"),
    )
    .await
    .unwrap();

    assert_eq!(
        completion.output,
        json!({"who": "Ben", "what": "Book the venue"})
    );
    assert_eq!(
        completion.usage,
        Usage {
            input_tokens: 189,
            output_tokens: 19
        }
    );
    assert_eq!(completion.model, MODEL);
    assert_eq!(
        completion.request_id.as_deref(),
        Some("dcb24c2f-0000-4000-8000-000000000000")
    );
}

#[tokio::test]
async fn usage_falls_back_to_prompt_and_completion_tokens() {
    let body = json!({
        "model": MODEL,
        "choices": [{
            "finish_reason": "stop",
            "message": {"role": "assistant", "content": "{}"}
        }],
        "usage": {"prompt_tokens": 40, "completion_tokens": 7}
    })
    .to_string();

    let completion = complete_with(200, &body).await.unwrap();

    assert_eq!(
        completion.usage,
        Usage {
            input_tokens: 40,
            output_tokens: 7
        }
    );
    assert_eq!(completion.request_id, None);
}

#[tokio::test]
async fn unauthorized_is_authentication() {
    let result = complete_with(401, r#"{"error":{"message":"bad key"}}"#).await;
    assert!(matches!(result, Err(meeting_ai::Error::Authentication(_))));
}

#[tokio::test]
async fn rate_limited_uses_retry_after_header() {
    let mut server = Server::new_async().await;
    let _mock = server
        .mock("POST", "/chat/completions")
        .with_status(429)
        .with_header("retry-after", "12")
        .create_async()
        .await;

    let result = provider(&server).complete(request()).await;

    assert!(matches!(
        result,
        Err(meeting_ai::Error::RateLimited {
            retry_after_seconds: 12
        })
    ));
}

#[tokio::test]
async fn rate_limited_defaults_to_thirty_seconds() {
    let result = complete_with(429, "{}").await;
    assert!(matches!(
        result,
        Err(meeting_ai::Error::RateLimited {
            retry_after_seconds: 30
        })
    ));
}

#[tokio::test]
async fn bad_request_is_configuration() {
    let result = complete_with(400, r#"{"message":"temperature is not supported"}"#).await;
    assert!(matches!(result, Err(meeting_ai::Error::Configuration(_))));
}

#[tokio::test]
async fn service_unavailable_is_provider() {
    let result = complete_with(503, "upstream down").await;
    assert!(matches!(result, Err(meeting_ai::Error::Provider(_))));
}

#[tokio::test]
async fn empty_choices_is_deserialization() {
    let result = complete_with(200, r#"{"choices":[]}"#).await;
    assert!(matches!(result, Err(meeting_ai::Error::Deserialization(_))));
}

#[tokio::test]
async fn non_json_content_is_deserialization() {
    let result = complete_with(200, &success_body("not json", "end_turn")).await;
    assert!(matches!(result, Err(meeting_ai::Error::Deserialization(_))));
}

#[tokio::test]
async fn truncated_output_is_provider() {
    let result = complete_with(200, &success_body(r#"{"who":"An"#, "max_tokens")).await;
    assert!(
        matches!(&result, Err(meeting_ai::Error::Provider(msg)) if msg.contains("truncated")),
        "got {result:?}"
    );
}

#[tokio::test]
async fn trailing_slash_in_base_url_is_trimmed() {
    let mut server = Server::new_async().await;
    let mock = server
        .mock("POST", "/v1/chat/completions")
        .with_status(200)
        .with_body(success_body("{}", "end_turn"))
        .expect(1)
        .create_async()
        .await;
    let base_url = format!("{}/v1/", server.url());

    Provider::new("test-key", &base_url, MODEL)
        .unwrap()
        .complete(request())
        .await
        .unwrap();

    mock.assert_async().await;
}

#[test]
fn provider_id_is_lowercase_gateway_name() {
    let provider = Provider::new("test-key", "http://localhost", MODEL).unwrap();
    assert_eq!(provider.provider_id(), "assembly_ai_llm_gateway");
}
