//! Phase 6.1B: Full offline multi-turn harness & native binary E2E tests.

use async_trait::async_trait;
use meg_core::finish_hook::FinishHookConfig;
use meg_core::shim_client::{ShimClient, ToolSpec};
use meg_core::{run_loop_with_config, ChatMessage, LoopConfig, LoopEngine};
use meg_tools::deliverable_guard::DeliverableGuard;
use meg_tools::journal::{depth as undo_depth, undo, FileCheckpoint};
use meg_tools::ToolRegistry;
use serde_json::{json, Value};
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

struct FullWorkspaceEngine {
    client: ShimClient,
    registry: ToolRegistry,
    workspace: PathBuf,
    guard: Arc<Mutex<DeliverableGuard>>,
}

#[async_trait]
impl LoopEngine for FullWorkspaceEngine {
    async fn execute_tool(&self, tool: &str, args: &Value) -> anyhow::Result<String> {
        let unlock = args["unlock_deliverable"].as_bool().unwrap_or(false);
        if matches!(tool, "write_file" | "edit_file") {
            if let Some(p) = args["path"].as_str() {
                let resolved = meg_tools::fs_tools::resolve(&self.workspace, p)?;
                let mut g = self.guard.lock().unwrap();
                if unlock {
                    g.unlock_deliverable(&resolved);
                } else if let Some(msg) = g.check_write(&resolved) {
                    anyhow::bail!("{msg}");
                }
            }
        }
        let cp = if matches!(tool, "write_file" | "edit_file") {
            Some(FileCheckpoint::capture(&self.workspace, args)?)
        } else {
            None
        };
        let res = self.registry.execute(tool, args);
        if res.ok {
            if let Some(cp) = cp {
                cp.commit(&self.workspace)?;
            }
            if tool == "bash" {
                let mut g = self.guard.lock().unwrap();
                let targets: Vec<PathBuf> = g.target_paths.iter().cloned().collect();
                for t in targets {
                    if t.is_file() {
                        g.lock_verified_deliverable(&t);
                    }
                }
            }
            Ok(res.output)
        } else {
            anyhow::bail!("{}", res.output)
        }
    }

    async fn generate(&self, transcript: &[ChatMessage]) -> anyhow::Result<String> {
        self.client.chat(transcript).await
    }
}

