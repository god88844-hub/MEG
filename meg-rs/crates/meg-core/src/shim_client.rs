//! Streaming client for explicitly configured local/cloud endpoints.
//! Supports native OpenAI & Anthropic tool calling (with automatic text-JSON
//! fallback), Anthropic prompt caching (`cache_control`), and cancellable
//! exponential-backoff retries on 429 / transient 5xx responses.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};

/// A standard local endpoint; personal gateways require explicit configuration.
pub fn default_base_url() -> String {
    std::env::var("MEG_TUI_BASE_URL")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "http://127.0.0.1:11434/v1".to_string())
}

/// A model is selected from discovery or explicitly configured, never invented.
pub fn default_model() -> String {
    std::env::var("MEG_TUI_MODEL")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_default()
}

/// Structured tool definition passed to OpenAI (`tools`) and Anthropic (`tools`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

/// One native tool call returned by an OpenAI or Anthropic endpoint.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NativeToolCall {
    pub id: String,
    pub tool: String,
    pub args: Value,
}

/// Internal role tags carried inside `ChatMessage.role` so `ChatMessage` keeps
/// its exact `{ role, content }` layout while preserving native tool-call IDs.
pub const ROLE_ASSISTANT_TOOL_CALLS: &str = "assistant_tool_calls";
pub const ROLE_TOOL_RESULT: &str = "tool";
pub const ROLE_USER_MULTIMODAL: &str = "user_multimodal";

pub fn encode_native_tool_calls(thought: &str, calls: &[NativeToolCall]) -> crate::ChatMessage {
    crate::ChatMessage {
        role: ROLE_ASSISTANT_TOOL_CALLS.into(),
        content: json!({
            "thought": thought,
            "calls": calls,
        })
        .to_string(),
    }
}

pub fn encode_native_tool_result(
    tool_call_id: &str,
    tool_name: &str,
    ok: bool,
    output: &str,
) -> crate::ChatMessage {
    crate::ChatMessage {
        role: ROLE_TOOL_RESULT.into(),
        content: json!({
            "tool_call_id": tool_call_id,
            "name": tool_name,
            "ok": ok,
            "content": output,
        })
        .to_string(),
    }
}

/// Parse native tool-call envelope emitted by `CompletionDecoder::finish`.
pub fn extract_native_tool_calls(raw: &str) -> Option<(String, Vec<NativeToolCall>)> {
    let value: Value = serde_json::from_str(raw).ok()?;
    if value.get("_native").and_then(Value::as_bool) != Some(true) {
        return None;
    }
    let thought = value
        .get("thought")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let arr = value.get("batch").and_then(Value::as_array)?;
    let mut calls = Vec::new();
    for (idx, item) in arr.iter().enumerate() {
        let tool = item.get("tool").and_then(Value::as_str)?.to_string();
        let id = item
            .get("id")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| format!("call_{idx}"));
        let args = item
            .get("args")
            .cloned()
            .unwrap_or_else(|| Value::Object(Default::default()));
        calls.push(NativeToolCall { id, tool, args });
    }
    if calls.is_empty() {
        None
    } else {
        Some((thought, calls))
    }
}

pub struct ShimClient {
    http: reqwest::Client,
    pub base_url: String,
    pub model: String,
    pub reasoning_effort: Option<String>,
    pub kind: crate::providers::ProviderKind,
    pub tools: Vec<ToolSpec>,
    pub prompt_caching: bool,
    pub max_retries: u32,
    pub base_retry_ms: u64,
    pub max_retry_delay_secs: u64,
    tools_rejected: AtomicBool,
    api_key: Option<crate::providers::Secret>,
}

#[derive(Debug, Clone)]
pub enum Delta {
    Thinking(String),
    Content(String),
    Usage {
        input: u64,
        output: u64,
        cached: u64,
    },
}

