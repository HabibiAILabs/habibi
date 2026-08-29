use anyhow::{Context, Result, bail};
use reqwest::{Client, Url, header};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::{
    auth::CredentialStore,
    catalog::{CatalogManager, CatalogModel, ModelCatalog, ModelPricing},
    tool::{ToolCall, ToolDefinition, provider_tool_name},
};

pub(crate) const SYSTEM_PROMPT: &str = r#"You are Habibi, a local event-driven personal AI.
Each invocation processes one immutable current event. Extension-provided context may accompany it.
Durable event history spans extension-level chat sessions; sessions are views, not memory boundaries. Use advertised or discovered history tools instead of claiming past sessions are inaccessible.
Act only through tools advertised for this invocation. Use habibi.tools.search when you need a tool that is not advertised.
Tool calls in one invocation are independent; their durable results are delivered in a subsequent action.batch.completed event.
Plain assistant text is operational output only; use an advertised extension tool for user-visible or domain effects."#;
const DEFAULT_CODEX_URL: &str = "https://chatgpt.com/backend-api/codex/responses";
const DEFAULT_OLLAMA_URL: &str = "http://127.0.0.1:11434";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelProvider {
    OpenAiCodex,
    Ollama,
}

impl ModelProvider {
    pub fn id(self) -> &'static str {
        match self {
            Self::OpenAiCodex => "openai-codex",
            Self::Ollama => "ollama",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ModelConfig {
    pub provider: ModelProvider,
    pub endpoint: String,
    pub model: String,
    pub thinking: Option<String>,
    pub credentials: Option<CredentialStore>,
}

#[derive(Debug, Clone, Serialize)]
pub struct EstimatedCost {
    pub currency: &'static str,
    pub input_usd: f64,
    pub output_usd: f64,
    pub cache_read_usd: f64,
    pub cache_write_usd: f64,
    pub total_usd: f64,
    pub pricing: ModelPricing,
    pub catalog_provider: String,
    pub catalog_model_id: String,
    pub catalog_source: Option<String>,
    pub catalog_updated_at: Option<String>,
}

impl ModelConfig {
    pub fn from_env() -> Result<Self> {
        let configured_model = nonempty_env("HABIBI_MODEL");
        let prefixed_provider = configured_model.as_deref().and_then(|model| {
            model
                .strip_prefix("openai-codex/")
                .map(|_| ModelProvider::OpenAiCodex)
                .or_else(|| model.strip_prefix("ollama/").map(|_| ModelProvider::Ollama))
        });
        let provider = match nonempty_env("HABIBI_MODEL_PROVIDER").as_deref() {
            None => prefixed_provider.unwrap_or(ModelProvider::OpenAiCodex),
            Some("openai-codex" | "openai") => ModelProvider::OpenAiCodex,
            Some("ollama") => ModelProvider::Ollama,
            Some(provider) => bail!("unsupported HABIBI_MODEL_PROVIDER '{provider}'"),
        };
        if prefixed_provider.is_some_and(|prefixed| prefixed != provider) {
            bail!("HABIBI_MODEL prefix conflicts with HABIBI_MODEL_PROVIDER");
        }
        if provider == ModelProvider::Ollama && configured_model.is_none() {
            bail!("HABIBI_MODEL is required for the Ollama provider");
        }
        let default_model = "gpt-5.6-luna";
        let configured_model = configured_model.as_deref().unwrap_or(default_model);
        let model = configured_model
            .strip_prefix("openai-codex/")
            .or_else(|| configured_model.strip_prefix("ollama/"))
            .unwrap_or(configured_model)
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
        let (endpoint, credentials) = match provider {
            ModelProvider::OpenAiCodex => (
                nonempty_env("HABIBI_OPENAI_CODEX_URL").unwrap_or_else(|| DEFAULT_CODEX_URL.into()),
                Some(CredentialStore::from_env()?),
            ),
            ModelProvider::Ollama => (
                ollama_chat_url(
                    &nonempty_env("HABIBI_OLLAMA_URL").unwrap_or_else(|| DEFAULT_OLLAMA_URL.into()),
                )?
                .to_string(),
                None,
            ),
        };
        Ok(Self {
            provider,
            endpoint,
            model,
            thinking,
            credentials,
        })
    }
}

fn nonempty_env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn ollama_chat_url(configured: &str) -> Result<Url> {
    let mut url = Url::parse(configured).context("HABIBI_OLLAMA_URL is not a valid URL")?;
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        bail!("HABIBI_OLLAMA_URL must not contain credentials, query, or fragment");
    }
    let loopback = url
        .host_str()
        .and_then(|host| host.parse::<std::net::IpAddr>().ok())
        .is_some_and(|address| address.is_loopback())
        || url.host_str() == Some("localhost");
    if url.scheme() != "https" && !(url.scheme() == "http" && loopback) {
        bail!("Ollama must use HTTPS or an explicitly configured loopback HTTP origin");
    }
    if !url.path().trim_end_matches('/').ends_with("/api/chat") {
        url.set_path(&format!("{}/api/chat", url.path().trim_end_matches('/')));
    }
    Ok(url)
}

pub struct ModelClient {
    client: Client,
    config: ModelConfig,
    catalog: CatalogManager,
}

#[derive(Debug)]
pub struct ModelResponse {
    pub content: String,
    pub output_items: Vec<Value>,
    pub tool_calls: Vec<ToolCall>,
    pub provider_response: Value,
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
    pub fn new(config: ModelConfig, catalog: CatalogManager) -> Result<Self> {
        let client = Client::builder()
            .user_agent(concat!("habibi/", env!("CARGO_PKG_VERSION")))
            .build()?;
        Ok(Self {
            client,
            config,
            catalog,
        })
    }

