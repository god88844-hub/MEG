use anyhow::Context;
use anyhow::Result;
use meg_core::shim_client::ShimClient;
use meg_tools::ToolRegistry;
use meg_tui::RunnerMsg;
use meg_tui::WorkstationState;
use std::path::Path;
use std::sync::mpsc::Sender;
use std::sync::Arc;
use crate::cli::TuiOptions;
use crate::engine::{ShimEngine, build_tool_specs, extract_finish_deliverables};

pub(crate) async fn run_request(
    request: meg_tui::RunRequest,
    tx: Option<Sender<RunnerMsg>>,
) -> Result<(Vec<meg_core::TurnOutcome>, Option<String>)> {
    run_request_with_plane(request, tx, None).await
}


#[derive(Default, Clone)]
pub(crate) struct RunExtras {
    pub(crate) max_steps: Option<usize>,
    pub(crate) verify_cmd: Option<String>,
    pub(crate) require_verify: bool,
    pub(crate) pair_mode: Option<String>,
    pub(crate) executor_model: Option<String>,
    pub(crate) reviewer_model: Option<String>,
}


#[derive(Default, Debug, Clone)]
pub(crate) struct RunTelemetry {
    pub(crate) planner_turns: usize,
    pub(crate) executor_turns: usize,
    pub(crate) reviewer_verdict: Option<String>,
}


pub(crate) struct ColdReviewRequest<'a> {
    pub(crate) provider: &'a Option<meg_core::providers::ProviderProfile>,
    pub(crate) base_url: &'a str,
    pub(crate) reviewer_model: &'a str,
    pub(crate) workspace: &'a std::path::Path,
    pub(crate) goal: &'a str,
    pub(crate) edited_files: &'a [std::path::PathBuf],
    pub(crate) final_answer: &'a str,
    pub(crate) cancel: &'a meg_core::CancellationToken,
}


pub(crate) async fn run_request_with_plane(
    request: meg_tui::RunRequest,
    tx: Option<Sender<RunnerMsg>>,
    plane: Option<Arc<meg_core::services::ServicePlane>>,
) -> Result<(Vec<meg_core::TurnOutcome>, Option<String>)> {
    run_request_with_options(request, tx, plane, RunExtras::default()).await
}


pub(crate) async fn run_request_with_options(
    request: meg_tui::RunRequest,
    tx: Option<Sender<RunnerMsg>>,
    plane: Option<Arc<meg_core::services::ServicePlane>>,
    extras: RunExtras,
) -> Result<(Vec<meg_core::TurnOutcome>, Option<String>)> {
    let (outcomes, answer, _telemetry) = run_request_detailed(request, tx, plane, extras).await?;
    Ok((outcomes, answer))
}


pub(crate) async fn run_cold_l3_reviewer(req: ColdReviewRequest<'_>) -> Result<String> {
    let mut rev_shim = if let Some(profile) = req.provider {
        ShimClient::with_profile(profile)?
    } else {
        ShimClient::with(req.base_url.to_string(), req.reviewer_model.to_string())
    };
    rev_shim.model = req.reviewer_model.to_string();
    rev_shim.tools = Vec::new(); // Strictly read-only, tool-less fresh context.

    let mut diff_context = String::new();
    for path in req.edited_files.iter().take(12) {
        let rel = path
            .strip_prefix(req.workspace)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");
        if let Ok(text) = std::fs::read_to_string(path) {
            let preview: String = text.chars().take(4_000).collect();
            diff_context.push_str(&format!("--- FILE: {rel} ---\n{preview}\n\n"));
        }
    }
    if diff_context.is_empty() {
        diff_context.push_str("(no source files modified)\n");
    }

    let messages = vec![
        meg_core::ChatMessage {
            role: "system".into(),
            content: "You are a cold, independent L3 code reviewer. You have ZERO builder history. Your default answer is REJECT unless the code clearly satisfies the goal without bugs, stubs, or regressions. Respond ONLY with JSON: {\"verdict\":\"PASS\"|\"REJECT\",\"reason\":\"...\"}.".into(),
        },
        meg_core::ChatMessage {
            role: "user".into(),
            content: format!(
                "GOAL: {}\n\nCLAIMED COMPLETION: {}\n\nMODIFIED WORKSPACE ARTIFACTS:\n{}",
                req.goal, req.final_answer, diff_context
            ),
        },
    ];
    let mut ignore_cb = |_| {};
    let reply = rev_shim
        .chat_stream_cancellable(&messages, Some(req.cancel), &mut ignore_cb)
        .await?;
    let (verdict, reason) = parse_l3_reviewer_verdict(&reply);
    if verdict != "PASS" {
        anyhow::bail!("L3_REVIEWER_REJECTED ({}): {reason}", req.reviewer_model);
    }
    Ok("PASS".into())
}