impl ShimClient {
    pub fn new() -> Self {
        Self::with(default_base_url(), default_model())
    }
    pub fn with(base_url: String, model: String) -> Self {
        Self {
            http: reqwest::Client::builder()
                .connect_timeout(std::time::Duration::from_secs(5))
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .expect("HTTP client initialization"),
            base_url: base_url.trim_end_matches('/').into(),
            model,
            reasoning_effort: None,
            kind: crate::providers::ProviderKind::OpenAi,
            tools: Vec::new(),
            prompt_caching: false,
            max_retries: 2,
            base_retry_ms: 500,
            max_retry_delay_secs: 10,
            tools_rejected: AtomicBool::new(false),
            api_key: std::env::var("MEG_TUI_API_KEY")
                .ok()
                .filter(|s| !s.is_empty())
                .map(crate::providers::Secret::new),
        }
    }
    pub fn with_profile(profile: &crate::providers::ProviderProfile) -> Result<Self> {
        profile.validate()?;
        let mut client = Self::with(profile.base_url.clone(), profile.default_model.clone());
        client.kind = profile.kind;
        client.prompt_caching = profile.kind == crate::providers::ProviderKind::Anthropic;
        client.api_key = crate::providers::api_key(profile)?;
        Ok(client)
    }
    pub fn native_tools_active(&self) -> bool {
        !self.tools.is_empty() && !self.tools_rejected.load(Ordering::Relaxed)
    }
    fn request(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        let request = self
            .http
            .request(method, format!("{}{path}", self.base_url));
        match &self.api_key {
            Some(key) if self.kind == crate::providers::ProviderKind::Anthropic => request
                .header("x-api-key", key.expose())
                .header("anthropic-version", "2023-06-01"),
            Some(key) => request.bearer_auth(key.expose()),
            None => request,
        }
    }
    pub async fn healthy(&self) -> bool {
        self.request(reqwest::Method::GET, "/models")
            .timeout(std::time::Duration::from_secs(3))
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false)
    }
    pub async fn models(&self) -> Result<Vec<String>> {
        self.models_with_timeout(std::time::Duration::from_secs(15))
            .await
    }
    pub async fn models_with_timeout(&self, timeout: std::time::Duration) -> Result<Vec<String>> {
        let host = reqwest::Url::parse(&self.base_url)
            .ok()
            .and_then(|u| u.host_str().map(str::to_owned));
        let path = if self.api_key.is_some()
            && matches!(host.as_deref(), Some("openrouter.ai" | "eu.openrouter.ai"))
        {
            "/models/user"
        } else {
            "/models"
        };
        let response = self
            .request(reqwest::Method::GET, path)
            .timeout(timeout)
            .send()
            .await?;
        let status = response.status();
        let mut value: Value = if status.is_success() {
            serde_json::from_slice(&bounded_body(response, 8 * 1024 * 1024).await?)?
        } else if self.kind == crate::providers::ProviderKind::Ollama {
            json!({})
        } else {
            anyhow::bail!(
                "{} (HTTP {status})",
                match status.as_u16() {
                    401 => "API key rejected; edit the connection key",
                    403 => "Provider denied access; check account/model permissions",
                    402 => "Provider requires credits",
                    429 => "Provider rate limited discovery; retry later",
                    _ => "Model discovery failed; inspect the endpoint",
                }
            )
        };
        if self.kind == crate::providers::ProviderKind::Ollama
            && value["data"].as_array().is_none_or(|a| a.is_empty())
        {
            let root = self.base_url.trim_end_matches('/').trim_end_matches("/v1");
            let mut request = self.http.get(format!("{root}/api/tags")).timeout(timeout);
            if let Some(key) = &self.api_key {
                request = request.bearer_auth(key.expose());
            }
            let response = request.send().await?.error_for_status()?;
            let tags: Value =
                serde_json::from_slice(&bounded_body(response, 8 * 1024 * 1024).await?)?;
            value = json!({"data":tags["models"].as_array().into_iter().flatten().map(|m|json!({"id":m["name"]})).collect::<Vec<_>>()});
        }
        Ok(value["data"]
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item["id"].as_str())
                    .filter(|id| {
                        !id.is_empty() && id.len() <= 256 && !id.chars().any(char::is_control)
                    })
                    .take(10_000)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default())
    }

    fn build_request_body(&self, messages: &[crate::ChatMessage], use_native_tools: bool) -> Value {
        let anthropic = self.kind == crate::providers::ProviderKind::Anthropic;
        if anthropic {
            let system_text = messages
                .iter()
                .filter(|m| m.role == "system")
                .map(|m| m.content.as_str())
                .collect::<Vec<_>>()
                .join("\n");
            let system_value = if self.prompt_caching && !system_text.is_empty() {
                json!([{
                    "type": "text",
                    "text": system_text,
                    "cache_control": { "type": "ephemeral" }
                }])
            } else {
                Value::String(system_text)
            };
            let mut wire_messages = build_anthropic_messages(messages, use_native_tools);
            if self.prompt_caching {
                mark_anthropic_history_cache_breakpoint(&mut wire_messages);
            }
            let base_max_tokens: u64 = std::env::var("MEG_ANTHROPIC_MAX_TOKENS")
                .ok()
                .and_then(|v| v.parse::<u64>().ok())
                .filter(|&v| v >= 256)
                .unwrap_or(4096);
            let thinking_budget: Option<u64> = self
                .reasoning_effort
                .as_deref()
                .and_then(|eff| match eff.to_ascii_lowercase().as_str() {
                    "low" => Some(2048),
                    "medium" => Some(6144),
                    "high" | "xhigh" | "max" => Some(12288),
                    _ => None,
                });
            let max_tokens = match thinking_budget {
                Some(b) => base_max_tokens.max(b + 4096),
                None => base_max_tokens,
            };
            let mut body = json!({
                "model": self.model,
                "messages": wire_messages,
                "system": system_value,
                "max_tokens": max_tokens,
                "stream": true
            });
            if let Some(budget) = thinking_budget {
                body["thinking"] = json!({
                    "type": "enabled",
                    "budget_tokens": budget
                });
            }
            if use_native_tools && !self.tools.is_empty() {
                let mut tools_json: Vec<Value> = self
                    .tools
                    .iter()
                    .map(|t| {
                        json!({
                            "name": t.name,
                            "description": t.description,
                            "input_schema": t.parameters,
                        })
                    })
                    .collect();
                if self.prompt_caching {
                    if let Some(last) = tools_json.last_mut() {
                        last["cache_control"] = json!({ "type": "ephemeral" });
                    }
                }
                body["tools"] = Value::Array(tools_json);
            }
            body
        } else {
            let wire_messages = build_openai_messages(messages, use_native_tools);
            let mut body = json!({
                "model": self.model,
                "messages": wire_messages,
                "stream": true,
                "stream_options": { "include_usage": true }
            });
            if use_native_tools && !self.tools.is_empty() {
                let tools_json: Vec<Value> = self
                    .tools
                    .iter()
                    .map(|t| {
                        json!({
                            "type": "function",
                            "function": {
                                "name": t.name,
                                "description": t.description,
                                "parameters": t.parameters,
                            }
                        })
                    })
                    .collect();
                body["tools"] = Value::Array(tools_json);
            }
            if let Some(effort) = &self.reasoning_effort {
                body["reasoning_effort"] = Value::String(effort.clone());
            }
            body
        }
    }

    pub async fn chat_stream(
        &self,
        messages: &[crate::ChatMessage],
        on_delta: &mut (dyn FnMut(Delta) + Send),
    ) -> Result<String> {
        self.chat_stream_cancellable(messages, None, on_delta).await
    }

    pub async fn chat_stream_cancellable(
        &self,
        messages: &[crate::ChatMessage],
        cancel: Option<&crate::CancellationToken>,
        on_delta: &mut (dyn FnMut(Delta) + Send),
    ) -> Result<String> {
        use futures_util::StreamExt;
        let anthropic = self.kind == crate::providers::ProviderKind::Anthropic;
        let endpoint_path = if anthropic {
            "/messages"
        } else {
            "/chat/completions"
        };

        let mut attempt: u32 = 0;
        let mut fallback_attempted = false;

        let response = loop {
            if cancel.is_some_and(|c| c.is_cancelled()) {
                anyhow::bail!("Run cancelled");
            }
            let use_native_tools = self.native_tools_active();
            let body = self.build_request_body(messages, use_native_tools);
            let send_fut = self
                .request(reqwest::Method::POST, endpoint_path)
                .json(&body)
                .timeout(std::time::Duration::from_secs(180))
                .send();
            let send_result = match cancel {
                Some(tok) => tokio::select! {
                    r = send_fut => r,
                    _ = tok.cancelled() => anyhow::bail!("Run cancelled"),
                },
                None => send_fut.await,
            };

            let resp = match send_result {
                Ok(r) => r,
                Err(err) => {
                    if attempt < self.max_retries && (err.is_timeout() || err.is_connect()) {
                        let delay = backoff_delay(self.base_retry_ms, attempt, self.max_retry_delay_secs);
                        attempt += 1;
                        sleep_cancellable(delay, cancel).await?;
                        continue;
                    }
                    return Err(err.into());
                }
            };

            if resp.status().is_success() {
                break resp;
            }

            let status = resp.status();
            let code = status.as_u16();
            let retry_after = resp
                .headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|v| v.to_str().ok())
                .filter(|s| s.len() <= 10 && s.chars().all(|c| c.is_ascii_digit()))
                .and_then(|s| s.parse::<u64>().ok());

            // Automatic fallback: if endpoint rejects native `tools`, remember
            // it and retry immediately with the text-JSON protocol.
            if use_native_tools
                && !fallback_attempted
                && matches!(code, 400 | 404 | 422)
            {
                let snippet = bounded_body(resp, 4096)
                    .await
                    .ok()
                    .and_then(|b| String::from_utf8(b).ok())
                    .unwrap_or_default()
                    .to_ascii_lowercase();
                let is_schema_or_name_error = snippet.contains("does not match")
                    || snippet.contains("must match")
                    || snippet.contains("invalid tool name")
                    || snippet.contains("invalid function name")
                    || snippet.contains("input_schema");
                let endpoint_lacks_tools = snippet.contains("not support")
                    || snippet.contains("doesn't support")
                    || snippet.contains("unsupported")
                    || snippet.contains("unknown field")
                    || snippet.contains("unrecognized")
                    || ((snippet.contains("tool") || snippet.contains("function"))
                        && !is_schema_or_name_error
                        && !snippet.contains("invalid"));
                if endpoint_lacks_tools && !is_schema_or_name_error {
                    self.tools_rejected.store(true, Ordering::Relaxed);
                    fallback_attempted = true;
                    continue;
                }
                anyhow::bail!(
                    "Model endpoint returned {status}. Provider rejected the request; check the model and endpoint settings."
                );
            }

            // Transient 429 / 5xx retry with Retry-After and cap.
            let retryable = code == 429 || matches!(code, 500 | 502 | 503 | 504);
            if retryable && attempt < self.max_retries {
                let wait = match retry_after {
                    Some(secs) if secs <= self.max_retry_delay_secs => {
                        Some(std::time::Duration::from_secs(secs))
                    }
                    Some(_) => None, // Retry-After exceeds cap -> surface immediately
                    None => Some(backoff_delay(
                        self.base_retry_ms,
                        attempt,
                        self.max_retry_delay_secs,
                    )),
                };
                if let Some(delay) = wait {
                    attempt += 1;
                    sleep_cancellable(delay, cancel).await?;
                    continue;
                }
            }

            let detail = match code {
                401 | 403 => {
                    "Authentication or access denied; check this provider's API key and permissions"
                        .into()
                }
                402 => "Provider credits or quota are exhausted; check the provider account".into(),
                429 => retry_after
                    .map(|n| format!("Provider rate limit reached; retry after {n} seconds"))
                    .unwrap_or_else(|| {
                        "Provider rate limit reached; wait before retrying or select another model"
                            .into()
                    }),
                500..=599 => {
                    "Provider service is unavailable; retry later or select another provider".into()
                }
                _ => "Provider rejected the request; check the model and endpoint settings".into(),
            };
            anyhow::bail!("Model endpoint returned {status}. {detail}.");
        };

        let sse = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|s| s.contains("text/event-stream"));
        let mut decoder = CompletionDecoder::default();
        if !sse {
            let bytes = bounded_body(response, 8 * 1024 * 1024).await?;
            let value: Value = serde_json::from_slice(&bytes).map_err(|_| {
                anyhow::anyhow!("Endpoint returned neither SSE nor completion JSON")
            })?;
            decoder.value(&value, on_delta)?;
        } else {
            let mut stream = response.bytes_stream();
            let mut bytes = Vec::new();
            let mut data = String::new();
            let mut done = false;
            let mut total = 0usize;
            while !done {
                let next_fut =
                    tokio::time::timeout(std::time::Duration::from_secs(30), stream.next());
                let chunk = match cancel {
                    Some(tok) => tokio::select! {
                        r = next_fut => r,
                        _ = tok.cancelled() => anyhow::bail!("Run cancelled"),
                    },
                    None => next_fut.await,
                }
                .map_err(|_| anyhow::anyhow!("No model data received for 30 seconds"))?;
                let Some(chunk) = chunk else {
                    break;
                };
                let chunk = chunk?;
                total += chunk.len();
                anyhow::ensure!(total <= 8 * 1024 * 1024, "Model response exceeded 8 MiB");
                bytes.extend_from_slice(&chunk);
                anyhow::ensure!(bytes.len() <= 8 * 1024 * 1024, "Model event exceeded 8 MiB");
                while let Some(end) = bytes.iter().position(|b| *b == b'\n') {
                    let line = bytes.drain(..=end).collect::<Vec<_>>();
                    let line = std::str::from_utf8(&line)?.trim_end_matches(['\r', '\n']);
                    if line.is_empty() {
                        if data.trim() == "[DONE]" {
                            done = true;
                            break;
                        }
                        if !data.is_empty() {
                            decoder.value(&serde_json::from_str::<Value>(&data)?, on_delta)?;
                            data.clear();
                        }
                    } else if let Some(payload) = line.strip_prefix("data:") {
                        if !data.is_empty() {
                            data.push('\n');
                        }
                        data.push_str(payload.trim_start());
                    }
                }
            }
            if !done {
                if !bytes.is_empty() {
                    let line = std::str::from_utf8(&bytes)?.trim();
                    if let Some(payload) = line.strip_prefix("data:") {
                        if !data.is_empty() {
                            data.push('\n');
                        }
                        data.push_str(payload.trim_start());
                    }
                }
                if data.trim() == "[DONE]" {
                    done = true;
                } else if !data.trim().is_empty() {
                    decoder.value(&serde_json::from_str::<Value>(&data)?, on_delta)?;
                }
                if !done && !decoder.saw_terminal {
                    anyhow::bail!(
                        "Model stream ended unexpectedly before terminal event (truncated response)"
                    );
                }
            }
        }
        decoder.finish(on_delta)
    }
    pub async fn chat(&self, messages: &[crate::ChatMessage]) -> Result<String> {
        self.chat_stream(messages, &mut |_| {}).await
    }
}

