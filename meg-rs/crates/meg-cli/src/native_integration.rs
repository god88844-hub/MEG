#[allow(unused_imports)]
use crate::{bridge::*, cli::*, engine::*, loop_cmd::*, runner::*, tui_launch::*};
#[allow(unused_imports)]
use anyhow::{Context, Result};
#[allow(unused_imports)]
use meg_core::shim_client::{Delta, ShimClient};
#[allow(unused_imports)]
use meg_core::{ChatMessage, LoopEngine};
#[allow(unused_imports)]
use meg_tools::ToolRegistry;
#[allow(unused_imports)]
use meg_tui::{RunnerMsg, WorkstationState};
#[allow(unused_imports)]
use std::path::Path;
#[allow(unused_imports)]
use std::sync::mpsc::Sender;
#[allow(unused_imports)]
use std::sync::Arc;

use serde_json::json;
use wiremock::{
    matchers::{method, path},
    Mock, MockServer, ResponseTemplate,
};
async fn fixture() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"data":[{"id":"fixture"}]})),
        )
        .mount(&server)
        .await;
    server
}
fn request(server: &MockServer) -> meg_tui::RunRequest {
    let state = WorkstationState {
        cwd: std::env::temp_dir().to_string_lossy().into_owned(),
        model: "fixture".into(),
        ..Default::default()
    };
    let mut request = state.run_request("second request".into());
    request.base_url = format!("{}/v1", server.uri());
    request
}
#[tokio::test]
async fn actual_runner_preserves_context_effort_and_reports_provider_usage() {
    let server = fixture().await;
    Mock::given(method("POST")).and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"choices":[{"message":{"content":"{\"final\":\"Context retained\"}"}}],"usage":{"prompt_tokens":45,"completion_tokens":12,"prompt_tokens_details":{"cached_tokens":8}}}))).mount(&server).await;
    let mut request = request(&server);
    request.effort = "low".into();
    request.history = vec![
        ChatMessage {
            role: "user".into(),
            content: "remember cobalt".into(),
        },
        ChatMessage {
            role: "assistant".into(),
            content: "remembered".into(),
        },
    ];
    let (tx, rx) = std::sync::mpsc::channel();
    let result = run_request(request, Some(tx)).await.unwrap();
    assert_eq!(result.1.as_deref(), Some("Context retained"));
    let events: Vec<_> = rx.try_iter().collect();
    assert!(events.iter().any(|e| matches!(
        e,
        RunnerMsg::Usage {
            input: 45,
            output: 12,
            cached: 8
        }
    )));
    assert!(!events
        .iter()
        .any(|e| matches!(e, RunnerMsg::TodoList(_) | RunnerMsg::SubagentState(_, _))));
    let requests = server.received_requests().await.unwrap();
    let body: serde_json::Value = requests
        .iter()
        .find(|r| r.method == "POST")
        .unwrap()
        .body_json()
        .unwrap();
    assert_eq!(body["reasoning_effort"], "low");
    assert_eq!(body["model"], "fixture");
    assert!(body["messages"]
        .as_array()
        .unwrap()
        .iter()
        .any(|m| m["content"] == "remember cobalt"));
}
#[tokio::test]
async fn configured_model_does_not_wait_for_or_require_catalog_discovery() {
    let server = MockServer::start().await;
    Mock::given(method("POST")).and(path("/v1/chat/completions")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"choices":[{"message":{"content":"{\"final\":\"Configured endpoint works\"}"}}]}))).expect(1).mount(&server).await;
    let mut request = request(&server);
    let mut profile = meg_core::providers::ProviderProfile::environment();
    profile.id = uuid::Uuid::new_v4().to_string();
    profile.base_url = request.base_url.clone();
    profile.default_model = "fixture".into();
    profile.key_env = None;
    request.provider = Some(profile);
    let result = run_request(request, None).await.unwrap();
    assert_eq!(result.1.as_deref(), Some("Configured endpoint works"));
    assert_eq!(
        server.received_requests().await.unwrap().len(),
        1,
        "No catalog request is needed to run a configured model"
    );
}
#[tokio::test]
async fn network_cancellation_does_not_wait_for_endpoint_timeout() {
    let server = fixture().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(std::time::Duration::from_secs(5))
                .set_body_json(json!({"choices":[{"message":{"content":"late"}}]})),
        )
        .mount(&server)
        .await;
    let request = request(&server);
    let cancel = request.cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(75)).await;
        cancel.cancel();
    });
    let started = std::time::Instant::now();
    let error = run_request(request, None).await.unwrap_err();
    assert!(error.to_string().contains("cancelled"));
    assert!(started.elapsed() < std::time::Duration::from_secs(2));
}
#[tokio::test]
async fn plan_mode_enforces_read_only_tools_at_execution_boundary() {
    let server = fixture().await;
    let request = request(&server);
    let guard = Arc::new(std::sync::Mutex::new(
        meg_tools::deliverable_guard::DeliverableGuard::from_goal(&request.workspace, "plan"),
    ));
    let engine = ShimEngine {
        plane: None,
        catalog: serde_json::json!({}),
        worker: serde_json::json!({}),
        shim: ShimClient::with(request.base_url, "fixture".into()),
        registry: Arc::new(ToolRegistry::with_core_tools(&request.workspace)),
        tx: None,
        workspace: request.workspace,
        cancel: request.cancel,
        mode: meg_tui::Mode::Plan,
        next_tool: std::sync::atomic::AtomicUsize::new(1),
        guard,
        read_only_lock: Arc::new(meg_tools::deliverable_guard::ReadOnlyLock::new()),
        sealed: Arc::new(meg_tools::deliverable_guard::SealedManifest::default()),
        aepn: Arc::new(std::sync::Mutex::new(meg_core::finish_hook::AepnSupervisor::new(10))),
        edited_files: Arc::new(std::sync::Mutex::new(Vec::new())),
        verify_cmd: None,
        require_verify: false,
        verify_passed: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        executor_shim: None,
        pair_mode: None,
        turn_counter: std::sync::atomic::AtomicUsize::new(0),
        planner_turns: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        executor_turns: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        consecutive_failures: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
    };
    for name in ["bash", "write_file", "edit_file"] {
        assert!(engine
            .execute_tool(name, &json!({}))
            .await
            .unwrap_err()
            .to_string()
            .contains("disabled in Plan mode"));
    }
    assert_eq!(
        engine
            .execute_tool(
                "todo",
                &json!({"items":[{"text":"Inspect tests","done":false}]})
            )
            .await
            .unwrap(),
        "Task checklist updated"
    );
}
#[tokio::test]
async fn native_tool_call_round_trip_preserves_tool_call_id() {
    let server = fixture().await;
    let ws = std::env::temp_dir().join(format!("meg-cli-native-tc-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::write(ws.join("hello.txt"), "secret-value-42").unwrap();

    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "choices": [{
                "message": {
                    "content": null,
                    "tool_calls": [{
                        "id": "call_read_1",
                        "type": "function",
                        "function": {
                            "name": "read_file",
                            "arguments": "{\"path\":\"hello.txt\"}"
                        }
                    }]
                }
            }]
        })))
        .up_to_n_times(1)
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "choices": [{
                "message": { "content": "The file contains secret-value-42" }
            }]
        })))
        .mount(&server)
        .await;

    let state = WorkstationState {
        cwd: ws.to_string_lossy().into_owned(),
        model: "fixture".into(),
        ..Default::default()
    };
    let mut req = state.run_request("Read hello.txt".into());
    req.base_url = format!("{}/v1", server.uri());

    let (outcomes, answer) = run_request(req, None).await.unwrap();
    assert_eq!(answer.as_deref(), Some("The file contains secret-value-42"));
    assert_eq!(outcomes.len(), 2);

    let posts: Vec<_> = server
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .filter(|r| r.method == "POST")
        .collect();
    assert_eq!(posts.len(), 2);
    let first_body: serde_json::Value = posts[0].body_json().unwrap();
    assert!(first_body["tools"].is_array(), "tools schema must be sent");
    let second_body: serde_json::Value = posts[1].body_json().unwrap();
    let msgs = second_body["messages"].as_array().unwrap();
    assert!(
        msgs.iter()
            .any(|m| m["role"] == "tool"
                && m["tool_call_id"] == "call_read_1"
                && m["content"].as_str().unwrap_or("").contains("secret-value-42")),
        "second request must include tool_call_id result: {msgs:?}"
    );
    let _ = std::fs::remove_dir_all(&ws);
}

