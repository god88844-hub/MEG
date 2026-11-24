//! JSON repair + tool-call extraction (Prompt 1 Task 1.1, Rust port).
//!
//! Free models emit unescaped control characters inside JSON strings, fenced
//! JSON, `<tool_call>` XML markup in content/reasoning, and `<think>` blocks.
//! The Python harness shipped this in commit 261af92 (repairJson +
//! _recover_tool_calls_from_markup); this is the Rust clean-room port.

use regex::Regex;
use serde_json::Value;
use std::sync::OnceLock;

/// Escape raw unescaped control characters (newline, tab, CR, ...) that appear
/// INSIDE JSON string literals so serde_json can parse the object, while
/// preserving multi-byte UTF-8 characters intact.
pub fn repair_json(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len() + 16);
    let mut in_string = false;
    let mut escaped = false;
    for ch in raw.chars() {
        if escaped {
            out.push(ch);
            escaped = false;
            continue;
        }
        if ch == '\\' && in_string {
            out.push(ch);
            escaped = true;
            continue;
        }
        if ch == '"' {
            in_string = !in_string;
            out.push(ch);
            continue;
        }
        let code = ch as u32;
        if in_string && code <= 0x1f {
            out.push_str(&format!("\\u{code:04x}"));
            continue;
        }
        out.push(ch);
    }
    out
}

/// Extract the first balanced JSON object from arbitrary model text (skips
/// prose, markdown fences, `<think>` blocks). Returns the substring.
pub fn extract_json_object(text: &str) -> Option<&str> {
    let mut start: Option<usize> = None;
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for (i, ch) in text.char_indices() {
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        match ch {
            '"' => in_string = true,
            '{' => {
                if depth == 0 {
                    start = Some(i);
                }
                depth += 1;
            }
            '}' if depth > 0 => {
                depth -= 1;
                if depth == 0 {
                    if let Some(s) = start {
                        return Some(&text[s..=i]);
                    }
                }
            }
            _ => {}
        }
    }
    None
}

/// Strip `<think>...</think>` reasoning blocks (OpenRouter free models).
pub fn strip_think(text: &str) -> String {
    static THINK_RE: OnceLock<Regex> = OnceLock::new();
    let re = THINK_RE.get_or_init(|| Regex::new(r"(?is)<think>.*?</think>").unwrap());
    let mut out = re.replace_all(text, "").to_string();
    // Unclosed think block: drop everything from <think> on.
    if let Some(idx) = out.find("<think>") {
        out.truncate(idx);
    }
    out.trim().to_string()
}

/// Recover `<tool_call><function=name><parameter=key>value</parameter>...`
/// markup (duck-harness pattern) into a JSON object {name, args}.
pub fn recover_tool_call_markup(text: &str) -> Vec<(String, serde_json::Map<String, Value>)> {
    static CALL_RE: OnceLock<Regex> = OnceLock::new();
    static PARAM_RE: OnceLock<Regex> = OnceLock::new();
    // Rust regex has no backreferences: match any </function...> closer (models
    // emit `</function=name>` too) and take the name from the opening tag.
    let re = CALL_RE.get_or_init(|| {
        Regex::new(
            r"(?is)<tool_call>\s*<function\s*=\s*([A-Za-z0-9_\-\.]+)\s*>(.*?)</function[^>]*>",
        )
        .unwrap()
    });
    let param_re = PARAM_RE.get_or_init(|| {
        Regex::new(r"(?is)<parameter\s*=\s*([A-Za-z0-9_\-\.]+)\s*>(.*?)</parameter>").unwrap()
    });
    let mut calls = Vec::new();
    for cap in re.captures_iter(text) {
        let name = cap[1].trim().to_string();
        let mut args = serde_json::Map::new();
        for p in param_re.captures_iter(&cap[2]) {
            let key = p[1].trim().to_string();
            let raw_val = p[2].trim().to_string();
            let value = match serde_json::from_str::<Value>(&raw_val) {
                Ok(v) => v,
                Err(_) => Value::String(raw_val),
            };
            args.insert(key, value);
        }
        calls.push((name, args));
    }
    calls
}

