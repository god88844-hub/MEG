//! Phase 6.1A: Offline wiremock-driven end-to-end harness tests for meg-core.

use async_trait::async_trait;
use meg_core::providers::ProviderKind;
use meg_core::shim_client::{ShimClient, ToolSpec};
use meg_core::{
    compact_transcript, is_read_only_tool, run_loop_with_config, ChatMessage, LoopConfig,
    LoopEngine,
};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

struct ShimTestEngine {
    client: ShimClient,
    on_tool: Box<dyn Fn(&str, &Value) -> anyhow::Result<String> + Send + Sync>,
    in_flight: Arc<AtomicUsize>,
    max_in_flight: Arc<AtomicUsize>,
    tool_delay_ms: u64,
    seen_transcripts: Mutex<Vec<Vec<ChatMessage>>>,
}

#[async_trait]
impl LoopEngine for ShimTestEngine {
    async fn execute_tool(&self, tool: &str, args: &Value) -> anyhow::Result<String> {
        let cur = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
        self.max_in_flight.fetch_max(cur, Ordering::SeqCst);
        if self.tool_delay_ms > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(self.tool_delay_ms)).await;
        }
        let res = (self.on_tool)(tool, args);
        self.in_flight.fetch_sub(1, Ordering::SeqCst);
        res
    }

    async fn generate(&self, transcript: &[ChatMessage]) -> anyhow::Result<String> {
        self.seen_transcripts
            .lock()
            .unwrap()
            .push(transcript.to_vec());
        self.client.chat(transcript).await
    }
}

#[tokio::test]
async fn e2e_anthropic_429_retry_prompt_caching_and_native_tool_use_round_trip() {
    let server = MockServer::start().await;

    // 1st POST: transient 429 with Retry-After: 0.
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "0"))
        .up_to_n_times(1)
        .mount(&server)
        .await;

    // 2nd POST (retry succeeds): native Anthropic tool_use block.
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "type": "message",
            "content": [
                { "type": "text", "text": "Checking config" },
                { "type": "tool_use", "id": "toolu_e2e_1", "name": "lookup", "input": { "key": "alpha" } }
            ],
            "usage": { "input_tokens": 100, "cache_creation_input_tokens": 50, "output_tokens": 20 }
        })))
        .up_to_n_times(1)
        .mount(&server)
        .await;

    // 3rd POST: final answer after tool_result.
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "type": "message",
            "content": [
                { "type": "text", "text": "resolved: value-alpha" }
            ],
            "usage": { "input_tokens": 10, "cache_read_input_tokens": 150, "output_tokens": 8 }
        })))
        .mount(&server)
        .await;

    let mut client = ShimClient::with(format!("{}/v1", server.uri()), "claude-sonnet".into());
    client.kind = ProviderKind::Anthropic;
    client.prompt_caching = true;
    client.base_retry_ms = 5;
    client.max_retries = 2;
    client.tools = vec![ToolSpec {
        name: "lookup".into(),
        description: "Lookup a key".into(),
        parameters: json!({"type": "object", "properties": {"key": {"type": "string"}}, "required": ["key"]}),
    }];

    let engine = ShimTestEngine {
        client,
        on_tool: Box::new(|tool, args| {
            assert_eq!(tool, "lookup");
            assert_eq!(args["key"], "alpha");
            Ok("value-alpha".into())
        }),
        in_flight: Arc::new(AtomicUsize::new(0)),
        max_in_flight: Arc::new(AtomicUsize::new(0)),
        tool_delay_ms: 0,
        seen_transcripts: Mutex::new(Vec::new()),
    };

    let (outcomes, answer) =
        run_loop_with_config(&engine, "Look up alpha", &[], &LoopConfig::default())
            .await
            .unwrap();
    assert_eq!(answer.as_deref(), Some("resolved: value-alpha"));
    assert_eq!(outcomes.len(), 2);

    let reqs = server.received_requests().await.unwrap();
    assert_eq!(reqs.len(), 3, "expected 429 + retry + final turn");
    let third_body: Value = reqs[2].body_json().unwrap();
    assert_eq!(third_body["system"][0]["cache_control"]["type"], "ephemeral");
    assert_eq!(third_body["tools"][0]["cache_control"]["type"], "ephemeral");
    let msgs = third_body["messages"].as_array().unwrap();
    assert_eq!(msgs[1]["content"][1]["type"], "tool_use");
    assert_eq!(msgs[1]["content"][1]["id"], "toolu_e2e_1");
    assert_eq!(msgs[2]["content"][0]["type"], "tool_result");
    assert_eq!(msgs[2]["content"][0]["tool_use_id"], "toolu_e2e_1");
    assert_eq!(msgs[2]["content"][0]["cache_control"]["type"], "ephemeral");
}

