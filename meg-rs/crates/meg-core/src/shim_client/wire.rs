use super::media::{anthropic_multimodal_content, openai_multimodal_content};
use super::{NativeToolCall, ROLE_ASSISTANT_TOOL_CALLS, ROLE_TOOL_RESULT, ROLE_USER_MULTIMODAL};
use serde_json::{json, Value};

pub(super) fn parse_assistant_calls_envelope(content: &str) -> Option<(String, Vec<NativeToolCall>)> {
    let v: Value = serde_json::from_str(content).ok()?;
    let thought = v
        .get("thought")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let calls: Vec<NativeToolCall> = serde_json::from_value(v.get("calls")?.clone()).ok()?;
    Some((thought, calls))
}


pub(super) fn parse_tool_result_envelope(content: &str) -> Option<(String, String, bool, String)> {
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


pub(super) fn build_openai_messages(messages: &[crate::ChatMessage], use_native_tools: bool) -> Vec<Value> {
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


pub(super) fn build_anthropic_messages(messages: &[crate::ChatMessage], use_native_tools: bool) -> Vec<Value> {
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


pub(super) fn mark_anthropic_history_cache_breakpoint(messages: &mut [Value]) {
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