fn backoff_delay(base_ms: u64, attempt: u32, max_secs: u64) -> std::time::Duration {
    let shift = attempt.min(6);
    let ms = base_ms.saturating_mul(1u64 << shift);
    let cap_ms = max_secs.saturating_mul(1000);
    std::time::Duration::from_millis(ms.min(cap_ms))
}

async fn sleep_cancellable(
    delay: std::time::Duration,
    cancel: Option<&crate::CancellationToken>,
) -> Result<()> {
    if delay.is_zero() {
        if cancel.is_some_and(|c| c.is_cancelled()) {
            anyhow::bail!("Run cancelled");
        }
        return Ok(());
    }
    match cancel {
        Some(tok) => tokio::select! {
            _ = tokio::time::sleep(delay) => Ok(()),
            _ = tok.cancelled() => anyhow::bail!("Run cancelled"),
        },
        None => {
            tokio::time::sleep(delay).await;
            Ok(())
        }
    }
}

fn parse_assistant_calls_envelope(content: &str) -> Option<(String, Vec<NativeToolCall>)> {
    let v: Value = serde_json::from_str(content).ok()?;
    let thought = v
        .get("thought")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let calls: Vec<NativeToolCall> = serde_json::from_value(v.get("calls")?.clone()).ok()?;
    Some((thought, calls))
}

