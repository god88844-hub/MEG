use anyhow::Result;
use meg_tui::RunnerMsg;
use std::io::{BufRead, Write};
use std::sync::{Arc, Mutex};
use crate::runner::{RunExtras, run_request_detailed, start_plane};

pub(crate) fn parse_bridge_mode(raw: &str) -> Result<meg_tui::Mode> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "plan" | "readonly" => Ok(meg_tui::Mode::Plan),
        "build" | "act" => Ok(meg_tui::Mode::Build),
        other => anyhow::bail!("Invalid mode '{other}'; expected 'plan' or 'build'"),
    }
}

fn emit_line(stdout_lock: &Arc<Mutex<()>>, payload: serde_json::Value) {
    if let Ok(line) = serde_json::to_string(&payload) {
        let _guard = stdout_lock.lock().unwrap();
        let mut out = std::io::stdout();
        let _ = writeln!(out, "{line}");
        let _ = out.flush();
    }
}

/// Open Bring-Your-Own-TUI NDJSON Bridge (`meg ui-bridge`):
/// Stateful, non-blocking, and cancellable in-flight. Persists conversation
/// history across `run` commands, attaches `ServicePlane::native()` for skills/MCP,
/// and services `cancel`, `status`, and `quit` while a run is active.
pub(crate) async fn run_ui_bridge(workspace: &std::path::Path) -> Result<()> {
    let stdout_lock = Arc::new(Mutex::new(()));
    let (ui_cfg, _) = meg_tui::UserUiConfig::load(Some(workspace));

    let provider = if std::env::var_os("MEG_TUI_BASE_URL").is_some() {
        Some(meg_core::providers::ProviderProfile::environment())
    } else {
        meg_core::providers::ProviderStore::global()
            .load()
            .ok()
            .and_then(|s| {
                s.profiles
                    .iter()
                    .find(|p| p.id == s.active)
                    .or_else(|| s.profiles.first())
                    .cloned()
            })
    };

    let mut model = std::env::var("MEG_TUI_MODEL")
        .ok()
        .or_else(|| provider.as_ref().map(|p| p.default_model.clone()))
        .unwrap_or_else(meg_core::shim_client::default_model);
    let base_url = std::env::var("MEG_TUI_BASE_URL")
        .ok()
        .or_else(|| provider.as_ref().map(|p| p.base_url.clone()))
        .unwrap_or_else(meg_core::shim_client::default_base_url);
    let mut pair_mode = ui_cfg.pair_mode.clone();
    let mut executor_model = ui_cfg.executor_model.clone().or(ui_cfg.roles.builder.clone());
    let mut reviewer_model = ui_cfg.reviewer_model.clone().or(ui_cfg.roles.reviewer.clone());
    let mut researcher_model = ui_cfg.roles.researcher.clone();
    let mut mode = meg_tui::Mode::Plan;

    let plane = if std::env::var("MEG_NO_PLANE").as_deref() == Ok("1")
        || std::env::var("MEG_PLANE_DISABLED").as_deref() == Ok("1")
    {
        None
    } else {
        start_plane().ok()
    };

    let history: Arc<Mutex<Vec<meg_core::ChatMessage>>> = Arc::new(Mutex::new(Vec::new()));
    let active_cancel: Arc<Mutex<Option<meg_core::CancellationToken>>> = Arc::new(Mutex::new(None));

    emit_line(
        &stdout_lock,
        serde_json::json!({
            "event": "ready",
            "protocol": "meg-ui-bridge/v2",
            "workspace": workspace.display().to_string(),
            "model": model,
            "mode": mode.label(),
            "native_plane": plane.as_ref().is_some_and(|p| p.is_native()),
            "roles": {
                "planner": model,
                "builder": executor_model,
                "reviewer": reviewer_model,
                "researcher": researcher_model,
            }
        }),
    );

    // Read stdin on a dedicated blocking thread so commands like `cancel` and `status`
    // are processed immediately even while a `run` task is in flight.
    let (cmd_tx, mut cmd_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    std::thread::spawn(move || {
        let stdin = std::io::stdin();
        for line_res in stdin.lock().lines() {
            match line_res {
                Ok(l) => {
                    if cmd_tx.send(l).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    while let Some(line) = cmd_rx.recv().await {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let req: serde_json::Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(e) => {
                emit_line(
                    &stdout_lock,
                    serde_json::json!({"event": "error", "message": format!("invalid json: {e}")}),
                );
                continue;
            }
        };
        let cmd = req["cmd"].as_str().unwrap_or("");
        match cmd {
            "quit" | "exit" => {
                if let Some(tok) = active_cancel.lock().unwrap().take() {
                    tok.cancel();
                }
                emit_line(&stdout_lock, serde_json::json!({"event": "bye"}));
                break;
            }
            "cancel" => {
                let cancelled = if let Some(tok) = active_cancel.lock().unwrap().as_ref() {
                    tok.cancel();
                    true
                } else {
                    false
                };
                emit_line(
                    &stdout_lock,
                    serde_json::json!({"event": "cancelled", "active": cancelled}),
                );
            }
            "clear_history" => {
                history.lock().unwrap().clear();
                emit_line(&stdout_lock, serde_json::json!({"event": "history_cleared"}));
            }
            "status" => {
                let running = active_cancel.lock().unwrap().is_some();
                let history_len = history.lock().unwrap().len();
                emit_line(
                    &stdout_lock,
                    serde_json::json!({
                        "event": "status",
                        "workspace": workspace.display().to_string(),
                        "model": model,
                        "mode": mode.label(),
                        "running": running,
                        "history_len": history_len,
                        "pair_mode": pair_mode,
                        "roles": {
                            "planner": model,
                            "builder": executor_model,
                            "reviewer": reviewer_model,
                            "researcher": researcher_model,
                        }
                    }),
                );
            }
            "set_mode" => {
                match req["mode"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Missing 'mode'; expected 'plan' or 'build'"))
                    .and_then(parse_bridge_mode)
                {
                    Ok(parsed) => {
                        mode = parsed;
                        emit_line(
                            &stdout_lock,
                            serde_json::json!({"event": "mode_updated", "mode": mode.label()}),
                        );
                    }
                    Err(e) => {
                        emit_line(
                            &stdout_lock,
                            serde_json::json!({"event": "error", "message": e.to_string()}),
                        );
                    }
                }
            }
            "set_roles" => {
                if let Some(p) = req["planner"].as_str().filter(|s| !s.is_empty()) {
                    model = p.to_string();
                }
                if req.get("builder").is_some() {
                    executor_model = req["builder"]
                        .as_str()
                        .filter(|s| !s.is_empty())
                        .map(str::to_string);
                    pair_mode = if executor_model.is_some() {
                        Some(
                            req["pair_mode"]
                                .as_str()
                                .unwrap_or("antigravity_orchestrator")
                                .to_string(),
                        )
                    } else {
                        None
                    };
                }
                if req.get("reviewer").is_some() {
                    reviewer_model = req["reviewer"]
                        .as_str()
                        .filter(|s| !s.is_empty())
                        .map(str::to_string);
                }
                if req.get("researcher").is_some() {
                    researcher_model = req["researcher"]
                        .as_str()
                        .filter(|s| !s.is_empty())
                        .map(str::to_string);
                }
                emit_line(
                    &stdout_lock,
                    serde_json::json!({
                        "event": "roles_updated",
                        "pair_mode": pair_mode,
                        "roles": {
                            "planner": model,
                            "builder": executor_model,
                            "reviewer": reviewer_model,
                            "researcher": researcher_model,
                        }
                    }),
                );
            }
            "run" => {
                let goal = req["goal"].as_str().unwrap_or("").trim().to_string();
                if goal.is_empty() {
                    emit_line(
                        &stdout_lock,
                        serde_json::json!({"event": "error", "message": "run requires non-empty 'goal'"}),
                    );
                    continue;
                }
                if std::env::var("MEG_OFFLINE").as_deref() == Ok("1") {
                    emit_line(
                        &stdout_lock,
                        serde_json::json!({"event": "error", "message": "MEG_OFFLINE=1: inference disabled"}),
                    );
                    continue;
                }
                let cancel_tok = meg_core::CancellationToken::default();
                {
                    let mut slot = active_cancel.lock().unwrap();
                    if slot.is_some() {
                        emit_line(
                            &stdout_lock,
                            serde_json::json!({"event": "error", "message": "a run is already active; send 'cancel' first"}),
                        );
                        continue;
                    }
                    *slot = Some(cancel_tok.clone());
                }

                let (tx, rx) = std::sync::mpsc::channel::<RunnerMsg>();
                let fwd_lock = stdout_lock.clone();
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
                        emit_line(&fwd_lock, ev);
                    }
                });

                let prior_history = history.lock().unwrap().clone();
                let run_req = meg_tui::RunRequest {
                    provider: provider.clone(),
                    goal: goal.clone(),
                    workspace: workspace.to_path_buf(),
                    model: model.clone(),
                    base_url: base_url.clone(),
                    history: prior_history,
                    mode,
                    effort: req["effort"].as_str().unwrap_or("medium").to_string(),
                    cancel: cancel_tok,
                    pair_mode: pair_mode.clone(),
                    executor_model: executor_model.clone(),
                    reviewer_model: reviewer_model.clone(),
                    attachments: Vec::new(),
                };
                let extras = RunExtras {
                    max_steps: req["max_steps"].as_u64().map(|n| n as usize),
                    verify_cmd: req["verify_cmd"].as_str().map(str::to_string),
                    require_verify: req["require_verify"].as_bool().unwrap_or(false),
                    pair_mode: pair_mode.clone(),
                    executor_model: executor_model.clone(),
                    reviewer_model: reviewer_model.clone(),
                };
                let plane_clone = plane.clone();
                let hist_clone = history.clone();
                let cancel_slot = active_cancel.clone();
                let done_lock = stdout_lock.clone();

                tokio::spawn(async move {
                    let res = run_request_detailed(run_req, Some(tx), plane_clone, extras).await;
                    let _ = forwarder.join();
                    *cancel_slot.lock().unwrap() = None;
                    match res {
                        Ok((outcomes, answer, telemetry)) => {
                            if let Some(ref ans) = answer {
                                let mut h = hist_clone.lock().unwrap();
                                h.push(meg_core::ChatMessage {
                                    role: "user".into(),
                                    content: goal,
                                });
                                h.push(meg_core::ChatMessage {
                                    role: "assistant".into(),
                                    content: ans.clone(),
                                });
                            }
                            emit_line(
                                &done_lock,
                                serde_json::json!({
                                    "event": "done",
                                    "ok": true,
                                    "turns": outcomes.len(),
                                    "answer": answer,
                                    "planner_turns": telemetry.planner_turns,
                                    "executor_turns": telemetry.executor_turns,
                                    "reviewer_verdict": telemetry.reviewer_verdict,
                                }),
                            );
                        }
                        Err(e) => emit_line(
                            &done_lock,
                            serde_json::json!({
                                "event": "done",
                                "ok": false,
                                "error": e.to_string(),
                            }),
                        ),
                    }
                });
            }
            other => emit_line(
                &stdout_lock,
                serde_json::json!({"event": "error", "message": format!("unknown cmd '{other}'")}),
            ),
        }
    }
    Ok(())
}