    pub fn model_name(&self) -> &str {
        &self.config.model
    }

    pub async fn verify(&self) -> Result<()> {
        if self.config.provider != ModelProvider::Ollama {
            return Ok(());
        }
        let mut endpoint = Url::parse(&self.config.endpoint)?;
        let base = endpoint
            .path()
            .strip_suffix("/api/chat")
            .unwrap_or_default();
        endpoint.set_path(&format!("{base}/api/show"));
        let response = self
            .client
            .post(endpoint)
            .json(&json!({ "model": self.config.model }))
            .send()
            .await
            .context("cannot reach Ollama; start it before Habibi")?;
        let status = response.status();
        let value: Value = response
            .json()
            .await
            .context("Ollama model metadata was not valid JSON")?;
        if !status.is_success() {
            let message = value
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("model lookup failed");
            bail!("Ollama returned {status}: {message}");
        }
        let supports_tools = value
            .get("capabilities")
            .and_then(Value::as_array)
            .is_some_and(|capabilities| capabilities.iter().any(|value| value == "tools"));
        if !supports_tools {
            bail!(
                "Ollama model '{}' does not declare tool support, which Habibi requires",
                self.config.model
            );
        }
        Ok(())
    }

    pub fn request_body(&self, input: &[Value], tools: &[ToolDefinition]) -> Value {
        match self.config.provider {
            ModelProvider::OpenAiCodex => self.codex_request_body(input, tools),
            ModelProvider::Ollama => self.ollama_request_body(input, tools),
        }
    }

    fn codex_request_body(&self, input: &[Value], tools: &[ToolDefinition]) -> Value {
        let tools = tools
            .iter()
            .map(|tool| {
                json!({
                    "type": "function", "name": provider_tool_name(&tool.name), "description": tool.description,
                    "parameters": tool.input_schema, "strict": false
                })
            })
            .collect::<Vec<_>>();
        let mut body = json!({
            "model": self.config.model,
            "store": false,
            "stream": true,
            "instructions": SYSTEM_PROMPT,
            "input": input,
            "text": { "verbosity": "low" },
            "include": ["reasoning.encrypted_content"],
            "tools": tools,
            "tool_choice": "auto",
            "parallel_tool_calls": true
        });
        if let Some(level) = self.config.thinking.as_deref()
            && level != "off"
        {
            body["reasoning"] = json!({ "effort": level, "summary": "auto" });
        }
        body
    }