fn parse_tool_result_envelope(content: &str) -> Option<(String, String, bool, String)> {
    let v: Value = serde_json::from_str(content).ok()?;
    let id = v.get("tool_call_id")?.as_str()?.to_string();
    let name = v
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let ok = v.get("ok").and_then(Value::as_bool).unwrap_or(true);
    let body = v.get("content")?.as_str()?.to_string();
    Some((id, name, ok, body))
}

fn build_openai_messages(messages: &[crate::ChatMessage], use_native_tools: bool) -> Vec<Value> {
    let mut out = Vec::with_capacity(messages.len());
    for m in messages {
        if m.role == ROLE_ASSISTANT_TOOL_CALLS {
            if let Some((thought, calls)) = parse_assistant_calls_envelope(&m.content) {
                if use_native_tools {
                    let tool_calls: Vec<Value> = calls
                        .iter()
                        .map(|c| {
                            json!({
                                "id": c.id,
                                "type": "function",
                                "function": {
                                    "name": c.tool,
                                    "arguments": c.args.to_string(),
                                }
                            })
                        })
                        .collect();
                    let content_val = if thought.is_empty() {
                        Value::Null
                    } else {
                        Value::String(thought)
                    };
                    out.push(json!({
                        "role": "assistant",
                        "content": content_val,
                        "tool_calls": tool_calls,
                    }));
                } else {
                    let batch: Vec<Value> = calls
                        .iter()
                        .map(|c| json!({ "tool": c.tool, "args": c.args }))
                        .collect();
                    out.push(json!({
                        "role": "assistant",
                        "content": json!({ "batch": batch }).to_string(),
                    }));
                }
                continue;
            }
        }
        if m.role == ROLE_TOOL_RESULT {
            if let Some((id, name, _ok, body)) = parse_tool_result_envelope(&m.content) {
                if use_native_tools {
                    out.push(json!({
                        "role": "tool",
                        "tool_call_id": id,
                        "content": body,
                    }));
                } else {
                    out.push(json!({
                        "role": "user",
                        "content": format!("Observation [{name}]: {body}"),
                    }));
                }
                continue;
            }
        }
        if m.role == ROLE_USER_MULTIMODAL {
            out.push(json!({ "role": "user", "content": openai_multimodal_content(&m.content) }));
            continue;
        }
        out.push(json!({ "role": m.role, "content": m.content }));
    }
    out
}

const MAX_IMAGE_BYTES: u64 = 10 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ImageAttachment {
    pub name: String,
    pub mime: String,
    pub data_base64: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct MultimodalEnvelope {
    text: String,
    images: Vec<ImageAttachment>,
}

fn base64_encode(input: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[((triple >> 18) & 0x3F) as usize] as char);
        out.push(TABLE[((triple >> 12) & 0x3F) as usize] as char);
        out.push(if chunk.len() > 1 {
            TABLE[((triple >> 6) & 0x3F) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[(triple & 0x3F) as usize] as char
        } else {
            '='
        });
    }
    out
}

fn detect_image_mime(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("image/jpeg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else if bytes.starts_with(b"BM") {
        Some("image/bmp")
    } else {
        None
    }
}

/// Snapshot and validate an image file from disk ONCE at user-attach time.
pub fn load_image_attachment(path: &std::path::Path) -> anyhow::Result<ImageAttachment> {
    let meta = std::fs::metadata(path)
        .map_err(|e| anyhow::anyhow!("Cannot read image attachment {}: {e}", path.display()))?;
    anyhow::ensure!(meta.is_file(), "Image attachment is not a regular file");
    anyhow::ensure!(
        meta.len() <= MAX_IMAGE_BYTES,
        "Image attachment exceeds 10 MiB limit: {}",
        path.display()
    );
    let bytes = std::fs::read(path)
        .map_err(|e| anyhow::anyhow!("Failed to read image bytes {}: {e}", path.display()))?;
    let mime = detect_image_mime(&bytes)
        .ok_or_else(|| anyhow::anyhow!("Unsupported or invalid image header: {}", path.display()))?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "image".into());
    Ok(ImageAttachment {
        name,
        mime: mime.to_string(),
        data_base64: base64_encode(&bytes),
    })
}

/// Encode a trusted user message with pre-snapshotted image attachments.
pub fn encode_user_multimodal(text: &str, images: &[ImageAttachment]) -> crate::ChatMessage {
    if images.is_empty() {
        return crate::ChatMessage {
            role: "user".into(),
            content: text.to_string(),
        };
    }
    let env = MultimodalEnvelope {
        text: text.to_string(),
        images: images.to_vec(),
    };
    crate::ChatMessage {
        role: ROLE_USER_MULTIMODAL.into(),
        content: serde_json::to_string(&env).unwrap_or_else(|_| text.to_string()),
    }
}

fn openai_multimodal_content(raw_envelope: &str) -> Value {
    let Ok(env) = serde_json::from_str::<MultimodalEnvelope>(raw_envelope) else {
        return Value::String(raw_envelope.to_string());
    };
    let mut parts = Vec::with_capacity(env.images.len() + 1);
    if !env.text.is_empty() {
        parts.push(json!({ "type": "text", "text": env.text }));
    }
    for img in env.images {
        parts.push(json!({
            "type": "image_url",
            "image_url": { "url": format!("data:{};base64,{}", img.mime, img.data_base64) }
        }));
    }
    Value::Array(parts)
}

fn anthropic_multimodal_content(raw_envelope: &str) -> Value {
    let Ok(env) = serde_json::from_str::<MultimodalEnvelope>(raw_envelope) else {
        return Value::String(raw_envelope.to_string());
    };
    let mut parts = Vec::with_capacity(env.images.len() + 1);
    if !env.text.is_empty() {
        parts.push(json!({ "type": "text", "text": env.text }));
    }
    for img in env.images {
        parts.push(json!({
            "type": "image",
            "source": { "type": "base64", "media_type": img.mime, "data": img.data_base64 }
        }));
    }
    Value::Array(parts)
}
fn build_anthropic_messages(messages: &[crate::ChatMessage], use_native_tools: bool) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    let mut i = 0;
    while i < messages.len() {
        let m = &messages[i];
        if m.role == "system" {
            i += 1;
            continue;
        }
        if m.role == ROLE_ASSISTANT_TOOL_CALLS {
            if let Some((thought, calls)) = parse_assistant_calls_envelope(&m.content) {
                if use_native_tools {
                    let mut blocks = Vec::new();
                    if !thought.is_empty() {
                        blocks.push(json!({ "type": "text", "text": thought }));
                    }
                    for c in calls {
                        blocks.push(json!({
                            "type": "tool_use",
                            "id": c.id,
                            "name": c.tool,
                            "input": c.args,
                        }));
                    }
                    out.push(json!({ "role": "assistant", "content": blocks }));
                } else {
                    let batch: Vec<Value> = calls
                        .iter()
                        .map(|c| json!({ "tool": c.tool, "args": c.args }))
                        .collect();
                    out.push(json!({
                        "role": "assistant",
                        "content": json!({ "batch": batch }).to_string(),
                    }));
                }
                i += 1;
                continue;
            }
        }
        if m.role == ROLE_TOOL_RESULT {
            // Merge consecutive tool results into one user message (Anthropic requirement).
            let mut tool_blocks = Vec::new();
            let mut text_parts = Vec::new();
            while i < messages.len() && messages[i].role == ROLE_TOOL_RESULT {
                if let Some((id, name, ok, body)) = parse_tool_result_envelope(&messages[i].content) {
                    if use_native_tools {
                        tool_blocks.push(json!({
                            "type": "tool_result",
                            "tool_use_id": id,
                            "content": body,
                            "is_error": !ok,
                        }));
                    } else {
                        text_parts.push(format!("[{name}] {body}"));
                    }
                }
                i += 1;
            }
            if use_native_tools && !tool_blocks.is_empty() {
                out.push(json!({ "role": "user", "content": tool_blocks }));
            } else if !text_parts.is_empty() {
                out.push(json!({ "role": "user", "content": format!("Observations:\n{}", text_parts.join("\n")) }));
            }
            continue;
        }
        if m.role == ROLE_USER_MULTIMODAL {
            out.push(json!({ "role": "user", "content": anthropic_multimodal_content(&m.content) }));
            i += 1;
            continue;
        }
        out.push(json!({ "role": m.role, "content": m.content }));
        i += 1;
    }
    out
}