pub(crate) fn parse_l3_reviewer_verdict(raw: &str) -> (String, String) {
    let trimmed = raw.trim();
    let verdict_val = serde_json::from_str::<serde_json::Value>(trimmed).ok().or_else(|| {
        trimmed
            .find('{')
            .and_then(|s| trimmed.rfind('}').map(|e| &trimmed[s..=e]))
            .and_then(|slice| serde_json::from_str::<serde_json::Value>(slice).ok())
    });
    if let Some(val) = verdict_val {
        let v = val
            .get("verdict")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_ascii_uppercase())
            .unwrap_or_else(|| "REJECT".into());
        let reason = val
            .get("reason")
            .and_then(|v| v.as_str())
            .unwrap_or(trimmed)
            .to_string();
        return (if v == "PASS" { "PASS".into() } else { "REJECT".into() }, reason);
    }
    // Strict plain-text fallback: accept only an explicit standalone PASS or VERDICT: PASS line.
    let explicit_pass = trimmed.eq_ignore_ascii_case("PASS")
        || trimmed.lines().any(|line| {
            let up = line.trim().to_ascii_uppercase();
            up == "VERDICT: PASS" || up == "VERDICT:PASS"
        });
    if explicit_pass {
        ("PASS".into(), trimmed.to_string())
    } else {
        ("REJECT".into(), trimmed.to_string())
    }
}