#[tokio::test]
async fn turn_one_injects_workspace_map_and_finish_hook_recovers_missing_deliverable() {
    let server = fixture().await;
    let ws = std::env::temp_dir().join(format!("meg-cli-p4-fh-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::write(ws.join("seed.rs"), "pub fn seed_fn() {}\n").unwrap();

    // Turn 0: model prematurely claims final without writing output.txt.
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "choices": [{ "message": { "content": "{\"final\":\"done early\"}" } }]
        })))
        .up_to_n_times(1)
        .mount(&server)
        .await;

    // Turn 1 (after finish_hook reprompt): model writes output.txt.
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "choices": [{
                "message": {
                    "content": null,
                    "tool_calls": [{
                        "id": "call_w1",
                        "type": "function",
                        "function": {
                            "name": "write_file",
                            "arguments": "{\"path\":\"output.txt\",\"content\":\"verified deliverable\"}"
                        }
                    }]
                }
            }]
        })))
        .up_to_n_times(1)
        .mount(&server)
        .await;

    // Turn 2: model finishes cleanly.
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "choices": [{ "message": { "content": "output.txt created and verified" } }]
        })))
        .mount(&server)
        .await;

    let state = WorkstationState {
        cwd: ws.to_string_lossy().into_owned(),
        model: "fixture".into(),
        ..Default::default()
    };
    let mut req = state.run_request("Create output.txt with verified deliverable".into());
    req.base_url = format!("{}/v1", server.uri());

    let (_outcomes, answer) = run_request(req, None).await.unwrap();
    assert_eq!(answer.as_deref(), Some("output.txt created and verified"));
    assert_eq!(
        std::fs::read_to_string(ws.join("output.txt")).unwrap(),
        "verified deliverable"
    );

    let posts: Vec<_> = server
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .filter(|r| r.method == "POST")
        .collect();
    assert_eq!(posts.len(), 3);
    let first_body: serde_json::Value = posts[0].body_json().unwrap();
    let goal_msg = first_body["messages"][1]["content"].as_str().unwrap_or("");
    assert!(goal_msg.contains("<workspace_map"), "missing workspace map: {goal_msg}");
    assert!(goal_msg.contains("seed.rs"), "missing seed.rs in map: {goal_msg}");
    let _ = std::fs::remove_dir_all(&ws);
}