#[tokio::test]
async fn e2e_full_coding_workflow_grep_read_crlf_edit_finish_hook_guard_and_multistep_undo() {
    let server = MockServer::start().await;
    let ws = std::env::temp_dir().join(format!("meg-e2e-ws-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(ws.join("src/nested/deep")).unwrap();
    let orig_crlf = "pub fn add(a: i32, b: i32) -> i32 {\r\n    a - b\r\n}\r\n";
    std::fs::write(ws.join("src/calc.rs"), orig_crlf).unwrap();
    std::fs::write(ws.join("src/nested/deep/helper.rs"), "pub fn ping() {}\n").unwrap();

    // Turn 0: parallel read-only batch (grep_files + list_dir + search_files).
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "choices": [{
                "message": {
                    "content": null,
                    "tool_calls": [
                        { "id": "tc_grep", "type": "function", "function": { "name": "grep_files", "arguments": "{\"pattern\":\"a - b\",\"glob\":\"**/*.rs\"}" } },
                        { "id": "tc_ls", "type": "function", "function": { "name": "list_dir", "arguments": "{\"path\":\"src\"}" } },
                        { "id": "tc_glob", "type": "function", "function": { "name": "search_files", "arguments": "{\"pattern\":\"**/*.rs\"}" } }
                    ]
                }
            }]
        })))
        .up_to_n_times(1)
        .mount(&server)
        .await;

    // Turn 1: paginated line-numbered read_file.
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "choices": [{
                "message": {
                    "content": null,
                    "tool_calls": [
                        { "id": "tc_read", "type": "function", "function": { "name": "read_file", "arguments": "{\"path\":\"src/calc.rs\",\"offset\":1,\"limit\":3}" } }
                    ]
                }
            }]
        })))
        .up_to_n_times(1)
        .mount(&server)
        .await;

    // Turn 2: CRLF-safe edit_file using LF strings.
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "choices": [{
                "message": {
                    "content": null,
                    "tool_calls": [
                        { "id": "tc_edit", "type": "function", "function": {
                            "name": "edit_file",
                            "arguments": "{\"path\":\"src/calc.rs\",\"old_string\":\"pub fn add(a: i32, b: i32) -> i32 {\\n    a - b\",\"new_string\":\"pub fn add(a: i32, b: i32) -> i32 {\\n    a + b\"}"
                        } }
                    ]
                }
            }]
        })))
        .up_to_n_times(1)
        .mount(&server)
        .await;

    // Turn 3: premature final before creating out.txt -> finish_hook reprompts!
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "choices": [{ "message": { "content": "Fixed add!" } }]
        })))
        .up_to_n_times(1)
        .mount(&server)
        .await;

    // Turn 4: create out.txt + run verification command (which locks out.txt).
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "choices": [{
                "message": {
                    "content": null,
                    "tool_calls": [
                        { "id": "tc_write", "type": "function", "function": { "name": "write_file", "arguments": "{\"path\":\"out.txt\",\"content\":\"add fixed\"}" } },
                        { "id": "tc_ver", "type": "function", "function": { "name": "bash", "arguments": "{\"command\":\"echo verify pass\"}" } }
                    ]
                }
            }]
        })))
        .up_to_n_times(1)
        .mount(&server)
        .await;

    // Turn 5: final answer after deliverable exists.
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "choices": [{ "message": { "content": "All changes verified and out.txt written." } }]
        })))
        .mount(&server)
        .await;

    let registry = ToolRegistry::with_core_tools(&ws);
    let mut client = ShimClient::with(format!("{}/v1", server.uri()), "fixture".into());
    client.tools = registry
        .tool_schemas()
        .into_iter()
        .map(|s| ToolSpec {
            name: s.name,
            description: s.description,
            parameters: s.parameters,
        })
        .collect();

    let out_target = meg_tools::fs_tools::resolve(&ws, "out.txt").unwrap();
    let mut guard = DeliverableGuard::from_goal(&ws, "Fix src/calc.rs and write out.txt");
    guard.target_paths = [out_target.clone()].into_iter().collect();

    let engine = FullWorkspaceEngine {
        client,
        registry,
        workspace: ws.clone(),
        guard: Arc::new(Mutex::new(guard)),
    };

    let cfg = LoopConfig {
        max_steps: 15,
        workspace: Some(ws.clone()),
        finish_hook: Some(FinishHookConfig {
            timeout_budget_secs: 300,
            max_turns: 15,
            simulated_elapsed_secs: Some(5),
            target_deliverables: vec![out_target.clone()],
        }),
        ..LoopConfig::default()
    };

    let (_outcomes, answer) =
        run_loop_with_config(&engine, "Fix src/calc.rs and write out.txt", &[], &cfg)
            .await
            .unwrap();
    assert_eq!(
        answer.as_deref(),
        Some("All changes verified and out.txt written.")
    );

    // 1. CRLF preserved after edit.
    let edited = std::fs::read_to_string(ws.join("src/calc.rs")).unwrap();
    assert_eq!(
        edited,
        "pub fn add(a: i32, b: i32) -> i32 {\r\n    a + b\r\n}\r\n"
    );
    assert_eq!(std::fs::read_to_string(&out_target).unwrap(), "add fixed");

    // 2. Verify Turn 0 had <workspace_map> and Turn 1 received grep/list/deep-glob results.
    let posts = server.received_requests().await.unwrap();
    assert_eq!(posts.len(), 6);
    let b0: Value = posts[0].body_json().unwrap();
    assert!(b0["messages"][1]["content"]
        .as_str()
        .unwrap()
        .contains("<workspace_map"));
    let b1: Value = posts[1].body_json().unwrap();
    let b1_text = b1.to_string();
    assert!(b1_text.contains("src/calc.rs:2:"));
    assert!(b1_text.contains("src/nested/deep/helper.rs"));
    let b2: Value = posts[2].body_json().unwrap();
    assert!(b2.to_string().contains("   2 |     a - b"));

    // 3. DeliverableGuard locked out.txt after verification bash command.
    let blocked = engine
        .execute_tool(
            "write_file",
            &json!({"path": "out.txt", "content": "clobber"}),
        )
        .await
        .unwrap_err()
        .to_string();
    assert!(blocked.contains("LOCKED"), "{blocked}");

    // 4. Multi-step undo rolls back out.txt (deletes) then src/calc.rs (restores CRLF).
    assert_eq!(undo_depth(&ws), 2);
    assert!(undo(&ws).unwrap().contains("1 remaining"));
    assert!(!out_target.exists());
    assert!(undo(&ws).unwrap().contains("0 remaining"));
    assert_eq!(std::fs::read_to_string(ws.join("src/calc.rs")).unwrap(), orig_crlf);
    assert_eq!(undo_depth(&ws), 0);

    let _ = std::fs::remove_dir_all(&ws);
}