    fn ollama_request_body(&self, input: &[Value], tools: &[ToolDefinition]) -> Value {
        let mut messages = vec![json!({ "role": "system", "content": SYSTEM_PROMPT })];
        messages.extend(input.iter().map(ollama_message));
        let tools = tools
            .iter()
            .map(|tool| {
                json!({
                    "type": "function",
                    "function": {
                        "name": provider_tool_name(&tool.name),
                        "description": tool.description,
                        "parameters": tool.input_schema
                    }
                })
            })
            .collect::<Vec<_>>();
        let mut body = json!({
            "model": self.config.model,
            "messages": messages,
            "stream": false,
            "tools": tools
        });
        if let Some(level) = self.config.thinking.as_deref() {
            body["think"] = match level {
                "off" => json!(false),
                "minimal" | "low" => json!("low"),
                "medium" => json!("medium"),
                "high" => json!("high"),
                "xhigh" | "max" => json!("max"),
                _ => unreachable!("thinking level was validated"),
            };
        }
        body
    }

    pub fn provider_name(&self) -> &'static str {
        self.config.provider.id()
    }

    pub fn endpoint(&self) -> &str {
        &self.config.endpoint
    }

    pub fn catalog(&self) -> Result<ModelCatalog> {
        self.catalog.snapshot()
    }

    pub async fn refresh_catalog(&self) -> Result<ModelCatalog> {
        self.catalog.refresh(&self.client).await
    }

    pub fn estimate_cost(&self, usage: &TokenUsage) -> Option<EstimatedCost> {
        let model: CatalogModel = self
            .catalog
            .lookup(self.provider_name(), &self.config.model)
            .ok()??;
        let pricing = &model.pricing;
        let input_rate = pricing.input_usd_per_million;
        let output_rate = pricing.output_usd_per_million;
        let cache_read_rate = pricing.cache_read_usd_per_million.unwrap_or(input_rate);
        let cache_write_rate = pricing.cache_write_usd_per_million.unwrap_or(input_rate);
        let cost = |tokens: Option<u64>, rate: f64| tokens.unwrap_or(0) as f64 * rate / 1_000_000.0;
        let input_usd = cost(usage.input, input_rate);
        let output_usd = cost(usage.output, output_rate);
        let cache_read_usd = cost(usage.cache_read, cache_read_rate);
        let cache_write_usd = cost(usage.cache_write, cache_write_rate);
        Some(EstimatedCost {
            currency: "USD",
            input_usd,
            output_usd,
            cache_read_usd,
            cache_write_usd,
            total_usd: input_usd + output_usd + cache_read_usd + cache_write_usd,
            pricing: pricing.clone(),
            catalog_provider: model.provider,
            catalog_model_id: model.id,
            catalog_source: model.source,
            catalog_updated_at: model.updated_at,
        })
    }

    pub async fn invoke(&self, body: Value) -> Result<ModelResponse> {
        match self.config.provider {
            ModelProvider::OpenAiCodex => self.invoke_codex(body).await,
            ModelProvider::Ollama => self.invoke_ollama(body).await,
        }
    }