#[tokio::test]
async fn live_antigravity_orchestrator_routes_planner_executor_reintervention_and_cold_l3_reviewer() {
    use wiremock::matchers::body_string_contains;

    let server = fixture().await;
    let ws = std::env::temp_dir().join(format!("meg-dual-l3-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::write(ws.join("calc.py"), "def mul(a, b):\n    return a + b\n").unwrap();

    // Turn 0 (Planner: neon-planner): read_file calc.py
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(body_string_contains("\"model\":\"neon-planner\""))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "choices": [{ "message": { "content": "{\"thought\":\"explore\",\"tool\":\"read_file\",\"args\":{\"path\":\"calc.py\"}}" } }]
        })))
        .up_to_n_times(1)
        .mount(&server)
        .await;

    // Turn 1 (Executor: hydrogen-exec): fails an edit (triggers friction!)
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(body_string_contains("\"model\":\"hydrogen-exec\""))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "choices": [{ "message": { "content": "{\"thought\":\"bad edit\",\"tool\":\"edit_file\",\"args\":{\"path\":\"calc.py\",\"old_string\":\"NON_EXISTENT\",\"new_string\":\"return a * b\"}}" } }]
        })))
        .up_to_n_times(1)
        .mount(&server)
        .await;

    // Turn 2 (Planner re-intervenes on friction!): applies the clean edit
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(body_string_contains("\"model\":\"neon-planner\""))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "choices": [{ "message": { "content": "{\"thought\":\"replan & fix\",\"tool\":\"edit_file\",\"args\":{\"path\":\"calc.py\",\"old_string\":\"return a + b\",\"new_string\":\"return a * b\"}}" } }]
        })))
        .up_to_n_times(1)
        .mount(&server)
        .await;

    // Turn 3 (Executor finishes cleanly)
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(body_string_contains("\"model\":\"hydrogen-exec\""))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "choices": [{ "message": { "content": "{\"thought\":\"done\",\"final\":\"Fixed mul()\"}" } }]
        })))
        .mount(&server)
        .await;

    // Cold L3 Reviewer (opus-reviewer): approves the clean diff
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(body_string_contains("\"model\":\"opus-reviewer\""))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "choices": [{ "message": { "content": "{\"verdict\":\"PASS\",\"reason\":\"Verified mul() uses multiplication.\"}" } }]
        })))
        .mount(&server)
        .await;

    let req = meg_tui::RunRequest {
        workspace: ws.clone(),
        goal: "Fix mul() in calc.py".into(),
        base_url: format!("{}/v1", server.uri()),
        model: "neon-planner".into(),
        provider: None,
        mode: meg_tui::Mode::default(),
        effort: "medium".into(),
        history: vec![],
        cancel: meg_core::CancellationToken::default(),
        pair_mode: None,
        executor_model: None,
        reviewer_model: None,
        attachments: Vec::new(),
    };
    let (_outcomes, ans, telemetry) = run_request_detailed(
        req,
        None,
        None,
        RunExtras {
            max_steps: Some(8),
            verify_cmd: Some("python -c \"import calc; assert calc.mul(3, 4) == 12\"".into()),
            require_verify: true,
            pair_mode: Some("antigravity_orchestrator".into()),
            executor_model: Some("hydrogen-exec".into()),
            reviewer_model: Some("opus-reviewer".into()),
        },
    )
    .await
    .unwrap();

    assert_eq!(ans.as_deref(), Some("Fixed mul()"));
    assert_eq!(telemetry.planner_turns, 2, "Planner should run Turn 0 + 1 friction re-intervention");
    assert_eq!(telemetry.executor_turns, 2, "Executor should run Turn 1 + final turn");
    assert_eq!(telemetry.reviewer_verdict.as_deref(), Some("PASS"));
    assert!(ws.join(".meg/receipt.json").is_file());
    let _ = std::fs::remove_dir_all(&ws);
}