fn mark_anthropic_history_cache_breakpoint(messages: &mut [Value]) {
    if messages.len() < 2 {
        return;
    }
    if let Some(last_user) = messages
        .iter_mut()
        .rev()
        .find(|m| m.get("role").and_then(Value::as_str) == Some("user"))
    {
        if let Some(arr) = last_user.get_mut("content").and_then(Value::as_array_mut) {
            if let Some(last_block) = arr.last_mut() {
                last_block["cache_control"] = json!({ "type": "ephemeral" });
            }
        } else if let Some(text) = last_user.get("content").and_then(Value::as_str) {
            let owned = text.to_string();
            last_user["content"] = json!([{
                "type": "text",
                "text": owned,
                "cache_control": { "type": "ephemeral" }
            }]);
        }
    }
}

async fn bounded_body(mut response: reqwest::Response, limit: usize) -> Result<Vec<u8>> {
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        anyhow::ensure!(
            body.len().saturating_add(chunk.len()) <= limit,
            "Endpoint response exceeded {} bytes",
            limit
        );
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

#[derive(Default)]
struct ThinkSplitter {
    pending: String,
    thinking: bool,
}
impl ThinkSplitter {
    fn push(
        &mut self,
        text: &str,
        finish: bool,
        full: &mut String,
        callback: &mut (dyn FnMut(Delta) + Send),
    ) {
        self.pending.push_str(text);
        loop {
            let marker = if self.thinking { "</think>" } else { "<think>" };
            if let Some(pos) = self.pending.find(marker) {
                let text = self.pending[..pos].to_string();
                self.pending.drain(..pos + marker.len());
                self.emit(text, full, callback);
                self.thinking = !self.thinking;
            } else {
                let keep = if finish {
                    0
                } else {
                    (1..marker.len())
                        .rev()
                        .find(|n| self.pending.ends_with(&marker[..*n]))
                        .unwrap_or(0)
                };
                let split = self.pending.len() - keep;
                let text = self.pending[..split].to_string();
                self.pending.drain(..split);
                self.emit(text, full, callback);
                break;
            }
        }
    }
    fn emit(&self, text: String, full: &mut String, callback: &mut (dyn FnMut(Delta) + Send)) {
        if text.is_empty() {
            return;
        }
        if self.thinking {
            callback(Delta::Thinking(text));
        } else {
            full.push_str(&text);
            callback(Delta::Content(text));
        }
    }
}
#[derive(Default)]
struct FunctionCall {
    id: String,
    name: String,
    arguments: String,
}
#[derive(Default)]
struct CompletionDecoder {
    saw_terminal: bool,
    full: String,
    splitter: ThinkSplitter,
    usage: Option<(u64, u64, u64)>,
    tools: std::collections::BTreeMap<usize, FunctionCall>,
}
impl CompletionDecoder {
    fn value(&mut self, value: &Value, callback: &mut (dyn FnMut(Delta) + Send)) -> Result<()> {
        if value["type"].is_string() || value["content"].is_array() {
            return self.anthropic_value(value, callback);
        }
        if !value["error"].is_null() {
            anyhow::bail!("Model endpoint reported a streaming error");
        }
        if value["usage"].is_object() {
            let u = &value["usage"];
            self.usage = Some((
                u["prompt_tokens"].as_u64().unwrap_or(0),
                u["completion_tokens"].as_u64().unwrap_or(0),
                u["prompt_tokens_details"]["cached_tokens"]
                    .as_u64()
                    .unwrap_or(0),
            ));
        }
        if value["choices"][0]["finish_reason"].is_string() {
            self.saw_terminal = true;
        }
        let delta = &value["choices"][0]["delta"];
        let message = &value["choices"][0]["message"];
        let src = if delta.is_object() { delta } else { message };
        if let Some(reasoning) = src["reasoning_content"]
            .as_str()
            .or_else(|| src["reasoning"].as_str())
        {
            callback(Delta::Thinking(reasoning.into()));
        }
        if let Some(content) = src["content"].as_str() {
            self.splitter.push(content, false, &mut self.full, callback);
        }
        if let Some(calls) = src["tool_calls"].as_array() {
            for (index, call) in calls.iter().enumerate() {
                let tool = self
                    .tools
                    .entry(call["index"].as_u64().unwrap_or(index as u64) as usize)
                    .or_default();
                if let Some(id) = call["id"].as_str() {
                    if !id.is_empty() {
                        tool.id = id.to_string();
                    }
                }
                if let Some(name) = call["function"]["name"].as_str() {
                    tool.name.push_str(name);
                }
                if let Some(args) = call["function"]["arguments"].as_str() {
                    tool.arguments.push_str(args);
                }
            }
        }
        Ok(())
    }
    fn anthropic_value(
        &mut self,
        value: &Value,
        callback: &mut (dyn FnMut(Delta) + Send),
    ) -> Result<()> {
        let kind = value["type"].as_str().unwrap_or("");
        if matches!(kind, "message_stop" | "message")
            || (kind == "message_delta" && value["delta"]["stop_reason"].is_string())
        {
            self.saw_terminal = true;
        }
        anyhow::ensure!(
            kind != "error",
            "Anthropic endpoint reported a streaming error"
        );
        let usage = if kind == "message_start" {
            &value["message"]["usage"]
        } else {
            &value["usage"]
        };
        if usage.is_object() {
            let old = self.usage.unwrap_or_default();
            let cached = usage["cache_read_input_tokens"].as_u64().unwrap_or(old.2);
            let input = usage["input_tokens"]
                .as_u64()
                .map(|n| n + cached + usage["cache_creation_input_tokens"].as_u64().unwrap_or(0))
                .unwrap_or(old.0);
            self.usage = Some((
                input,
                usage["output_tokens"].as_u64().unwrap_or(old.1),
                cached,
            ));
        }
        if kind == "content_block_delta" {
            let d = &value["delta"];
            if let Some(text) = d["text"].as_str() {
                self.splitter.push(text, false, &mut self.full, callback);
            }
            if let Some(text) = d["thinking"].as_str() {
                callback(Delta::Thinking(text.into()));
            }
            if let Some(args) = d["partial_json"].as_str() {
                self.tools
                    .entry(value["index"].as_u64().unwrap_or(0) as usize)
                    .or_default()
                    .arguments
                    .push_str(args);
            }
        }
        let blocks = if kind == "content_block_start" {
            vec![&value["content_block"]]
        } else {
            value["content"]
                .as_array()
                .map(|v| v.iter().collect())
                .unwrap_or_default()
        };
        for (index, block) in blocks.into_iter().enumerate() {
            match block["type"].as_str() {
                Some("text") => {
                    if let Some(text) = block["text"].as_str() {
                        self.splitter.push(text, false, &mut self.full, callback);
                    }
                }
                Some("thinking") => {
                    if let Some(text) = block["thinking"].as_str() {
                        callback(Delta::Thinking(text.into()));
                    }
                }
                Some("tool_use") => {
                    let tool = self
                        .tools
                        .entry(value["index"].as_u64().unwrap_or(index as u64) as usize)
                        .or_default();
                    if let Some(id) = block["id"].as_str() {
                        if !id.is_empty() {
                            tool.id = id.to_string();
                        }
                    }
                    tool.name = block["name"].as_str().unwrap_or_default().into();
                    if block["input"].as_object().is_some_and(|v| !v.is_empty()) {
                        tool.arguments = block["input"].to_string();
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }
    fn finish(mut self, callback: &mut (dyn FnMut(Delta) + Send)) -> Result<String> {
        self.splitter.push("", true, &mut self.full, callback);
        if let Some((input, output, cached)) = self.usage {
            callback(Delta::Usage {
                input,
                output,
                cached,
            });
        }
        if !self.tools.is_empty() {
            let mut calls = Vec::new();
            for (slot, tool) in self.tools {
                anyhow::ensure!(!tool.name.is_empty(), "Tool call missing its name");
                let args: Value = serde_json::from_str(if tool.arguments.trim().is_empty() {
                    "{}"
                } else {
                    &tool.arguments
                })
                .map_err(|_| anyhow::anyhow!("Tool call arguments are not valid JSON"))?;
                let id = if tool.id.is_empty() {
                    format!("call_{slot}")
                } else {
                    tool.id
                };
                calls.push(json!({"id": id, "tool": tool.name, "args": args}));
            }
            return Ok(json!({
                "_native": true,
                "thought": self.full,
                "batch": calls,
            })
            .to_string());
        }
        anyhow::ensure!(
            !self.full.trim().is_empty(),
            "Model returned an empty completion"
        );
        Ok(self.full)
    }
}

#[cfg(test)]
mod protocol_regressions {
    use super::*;

    #[tokio::test]
    async fn provider_redirects_are_not_followed_with_credentials() {
        use wiremock::{matchers::method, Mock, MockServer, ResponseTemplate};
        let issuer = MockServer::start().await;
        let other = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":[]})))
            .expect(0)
            .mount(&other)
            .await;
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(302)
                    .insert_header("Location", format!("{}/models", other.uri())),
            )
            .mount(&issuer)
            .await;
        let mut client = ShimClient::with(format!("{}/v1", issuer.uri()), "fixture".into());
        client.api_key = Some(crate::providers::Secret::new(
            "offline-redirect-fixture".into(),
        ));
        assert!(client.models().await.is_err());
        assert!(other.received_requests().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn quota_and_rate_errors_are_actionable_and_do_not_echo_response_secrets() {
        use wiremock::{
            matchers::{method, path},
            Mock, MockServer, ResponseTemplate,
        };
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(
                ResponseTemplate::new(429)
                    .insert_header("Retry-After", "30")
                    .set_body_string("do-not-echo-provider-secret"),
            )
            .mount(&server)
            .await;
        let client = ShimClient::with(format!("{}/v1", server.uri()), "fixture".into());
        let error = client
            .chat_stream(
                &[crate::ChatMessage {
                    role: "user".into(),
                    content: "test".into(),
                }],
                &mut |_| {},
            )
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("rate limit") && error.contains("30 seconds"));
        assert!(!error.contains("do-not-echo"));
    }

    #[tokio::test]
    async fn adversarial_schema_error_does_not_disable_native_tools_and_anthropic_maps_effort() {
        use wiremock::{
            matchers::{method, path},
            Mock, MockServer, ResponseTemplate,
        };
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(
                ResponseTemplate::new(400)
                    .set_body_string(r#"{"error":"Invalid function name: string does not match pattern"}"#),
            )
            .expect(1)
            .mount(&server)
            .await;

        let mut client = ShimClient::with(format!("{}/v1", server.uri()), "strict-model".into());
        client.tools = vec![ToolSpec {
            name: "read_file".into(),
            description: "Read".into(),
            parameters: json!({"type": "object"}),
        }];
        let err = client
            .chat(&[crate::ChatMessage {
                role: "user".into(),
                content: "hi".into(),
            }])
            .await
            .expect_err("Schema validation error must not silently disable native tools");
        assert!(err.to_string().contains("400"), "{err}");
        assert!(
            !client.tools_rejected.load(std::sync::atomic::Ordering::Relaxed),
            "tools_rejected must stay false on a single bad schema/name error"
        );

        let mut anth = ShimClient::with("https://api.anthropic.com/v1".into(), "claude-sonnet".into());
        anth.kind = crate::providers::ProviderKind::Anthropic;
        anth.reasoning_effort = Some("high".into());
        let body = anth.build_request_body(
            &[crate::ChatMessage {
                role: "user".into(),
                content: "plan".into(),
            }],
            false,
        );
        assert_eq!(body["thinking"]["type"], "enabled");
        assert_eq!(body["thinking"]["budget_tokens"], 12288);
        assert!(body["max_tokens"].as_u64().unwrap() >= 16384);
    }
    #[tokio::test]
    async fn adversarial_truncated_sse_stream_without_terminal_event_errors() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        let truncated_sse = "data: {\"choices\":[{\"delta\":{\"content\":\"partial answer\"}}]}\n\n";
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(
                ResponseTemplate::new(200).set_body_raw(truncated_sse, "text/event-stream"),
            )
            .mount(&server)
            .await;

        let client = ShimClient::with(format!("{}/v1", server.uri()), "fixture".into());
        let err = client
            .chat(&[crate::ChatMessage {
                role: "user".into(),
                content: "hi".into(),
            }])
            .await
            .expect_err("Truncated SSE stream must not be accepted as a final answer");
        assert!(
            err.to_string().contains("truncated response"),
            "Unexpected error: {err}"
        );
    }

    #[tokio::test]
    async fn anthropic_uses_native_headers_body_and_cached_usage() {
        use wiremock::{
            matchers::{body_partial_json, header, method, path},
            Mock, MockServer, ResponseTemplate,
        };
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .and(header("x-api-key", "offline-anthropic-key"))
            .and(header("anthropic-version", "2023-06-01"))
            .and(body_partial_json(json!({"system":"Guide","messages":[{"role":"user","content":"hello"}],"max_tokens":4096})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"type":"message","content":[{"type":"thinking","thinking":"Check evidence"},{"type":"text","text":"Hello"}],"usage":{"input_tokens":12,"cache_read_input_tokens":3,"cache_creation_input_tokens":2,"output_tokens":4}})))
            .expect(1)
            .mount(&server)
            .await;
        let mut client = ShimClient::with(format!("{}/v1", server.uri()), "fixture".into());
        client.kind = crate::providers::ProviderKind::Anthropic;
        client.api_key = Some(crate::providers::Secret::new(
            "offline-anthropic-key".into(),
        ));
        let mut usage = None;
        let mut thinking = String::new();
        let result = client
            .chat_stream(
                &[
                    crate::ChatMessage {
                        role: "system".into(),
                        content: "Guide".into(),
                    },
                    crate::ChatMessage {
                        role: "user".into(),
                        content: "hello".into(),
                    },
                ],
                &mut |d| match d {
                    Delta::Usage {
                        input,
                        output,
                        cached,
                    } => usage = Some((input, output, cached)),
                    Delta::Thinking(t) => thinking.push_str(&t),
                    _ => {}
                },
            )
            .await
            .unwrap();
        assert_eq!(result, "Hello");
        assert_eq!(usage, Some((17, 4, 3)));
        assert_eq!(thinking, "Check evidence");
        assert!(server.received_requests().await.unwrap()[0]
            .headers
            .get("authorization")
            .is_none());
    }

    #[tokio::test]
    async fn anthropic_prompt_caching_and_native_tools_serialize_cache_control_and_ids() {
        use wiremock::{
            matchers::{method, path},
            Mock, MockServer, ResponseTemplate,
        };
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "type": "message",
                "content": [{ "type": "text", "text": "Done" }],
                "usage": { "input_tokens": 5, "output_tokens": 2 }
            })))
            .expect(1)
            .mount(&server)
            .await;

        let mut client = ShimClient::with(format!("{}/v1", server.uri()), "claude-sonnet".into());
        client.kind = crate::providers::ProviderKind::Anthropic;
        client.prompt_caching = true;
        client.tools = vec![ToolSpec {
            name: "read_file".into(),
            description: "Read a file".into(),
            parameters: json!({"type": "object", "properties": {"path": {"type": "string"}}, "required": ["path"]}),
        }];

        let messages = vec![
            crate::ChatMessage { role: "system".into(), content: "System rules".into() },
            crate::ChatMessage { role: "user".into(), content: "Inspect README".into() },
            encode_native_tool_calls(
                "",
                &[NativeToolCall {
                    id: "toolu_123".into(),
                    tool: "read_file".into(),
                    args: json!({"path": "README.md"}),
                }],
            ),
            encode_native_tool_result("toolu_123", "read_file", true, "# MEG"),
        ];

        let res = client.chat(&messages).await.unwrap();
        assert_eq!(res, "Done");

        let reqs = server.received_requests().await.unwrap();
        let body: Value = reqs[0].body_json().unwrap();
        assert_eq!(body["system"][0]["cache_control"]["type"], "ephemeral");
        assert_eq!(body["tools"][0]["name"], "read_file");
        assert_eq!(body["tools"][0]["cache_control"]["type"], "ephemeral");
        assert_eq!(body["messages"][1]["content"][0]["type"], "tool_use");
        assert_eq!(body["messages"][1]["content"][0]["id"], "toolu_123");
        assert_eq!(body["messages"][2]["content"][0]["type"], "tool_result");
        assert_eq!(body["messages"][2]["content"][0]["tool_use_id"], "toolu_123");
        assert_eq!(body["messages"][2]["content"][0]["cache_control"]["type"], "ephemeral");
    }

    #[tokio::test]
    async fn openai_native_tools_fall_back_to_text_json_when_endpoint_rejects_tools() {
        use wiremock::{
            matchers::{method, path},
            Mock, MockServer, ResponseTemplate,
        };
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(
                ResponseTemplate::new(400)
                    .set_body_string(r#"{"error":"This model does not support tools"}"#),
            )
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "choices": [{"message": {"content": "{\"final\":\"fallback ok\"}"}}]
            })))
            .mount(&server)
            .await;

        let mut client = ShimClient::with(format!("{}/v1", server.uri()), "local-7b".into());
        client.tools = vec![ToolSpec {
            name: "read_file".into(),
            description: "Read a file".into(),
            parameters: json!({"type": "object", "properties": {"path": {"type": "string"}}}),
        }];

        let out = client
            .chat(&[crate::ChatMessage {
                role: "user".into(),
                content: "hi".into(),
            }])
            .await
            .unwrap();
        assert_eq!(out, "{\"final\":\"fallback ok\"}");
        assert!(!client.native_tools_active());

        let reqs = server.received_requests().await.unwrap();
        assert_eq!(reqs.len(), 2);
        let first: Value = reqs[0].body_json().unwrap();
        let second: Value = reqs[1].body_json().unwrap();
        assert!(first["tools"].is_array());
        assert!(second.get("tools").is_none());
    }

    #[tokio::test]
    async fn transient_503_and_429_retry_with_backoff_and_recover() {
        use wiremock::{
            matchers::{method, path},
            Mock, MockServer, ResponseTemplate,
        };
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(503))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "0"))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "choices": [{"message": {"content": "recovered"}}]
            })))
            .mount(&server)
            .await;

        let mut client = ShimClient::with(format!("{}/v1", server.uri()), "fixture".into());
        client.base_retry_ms = 10;
        client.max_retries = 2;

        let out = client
            .chat(&[crate::ChatMessage {
                role: "user".into(),
                content: "ping".into(),
            }])
            .await
            .unwrap();
        assert_eq!(out, "recovered");
        assert_eq!(server.received_requests().await.unwrap().len(), 3);
    }

    #[tokio::test]
    async fn retry_backoff_aborts_immediately_on_cancellation() {
        use wiremock::{
            matchers::{method, path},
            Mock, MockServer, ResponseTemplate,
        };
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "5"))
            .mount(&server)
            .await;

        let mut client = ShimClient::with(format!("{}/v1", server.uri()), "fixture".into());
        client.max_retry_delay_secs = 10;
        let cancel = crate::CancellationToken::default();
        let stopper = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(60)).await;
            stopper.cancel();
        });

        let started = std::time::Instant::now();
        let err = client
            .chat_stream_cancellable(
                &[crate::ChatMessage {
                    role: "user".into(),
                    content: "ping".into(),
                }],
                Some(&cancel),
                &mut |_| {},
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("cancelled"), "{err}");
        assert!(started.elapsed() < std::time::Duration::from_secs(2));
    }

    #[test]
    fn anthropic_stream_reassembles_tool_arguments_and_usage() {
        let mut decoder = CompletionDecoder::default();
        let mut usage = None;
        let mut cb = |d| {
            if let Delta::Usage {
                input,
                output,
                cached,
            } = d
            {
                usage = Some((input, output, cached));
            }
        };
        for packet in [
            json!({"type":"message_start","message":{"usage":{"input_tokens":10,"cache_read_input_tokens":2,"output_tokens":0}}}),
            json!({"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_01","name":"read_file","input":{}}}),
            json!({"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"path\":"}}),
            json!({"type":"content_block_delta","index":1,"delta":{"partial_json":"\"README.md\"}"}}),
            json!({"type":"message_delta","usage":{"output_tokens":7}}),
        ] {
            decoder.value(&packet, &mut cb).unwrap();
        }
        let finished = decoder.finish(&mut cb).unwrap();
        let (_thought, calls) = extract_native_tool_calls(&finished).unwrap();
        assert_eq!(calls[0].id, "toolu_01");
        assert_eq!(calls[0].tool, "read_file");
        let action = crate::parse_model_output(&finished).unwrap();
        assert!(matches!(action, crate::Action::Batch { .. }));
        assert_eq!(usage, Some((12, 7, 2)));
    }

    #[tokio::test]
    async fn pretty_printed_plain_completion_is_not_discarded() {
        use wiremock::{
            matchers::{method, path},
            Mock, MockServer, ResponseTemplate,
        };
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                "{\n  \"choices\": [{\"message\": {\"content\": \"Hello \u{4E16}\u{754C}\"}}]\n}\n",
            ))
            .mount(&server)
            .await;
        let client = ShimClient::with(format!("{}/v1", server.uri()), "fixture".into());
        let result = client
            .chat_stream(
                &[crate::ChatMessage {
                    role: "user".into(),
                    content: "hello".into(),
                }],
                &mut |_| {},
            )
            .await
            .unwrap();
        assert_eq!(result, "Hello \u{4E16}\u{754C}");
    }
}

