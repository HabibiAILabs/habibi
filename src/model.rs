use anyhow::{Context, Result, bail};
use reqwest::{Client, header};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::{auth::CredentialStore, event::ConversationMessage};

const SYSTEM_PROMPT: &str = r#"You are Habibi, a local personal AI with one continuous conversation.
The messages supplied to you are selected from the durable event history and may span long periods of time.
For now you have no tools or actions. Respond directly and helpfully to the latest user message.
Do not claim to have capabilities that are not present."#;
const DEFAULT_CODEX_URL: &str = "https://chatgpt.com/backend-api/codex/responses";

#[derive(Debug, Clone)]
pub struct ModelConfig {
    pub endpoint: String,
    pub model: String,
    pub thinking: Option<String>,
    pub credentials: CredentialStore,
}

impl ModelConfig {
    pub fn from_env() -> Result<Self> {
        let endpoint =
            nonempty_env("HABIBI_OPENAI_CODEX_URL").unwrap_or_else(|| DEFAULT_CODEX_URL.into());
        let model = nonempty_env("HABIBI_MODEL").unwrap_or_else(|| "gpt-5.4".into());
        let model = model
            .strip_prefix("openai-codex/")
            .unwrap_or(&model)
            .to_owned();
        let thinking = nonempty_env("HABIBI_THINKING");

        if let Some(level) = &thinking
            && !matches!(
                level.as_str(),
                "off" | "minimal" | "low" | "medium" | "high" | "xhigh" | "max"
            )
        {
            bail!("HABIBI_THINKING has invalid level '{level}'");
        }

        Ok(Self {
            endpoint,
            model,
            thinking,
            credentials: CredentialStore::from_env()?,
        })
    }
}

fn nonempty_env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

pub struct ModelClient {
    client: Client,
    config: ModelConfig,
}

#[derive(Debug)]
pub struct ModelResponse {
    pub content: String,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub usage: Option<TokenUsage>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TokenUsage {
    pub input: Option<u64>,
    pub output: Option<u64>,
    pub cache_read: Option<u64>,
    pub cache_write: Option<u64>,
    pub total_tokens: Option<u64>,
}

impl ModelClient {
    pub fn new(config: ModelConfig) -> Result<Self> {
        let client = Client::builder()
            .user_agent(concat!("habibi/", env!("CARGO_PKG_VERSION")))
            .build()?;
        Ok(Self { client, config })
    }

    pub fn model_name(&self) -> &str {
        &self.config.model
    }