#[tokio::test]
async fn e2e_native_tool_fallback_and_doom_loop_recovery() {
    let server = MockServer::start().await;

    // 1st POST: endpoint rejects native tools -> client falls back automatically.
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(400)
                .set_body_string(r#"{"error":"Model does not support tools"}"#),
        )
        .up_to_n_times(1)
        .mount(&server)
        .await;

    // Next 3 responses: model repeats the exact same failing call 3 times.
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "choices": [{
                "message": { "content": "{\"tool\":\"read_file\",\"args\":{\"path\":\"wrong.txt\"}}" }
            }]
        })))
        .up_to_n_times(3)
        .mount(&server)
        .await;

    // 4th response (after DOOM_LOOP_WARNING): model switches to the right file.
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "choices": [{
                "message": { "content": "{\"tool\":\"read_file\",\"args\":{\"path\":\"right.txt\"}}" }
            }]
        })))
        .up_to_n_times(1)
        .mount(&server)
        .await;

    // 5th response: final answer.
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "choices": [{
                "message": { "content": "{\"final\":\"recovered from doom loop\"}" }
            }]
        })))
        .mount(&server)
        .await;

    let mut client = ShimClient::with(format!("{}/v1", server.uri()), "local-7b".into());
    client.tools = vec![ToolSpec {
        name: "read_file".into(),
        description: "Read file".into(),
        parameters: json!({"type": "object", "properties": {"path": {"type": "string"}}}),
    }];

    let engine = ShimTestEngine {
        client,
        on_tool: Box::new(|_tool, args| {
            if args["path"] == "right.txt" {
                Ok("good-data".into())
            } else {
                anyhow::bail!("missing file")
            }
        }),
        in_flight: Arc::new(AtomicUsize::new(0)),
        max_in_flight: Arc::new(AtomicUsize::new(0)),
        tool_delay_ms: 0,
        seen_transcripts: Mutex::new(Vec::new()),
    };

    let cfg = LoopConfig {
        max_steps: 10,
        doom_loop_threshold: 3,
        ..LoopConfig::default()
    };
    let (outcomes, answer) = run_loop_with_config(&engine, "Read right file", &[], &cfg)
        .await
        .unwrap();
    assert_eq!(answer.as_deref(), Some("recovered from doom loop"));
    assert_eq!(outcomes.len(), 5);
    assert!(!engine.client.native_tools_active());

    let seen = engine.seen_transcripts.lock().unwrap();
    assert!(
        seen[3]
            .last()
            .unwrap()
            .content
            .contains("DOOM_LOOP_WARNING"),
        "expected doom-loop nudge before recovery turn"
    );
}

#[tokio::test]
async fn e2e_parallel_readonly_batch_truncation_and_compaction() {
    assert!(is_read_only_tool("read_file"));
    assert!(is_read_only_tool("grep_files"));
    assert!(is_read_only_tool("list_dir"));
    assert!(!is_read_only_tool("edit_file"));

    let server = MockServer::start().await;

    // Turn 1: 3 read-only tool calls in a single native batch.
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "choices": [{
                "message": {
                    "content": null,
                    "tool_calls": [
                        { "id": "c1", "type": "function", "function": { "name": "read_file", "arguments": "{\"path\":\"a.rs\"}" } },
                        { "id": "c2", "type": "function", "function": { "name": "grep_files", "arguments": "{\"pattern\":\"foo\"}" } },
                        { "id": "c3", "type": "function", "function": { "name": "list_dir", "arguments": "{}" } }
                    ]
                }
            }]
        })))
        .up_to_n_times(1)
        .mount(&server)
        .await;

    // Turn 2: final answer.
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "choices": [{ "message": { "content": "batch complete" } }]
        })))
        .mount(&server)
        .await;

    let mut client = ShimClient::with(format!("{}/v1", server.uri()), "fixture".into());
    client.tools = vec![ToolSpec {
        name: "read_file".into(),
        description: "Read".into(),
        parameters: json!({"type": "object"}),
    }];

    let max_in_flight = Arc::new(AtomicUsize::new(0));
    let engine = ShimTestEngine {
        client,
        on_tool: Box::new(|tool, _| {
            if tool == "read_file" {
                Ok(format!(
                    "{}\nFATAL: critical diagnostic\n{}",
                    "x".repeat(3_000),
                    "y".repeat(3_000)
                ))
            } else {
                Ok(format!("ok-{tool}"))
            }
        }),
        in_flight: Arc::new(AtomicUsize::new(0)),
        max_in_flight: max_in_flight.clone(),
        tool_delay_ms: 50,
        seen_transcripts: Mutex::new(Vec::new()),
    };

    let cfg = LoopConfig {
        max_steps: 6,
        observation_char_limit: 900,
        ..LoopConfig::default()
    };
    let (outcomes, answer) = run_loop_with_config(&engine, "parallel check", &[], &cfg)
        .await
        .unwrap();
    assert_eq!(answer.as_deref(), Some("batch complete"));
    assert_eq!(outcomes.len(), 2);
    assert!(
        max_in_flight.load(Ordering::SeqCst) >= 2,
        "read-only batch should run concurrently"
    );

    let posts = server.received_requests().await.unwrap();
    let second_body: Value = posts[1].body_json().unwrap();
    let tool_msgs: Vec<&Value> = second_body["messages"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|m| m["role"] == "tool")
        .collect();
    assert_eq!(tool_msgs.len(), 3);
    let c1_body = tool_msgs[0]["content"].as_str().unwrap();
    assert!(c1_body.len() <= 900, "observation exceeded cap: {}", c1_body.len());
    assert!(c1_body.contains("FATAL: critical diagnostic"));

    // Verify compact_transcript handles mixed histories cleanly.
    let mut history = vec![
        ChatMessage { role: "system".into(), content: "sys".into() },
        ChatMessage { role: "user".into(), content: "GOAL: big".into() },
    ];
    for i in 0..8 {
        history.push(ChatMessage { role: "assistant".into(), content: format!("step {i}") });
        history.push(ChatMessage { role: "user".into(), content: "d".repeat(1_200) });
    }
    let dropped = compact_transcript(&mut history, 1, 3_000);
    assert!(dropped > 0);
    assert!(history[1].content.contains("Context compacted"));
}