#[tokio::test]
async fn cold_l3_reviewer_reject_blocks_receipt_signing() {
    use wiremock::matchers::body_string_contains;

    let server = fixture().await;
    let ws = std::env::temp_dir().join(format!("meg-l3-reject-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::write(ws.join("calc.py"), "def mul(a, b):\n    return a + b\n").unwrap();

    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(body_string_contains("\"model\":\"builder-m\""))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "choices": [{ "message": { "content": "{\"thought\":\"edit\",\"tool\":\"edit_file\",\"args\":{\"path\":\"calc.py\",\"old_string\":\"return a + b\",\"new_string\":\"return a * b\"}}" } }]
        })))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(body_string_contains("\"model\":\"builder-m\""))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "choices": [{ "message": { "content": "{\"thought\":\"done\",\"final\":\"done\"}" } }]
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(body_string_contains("\"model\":\"strict-critic\""))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "choices": [{ "message": { "content": "{\"verdict\":\"REJECT\",\"reason\":\"Missing overflow check\"}" } }]
        })))
        .mount(&server)
        .await;

    let req = meg_tui::RunRequest {
        workspace: ws.clone(),
        goal: "Fix mul()".into(),
        base_url: format!("{}/v1", server.uri()),
        model: "builder-m".into(),
        provider: None,
        mode: meg_tui::Mode::default(),
        effort: "medium".into(),
        history: vec![],
        cancel: meg_core::CancellationToken::default(),
        pair_mode: None,
        executor_model: None,
        reviewer_model: None,
        attachments: Vec::new(),
    };
    let err = run_request_detailed(
        req,
        None,
        None,
        RunExtras {
            max_steps: Some(6),
            reviewer_model: Some("strict-critic".into()),
            ..RunExtras::default()
        },
    )
    .await
    .unwrap_err();
    assert!(err.to_string().contains("L3_REVIEWER_REJECTED"), "{err}");
    assert!(!ws.join(".meg/receipt.json").exists(), "Rejected run must NEVER sign receipt");
    let _ = std::fs::remove_dir_all(&ws);
}