pub(crate) async fn run_request_detailed(
    request: meg_tui::RunRequest,
    tx: Option<Sender<RunnerMsg>>,
    plane: Option<Arc<meg_core::services::ServicePlane>>,
    extras: RunExtras,
) -> Result<(Vec<meg_core::TurnOutcome>, Option<String>, RunTelemetry)> {
    anyhow::ensure!(
        !request.model.trim().is_empty(),
        "No model selected. Connect a provider and choose /model first."
    );
    let (role_ui_cfg, _) = meg_tui::UserUiConfig::load(Some(&request.workspace));
    let mut role_models = serde_json::Map::new();
    if let Some(m) = role_ui_cfg.roles.researcher.as_ref().filter(|s| !s.trim().is_empty()) {
        role_models.insert("researcher".into(), serde_json::Value::String(m.clone()));
    }
    if let Some(m) = role_ui_cfg.roles.planner.as_ref().filter(|s| !s.trim().is_empty()) {
        role_models.insert("planner".into(), serde_json::Value::String(m.clone()));
    }
    if let Some(m) = request
        .reviewer_model
        .as_ref()
        .or(role_ui_cfg.roles.reviewer.as_ref())
        .filter(|s| !s.trim().is_empty())
    {
        role_models.insert("reviewer".into(), serde_json::Value::String(m.clone()));
    }
    let worker = serde_json::json!({
        "provider": request.provider,
        "model": request.model,
        "base_url": request.base_url,
        "effort": request.effort,
        "handoff_id": uuid::Uuid::new_v4().to_string(),
        "role_models": role_models,
    });
    let mut shim = if let Some(profile) = &request.provider {
        ShimClient::with_profile(profile)?
    } else {
        ShimClient::with(request.base_url.clone(), request.model.clone())
    };
    shim.model = request.model.clone();
    shim.reasoning_effort = if request.provider.as_ref().is_none_or(|p| p.send_effort) {
        Some(if request.effort == "max" {
            "xhigh".into()
        } else {
            request.effort
        })
    } else {
        None
    };
    let mut catalog = serde_json::json!({});
    if let Some(plane) = &plane {
        let p = plane.clone();
        let workspace = request.workspace.clone();
        let cancel = request.cancel.clone();
        match tokio::task::spawn_blocking(move || {
            p.call(
                "runtime.catalog",
                serde_json::json!({"workspace":workspace}),
                &cancel,
            )
        })
        .await?
        {
            Ok(value) => {
                catalog = value["catalog"].clone();
            }
            Err(err) => {
                if request.cancel.is_cancelled() {
                    anyhow::bail!("Run cancelled");
                }
                let msg = format!(
                    "Control-plane catalog unavailable ({err}); continuing with native tools."
                );
                if let Some(tx) = &tx {
                    let _ = tx.send(RunnerMsg::Line(msg));
                } else {
                    eprintln!("[meg] warning: {msg}");
                }
            }
        }
    }
    // Catalog discovery belongs to the UI service. Configured endpoints can run
    // without a /models route and without downloading the catalog every turn.
    if request.provider.is_none() {
        let models = tokio::select! {result=shim.models()=>result,_=request.cancel.cancelled()=>anyhow::bail!("Run cancelled")};
        if let Some(tx) = &tx {
            let _ = tx.send(RunnerMsg::Online(models.is_ok()));
            if let Ok(models) = models {
                let _ = tx.send(RunnerMsg::ModelList(models));
            }
        }
    }
    let strict_core_six = std::env::var("MEG_CORE_SIX").as_deref() == Ok("1");
    let registry = Arc::new(if strict_core_six {
        ToolRegistry::with_core_six_cancellable(&request.workspace, request.cancel.0.clone())
    } else {
        ToolRegistry::with_cancellation(&request.workspace, request.cancel.0.clone())
    });
    shim.tools = build_tool_specs(&registry, request.mode, &catalog, plane.is_some());
    let executor_shim = if let Some(exec_model) = &extras.executor_model {
        let mut es = if let Some(profile) = &request.provider {
            ShimClient::with_profile(profile)?
        } else {
            ShimClient::with(request.base_url.clone(), exec_model.clone())
        };
        es.model = exec_model.clone();
        es.reasoning_effort = shim.reasoning_effort.clone();
        es.tools = shim.tools.clone();
        Some(es)
    } else {
        None
    };
    let max_steps = extras.max_steps.unwrap_or_else(default_max_steps);
    let targets = extract_finish_deliverables(&request.workspace, &request.goal, request.mode);
    let mut deliverable_guard =
        meg_tools::deliverable_guard::DeliverableGuard::from_goal(&request.workspace, &request.goal);
    deliverable_guard.target_paths = targets.iter().cloned().collect();
    let finish_hook = if targets.is_empty() {
        None
    } else {
        Some(meg_core::finish_hook::FinishHookConfig {
            timeout_budget_secs: 300,
            max_turns: max_steps as u64,
            simulated_elapsed_secs: None,
            target_deliverables: targets,
        })
    };
    let loop_config = meg_core::LoopConfig {
        max_steps,
        workspace: Some(request.workspace.clone()),
        finish_hook,
        native_tools: !shim.tools.is_empty(),
        user_attachments: request.attachments.clone(),
        ..meg_core::LoopConfig::default()
    };
    let receipt_workspace = request.workspace.clone();
    let receipt_mode = request.mode;
    let reviewer_provider = request.provider.clone();
    let reviewer_base_url = request.base_url.clone();
    let reviewer_cancel = request.cancel.clone();
    let planner_turns = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let executor_turns = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let consecutive_failures = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let sealed_manifest = Arc::new(
        meg_tools::deliverable_guard::SealedManifest::seal_protected_paths(&request.workspace),
    );
    let read_only_lock = Arc::new(meg_tools::deliverable_guard::ReadOnlyLock::new());
    if read_only_lock.check_stop_phrase(&request.goal) {
        anyhow::bail!("RING0_READ_ONLY_LOCK: stop phrase received; execution halted before any mutation");
    }
    let mut aepn_sup = meg_core::finish_hook::AepnSupervisor::new(max_steps as u32);
    aepn_sup.lock_contract();
    let aepn = Arc::new(std::sync::Mutex::new(aepn_sup));
    let edited_files = Arc::new(std::sync::Mutex::new(Vec::<std::path::PathBuf>::new()));
    let verify_passed = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let engine = ShimEngine {
        plane,
        catalog,
        worker,
        shim,
        registry,
        tx,
        workspace: request.workspace,
        cancel: request.cancel,
        mode: request.mode,
        next_tool: std::sync::atomic::AtomicUsize::new(1),
        guard: Arc::new(std::sync::Mutex::new(deliverable_guard)),
        read_only_lock,
        sealed: sealed_manifest.clone(),
        aepn: aepn.clone(),
        edited_files: edited_files.clone(),
        verify_cmd: extras.verify_cmd.clone(),
        require_verify: extras.require_verify,
        verify_passed: verify_passed.clone(),
        executor_shim,
        pair_mode: extras.pair_mode.clone(),
        turn_counter: std::sync::atomic::AtomicUsize::new(0),
        planner_turns: planner_turns.clone(),
        executor_turns: executor_turns.clone(),
        consecutive_failures,
    };
    let result = meg_core::run_loop_with_config(
        &engine,
        &request.goal,
        &request.history,
        &loop_config,
    )
    .await?;
    anyhow::ensure!(
        result.1.is_some(),
        "Turn budget exhausted after {max_steps} steps; no final answer was produced"
    );
    if !sealed_manifest.is_empty() {
        if let Err(tampered) = sealed_manifest.verify(&receipt_workspace) {
            let _ = sealed_manifest.restore_tampered(&receipt_workspace);
            anyhow::bail!(
                "SEALED_MANIFEST_VIOLATION: protected file(s) modified during run (restored): {}",
                tampered.join("; ")
            );
        }
    }
    // Hard post-loop check: never sign a green receipt if stubs remain or --verify-cmd still fails.
    if receipt_mode != meg_tui::Mode::Plan {
        let edited_snapshot = edited_files.lock().unwrap().clone();
        let mut final_stubs = Vec::new();
        for p in &edited_snapshot {
            if let Ok(text) = std::fs::read_to_string(p) {
                final_stubs.extend(meg_tools::deliverable_guard::scan_anti_stub_and_false_zero(
                    p, &text,
                ));
            }
        }
        if !final_stubs.is_empty() {
            anyhow::bail!(
                "ANTI_STUB_VIOLATION: unresolved stub/false-zero code remained at finish: {}",
                final_stubs.join(" | ")
            );
        }
        if let Some(cmd) = &extras.verify_cmd {
            if !verify_passed.load(std::sync::atomic::Ordering::SeqCst) {
                let ws_v = receipt_workspace.clone();
                let cmd_v = cmd.clone();
                let final_check = tokio::task::spawn_blocking(move || {
                    meg_tools::bash::run(
                        &ws_v,
                        &serde_json::json!({ "command": cmd_v, "timeout_seconds": 30 }),
                    )
                })
                .await?;
                if let Err(err) = final_check {
                    anyhow::bail!("VERIFY_CMD_FAILED: external verification `{cmd}` did not pass: {err}");
                }
            }
        } else if extras.require_verify && !edited_snapshot.is_empty() && aepn.lock().unwrap().p_tests_green == 0 {
            anyhow::bail!(
                "AEPN_VERIFICATION_REQUIRED: code was edited without passing verification after the last edit"
            );
        }
    }
    let mut reviewer_verdict_out: Option<String> = None;
    if receipt_mode != meg_tui::Mode::Plan {
        let mutated = !edited_files.lock().unwrap().is_empty() || result.0.iter().any(|t| match &t.action {
            meg_core::Action::Tool { tool, .. } => {
                matches!(tool.as_str(), "write_file" | "edit_file")
            }
            meg_core::Action::Batch { calls, .. } => calls
                .iter()
                .any(|(tool, _)| matches!(tool.as_str(), "write_file" | "edit_file")),
            _ => false,
        });
        if mutated {
            // Cold cross-model L3 reviewer runs before signing .meg/receipt.json
            if let Some(rev_model) = &extras.reviewer_model {
                let edited_now = edited_files.lock().unwrap().clone();
                let verdict = run_cold_l3_reviewer(ColdReviewRequest {
                    provider: &reviewer_provider,
                    base_url: &reviewer_base_url,
                    reviewer_model: rev_model,
                    workspace: &receipt_workspace,
                    goal: &request.goal,
                    edited_files: &edited_now,
                    final_answer: result.1.as_deref().unwrap_or(""),
                    cancel: &reviewer_cancel,
                })
                .await?;
                let _ = aepn.lock().unwrap().record_critic_approved();
                reviewer_verdict_out = Some(verdict);
            }
            let real_checks_passed =
                extras.verify_cmd.is_some() || aepn.lock().unwrap().p_tests_green > 0;
            meg_tools::ledger::write_receipt(&receipt_workspace, "meg-run", real_checks_passed)?;
        }
    }
    let telemetry = RunTelemetry {
        planner_turns: planner_turns.load(std::sync::atomic::Ordering::SeqCst),
        executor_turns: executor_turns.load(std::sync::atomic::Ordering::SeqCst),
        reviewer_verdict: reviewer_verdict_out,
    };
    Ok((result.0, result.1, telemetry))
}