#[cfg(test)]
mod streaming_regressions {
    use super::*;

    #[test]
    fn split_thinking_markers_are_reassembled_without_leaking_markup() {
        let mut decoder = CompletionDecoder::default();
        let mut content = String::new();
        let mut thinking = String::new();
        let mut cb = |delta| match delta {
            Delta::Content(t) => content.push_str(&t),
            Delta::Thinking(t) => thinking.push_str(&t),
            _ => {}
        };
        for chunk in ["Hello <th", "ink>internal", "</thi", "nk> \u{4E16}\u{754C}"] {
            decoder
                .value(
                    &json!({"choices":[{"delta":{"content":chunk}}]}),
                    &mut cb,
                )
                .unwrap();
        }
        let full = decoder.finish(&mut cb).unwrap();
        assert_eq!(full, "Hello  \u{4E16}\u{754C}");
        assert_eq!(content, full);
        assert_eq!(thinking, "internal");
    }

    #[test]
    fn fragmented_function_calls_and_usage_are_not_discarded() {
        let mut decoder = CompletionDecoder::default();
        let mut usage = None;
        let mut cb = |d| {
            if let Delta::Usage {
                input,
                output,
                cached,
            } = d
            {
                usage = Some((input, output, cached));
            }
        };
        for value in [
            json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_abc","function":{"name":"read_file","arguments":"{\"path\":"}}]}}]}),
            json!({"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"README.md\"}"}}]}}],"usage":{"prompt_tokens":10,"completion_tokens":5}}),
        ] {
            decoder.value(&value, &mut cb).unwrap();
        }
        let text = decoder.finish(&mut cb).unwrap();
        let (_thought, calls) = extract_native_tool_calls(&text).unwrap();
        assert_eq!(calls[0].id, "call_abc");
        let action = crate::parse_model_output(&text).unwrap();
        assert!(matches!(action, crate::Action::Batch { .. }));
        assert_eq!(usage, Some((10, 5, 0)));
    }
}
impl Default for ShimClient {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod multimodal_tests {
    use super::*;

