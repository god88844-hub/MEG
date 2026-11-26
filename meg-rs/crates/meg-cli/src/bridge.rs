use anyhow::Result;
use meg_tui::RunnerMsg;
use crate::runner::{RunExtras, run_request_detailed};

pub(crate) fn parse_bridge_mode(raw: &str) -> Result<meg_tui::Mode> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "plan" | "readonly" => Ok(meg_tui::Mode::Plan),
        "build" | "act" => Ok(meg_tui::Mode::Build),
        other => anyhow::bail!("Invalid mode '{other}'; expected 'plan' or 'build'"),
    }
}


/// Open Bring-Your-Own-TUI NDJSON Bridge (`meg ui-bridge`):
/// Emits structured JSON-line events on stdout and accepts JSON-line commands on stdin
/// so users can build or plug in any custom TUI/GUI (Python, Go, JS/TS, Rust, Web)
/// while keeping MEG's Ring-0 sandbox, verification, and multi-model orchestration intact.
pub(crate) async fn run_ui_bridge(workspace: &std::path::Path) -> Result<()> {
    use std::io::{BufRead, Write};

    let emit = |payload: serde_json::Value| {
        if let Ok(line) = serde_json::to_string(&payload) {
            let mut out = std::io::stdout();
            let _ = writeln!(out, "{line}");
            let _ = out.flush();
        }
    };

    let (ui_cfg, _) = meg_tui::UserUiConfig::load(Some(workspace));
    let mut model = std::env::var("MEG_TUI_MODEL").unwrap_or_else(|_| meg_core::shim_client::default_model());
    let base_url = std::env::var("MEG_TUI_BASE_URL").unwrap_or_else(|_| meg_core::shim_client::default_base_url());
    let mut pair_mode = ui_cfg.pair_mode.clone();
    let mut executor_model = ui_cfg.executor_model.clone().or(ui_cfg.roles.builder.clone());
    let mut reviewer_model = ui_cfg.reviewer_model.clone().or(ui_cfg.roles.reviewer.clone());
    let mut researcher_model = ui_cfg.roles.researcher.clone();
    let mut mode = meg_tui::Mode::Plan;

    emit(serde_json::json!({
        "event": "ready",
        "protocol": "meg-ui-bridge/v1",
        "workspace": workspace.display().to_string(),
        "model": model,
        "mode": mode.label(),
        "roles": {
            "planner": model,
            "builder": executor_model,
            "reviewer": reviewer_model,
            "researcher": researcher_model,
        }
    }));

    let stdin = std::io::stdin();
    for line_res in stdin.lock().lines() {
        let line = match line_res {
            Ok(l) => l,
            Err(_) => break,
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let req: serde_json::Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(e) => {
                emit(serde_json::json!({"event": "error", "message": format!("invalid json: {e}")}));
                continue;
            }
        };
        let cmd = req["cmd"].as_str().unwrap_or("");
        match cmd {
            "quit" | "exit" => {
                emit(serde_json::json!({"event": "bye"}));
                break;
            }
            "status" => {
                emit(serde_json::json!({
                    "event": "status",
                    "workspace": workspace.display().to_string(),
                    "model": model,
                    "mode": mode.label(),
                    "pair_mode": pair_mode,
                    "roles": {
                        "planner": model,
                        "builder": executor_model,
                        "reviewer": reviewer_model,
                        "researcher": researcher_model,
                    }
                }));
            }
            "set_mode" => {
                match req["mode"].as_str().ok_or_else(|| anyhow::anyhow!("Missing 'mode'; expected 'plan' or 'build'")).and_then(parse_bridge_mode) {
                    Ok(parsed) => {
                        mode = parsed;
                        emit(serde_json::json!({"event": "mode_updated", "mode": mode.label()}));
                    }
                    Err(e) => {
                        emit(serde_json::json!({"event": "error", "message": e.to_string()}));
                    }
                }
            }
            "set_roles" => {
                if let Some(p) = req["planner"].as_str().filter(|s| !s.is_empty()) {
                    model = p.to_string();
                }
                if req.get("builder").is_some() {
                    executor_model = req["builder"].as_str().filter(|s| !s.is_empty()).map(str::to_string);
                    pair_mode = if executor_model.is_some() {
                        Some(req["pair_mode"].as_str().unwrap_or("antigravity_orchestrator").to_string())
                    } else {
                        None
                    };
                }
                if req.get("reviewer").is_some() {
                    reviewer_model = req["reviewer"].as_str().filter(|s| !s.is_empty()).map(str::to_string);
                }
                if req.get("researcher").is_some() {
                    researcher_model = req["researcher"].as_str().filter(|s| !s.is_empty()).map(str::to_string);
                }
                emit(serde_json::json!({
                    "event": "roles_updated",
                    "pair_mode": pair_mode,
                    "roles": {
                        "planner": model,
                        "builder": executor_model,
                        "reviewer": reviewer_model,
                        "researcher": researcher_model,
                    }
                }));
            }
            "run" => {
                let goal = req["goal"].as_str().unwrap_or("").trim().to_string();
                if goal.is_empty() {
                    emit(serde_json::json!({"event": "error", "message": "run requires non-empty 'goal'"}));
                    continue;
                }
                if std::env::var("MEG_OFFLINE").as_deref() == Ok("1") {
                    emit(serde_json::json!({"event": "error", "message": "MEG_OFFLINE=1: inference disabled"}));
                    continue;
                }
                let (tx, rx) = std::sync::mpsc::channel::<RunnerMsg>();
                let forwarder = std::thread::spawn(move || {
                    for msg in rx {
                        let ev = match msg {
                            RunnerMsg::StreamStart => serde_json::json!({"event": "stream_start"}),
                            RunnerMsg::ThinkingChunk(t) => serde_json::json!({"event": "thinking", "text": t}),
                            RunnerMsg::ContentChunk(c) => serde_json::json!({"event": "chunk", "text": c}),
                            RunnerMsg::ToolStarted { id, name, args } => {
                                serde_json::json!({"event": "tool_start", "id": id, "name": name, "args": args})
                            }
                            RunnerMsg::ToolFinished { id, ok, output } => {
                                serde_json::json!({"event": "tool_end", "id": id, "ok": ok, "output": output})
                            }
                            RunnerMsg::Usage { input, output, cached } => {
                                serde_json::json!({"event": "usage", "input": input, "output": output, "cached": cached})
                            }
                            _ => continue,
                        };
                        if let Ok(s) = serde_json::to_string(&ev) {
                            let mut out = std::io::stdout();
                            let _ = writeln!(out, "{s}");
                            let _ = out.flush();
                        }
                    }
                });

                let run_req = meg_tui::RunRequest {
                    provider: None,
                    goal,
                    workspace: workspace.to_path_buf(),
                    model: model.clone(),
                    base_url: base_url.clone(),
                    history: Vec::new(),
                    mode,
                    effort: req["effort"].as_str().unwrap_or("medium").to_string(),
                    cancel: meg_core::CancellationToken::default(),
                    pair_mode: pair_mode.clone(),
                    executor_model: executor_model.clone(),
                    reviewer_model: reviewer_model.clone(),
                    attachments: Vec::new(),
                };
                let res = run_request_detailed(
                    run_req,
                    Some(tx),
                    None,
                    RunExtras {
                        max_steps: req["max_steps"].as_u64().map(|n| n as usize),
                        verify_cmd: req["verify_cmd"].as_str().map(str::to_string),
                        require_verify: req["require_verify"].as_bool().unwrap_or(false),
                        pair_mode: pair_mode.clone(),
                        executor_model: executor_model.clone(),
                        reviewer_model: reviewer_model.clone(),
                    },
                )
                .await;
                let _ = forwarder.join();
                match res {
                    Ok((outcomes, answer, telemetry)) => emit(serde_json::json!({
                        "event": "done",
                        "ok": true,
                        "turns": outcomes.len(),
                        "answer": answer,
                        "planner_turns": telemetry.planner_turns,
                        "executor_turns": telemetry.executor_turns,
                        "reviewer_verdict": telemetry.reviewer_verdict,
                    })),
                    Err(e) => emit(serde_json::json!({
                        "event": "done",
                        "ok": false,
                        "error": e.to_string(),
                    })),
                }
            }
            other => emit(serde_json::json!({"event": "error", "message": format!("unknown cmd '{other}'")})),
        }
    }
    Ok(())
}