pub(crate) fn default_max_steps() -> usize {
    std::env::var("MEG_MAX_STEPS")
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .map(|n| n.clamp(1, 200))
        .unwrap_or(40)
}


pub(crate) fn start_plane() -> Result<Arc<meg_core::services::ServicePlane>> {
    let use_legacy_bun = std::env::var("MEG_LEGACY_BUN_PLANE").as_deref() == Ok("1")
        || std::env::var_os("MEG_PLANE_ENTRY").is_some();
    if !use_legacy_bun {
        return Ok(meg_core::services::ServicePlane::native());
    }
    let exe = std::env::current_exe()?;
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .unwrap();
    let entry = std::env::var_os("MEG_PLANE_ENTRY")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            [
                exe.parent().unwrap().join("meg-plane/plane.js"),
                repo.join("meg-plane/dist/plane.js"),
                repo.join("meg-plane/src/index.ts"),
            ]
            .into_iter()
            .find(|p| p.is_file())
        })
        .context("MEG control-plane bundle is missing")?;
    let builtin = builtin_skills_path(&entry, repo);
    let client = meg_core::plane_client::PlaneClient::spawn(&entry, &exe, Some(&builtin))?;
    Ok(meg_core::services::ServicePlane::external(client))
}


pub(crate) fn builtin_skills_path(entry: &Path, repo: &Path) -> std::path::PathBuf {
    let bundled = entry
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("skills");
    if bundled.is_dir() {
        bundled
    } else {
        repo.join("meg-plane/skills")
    }
}