    async fn invoke_codex(&self, body: Value) -> Result<ModelResponse> {
        let credential = self
            .config
            .credentials
            .as_ref()
            .context("OpenAI credentials are not configured")?
            .valid_openai_credential(&self.client)
            .await?;
        let request_id = Uuid::now_v7().to_string();
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

    async fn invoke_ollama(&self, body: Value) -> Result<ModelResponse> {
        let response = self
            .client
            .post(&self.config.endpoint)
            .json(&body)
            .send()
            .await
            .context("Ollama request failed")?;
        let status = response.status();
        let response_body = response
            .text()
            .await
            .context("failed to read Ollama response")?;
        if !status.is_success() {
            bail!("Ollama returned {status}: {response_body}");
        }
        let response: Value =
            serde_json::from_str(&response_body).context("Ollama returned invalid JSON")?;
        parse_ollama_response(response, &self.config.model)
    }
}

fn ollama_message(input: &Value) -> Value {
    let role = input.get("role").and_then(Value::as_str).unwrap_or("user");
    let content = match input.get("content") {
        Some(Value::String(content)) => content.clone(),
        Some(Value::Array(blocks)) => blocks
            .iter()
            .filter_map(|block| {
                block
                    .get("text")
                    .or_else(|| block.get("content"))
                    .and_then(Value::as_str)
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => serde_json::to_string(input).unwrap_or_default(),
    };
    json!({ "role": role, "content": content })
}

fn parse_ollama_response(response: Value, configured_model: &str) -> Result<ModelResponse> {
    if let Some(error) = response.get("error").and_then(Value::as_str) {
        bail!("Ollama response failed: {error}");
    }
    let message = response
        .get("message")
        .context("Ollama response missing message")?;
    let content = message
        .get("content")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let tool_calls = message
        .get("tool_calls")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|call| -> Result<ToolCall> {
            let function = call
                .get("function")
                .context("Ollama tool call missing function")?;
            let arguments = match function.get("arguments") {
                Some(Value::String(arguments)) => serde_json::from_str(arguments)
                    .context("Ollama tool call contained invalid JSON arguments")?,
                Some(arguments) => arguments.clone(),
                None => json!({}),
            };
            Ok(ToolCall {
                call_id: call
                    .get("id")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .unwrap_or_else(|| format!("ollama-{}", Uuid::now_v7())),
                name: function
                    .get("name")
                    .and_then(Value::as_str)
                    .context("Ollama tool call missing name")?
                    .to_owned(),
                arguments,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let mut output_items = Vec::new();
    if !content.is_empty() {
        output_items.push(json!({
            "type": "message",
            "role": "assistant",
            "content": [{ "type": "output_text", "text": content }]
        }));
    }
    output_items.extend(tool_calls.iter().map(|call| {
        json!({
            "type": "function_call",
            "call_id": call.call_id,
            "name": call.name,
            "arguments": serde_json::to_string(&call.arguments).unwrap_or_else(|_| "{}".into())
        })
    }));
    let input = response.get("prompt_eval_count").and_then(Value::as_u64);
    let output = response.get("eval_count").and_then(Value::as_u64);
    let usage = (input.is_some() || output.is_some()).then(|| TokenUsage {
        input,
        output,
        cache_read: None,
        cache_write: None,
        total_tokens: match (input, output) {
            (Some(input), Some(output)) => Some(input.saturating_add(output)),
            _ => None,
        },
    });
    let model = response
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or(configured_model)
        .to_owned();
    Ok(ModelResponse {
        content,
        output_items,
        tool_calls,
        provider_response: response,
        provider: Some("ollama".into()),
        model: Some(model),
        usage,
    })
}

fn parse_sse_response(sse: &str, model: &str) -> Result<ModelResponse> {
    let mut content = String::new();
    let mut completed_response = None;
    let mut streamed_output_items = Vec::new();

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
            Some("response.output_item.done") => {
                if let Some(item) = event.get("item") {
                    streamed_output_items.push(item.clone());
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

    let output_items = response
        .get("output")
        .and_then(Value::as_array)
        .filter(|items| !items.is_empty())
        .cloned()
        .unwrap_or(streamed_output_items);
    let tool_calls = output_items
        .iter()
        .filter(|item| item.get("type").and_then(Value::as_str) == Some("function_call"))
        .map(|item| -> Result<ToolCall> {
            let arguments = item
                .get("arguments")
                .and_then(Value::as_str)
                .map(serde_json::from_str)
                .transpose()?
                .unwrap_or_else(|| json!({}));
            Ok(ToolCall {
                call_id: item
                    .get("call_id")
                    .and_then(Value::as_str)
                    .context("function call missing call_id")?
                    .to_owned(),
                name: item
                    .get("name")
                    .and_then(Value::as_str)
                    .context("function call missing name")?
                    .to_owned(),
                arguments,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let usage = response.get("usage").map(parse_usage);
    Ok(ModelResponse {
        content,
        output_items,
        tool_calls,
        provider_response: response.clone(),
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
    let cache_write = value
        .pointer("/input_tokens_details/cache_write_tokens")
        .or_else(|| value.pointer("/input_tokens_details/cache_creation_tokens"))
        .and_then(Value::as_u64);
    TokenUsage {
        input: total_input.map(|input| {
            input
                .saturating_sub(cached.unwrap_or(0))
                .saturating_sub(cache_write.unwrap_or(0))
        }),
        output: value.get("output_tokens").and_then(Value::as_u64),
        cache_read: cached,
        cache_write,
        total_tokens: value.get("total_tokens").and_then(Value::as_u64),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_native_ollama_messages_and_tools() {
        let client = ModelClient::new(
            ModelConfig {
                provider: ModelProvider::Ollama,
                endpoint: "http://127.0.0.1:11434/api/chat".into(),
                model: "qwen3:8b".into(),
                thinking: Some("medium".into()),
                credentials: None,
            },
            CatalogManager::from_env().unwrap(),
        )
        .unwrap();
        let body = client.request_body(
            &[json!({
                "role": "user",
                "content": [{ "type": "input_text", "text": "hello" }]
            })],
            &[ToolDefinition {
                name: "chat.send_message".into(),
                description: "Reply".into(),
                input_schema: json!({ "type": "object" }),
            }],
        );
        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(body["messages"][1]["content"], "hello");
        assert_eq!(body["tools"][0]["function"]["name"], "chat__send_message");
        assert_eq!(body["think"], "medium");
        assert_eq!(body["stream"], false);
    }

    #[test]
    fn parses_ollama_tool_calls_and_usage() {
        let response = parse_ollama_response(
            json!({
                "model": "qwen3:8b",
                "message": {
                    "role": "assistant",
                    "content": "",
                    "thinking": "private reasoning",
                    "tool_calls": [{
                        "function": {
                            "name": "chat__send_message",
                            "arguments": { "session_id": "current", "content": "hello" }
                        }
                    }]
                },
                "prompt_eval_count": 41,
                "eval_count": 7,
                "done": true
            }),
            "qwen3:8b",
        )
        .unwrap();
        assert_eq!(response.provider.as_deref(), Some("ollama"));
        assert_eq!(response.model.as_deref(), Some("qwen3:8b"));
        assert_eq!(response.tool_calls[0].name, "chat__send_message");
        assert_eq!(response.tool_calls[0].arguments["content"], "hello");
        assert!(response.tool_calls[0].call_id.starts_with("ollama-"));
        assert_eq!(response.usage.unwrap().total_tokens, Some(48));
    }

    #[test]
    fn converts_responses_context_to_ollama_messages() {
        assert_eq!(
            ollama_message(&json!({
                "role": "user",
                "content": [{ "type": "input_text", "text": "first" }, { "type": "input_text", "text": "second" }]
            })),
            json!({ "role": "user", "content": "first\nsecond" })
        );
        assert!(ollama_chat_url("http://127.0.0.1:11434").is_ok());
        assert!(ollama_chat_url("http://ollama.example").is_err());
    }

    #[test]
    fn parses_streamed_function_call_when_completed_output_is_empty() {
        let sse = concat!(
            "data: {\"type\":\"response.output_item.done\",\"item\":{\"type\":\"function_call\",\"call_id\":\"call_1\",\"name\":\"chat__send_message\",\"arguments\":\"{\\\"content\\\":\\\"hi\\\"}\"}}\n\n",
            "data: {\"type\":\"response.completed\",\"response\":{\"output\":[],\"usage\":{}}}\n\n"
        );
        let response = parse_sse_response(sse, "gpt-test").unwrap();
        assert_eq!(response.tool_calls.len(), 1);
        assert_eq!(response.tool_calls[0].name, "chat__send_message");
        assert_eq!(response.tool_calls[0].arguments["content"], "hi");
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