#[tokio::test]
async fn e2e_meg_worker_binary_runs_native_tool_loop_offline() {
    let server = MockServer::start().await;
    let ws = std::env::temp_dir().join(format!("meg-bin-worker-{}", uuid::Uuid::new_v4()));
    let cfg_dir = std::env::temp_dir().join(format!("meg-bin-cfg-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::create_dir_all(&cfg_dir).unwrap();
    std::fs::write(ws.join("README.md"), "MEG_WORKER_TOKEN_987\n").unwrap();

    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":[{"id":"offline-m"}]})))
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "choices": [{
                "message": {
                    "content": null,
                    "tool_calls": [{
                        "id": "w_call_1",
                        "type": "function",
                        "function": {
                            "name": "grep_files",
                            "arguments": "{\"pattern\":\"MEG_WORKER_TOKEN\"}"
                        }
                    }]
                }
            }],
            "usage": { "prompt_tokens": 30, "completion_tokens": 10 }
        })))
        .up_to_n_times(1)
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "choices": [{ "message": { "content": "Found MEG_WORKER_TOKEN_987 in README.md" } }],
            "usage": { "prompt_tokens": 45, "completion_tokens": 12 }
        })))
        .mount(&server)
        .await;

    let payload = json!({
        "workspace": ws.to_string_lossy(),
        "model": "offline-m",
        "provider": null,
        "base_url": format!("{}/v1", server.uri()),
        "effort": "low",
        "goal": "Find MEG_WORKER_TOKEN"
    });

    let bin = env!("CARGO_BIN_EXE_meg");
    let output = tokio::task::spawn_blocking(move || {
        let mut child = Command::new(bin)
            .arg("worker")
            .env("MEG_CONFIG_DIR", &cfg_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(payload.to_string().as_bytes())
            .unwrap();
        let out = child.wait_with_output().unwrap();
        let _ = std::fs::remove_dir_all(&cfg_dir);
        out
    })
    .await
    .unwrap();

    assert!(
        output.status.success(),
        "worker failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let parsed: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        parsed["answer"].as_str(),
        Some("Found MEG_WORKER_TOKEN_987 in README.md")
    );
    assert_eq!(parsed["usage"]["input_tokens"].as_u64(), Some(75));
    assert_eq!(parsed["usage"]["output_tokens"].as_u64(), Some(22));
    let _ = std::fs::remove_dir_all(&ws);
}

#[test]
fn e2e_meg_cli_version_and_offline_guard() {
    let bin = env!("CARGO_BIN_EXE_meg");
    let ver = Command::new(bin).arg("--version").output().unwrap();
    assert!(ver.status.success());
    assert!(String::from_utf8_lossy(&ver.stdout).starts_with("MEG "));

    let offline = Command::new(bin)
        .args(["run", "say hello"])
        .env("MEG_OFFLINE", "1")
        .output()
        .unwrap();
    assert!(!offline.status.success());
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&offline.stdout),
        String::from_utf8_lossy(&offline.stderr)
    );
    assert!(combined.contains("MEG_OFFLINE=1"), "{combined}");
}