pub(crate) async fn run_worker() -> Result<()> {
    use std::io::Read;
    let mut bytes = Vec::new();
    std::io::stdin()
        .take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    anyhow::ensure!(bytes.len() <= 1024 * 1024, "Worker request is too large");
    let value: serde_json::Value = serde_json::from_slice(&bytes)?;
    let state = WorkstationState {
        cwd: value["workspace"]
            .as_str()
            .context("Worker needs a workspace")?
            .into(),
        model: value["model"].as_str().unwrap_or_default().into(),
        provider: serde_json::from_value(value["provider"].clone())?,
        mode: meg_tui::Mode::Plan,
        effort: value["effort"].as_str().unwrap_or("medium").into(),
        ..Default::default()
    };
    let mut request = state.run_request(
        value["goal"]
            .as_str()
            .context("Worker needs a goal")?
            .into(),
    );
    if let Some(url) = value["base_url"].as_str() {
        request.base_url = url.into();
    }
    let (tx, rx) = std::sync::mpsc::channel();
    let result = run_request(request, Some(tx)).await;
    let mut usage = meg_tui::session::Usage::default();
    for event in rx.try_iter() {
        match event {
            RunnerMsg::StreamStart => usage.requests += 1,
            RunnerMsg::Usage {
                input,
                output,
                cached,
            } => {
                usage.input_tokens += input;
                usage.output_tokens += output;
                usage.cached_tokens += cached;
                usage.reported_requests += 1;
            }
            _ => {}
        }
    }
    let output = match result {
        Ok((_, Some(answer))) => serde_json::json!({"answer":answer,"usage":usage}),
        Ok(_) => serde_json::json!({"error":"Worker produced no final answer"}),
        Err(error) => serde_json::json!({"error":error.to_string()}),
    };
    println!("{}", serde_json::to_string(&output)?);
    Ok(())
}


pub(crate) fn detect_git_branch(workspace: &Path) -> String {
    std::process::Command::new("git")
        .arg("-C")
        .arg(workspace)
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}


pub(crate) fn meg_tui_clock() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{:02}:{:02} UTC", (secs / 3600) % 24, (secs / 60) % 60)
}