#[tokio::test]
async fn anti_stub_gate_withholds_finish_and_recovers_once_stub_removed() {
    let server = fixture().await;
    let ws = std::env::temp_dir().join(format!("meg-stub-gate-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::write(ws.join("app.py"), "def run():\n    return 1\n").unwrap();

    // Turn 1: writes a NotImplementedError stub
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "choices": [{ "message": { "content": "{\"thought\":\"write stub\",\"tool\":\"edit_file\",\"args\":{\"path\":\"app.py\",\"old_string\":\"return 1\",\"new_string\":\"raise NotImplementedError()\"}}" } }]
        })))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    // Turn 2: prematurely claims finish -> withheld by ANTI_STUB_GATE
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "choices": [{ "message": { "content": "{\"thought\":\"done\",\"final\":\"finished\"}" } }]
        })))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    // Turn 3: replaces stub with real implementation
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "choices": [{ "message": { "content": "{\"thought\":\"fix stub\",\"tool\":\"edit_file\",\"args\":{\"path\":\"app.py\",\"old_string\":\"raise NotImplementedError()\",\"new_string\":\"return 42\"}}" } }]
        })))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    // Turn 4: clean finish -> approved!
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "choices": [{ "message": { "content": "{\"thought\":\"verified\",\"final\":\"implemented run()\"}" } }]
        })))
        .mount(&server)
        .await;

    let req = meg_tui::RunRequest {
        workspace: ws.clone(),
        goal: "Implement run() in app.py".into(),
        base_url: format!("{}/v1", server.uri()),
        model: "fixture".into(),
        provider: None,
        mode: meg_tui::Mode::default(),
        effort: "medium".into(),
        history: vec![],
        cancel: meg_core::CancellationToken::default(),
        pair_mode: None,
        executor_model: None,
        reviewer_model: None,
        attachments: Vec::new(),
    };
    let (outcomes, ans) = run_request_with_options(
        req,
        None,
        None,
        RunExtras {
            max_steps: Some(8),
            verify_cmd: Some("python -c \"import app; assert app.run() == 42\"".into()),
            require_verify: true,
            ..RunExtras::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(ans.as_deref(), Some("implemented run()"));
    assert!(outcomes.iter().any(|o| o.observation.as_deref().is_some_and(|s| s.contains("ANTI_STUB_GATE"))));
    assert!(ws.join(".meg/receipt.json").is_file(), "Verified run must sign .meg/receipt.json");
    let _ = std::fs::remove_dir_all(&ws);
}

#[tokio::test]
async fn sealed_manifest_blocks_direct_edit_and_detects_bash_tampering_on_protected_tests() {
    let server = fixture().await;
    let ws = std::env::temp_dir().join(format!("meg-sealed-live-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(ws.join("tests")).unwrap();
    std::fs::write(ws.join("tests/spec.rs"), "assert!(true);\n").unwrap();

    // Turn 1: model tries edit_file on tests/spec.rs -> blocked by SEALED_FILE_WALL
    // Turn 2: model tries to bypass via bash overwrite -> post-run SealedManifest fails the run
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "choices": [{ "message": { "content": "{\"thought\":\"try edit\",\"tool\":\"edit_file\",\"args\":{\"path\":\"tests/spec.rs\",\"old_string\":\"true\",\"new_string\":\"false\"}}" } }]
        })))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "choices": [{ "message": { "content": "{\"thought\":\"bypass via bash\",\"tool\":\"bash\",\"args\":{\"command\":\"echo tampered > tests/spec.rs\"}}" } }]
        })))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "choices": [{ "message": { "content": "{\"thought\":\"done\",\"final\":\"all done\"}" } }]
        })))
        .mount(&server)
        .await;

    let req = meg_tui::RunRequest {
        workspace: ws.clone(),
        goal: "Update code without touching tests".into(),
        base_url: format!("{}/v1", server.uri()),
        model: "fixture".into(),
        provider: None,
        mode: meg_tui::Mode::default(),
        effort: "medium".into(),
        history: vec![],
        cancel: meg_core::CancellationToken::default(),
        pair_mode: None,
        executor_model: None,
        reviewer_model: None,
        attachments: Vec::new(),
    };
    let err = run_request(req, None).await.unwrap_err();
    assert!(err.to_string().contains("SEALED_MANIFEST_VIOLATION"), "{err}");
    assert!(!ws.join(".meg/receipt.json").exists(), "Tampered run must NEVER sign a receipt");
    let _ = std::fs::remove_dir_all(&ws);
}

