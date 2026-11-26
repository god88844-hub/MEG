//! Streaming client for explicitly configured local/cloud endpoints.
//! Supports native OpenAI & Anthropic tool calling (with automatic text-JSON
//! fallback), Anthropic prompt caching (`cache_control`), and cancellable
//! exponential-backoff retries on 429 / transient 5xx responses.

mod decoder;
mod media;
mod wire;
#[cfg(test)]
mod tests;

pub use media::{encode_user_multimodal, load_image_attachment, ImageAttachment};

use anyhow::Result;
use decoder::CompletionDecoder;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use wire::{build_anthropic_messages, build_openai_messages, mark_anthropic_history_cache_breakpoint};

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
    dynamic_tools: std::sync::Mutex<Vec<ToolSpec>>,
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
            dynamic_tools: std::sync::Mutex::new(Vec::new()),
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
    /// Register a deferred tool schema for subsequent turns (deduplicated by tool name).
    pub fn register_dynamic_tool(&self, spec: ToolSpec) -> bool {
        if self.tools.iter().any(|t| t.name == spec.name) {
            return false;
        }
        let mut dyn_tools = self.dynamic_tools.lock().unwrap();
        if dyn_tools.iter().any(|t| t.name == spec.name) {
            return false;
        }
        dyn_tools.push(spec);
        true
    }
    /// Return base tools combined with any dynamically loaded deferred tools.
    pub fn effective_tools(&self) -> Vec<ToolSpec> {
        let mut combined = self.tools.clone();
        let dyn_tools = self.dynamic_tools.lock().unwrap();
        for t in dyn_tools.iter() {
            if !combined.iter().any(|existing| existing.name == t.name) {
                combined.push(t.clone());
            }
        }
        combined
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
                let active_tools = self.effective_tools();
                let mut tools_json: Vec<Value> = active_tools
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
                let active_tools = self.effective_tools();
                let tools_json: Vec<Value> = active_tools
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


impl Default for ShimClient {
    fn default() -> Self {
        Self::new()
    }
}