    pub async fn invoke(&self, conversation: &[ConversationMessage]) -> Result<ModelResponse> {
        let credential = self
            .config
            .credentials
            .valid_openai_credential(&self.client)
            .await?;
        let request_id = Uuid::now_v7().to_string();
        let input = conversation
            .iter()
            .enumerate()
            .map(|(index, message)| conversation_input(index, message))
            .collect::<Vec<_>>();

        let mut body = json!({
            "model": self.config.model,
            "store": false,
            "stream": true,
            "instructions": SYSTEM_PROMPT,
            "input": input,
            "text": { "verbosity": "low" },
            "include": ["reasoning.encrypted_content"],
            "tool_choice": "auto",
            "parallel_tool_calls": true
        });
        if let Some(level) = self.config.thinking.as_deref()
            && level != "off"
        {
            body["reasoning"] = json!({ "effort": level, "summary": "auto" });
        }

        let response = self
            .client
            .post(&self.config.endpoint)
            .header(
                header::AUTHORIZATION,
                format!("Bearer {}", credential.access),
            )
            .header("chatgpt-account-id", credential.account_id)
            .header("originator", "habibi")
            .header("OpenAI-Beta", "responses=experimental")
            .header("session-id", &request_id)
            .header("x-client-request-id", &request_id)
            .header(header::ACCEPT, "text/event-stream")
            .json(&body)
            .send()
            .await
            .context("OpenAI Codex request failed")?;

        let status = response.status();
        let response_body = response
            .text()
            .await
            .context("failed to read OpenAI Codex response")?;
        if !status.is_success() {
            bail!("OpenAI Codex returned {status}: {response_body}");
        }

        parse_sse_response(&response_body, &self.config.model)
    }
}

fn conversation_input(index: usize, message: &ConversationMessage) -> Value {
    if message.role == "assistant" {
        json!({
            "type": "message",
            "id": format!("msg_habibi_{index}"),
            "role": "assistant",
            "status": "completed",
            "content": [{
                "type": "output_text",
                "text": message.content,
                "annotations": []
            }]
        })
    } else {
        json!({
            "role": "user",
            "content": [{ "type": "input_text", "text": message.content }]
        })
    }
}

fn parse_sse_response(sse: &str, model: &str) -> Result<ModelResponse> {
    let mut content = String::new();
    let mut completed_response = None;

    for line in sse.lines() {
        let Some(data) = line.strip_prefix("data:") else {
            continue;
        };
        let data = data.trim();
        if data.is_empty() || data == "[DONE]" {
            continue;
        }

        let event: Value =
            serde_json::from_str(data).context("OpenAI Codex returned invalid SSE JSON")?;
        match event.get("type").and_then(Value::as_str) {
            Some("response.output_text.delta") => {
                if let Some(delta) = event.get("delta").and_then(Value::as_str) {
                    content.push_str(delta);
                }
            }
            Some("response.completed" | "response.done" | "response.incomplete") => {
                completed_response = event.get("response").cloned();
            }
            Some("response.failed") | Some("error") => {
                let error = event
                    .pointer("/response/error/message")
                    .or_else(|| event.pointer("/error/message"))
                    .or_else(|| event.get("message"))
                    .and_then(Value::as_str)
                    .unwrap_or("unknown OpenAI Codex error");
                bail!("OpenAI Codex response failed: {error}");
            }
            _ => {}
        }
    }

    let response =
        completed_response.context("OpenAI Codex stream ended without a completed response")?;
    if content.is_empty() {
        content = response
            .get("output")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .flat_map(|item| {
                item.get("content")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
            })
            .filter(|block| block.get("type").and_then(Value::as_str) == Some("output_text"))
            .filter_map(|block| block.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("");
    }

    let usage = response.get("usage").map(parse_usage);
    Ok(ModelResponse {
        content,
        provider: Some("openai-codex".into()),
        model: Some(model.into()),
        usage,
    })
}

fn parse_usage(value: &Value) -> TokenUsage {
    let cached = value
        .pointer("/input_tokens_details/cached_tokens")
        .and_then(Value::as_u64);
    let total_input = value.get("input_tokens").and_then(Value::as_u64);
    TokenUsage {
        input: total_input.map(|input| input.saturating_sub(cached.unwrap_or(0))),
        output: value.get("output_tokens").and_then(Value::as_u64),
        cache_read: cached,
        cache_write: None,
        total_tokens: value.get("total_tokens").and_then(Value::as_u64),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_conversation_roles_to_responses_input() {
        let user = ConversationMessage {
            role: "user".into(),
            content: "hello".into(),
        };
        let assistant = ConversationMessage {
            role: "assistant".into(),
            content: "hi".into(),
        };
        assert_eq!(conversation_input(0, &user)["role"], "user");
        assert_eq!(conversation_input(1, &assistant)["type"], "message");
    }

    #[test]
    fn parses_codex_sse_text_and_usage() {
        let sse = concat!(
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"Hello \"}\n\n",
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"world\"}\n\n",
            "data: {\"type\":\"response.completed\",\"response\":{\"usage\":{\"input_tokens\":10,\"output_tokens\":2,\"total_tokens\":12,\"input_tokens_details\":{\"cached_tokens\":3}}}}\n\n"
        );

        let response = parse_sse_response(sse, "gpt-test").unwrap();
        assert_eq!(response.content, "Hello world");
        let usage = response.usage.unwrap();
        assert_eq!(usage.input, Some(7));
        assert_eq!(usage.cache_read, Some(3));
    }
}