    #[test]
    fn multimodal_attachments_snapshot_once_and_block_user_string_smuggling() {
        let dir = std::env::temp_dir().join(format!("meg-mm-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let img = dir.join("pixel.png");
        let png_bytes: &[u8] = &[
            0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D,
        ];
        std::fs::write(&img, png_bytes).unwrap();

        // 1. Snapshot at attach time, then delete the source file to prove serializer never re-reads disk
        let att = load_image_attachment(&img).unwrap();
        std::fs::remove_file(&img).unwrap();
        let msg = encode_user_multimodal("Describe this", &[att]);

        let oa_msgs = build_openai_messages(std::slice::from_ref(&msg), true);
        let oa_arr = oa_msgs[0]["content"].as_array().expect("OpenAI multimodal array");
        assert_eq!(oa_arr.len(), 2);
        assert_eq!(oa_arr[1]["type"], "image_url");
        assert!(oa_arr[1]["image_url"]["url"]
            .as_str()
            .unwrap()
            .starts_with("data:image/png;base64,"));

        let an_msgs = build_anthropic_messages(std::slice::from_ref(&msg), true);
        let an_arr = an_msgs[0]["content"].as_array().expect("Anthropic multimodal array");
        assert_eq!(an_arr.len(), 2);
        assert_eq!(an_arr[1]["type"], "image");
        assert_eq!(an_arr[1]["source"]["media_type"], "image/png");

        // 2. Adversarial smuggling: a plain role:"user" observation containing [[MEG_IMAGE:...]]
        //    must NEVER read disk or produce multimodal blocks.
        let secret_img = dir.join("secret.png");
        std::fs::write(&secret_img, png_bytes).unwrap();
        let smuggled = crate::ChatMessage {
            role: "user".into(),
            content: format!("Observation [read_file]: [[MEG_IMAGE:{}]]", secret_img.display()),
        };
        let oa_smug = build_openai_messages(std::slice::from_ref(&smuggled), false);
        assert!(
            oa_smug[0]["content"].is_string(),
            "Untrusted user-role observation must never turn into image_url blocks"
        );
        let an_smug = build_anthropic_messages(std::slice::from_ref(&smuggled), false);
        assert!(
            an_smug[0]["content"].is_string(),
            "Untrusted user-role observation must never turn into Anthropic image blocks"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
