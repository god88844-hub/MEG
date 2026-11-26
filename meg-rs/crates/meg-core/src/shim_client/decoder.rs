use super::Delta;
use anyhow::Result;
use serde_json::{json, Value};

#[derive(Default)]
pub(super) struct ThinkSplitter {
    pub(super) pending: String,
    pub(super) thinking: bool,
}


impl ThinkSplitter {
    pub(super) fn push(
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
    pub(super) fn emit(&self, text: String, full: &mut String, callback: &mut (dyn FnMut(Delta) + Send)) {
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
pub(super) struct FunctionCall {
    pub(super) id: String,
    pub(super) name: String,
    pub(super) arguments: String,
}


#[derive(Default)]
pub(super) struct CompletionDecoder {
    pub(super) saw_terminal: bool,
    pub(super) full: String,
    pub(super) splitter: ThinkSplitter,
    pub(super) usage: Option<(u64, u64, u64)>,
    pub(super) tools: std::collections::BTreeMap<usize, FunctionCall>,
}


impl CompletionDecoder {
    pub(super) fn value(&mut self, value: &Value, callback: &mut (dyn FnMut(Delta) + Send)) -> Result<()> {
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
    pub(super) fn anthropic_value(
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
    pub(super) fn finish(mut self, callback: &mut (dyn FnMut(Delta) + Send)) -> Result<String> {
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