/// Executes a goal with the workspace's saved local/cloud connection settings.
pub(crate) async fn run_goal_headless(workspace: &Path, goal: &str, options: &TuiOptions) -> Result<()> {
    if !options.json_output {
        println!("MEG (rust) \u{2502} goal: {goal}\n");
    }
    anyhow::ensure!(
        std::env::var("MEG_OFFLINE").as_deref() != Ok("1"),
        "MEG_OFFLINE=1: inference is disabled; no provider was contacted"
    );
    let store = meg_core::providers::ProviderStore::global();
    let settings = store.load()?;
    let provider = if std::env::var_os("MEG_TUI_BASE_URL").is_some() {
        Some(meg_core::providers::ProviderProfile::environment())
    } else {
        settings
            .profiles
            .iter()
            .find(|p| p.id == settings.active)
            .cloned()
    };
    let model = std::env::var("MEG_TUI_MODEL")
        .ok()
        .or_else(|| provider.as_ref().map(|p| p.default_model.clone()))
        .unwrap_or_default();
    let mut state = WorkstationState {
        cwd: meg_tui::friendly_path(workspace),
        providers: settings.profiles,
        provider,
        model,
        ..Default::default()
    };
    if std::env::var_os("MEG_TUI_BASE_URL").is_none()
        && workspace.join(".meg/tui/preferences.json").is_file()
    {
        meg_tui::session::SessionStore::open(workspace)?.restore_preferences(&mut state)?;
    }
    if let Ok(model) = std::env::var("MEG_TUI_MODEL") {
        if !model.trim().is_empty() {
            state.select_model(model);
        }
    }
    if let Some(model) = &options.model {
        state.select_model(model.clone());
    }
    if let Some(effort) = &options.effort {
        state.select_effort(effort.clone());
    }
    if let Some(pinned) = &options.pinned_model {
        state.select_model(pinned.clone());
        anyhow::ensure!(
            state.model == *pinned,
            "PINNED_MODEL_VIOLATION: expected '{pinned}', resolved '{}'",
            state.model
        );
    }
    if options.core_six {
        std::env::set_var("MEG_CORE_SIX", "1");
    }
    let plane = if std::env::var("MEG_NO_PLANE").as_deref() == Ok("1")
        || std::env::var("MEG_PLANE_DISABLED").as_deref() == Ok("1")
    {
        None
    } else {
        Some(start_plane()?)
    };
    let (outcomes, final_answer, telemetry) = run_request_detailed(
        state.run_request(goal.into()),
        None,
        plane,
        RunExtras {
            max_steps: options.max_steps,
            verify_cmd: options.verify_cmd.clone(),
            require_verify: options.require_verify
                || std::env::var("MEG_REQUIRE_VERIFY").as_deref() == Ok("1"),
            pair_mode: options.pair_mode.clone(),
            executor_model: options.executor_model.clone(),
            reviewer_model: options.reviewer_model.clone(),
        },
    )
    .await?;
    if options.json_output {
        let receipt_path = workspace.join(".meg").join("receipt.json");
        let receipt_json: Option<serde_json::Value> = if receipt_path.is_file() {
            std::fs::read_to_string(&receipt_path)
                .ok()
                .and_then(|s| serde_json::from_str(&s).ok())
        } else {
            None
        };
        let payload = serde_json::json!({
            "ok": final_answer.is_some(),
            "goal": goal,
            "model": state.model,
            "pinned_model": options.pinned_model,
            "pair_mode": options.pair_mode,
            "executor_model": options.executor_model,
            "reviewer_model": options.reviewer_model,
            "planner_turns": telemetry.planner_turns,
            "executor_turns": telemetry.executor_turns,
            "reviewer_verdict": telemetry.reviewer_verdict,
            "turns": outcomes.len(),
            "receipt_path": if receipt_path.is_file() { Some(receipt_path.display().to_string()) } else { None },
            "receipt": receipt_json,
            "final_answer": final_answer,
        });
        println!("{}", serde_json::to_string(&payload)?);
        if final_answer.is_none() {
            anyhow::bail!("Budget exhausted without a final answer");
        }
        return Ok(());
    }
    for (i, o) in outcomes.iter().enumerate() {
        match &o.action {
            meg_core::Action::Tool { tool, .. } => {
                println!(
                    "[{i}] {tool} -> {:.120}",
                    o.observation.as_deref().unwrap_or("")
                );
            }
            meg_core::Action::Batch { calls, .. } => {
                println!("[{i}] batch of {} calls", calls.len());
            }
            meg_core::Action::Final { answer, .. } => {
                println!("[{i}] final: {:.120}", answer);
            }
            meg_core::Action::Unknown(_) => println!("[{i}] unparseable turn"),
        }
    }
    match final_answer {
        Some(answer) => println!("\nFINAL: {answer}"),
        None => anyhow::bail!("Budget exhausted without a final answer"),
    }
    Ok(())
}