/// Recover fenced ```json blocks from content (some models answer in fences
/// even when told not to).
pub fn recover_fenced_json(text: &str) -> Vec<Value> {
    static FENCED_RE: OnceLock<Regex> = OnceLock::new();
    let re = FENCED_RE.get_or_init(|| {
        Regex::new(r"(?is)```(?:json)?\s*(\{.*?\}|\[.*?\])\s*```").unwrap()
    });
    let mut out = Vec::new();
    for cap in re.captures_iter(text) {
        if let Ok(v) = serde_json::from_str::<Value>(&repair_json(&cap[1])) {
            out.push(v);
        }
    }
    out
}

/// The tolerant action parse: given raw model output, find the next action
/// object. Returns the parsed Value. Mirrors the Python `_parse_action`
/// fallback ladder: fenced json -> balanced object -> repaired object ->
/// markup recovery -> fenced recovery.
pub fn parse_action(raw: &str) -> Option<Value> {
    let cleaned = strip_think(raw);
    // 1. fenced blocks first
    for v in recover_fenced_json(&cleaned) {
        if v.get("tool").is_some() || v.get("final").is_some() || v.get("batch").is_some() {
            return Some(v);
        }
    }
    // 2. balanced object, repaired if needed
    if let Some(obj) = extract_json_object(&cleaned) {
        if let Ok(v) = serde_json::from_str::<Value>(obj) {
            return Some(v);
        }
        if let Ok(v) = serde_json::from_str::<Value>(&repair_json(obj)) {
            return Some(v);
        }
    }
    // 3. whole-text repair
    if let Ok(v) = serde_json::from_str::<Value>(&repair_json(&cleaned)) {
        if v.is_object() {
            return Some(v);
        }
    }
    // 4. tool-call markup recovery
    let markup = recover_tool_call_markup(&cleaned);
    if let Some((name, args)) = markup.first() {
        let mut obj = serde_json::Map::new();
        obj.insert("thought".into(), Value::String("recovered from markup".into()));
        obj.insert("tool".into(), Value::String(name.clone()));
        obj.insert("args".into(), Value::Object(args.clone()));
        return Some(Value::Object(obj));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repairs_unescaped_newline_in_string() {
        let raw = r#"{"thought":"line1
line2","tool":"read"}"#;
        let v: Value = serde_json::from_str(&repair_json(raw)).unwrap();
        assert_eq!(v["tool"], "read");
        assert!(v["thought"].as_str().unwrap().contains('\n'));
    }

    #[test]
    fn preserves_multibyte_utf8_during_repair() {
        let raw = "{\"thought\":\"\u{4E16}\u{754C}\n\u{0928}\u{092E}\u{0938}\u{094D}\u{0924}\u{0947} \u{1F680}\",\"final\":\"ok\"}";
        let v: Value = serde_json::from_str(&repair_json(raw)).unwrap();
        assert_eq!(
            v["thought"].as_str().unwrap(),
            "\u{4E16}\u{754C}\n\u{0928}\u{092E}\u{0938}\u{094D}\u{0924}\u{0947} \u{1F680}"
        );
        assert_eq!(v["final"], "ok");
    }

    #[test]
    fn extracts_object_from_prose() {
        let text = "Sure! Here is my action:\n{\"tool\":\"bash\",\"args\":{\"cmd\":\"ls\"}}\nDone.";
        let v = parse_action(text).unwrap();
        assert_eq!(v["tool"], "bash");
    }

    #[test]
    fn parses_fenced_json_with_newlines_in_strings() {
        let text = "```json\n{\"thought\":\"a\nb\",\"final\":\"done\"}\n```";
        let v = parse_action(text).unwrap();
        assert_eq!(v["final"], "done");
    }

    #[test]
    fn strips_think_blocks() {
        let text = "<think>internal reasoning here</think>{\"final\":\"ok\"}";
        let v = parse_action(text).unwrap();
        assert_eq!(v["final"], "ok");
    }

    #[test]
    fn recovers_tool_call_markup() {
        let text = "<tool_call><function=read_file><parameter=path>src/main.rs</parameter></function=read_file></tool_call>";
        let v = parse_action(text).unwrap();
        assert_eq!(v["tool"], "read_file");
        assert_eq!(v["args"]["path"], "src/main.rs");
    }
}