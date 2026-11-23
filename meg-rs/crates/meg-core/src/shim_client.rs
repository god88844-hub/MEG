//! Streaming client for explicitly configured local/cloud endpoints.
//! The historical module name remains for source compatibility.

use anyhow::Result;
use serde_json::Value;

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

pub struct ShimClient {
    http: reqwest::Client,
    pub base_url: String,
    pub model: String,
    pub reasoning_effort: Option<String>,
    pub kind: crate::providers::ProviderKind,
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
        client.api_key = crate::providers::api_key(profile)?;
        Ok(client)
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
            serde_json::json!({})
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
            value = serde_json::json!({"data":tags["models"].as_array().into_iter().flatten().map(|m|serde_json::json!({"id":m["name"]})).collect::<Vec<_>>()});
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
    pub async fn chat_stream(
        &self,
        messages: &[crate::ChatMessage],
        on_delta: &mut (dyn FnMut(Delta) + Send),
    ) -> Result<String> {
        use futures_util::StreamExt;
        let mut body = serde_json::json!({"model":self.model,"messages":messages,"stream":true,"stream_options":{"include_usage":true}});
        let anthropic = self.kind == crate::providers::ProviderKind::Anthropic;
        if anthropic {
            body = serde_json::json!({"model":self.model,"messages":messages.iter().filter(|m|m.role!="system").collect::<Vec<_>>(),"system":messages.iter().filter(|m|m.role=="system").map(|m|m.content.as_str()).collect::<Vec<_>>().join("\n"),"max_tokens":4096,"stream":true});
        }
        if let Some(effort) = &self.reasoning_effort {
            if !anthropic {
                body["reasoning_effort"] = Value::String(effort.clone());
            }
        }
        let response = self
            .request(
                reqwest::Method::POST,
                if anthropic {
                    "/messages"
                } else {
                    "/chat/completions"
                },
            )
            .json(&body)
            .timeout(std::time::Duration::from_secs(180))
            .send()
            .await?;
        if !response.status().is_success() {
            let status = response.status();
            let retry = response
                .headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|v| v.to_str().ok())
                .filter(|s| s.len() <= 10 && s.chars().all(|c| c.is_ascii_digit()));
            let detail = match status.as_u16() {
                401 | 403 => {
                    "Authentication or access denied; check this provider's API key and permissions"
                        .into()
                }
                402 => "Provider credits or quota are exhausted; check the provider account".into(),
                429 => retry
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
        }
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
                let chunk = tokio::time::timeout(std::time::Duration::from_secs(30), stream.next())
                    .await
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
                if !data.trim().is_empty() && data.trim() != "[DONE]" {
                    decoder.value(&serde_json::from_str::<Value>(&data)?, on_delta)?;
                }
            }
        }
        decoder.finish(on_delta)
    }
    pub async fn chat(&self, messages: &[crate::ChatMessage]) -> Result<String> {
        self.chat_stream(messages, &mut |_| {}).await
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
    name: String,
    arguments: String,
}
#[derive(Default)]
struct CompletionDecoder {
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
            for (_, tool) in self.tools {
                anyhow::ensure!(!tool.name.is_empty(), "Tool call missing its name");
                let args: Value = serde_json::from_str(if tool.arguments.trim().is_empty() {
                    "{}"
                } else {
                    &tool.arguments
                })
                .map_err(|_| anyhow::anyhow!("Tool call arguments are not valid JSON"))?;
                calls.push(serde_json::json!({"tool":tool.name,"args":args}));
            }
            return Ok(serde_json::json!({"batch":calls}).to_string());
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
        use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};
        let issuer = MockServer::start().await;
        let other = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"data":[]})))
            .expect(0)
            .mount(&other).await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(302).insert_header("Location", format!("{}/models",other.uri())))
            .mount(&issuer).await;
        let mut client = ShimClient::with(format!("{}/v1",issuer.uri()),"fixture".into());
        client.api_key = Some(crate::providers::Secret::new("offline-redirect-fixture".into()));
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
    async fn anthropic_uses_native_headers_body_and_cached_usage() {
        use wiremock::{
            matchers::{body_partial_json, header, method, path},
            Mock, MockServer, ResponseTemplate,
        };
        let server = MockServer::start().await;
        Mock::given(method("POST")).and(path("/v1/messages"))
            .and(header("x-api-key","offline-anthropic-key"))
            .and(header("anthropic-version","2023-06-01"))
            .and(body_partial_json(serde_json::json!({"system":"Guide","messages":[{"role":"user","content":"hello"}],"max_tokens":4096})))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"type":"message","content":[{"type":"thinking","thinking":"Check evidence"},{"type":"text","text":"Hello"}],"usage":{"input_tokens":12,"cache_read_input_tokens":3,"cache_creation_input_tokens":2,"output_tokens":4}})))
            .expect(1).mount(&server).await;
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
            serde_json::json!({"type":"message_start","message":{"usage":{"input_tokens":10,"cache_read_input_tokens":2,"output_tokens":0}}}),
            serde_json::json!({"type":"content_block_start","index":1,"content_block":{"type":"tool_use","name":"read_file","input":{}}}),
            serde_json::json!({"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"path\":"}}),
            serde_json::json!({"type":"content_block_delta","index":1,"delta":{"partial_json":"\"README.md\"}"}}),
            serde_json::json!({"type":"message_delta","usage":{"output_tokens":7}}),
        ] {
            decoder.value(&packet, &mut cb).unwrap();
        }
        let action = crate::parse_model_output(&decoder.finish(&mut cb).unwrap()).unwrap();
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
                "{\n  \"choices\": [{\"message\": {\"content\": \"Hello 世界\"}}]\n}\n",
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
        assert_eq!(result, "Hello 世界");
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
        for chunk in ["Hello <th", "ink>internal", "</thi", "nk> 世界"] {
            decoder
                .value(
                    &serde_json::json!({"choices":[{"delta":{"content":chunk}}]}),
                    &mut cb,
                )
                .unwrap();
        }
        let full = decoder.finish(&mut cb).unwrap();
        assert_eq!(full, "Hello  世界");
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
            serde_json::json!({"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"name":"read_file","arguments":"{\"path\":"}}]}}]}),
            serde_json::json!({"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"README.md\"}"}}]}}],"usage":{"prompt_tokens":10,"completion_tokens":5}}),
        ] {
            decoder.value(&value, &mut cb).unwrap();
        }
        let text = decoder.finish(&mut cb).unwrap();
        let action = crate::parse_model_output(&text).unwrap();
        assert!(matches!(action, crate::Action::Batch { .. }));
        assert_eq!(usage, Some((10, 5, 0)));
    }
}