#[tokio::test]
async fn deliverable_guard_locks_verified_file_and_allows_explicit_unlock() {
    let server = fixture().await;
    let ws = std::env::temp_dir().join(format!("meg-cli-p4-dg-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&ws).unwrap();
    let target = meg_tools::fs_tools::resolve(&ws, "deliverable.txt").unwrap();
    let mut guard =
        meg_tools::deliverable_guard::DeliverableGuard::from_goal(&ws, "Create deliverable.txt");
    guard.target_paths = [target.clone()].into_iter().collect();

    let engine = ShimEngine {
        plane: None,
        catalog: serde_json::json!({}),
        worker: serde_json::json!({}),
        shim: ShimClient::with(format!("{}/v1", server.uri()), "fixture".into()),
        registry: Arc::new(ToolRegistry::with_core_tools(&ws)),
        tx: None,
        workspace: ws.clone(),
        cancel: meg_core::CancellationToken::default(),
        mode: meg_tui::Mode::default(),
        next_tool: std::sync::atomic::AtomicUsize::new(1),
        guard: Arc::new(std::sync::Mutex::new(guard)),
        read_only_lock: Arc::new(meg_tools::deliverable_guard::ReadOnlyLock::new()),
        sealed: Arc::new(meg_tools::deliverable_guard::SealedManifest::default()),
        aepn: Arc::new(std::sync::Mutex::new(meg_core::finish_hook::AepnSupervisor::new(10))),
        edited_files: Arc::new(std::sync::Mutex::new(Vec::new())),
        verify_cmd: None,
        require_verify: false,
        verify_passed: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        executor_shim: None,
        pair_mode: None,
        turn_counter: std::sync::atomic::AtomicUsize::new(0),
        planner_turns: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        executor_turns: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        consecutive_failures: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
    };

    engine
        .execute_tool(
            "write_file",
            &json!({"path": "deliverable.txt", "content": "v1 valid content"}),
        )
        .await
        .unwrap();
    // Decoy echo command must NOT lock the deliverable.
    engine
        .execute_tool("bash", &json!({"command": "echo verify ok"}))
        .await
        .unwrap();
    engine
        .execute_tool(
            "write_file",
            &json!({"path": "deliverable.txt", "content": "v1 valid content"}),
        )
        .await
        .unwrap();
    // Genuine passing test command locks verified deliverable.
    std::fs::write(
        target.parent().unwrap().join("verify.test.ts"),
        "import { test, expect } from 'bun:test'; test('ok', () => expect(1).toBe(1));\n",
    )
    .unwrap();
    engine
        .execute_tool("bash", &json!({"command": "bun test verify.test.ts"}))
        .await
        .unwrap();
    // Subsequent overwrite without unlock_deliverable is blocked.
    let blocked = engine
        .execute_tool(
            "write_file",
            &json!({"path": "deliverable.txt", "content": "accidental clobber"}),
        )
        .await
        .unwrap_err()
        .to_string();
    assert!(blocked.contains("LOCKED"), "{blocked}");
    // Explicit unlock allows modification.
    engine
        .execute_tool(
            "write_file",
            &json!({"path": "deliverable.txt", "content": "v2 intentional", "unlock_deliverable": true}),
        )
        .await
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(&target).unwrap(),
        "v2 intentional"
    );
    let _ = std::fs::remove_dir_all(&ws);
}

#[test]
fn protocol_wrapper_is_not_shown_as_chat_text() {
    assert_eq!(
        visible_preview("{\"thought\":\"x\",\"final\":\"Hello"),
        "Hello"
    );
    assert_eq!(visible_preview("{\"tool\":\"bash\",\"args\":"), "");
    assert_eq!(visible_preview("Hello, \u{4E16}\u{754C}"), "Hello, \u{4E16}\u{754C}");
}
