//! meg â€” the Rust binary (Codex shape: Rust owns the loop + TUI).
//!
//! Entrypoints:
//! - Bare invocation (`meg`) or `meg tui`: opens the interactive Workstation TUI.
//! - `meg run "<goal>"`: runs the agent loop headless against the local brain.
//! - `meg -h` / `meg --help`: displays CLI usage.
//!
//! Local and cloud models use user-selected provider profiles. Personal gateways
//! require explicit endpoint configuration and are never shipped as a default.

use anyhow::{Context, Result};
use meg_core::shim_client::{Delta, ShimClient};
use meg_core::{ChatMessage, LoopEngine};
use meg_tools::ToolRegistry;
use meg_tui::{RunnerMsg, WorkstationState};
use std::path::Path;
use std::sync::mpsc::Sender;
use std::sync::Arc;

/// One request owns its cancellation token, registry and event stream.
struct ShimEngine {
    plane: Option<Arc<meg_core::plane_client::PlaneClient>>,
    catalog: serde_json::Value,
    worker: serde_json::Value,
    shim: ShimClient,
    registry: Arc<ToolRegistry>,
    tx: Option<Sender<RunnerMsg>>,
    workspace: std::path::PathBuf,
    cancel: meg_core::CancellationToken,
    mode: meg_tui::Mode,
    next_tool: std::sync::atomic::AtomicUsize,
    guard: Arc<std::sync::Mutex<meg_tools::deliverable_guard::DeliverableGuard>>,
    read_only_lock: Arc<meg_tools::deliverable_guard::ReadOnlyLock>,
    sealed: Arc<meg_tools::deliverable_guard::SealedManifest>,
    aepn: Arc<std::sync::Mutex<meg_core::finish_hook::AepnSupervisor>>,
    edited_files: Arc<std::sync::Mutex<Vec<std::path::PathBuf>>>,
    verify_cmd: Option<String>,
    require_verify: bool,
    verify_passed: Arc<std::sync::atomic::AtomicBool>,
    executor_shim: Option<ShimClient>,
    pair_mode: Option<String>,
    turn_counter: std::sync::atomic::AtomicUsize,
    planner_turns: Arc<std::sync::atomic::AtomicUsize>,
    executor_turns: Arc<std::sync::atomic::AtomicUsize>,
    consecutive_failures: Arc<std::sync::atomic::AtomicUsize>,
}
impl ShimEngine {
    fn emit(&self, msg: RunnerMsg) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(msg);
        }
    }
}

#[async_trait::async_trait]
impl LoopEngine for ShimEngine {
    fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }
    async fn execute_tool(&self, tool: &str, args: &serde_json::Value) -> Result<String> {
        anyhow::ensure!(!self.cancel.is_cancelled(), "Run cancelled");
        anyhow::ensure!(
            self.mode != meg_tui::Mode::Plan
                || matches!(
                    tool,
                    "read_file"
                        | "search_files"
                        | "grep_files"
                        | "glob_grep"
                        | "list_dir"
                        | "todo"
                        | "read_skill"
                        | "delegate"
                )
                || tool.starts_with("mcp:"),
            "{tool} is disabled in Plan mode; switch to Act with /code"
        );
        if tool == "todo" {
            let items = args["items"]
                .as_array()
                .ok_or_else(|| anyhow::anyhow!("todo requires items with text and done fields"))?;
            anyhow::ensure!(items.len() <= 100, "Todo list exceeds 100 items");
            let mut steps = Vec::new();
            for item in items {
                steps.push((
                    item["done"].as_bool().unwrap_or(false),
                    item["text"]
                        .as_str()
                        .ok_or_else(|| anyhow::anyhow!("Todo item needs text"))?
                        .to_string(),
                ));
            }
            self.emit(RunnerMsg::TodoList(steps));
            return Ok("Task checklist updated".into());
        }
        let id = format!(
            "tool-{}",
            self.next_tool
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        );
        self.emit(RunnerMsg::ToolStarted {
            id: id.clone(),
            name: tool.into(),
            args: meg_tui::fit_text(&args.to_string(), 120),
        });
        let registry = self.registry.clone();
        let workspace = self.workspace.clone();
        let cancel = self.cancel.clone();
        let name = tool.to_string();
        let args = args.clone();
        let connected = tool.starts_with("mcp__") || tool.starts_with("mcp:") || matches!(tool, "delegate" | "read_skill");
        let plane = self.plane.clone();
        let worker = self.worker.clone();
        let read_only = self.mode == meg_tui::Mode::Plan;
        let events = self.tx.clone();
        let guard = self.guard.clone();
        let read_only_lock = self.read_only_lock.clone();
        let sealed = self.sealed.clone();
        let aepn = self.aepn.clone();
        let edited_files = self.edited_files.clone();
        let result = tokio::task::spawn_blocking(move || -> Result<meg_tools::ToolResult> {
            anyhow::ensure!(!cancel.is_cancelled(), "Run cancelled");
            if connected{
                let plane=plane.context("The MEG control plane is unavailable; run /doctor")?;
                let (method,params)=if name=="delegate"{("agents.run",serde_json::json!({"workspace":workspace,"role":args["role"].as_str().unwrap_or("researcher"),"task":args["task"],"worker":worker}))}else if name=="read_skill"{("skills.read",serde_json::json!({"workspace":workspace,"name":args["name"]}))}else{("runtime.tool",serde_json::json!({"workspace":workspace,"tool":name,"args":args,"readOnly":read_only}))};
                let value=plane.call_with_events(method,params,&cancel,|event|forward_agent_event(&events,event))?;
                if name=="delegate"{if let Some(tx)=&events{let _=tx.send(RunnerMsg::DelegatedUsage(value["usage"].clone()));}}
                return Ok(meg_tools::ToolResult{ok:true,output:if let Some(answer)=value["answer"].as_str(){answer.to_string()}else{serde_json::to_string(&value)?}});
            }
            if let Err(locked_msg) = read_only_lock.guard_mutation(&name) {
                return Ok(meg_tools::ToolResult { ok: false, output: locked_msg });
            }
            let unlock = args["unlock_deliverable"].as_bool().unwrap_or(false);
            if matches!(name.as_str(), "write_file" | "edit_file") {
                if let Some(raw_path) = args["path"].as_str() {
                    let norm = raw_path.replace('\\', "/");
                    let hit_sealed = sealed.contains_rel_path(&norm)
                        || meg_tools::fs_tools::resolve(&workspace, raw_path)
                            .map(|resolved| sealed.contains_path(&workspace, &resolved))
                            .unwrap_or(false);
                    if hit_sealed {
                        return Ok(meg_tools::ToolResult {
                            ok: false,
                            output: format!(
                                "SEALED_FILE_WALL: '{norm}' is a protected test/migration/check file and cannot be modified"
                            ),
                        });
                    }
                }
            }
            if matches!(name.as_str(), "write_file" | "edit_file") {
                if let Some(raw_path) = args["path"].as_str() {
                    if let Ok(resolved) = meg_tools::fs_tools::resolve(&workspace, raw_path) {
                        let mut g = guard.lock().unwrap();
                        if unlock {
                            g.unlock_deliverable(&resolved);
                        } else if let Some(blocked) = g.check_write(&resolved) {
                            return Ok(meg_tools::ToolResult { ok: false, output: blocked });
                        }
                    }
                }
            } else if name == "bash" && !unlock {
                if let Some(cmd) = args.get("command").or_else(|| args.get("cmd")).and_then(|v| v.as_str()) {
                    let g = guard.lock().unwrap();
                    if let Some(blocked) = g.check_bash(cmd) {
                        return Ok(meg_tools::ToolResult { ok: false, output: blocked });
                    }
                    let locked_targets: Vec<std::path::PathBuf> = g.target_paths.iter().cloned().collect();
                    if !locked_targets.is_empty() {
                        if let Err(pub_err) =
                            meg_tools::deliverable_guard::check_publish_state_shell(cmd, &locked_targets)
                        {
                            return Ok(meg_tools::ToolResult { ok: false, output: pub_err });
                        }
                    }
                }
            }
            let checkpoint = if matches!(name.as_str(), "write_file" | "edit_file") {
                if let Err(e) = meg_tools::journal::side_git_snapshot(&workspace, &format!("pre-{name}")) { if events.is_none() { eprintln!("[meg] warning: pre-edit shadow snapshot skipped: {e}"); } }
                Some(meg_tools::journal::FileCheckpoint::capture(
                    &workspace, &args,
                )?)
            } else {
                None
            };
            let pre_bash_snap = if name == "bash" {
                snapshot_workspace_files(&workspace)
            } else {
                None
            };
            let result = registry.execute(&name, &args);
            if let Some(before) = pre_bash_snap {
                if let Some(after) = snapshot_workspace_files(&workspace) {
                    let mut shell_changed = Vec::new();
                    for (path, sig) in &after {
                        if before.get(path) != Some(sig) {
                            shell_changed.push(path.clone());
                        }
                    }
                    if !shell_changed.is_empty() {
                        let _ = aepn.lock().unwrap().on_code_edit();
                        let mut ef = edited_files.lock().unwrap();
                        for p in shell_changed {
                            if !ef.contains(&p) {
                                ef.push(p);
                            }
                        }
                    }
                }
            }
            if result.ok {
                if let Some(checkpoint) = checkpoint {
                    checkpoint.commit_or_rollback(&workspace).map_err(|e| {
                        anyhow::anyhow!("Undo journal could not be saved; file edit was rolled back: {e}")
                    })?;
                    let _ = aepn.lock().unwrap().on_code_edit();
                    if let Some(raw_path) = args["path"].as_str() {
                        if let Ok(resolved) = meg_tools::fs_tools::resolve(&workspace, raw_path) {
                            let mut ef = edited_files.lock().unwrap();
                            if !ef.contains(&resolved) {
                                ef.push(resolved);
                            }
                        }
                    }
                }
                if name == "bash" {
                    let cmd = args
                        .get("command")
                        .or_else(|| args.get("cmd"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    if is_verification_command(cmd) {
                        let _ = aepn.lock().unwrap().record_tests_green();
                        let mut g = guard.lock().unwrap();
                        let targets: Vec<std::path::PathBuf> = g.target_paths.iter().cloned().collect();
                        for target in targets {
                            if target.is_file()
                                && meg_core::finish_hook::local_deliverable_check(std::slice::from_ref(&target)).is_empty()
                                && meg_tools::deliverable_guard::deep_artifact_sanity(&workspace, std::slice::from_ref(&target)).is_ok()
                            {
                                g.lock_verified_deliverable(&target);
                            }
                        }
                    }
                }
            }
            Ok(result)
        })
        .await?;
        match result {
            Ok(result) => {
                self.emit(RunnerMsg::ToolFinished {
                    id,
                    ok: result.ok,
                    output: result.output.clone(),
                });
                if result.ok {
                    self.consecutive_failures
                        .store(0, std::sync::atomic::Ordering::SeqCst);
                    Ok(result.output)
                } else {
                    self.consecutive_failures
                        .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    anyhow::bail!("{}", result.output)
                }
            }
            Err(error) => {
                self.consecutive_failures
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                self.emit(RunnerMsg::ToolFinished {
                    id,
                    ok: false,
                    output: error.to_string(),
                });
                Err(error)
            }
        }
    }
    async fn generate(&self, transcript: &[ChatMessage]) -> Result<String> {
        self.emit(RunnerMsg::StreamStart);
        let mut messages = transcript.to_vec();
        if self.shim.tools.is_empty() {
            let tools = if self.mode == meg_tui::Mode::Plan {
                "read_file {path, offset?, limit?, line_numbers?}; search_files {pattern}; grep_files {pattern, path?, glob?, case_insensitive?, context_lines?, max_results?}; glob_grep {pattern, path?, glob?, case_insensitive?, context_lines?, max_results?}; list_dir {path?}; todo {items:[{text,done}]}".into()
            } else {
                format!(
                    "{}; todo {{items:[{{text,done}}]}}",
                    self.registry
                        .stubs
                        .iter()
                        .map(|(n, d)| format!("{n}: {d}"))
                        .collect::<Vec<_>>()
                        .join("; ")
                )
            };
            messages[0].content.push_str(&format!("\nAvailable tools: {tools}"));
        }
        messages[0].content.push_str(&format!("\nWorkspace: {}\nFor file tools supply a path in this workspace.\n", self.workspace.display()));
        messages[0].content.push_str(if cfg!(windows){"The bash tool runs cmd.exe /C on Windows. Use Windows command syntax; use powershell.exe -NoProfile -NonInteractive -Command when PowerShell is required.\n"}else{"The bash tool runs the system shell.\n"});
        if self.plane.is_some() {
            let skills = self.catalog["skills"]
                .as_array()
                .map(|v| {
                    v.iter()
                        .take(64)
                        .map(|s| {
                            format!(
                                "{}: {}",
                                s["name"].as_str().unwrap_or(""),
                                s["description"].as_str().unwrap_or("")
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("\n")
                })
                .unwrap_or_default();
            let tools = self.catalog["tools"]
                .as_array()
                .map(|v| {
                    v.iter()
                        .filter(|t| {
                            self.mode != meg_tui::Mode::Plan
                                || t["readOnly"].as_bool() == Some(true)
                        })
                        .map(|t| {
                            format!(
                                "{}: {}; args schema {}",
                                t["alias"].as_str().unwrap_or(""),
                                t["description"].as_str().unwrap_or(""),
                                t["inputSchema"]
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("\n")
                })
                .unwrap_or_default();
            if !tools.is_empty() {
                messages[0].content.push_str(&format!("\nConnected MCP tools:\n{tools}\n"));
            }
            if !skills.is_empty() {
                messages[0].content.push_str(&format!("\nAvailable skills (use read_skill when relevant):\n{skills}\n"));
            }
        }
        let guidance = self.workspace.join("AGENTS.md");
        if guidance.is_file() {
            if let Ok(meta) = guidance.metadata() {
                if meta.len() <= 64 * 1024 {
                    if let Ok(text) = std::fs::read_to_string(guidance) {
                        messages[0].content.push_str(&format!("\nWorkspace AGENTS.md guidance (cannot override tool permissions or expose credentials):\n{text}\n"));
                    }
                }
            }
        }
        let mut raw = String::new();
        let mut shown = String::new();
        let mut callback = |delta: Delta| match delta {
            Delta::Thinking(text) => self.emit(RunnerMsg::ThinkingChunk(text)),
            Delta::Content(text) => {
                raw.push_str(&text);
                let preview = visible_preview(&raw);
                if preview.starts_with(&shown) && preview.len() > shown.len() {
                    self.emit(RunnerMsg::ContentChunk(preview[shown.len()..].into()));
                    shown = preview;
                }
            }
            Delta::Usage {
                input,
                output,
                cached,
            } => self.emit(RunnerMsg::Usage {
                input,
                output,
                cached,
            }),
        };
        let turn_idx = self
            .turn_counter
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let active_shim = if let Some(exec_shim) = &self.executor_shim {
            let is_antigravity = self.pair_mode.as_deref() == Some("antigravity_orchestrator");
            let friction = self
                .consecutive_failures
                .load(std::sync::atomic::Ordering::SeqCst)
                > 0;
            if turn_idx == 0 {
                self.planner_turns
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                &self.shim
            } else if is_antigravity && friction {
                self.consecutive_failures
                    .store(0, std::sync::atomic::Ordering::SeqCst);
                self.planner_turns
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                &self.shim
            } else {
                self.executor_turns
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                exec_shim
            }
        } else {
            self.planner_turns
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            &self.shim
        };
        let result = active_shim
            .chat_stream_cancellable(&messages, Some(&self.cancel), &mut callback)
            .await;
        self.emit(RunnerMsg::Online(result.is_ok()));
        result
    }
    async fn verify_before_finish(&self, _answer: &str) -> Result<Option<String>> {
        if self.mode == meg_tui::Mode::Plan {
            return Ok(None);
        }
        let edited: Vec<std::path::PathBuf> = self.edited_files.lock().unwrap().clone();
        // 1. Live Poteto anti-stub & false-zero check on every edited code file.
        let mut stub_findings = Vec::new();
        for path in &edited {
            if let Ok(text) = std::fs::read_to_string(path) {
                stub_findings.extend(meg_tools::deliverable_guard::scan_anti_stub_and_false_zero(
                    path, &text,
                ));
            }
        }
        if !stub_findings.is_empty() {
            return Ok(Some(format!(
                "[ANTI_STUB_GATE WITHHOLD] Incomplete stub or false-zero handler detected in edited code:\n{}\nFix all items above and verify before declaring completion.",
                stub_findings.join("\n")
            )));
        }

        // 2. Out-of-band `+OO` external verification command (30s timeout).
        if let Some(cmd) = self.verify_cmd.clone() {
            let ws = self.workspace.clone();
            let cancel = self.cancel.clone();
            let res = tokio::task::spawn_blocking(move || {
                meg_tools::bash::run_cancellable(
                    &ws,
                    &serde_json::json!({ "command": cmd, "timeout_seconds": 30 }),
                    &cancel.0,
                )
            })
            .await?;
            match res {
                Ok(_) => {
                    self.verify_passed.store(true, std::sync::atomic::Ordering::SeqCst);
                    let _ = self.aepn.lock().unwrap().record_tests_green();
                }
                Err(e) => {
                    self.verify_passed.store(false, std::sync::atomic::Ordering::SeqCst);
                    return Ok(Some(format!(
                        "[+OO VERIFICATION WITHHOLD] External check failed:\n{e}\nFix the underlying code (never protected tests) and try again."
                    )));
                }
            }
        } else if self.require_verify && !edited.is_empty() {
            // 3. Strict AEPN check when --require-verify is active: tests must be green AFTER the last code edit.
            let tests_green = self.aepn.lock().unwrap().p_tests_green;
            if tests_green == 0 {
                return Ok(Some(
                    "[AEPN VERIFICATION WITHHOLD] Code was edited without a passing verification command afterward. Run your test/build check via bash before finishing.".into()
                ));
            }
        }
        Ok(None)
    }
}

fn visible_preview(raw: &str) -> String {
    let protocol = raw
        .trim_start()
        .strip_prefix("```json")
        .unwrap_or(raw)
        .trim_start();
    if !protocol.starts_with('{') {
        return if raw.trim_start().starts_with("```json") {
            String::new()
        } else {
            raw.to_string()
        };
    }
    let raw = protocol;
    for key in ["final", "final_answer", "answer", "response"] {
        if let Some(index) = raw.find(&format!("\"{key}\"")) {
            let rest = raw[index + key.len() + 2..].trim_start();
            let Some(rest) = rest.strip_prefix(':') else {
                continue;
            };
            let rest = rest.trim_start();
            if !rest.starts_with('"') {
                continue;
            }
            let mut escaped = false;
            let mut end = None;
            for (index, ch) in rest.char_indices().skip(1) {
                if !escaped && ch == '"' {
                    end = Some(index + 1);
                    break;
                }
                escaped = !escaped && ch == '\\';
            }
            let candidate = if let Some(end) = end {
                rest[..end].to_string()
            } else {
                let mut trimmed = rest;
                // Trim incomplete trailing escape (`\` or partial `\uXXXX`) before closing quote.
                if escaped {
                    trimmed = trimmed.strip_suffix('\\').unwrap_or(trimmed);
                } else if let Some(u_idx) = trimmed.rfind("\\u") {
                    let tail = &trimmed[u_idx + 2..];
                    if tail.len() < 4 && tail.chars().all(|c| c.is_ascii_hexdigit()) {
                        trimmed = &trimmed[..u_idx];
                    }
                }
                format!("{trimmed}\"")
            };
            return serde_json::from_str::<String>(&candidate).unwrap_or_default();
        }
    }
    String::new()
}
fn parse_cli_args(args: &[String]) -> CliCommand {
    if args.get(1).is_some_and(|s| s == "tui")
        && args.iter().skip(2).any(|s| s == "--help" || s == "-h")
    {
        return CliCommand::Help;
    }
    match args.get(1).map(|s| s.as_str()) {
        None | Some("tui") => CliCommand::Tui,
        Some(
            "-C" | "--cd" | "-m" | "--model" | "--effort" | "--resume" | "--continue"
            | "--no-picker" | "--workspace-picker",
        ) => CliCommand::Tui,
        Some("-h") | Some("--help") => CliCommand::Help,
        Some("-V") | Some("--version") => CliCommand::Version,
        Some("worker") => CliCommand::Worker,
        Some("run") => {
            if args
                .get(2)
                .is_some_and(|arg| matches!(arg.as_str(), "-h" | "--help"))
            {
                return CliCommand::Help;
            }
            let goal = parse_run_options(args)
                .map(|(goal, _)| goal)
                .unwrap_or_default();
            CliCommand::Run { goal }
        }
        Some("loop") => {
            if args
                .get(2)
                .is_some_and(|arg| matches!(arg.as_str(), "-h" | "--help"))
            {
                return CliCommand::Help;
            }
            CliCommand::Loop
        }
        Some("ui-bridge") => {
            if args
                .get(2)
                .is_some_and(|arg| matches!(arg.as_str(), "-h" | "--help"))
            {
                return CliCommand::Help;
            }
            CliCommand::UiBridge
        }
        Some(cmd) => CliCommand::Unknown(cmd.to_string()),
    }
}

enum CliCommand {
    Worker,
    Tui,
    Run { goal: String },
    Loop,
    UiBridge,
    Help,
    Version,
    Unknown(String),
}

#[derive(Default)]
struct TuiOptions {
    cwd: Option<std::path::PathBuf>,
    model: Option<String>,
    pinned_model: Option<String>,
    effort: Option<String>,
    resume: Option<String>,
    picker: Option<bool>,
    max_steps: Option<usize>,
    json_output: bool,
    core_six: bool,
    pair_mode: Option<String>,
    executor_model: Option<String>,
    verify_cmd: Option<String>,
    require_verify: bool,
    reviewer_model: Option<String>,
    ui_mode: Option<String>,
}
fn parse_tui_options(args: &[String]) -> Result<TuiOptions> {
    let mut options = TuiOptions::default();
    let mut i = if args.get(1).is_some_and(|s| s == "tui") {
        2
    } else {
        1
    };
    while i < args.len() {
        let flag = args[i].as_str();
        match flag {
            "--continue" => options.resume = Some("latest".into()),
            "--no-picker" => options.picker = Some(false),
            "--workspace-picker" => options.picker = Some(true),
            "-C" | "--cd" | "-m" | "--model" | "--effort" | "--resume" | "--ui" | "--pair-mode" | "--executor-model" | "--reviewer-model" => {
                i += 1;
                let value = args
                    .get(i)
                    .ok_or_else(|| anyhow::anyhow!("{flag} requires a value"))?
                    .clone();
                match flag {
                    "-C" | "--cd" => options.cwd = Some(value.into()),
                    "-m" | "--model" => options.model = Some(value),
                    "--effort" => {
                        anyhow::ensure!(
                            matches!(value.as_str(), "low" | "medium" | "high" | "max"),
                            "Effort must be low, medium, high or max"
                        );
                        options.effort = Some(value);
                    }
                    _ => options.resume = Some(value),
                }
            }
            _ => anyhow::bail!("Unknown TUI option '{flag}'. Use meg --help."),
        }
        i += 1;
    }
    Ok(options)
}

/// Recognized options are parsed only before the goal; `--` preserves literal flags.
fn parse_run_options(args: &[String]) -> Result<(String, TuiOptions)> {
    let mut options = TuiOptions::default();
    let mut i = 2;
    while i < args.len() {
        match args[i].as_str() {
            "--" => {
                i += 1;
                break;
            }
            "--headless" => i += 1,
            "--json" => {
                options.json_output = true;
                i += 1;
            }
            "--core-six" => {
                options.core_six = true;
                i += 1;
            }
            "--require-verify" => {
                options.require_verify = true;
                i += 1;
            }
            "-C" | "--cd" | "-m" | "--model" | "--pinned-model" | "--effort" | "--max-steps" | "--pair-mode" | "--executor-model" | "--verify-cmd" | "--reviewer-model" => {
                let flag = &args[i];
                let value = args
                    .get(i + 1)
                    .context(format!("{flag} requires a value"))?;
                anyhow::ensure!(!value.is_empty(), "{flag} requires a non-empty value");
                match flag.as_str() {
                    "-C" | "--cd" => options.cwd = Some(value.into()),
                    "-m" | "--model" => options.model = Some(value.clone()),
                    "--pinned-model" => {
                        options.pinned_model = Some(value.clone());
                        options.model = Some(value.clone());
                    }
                    "--pair-mode" => {
                        anyhow::ensure!(
                            matches!(value.as_str(), "devin_fusion" | "antigravity_orchestrator"),
                            "--pair-mode must be 'devin_fusion' or 'antigravity_orchestrator'"
                        );
                        options.pair_mode = Some(value.clone());
                    }
                    "--executor-model" => options.executor_model = Some(value.clone()),
                    "--verify-cmd" => options.verify_cmd = Some(value.clone()),
                    "--reviewer-model" => options.reviewer_model = Some(value.clone()),
                    "--max-steps" => {
                        let parsed: usize = value
                            .parse()
                            .context("--max-steps requires an integer in 1..=200")?;
                        anyhow::ensure!(
                            (1..=200).contains(&parsed),
                            "--max-steps must be between 1 and 200"
                        );
                        options.max_steps = Some(parsed);
                    }
                    _ => {
                        anyhow::ensure!(
                            matches!(value.as_str(), "low" | "medium" | "high" | "max"),
                            "Effort must be low, medium, high or max"
                        );
                        options.effort = Some(value.clone());
                    }
                }
                i += 2;
            }
            _ => break,
        }
    }
    let goal = args[i..].join(" ");
    anyhow::ensure!(!goal.trim().is_empty(), "meg run requires a goal");
    Ok((goal, options))
}

struct LoopOptions {
    goal: String,
    check_cmd: String,
    max_runs: usize,
    max_same: usize,
    base: TuiOptions,
}

fn parse_loop_options(args: &[String]) -> Result<LoopOptions> {
    let mut base = TuiOptions::default();
    let mut check_cmd: Option<String> = None;
    let mut max_runs: usize = 5;
    let mut max_same: usize = 2;
    let mut i = 2;
    while i < args.len() {
        match args[i].as_str() {
            "--" => {
                i += 1;
                break;
            }
            "--json" => {
                base.json_output = true;
                i += 1;
            }
            "--core-six" => {
                base.core_six = true;
                i += 1;
            }
            "-C" | "--cd" | "-m" | "--model" | "--pinned-model" | "--effort" | "--max-steps"
            | "--pair-mode" | "--executor-model" | "--reviewer-model" | "--check"
            | "--max-runs" | "--max-same" => {
                let flag = &args[i];
                let value = args
                    .get(i + 1)
                    .context(format!("{flag} requires a value"))?;
                anyhow::ensure!(!value.is_empty(), "{flag} requires a non-empty value");
                match flag.as_str() {
                    "-C" | "--cd" => base.cwd = Some(value.into()),
                    "-m" | "--model" => base.model = Some(value.clone()),
                    "--pinned-model" => {
                        base.pinned_model = Some(value.clone());
                        base.model = Some(value.clone());
                    }
                    "--pair-mode" => {
                        anyhow::ensure!(
                            matches!(value.as_str(), "devin_fusion" | "antigravity_orchestrator"),
                            "--pair-mode must be 'devin_fusion' or 'antigravity_orchestrator'"
                        );
                        base.pair_mode = Some(value.clone());
                    }
                    "--executor-model" => base.executor_model = Some(value.clone()),
                    "--reviewer-model" => base.reviewer_model = Some(value.clone()),
                    "--check" => check_cmd = Some(value.clone()),
                    "--max-steps" => {
                        let n: usize = value.parse().context("--max-steps requires integer 1..=200")?;
                        anyhow::ensure!((1..=200).contains(&n), "--max-steps must be 1..=200");
                        base.max_steps = Some(n);
                    }
                    "--max-runs" => {
                        let n: usize = value.parse().context("--max-runs requires integer 1..=20")?;
                        anyhow::ensure!((1..=20).contains(&n), "--max-runs must be 1..=20");
                        max_runs = n;
                    }
                    "--max-same" => {
                        let n: usize = value.parse().context("--max-same requires integer 1..=5")?;
                        anyhow::ensure!((1..=5).contains(&n), "--max-same must be 1..=5");
                        max_same = n;
                    }
                    _ => {
                        anyhow::ensure!(
                            matches!(value.as_str(), "low" | "medium" | "high" | "max"),
                            "Effort must be low, medium, high or max"
                        );
                        base.effort = Some(value.clone());
                    }
                }
                i += 2;
            }
            _ => break,
        }
    }
    let goal = args[i..].join(" ");
    anyhow::ensure!(!goal.trim().is_empty(), "meg loop requires a goal");
    let check_cmd = check_cmd.context("meg loop requires --check <cmd>")?;
    Ok(LoopOptions {
        goal,
        check_cmd,
        max_runs,
        max_same,
        base,
    })
}

fn print_usage() {
    println!("MEG â€” native agent harness & workstation\n");
    println!("Usage:");
    println!("  meg [COMMAND] [OPTIONS]\n");
    println!("Commands:");
    println!("  tui             Launch the interactive Workstation TUI (default)");
    println!("  run [OPTIONS] [--] <goal>  Execute headlessly (-C, -m, --pinned-model, --verify-cmd, --pair-mode, --executor-model, --reviewer-model, --json)");
    println!("  loop --check <cmd> [OPTIONS] [--] <goal>  Run verified Loop-2 outer work loop (.loop/state.json, check_cannot_fail, stall exit)");
    println!("  ui-bridge [-C <dir>]  Open NDJSON stdin/stdout protocol so any custom TUI/GUI can drive MEG");
    println!("\nOptions:");
    println!("  -h, --help      Print this help message");
    println!("  -C, --cd PATH   Open a workspace\n  -m, --model ID  Select the session model\n  --effort LEVEL  low, medium, high or max\n  --resume ID     Resume a saved session (or latest)\n  --continue      Resume the latest saved session\n  --workspace-picker  Choose a workspace before opening\n  --no-picker     Open the current workspace directly");
    println!("\nBare meg opens the workspace selector. Use /provider for local or cloud connections,\n/model for model selection and /help for skills, plugins, MCP and worker commands.\nNo MEG account is required; cloud providers use your own API key.\nMEG_TUI_BASE_URL / MEG_TUI_MODEL / MEG_TUI_API_KEY remain supported overrides.");
}

/// Launches the interactive Workstation with the selected local/cloud provider.
fn launch_tui(workspace: &Path) -> Result<()> {
    let options = parse_tui_options(&std::env::args().collect::<Vec<_>>())?;
    let workspace_path = options
        .cwd
        .clone()
        .unwrap_or_else(|| workspace.to_path_buf())
        .canonicalize()?;
    let workspace = workspace_path.as_path();
    let provider_store = meg_core::providers::ProviderStore::global();
    let settings = provider_store.load()?;
    let mut profiles = settings.profiles;
    let provider = if std::env::var_os("MEG_TUI_BASE_URL").is_some() {
        let p = meg_core::providers::ProviderProfile::environment();
        profiles.insert(0, p.clone());
        Some(p)
    } else {
        profiles
            .iter()
            .find(|p| p.id == settings.active)
            .or_else(|| profiles.first())
            .cloned()
    };
    let model = provider
        .as_ref()
        .map(|p| p.default_model.clone())
        .unwrap_or_default();
    let available_models = if model.is_empty() {
        Vec::new()
    } else {
        vec![model.clone()]
    };
    let show_picker = options.picker.unwrap_or(
        options.cwd.is_none()
            && options.resume.is_none()
            && std::env::var("MEG_NO_PICKER").ok().as_deref() != Some("1"),
    );
    let plane_result = start_plane();
    let plane_error = plane_result.as_ref().err().map(ToString::to_string);
    let plane = plane_result.ok();
    let mut st = WorkstationState {
        provider,
        providers: profiles,
        provider_root: Some(provider_store.root),
        notice: plane_error
            .map(|e| format!("Control plane: {e}"))
            .unwrap_or_default(),
        startup: meg_tui::StartupOptions {
            resume: options.resume,
            model: options.model,
            effort: options.effort,
        },
        project: workspace
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default(),
        model,
        cwd: meg_tui::friendly_path(workspace),
        git_branch: detect_git_branch(workspace),
        clock: meg_tui_clock(),
        theme: meg_tui::DASHBOARD,
        online: false,
        available_models,
        workspace_picker: if show_picker {
            Some(meg_tui::WorkspacePicker::open_startup(
                workspace.to_path_buf(),
            ))
        } else {
            None
        },
        ..Default::default()
    };
    // The goal runner: shim agent loop per goal on a background thread
    // (its own tokio runtime), streaming deltas into the TUI.
    let goal_plane = plane.clone();
    let runner = Arc::new(
        move |request: meg_tui::RunRequest, tx: Sender<RunnerMsg>| -> Result<Option<String>> {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            let extras = RunExtras {
                pair_mode: request.pair_mode.clone(),
                executor_model: request.executor_model.clone(),
                reviewer_model: request.reviewer_model.clone(),
                ..RunExtras::default()
            };
            let (_outcomes, final_answer) = rt.block_on(run_request_with_options(
                request,
                Some(tx),
                goal_plane.clone(),
                extras,
            ))?;
            Ok(final_answer)
        },
    );
    let services: Arc<meg_tui::ServiceRunner> = Arc::new(move |request, cancel, events| {
        if request.method == "provider.models" {
            let profile: meg_core::providers::ProviderProfile =
                serde_json::from_value(request.params)?;
            let client = ShimClient::with_profile(&profile)?;
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            let timeout =
                std::time::Duration::from_secs(if meg_core::provider_catalog::is_local(&profile) {
                    3
                } else {
                    15
                });
            let models=runtime.block_on(async{tokio::select!{result=client.models_with_timeout(timeout)=>result,_=cancel.cancelled()=>anyhow::bail!("Connection check cancelled")}})?;
            return Ok(
                serde_json::json!({"provider_id":profile.id,"base_url":profile.base_url,"models":models,"message":format!("Connected to {}",profile.name)}),
            );
        }
        let plane = plane
            .as_ref()
            .context("MEG control plane unavailable; install Bun and the MEG runtime bundle")?;
        let mut params = request.params;
        params["workspace"] = serde_json::to_value(request.workspace)?;
        plane.call_with_events(&request.method, params, &cancel, |value| {
            forward_agent_event(&Some(events.clone()), value)
        })
    });
    let (ui_cfg, _) = meg_tui::UserUiConfig::load(Some(workspace));
    st.ui_mode = options
        .ui_mode
        .as_deref()
        .and_then(meg_tui::UiMode::parse)
        .unwrap_or(ui_cfg.ui_mode);
    st.collapse_tool_cards = ui_cfg.compact_tool_cards;
    if let Some(tname) = &ui_cfg.theme_name {
        if let Some(p) = meg_tui::get_palette(tname) {
            st.theme = p;
        }
    }
    st.pair_mode = options.pair_mode.clone().or_else(|| ui_cfg.pair_mode.clone());
    st.executor_model = options
        .executor_model
        .clone()
        .or_else(|| ui_cfg.executor_model.clone());
    st.reviewer_model = options
        .reviewer_model
        .clone()
        .or_else(|| ui_cfg.reviewer_model.clone());
    st.ui_config = ui_cfg;
    meg_tui::run_app_with_services(st, Some(runner), Some(services))?;
    Ok(())
}

async fn run_request(
    request: meg_tui::RunRequest,
    tx: Option<Sender<RunnerMsg>>,
) -> Result<(Vec<meg_core::TurnOutcome>, Option<String>)> {
    run_request_with_plane(request, tx, None).await
}
#[derive(Default, Clone)]
struct RunExtras {
    max_steps: Option<usize>,
    verify_cmd: Option<String>,
    require_verify: bool,
    pair_mode: Option<String>,
    executor_model: Option<String>,
    reviewer_model: Option<String>,
}

#[derive(Default, Debug, Clone)]
struct RunTelemetry {
    planner_turns: usize,
    executor_turns: usize,
    reviewer_verdict: Option<String>,
}

struct ColdReviewRequest<'a> {
    provider: &'a Option<meg_core::providers::ProviderProfile>,
    base_url: &'a str,
    reviewer_model: &'a str,
    workspace: &'a std::path::Path,
    goal: &'a str,
    edited_files: &'a [std::path::PathBuf],
    final_answer: &'a str,
    cancel: &'a meg_core::CancellationToken,
}

async fn run_request_with_plane(
    request: meg_tui::RunRequest,
    tx: Option<Sender<RunnerMsg>>,
    plane: Option<Arc<meg_core::plane_client::PlaneClient>>,
) -> Result<(Vec<meg_core::TurnOutcome>, Option<String>)> {
    run_request_with_options(request, tx, plane, RunExtras::default()).await
}

async fn run_request_with_options(
    request: meg_tui::RunRequest,
    tx: Option<Sender<RunnerMsg>>,
    plane: Option<Arc<meg_core::plane_client::PlaneClient>>,
    extras: RunExtras,
) -> Result<(Vec<meg_core::TurnOutcome>, Option<String>)> {
    let (outcomes, answer, _telemetry) = run_request_detailed(request, tx, plane, extras).await?;
    Ok((outcomes, answer))
}

async fn run_cold_l3_reviewer(req: ColdReviewRequest<'_>) -> Result<String> {
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

fn parse_l3_reviewer_verdict(raw: &str) -> (String, String) {
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

fn snapshot_workspace_files(
    workspace: &Path,
) -> Option<std::collections::BTreeMap<std::path::PathBuf, (u64, String)>> {
    const MAX_FILES: usize = 4_000;
    const MAX_HASH_BYTES: u64 = 2 * 1024 * 1024;
    let mut map = std::collections::BTreeMap::new();
    let mut stack = vec![workspace.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let p = entry.path();
            if let Some(name) = entry.file_name().to_str() {
                if matches!(
                    name,
                    ".git"
                        | "target"
                        | "node_modules"
                        | "__pycache__"
                        | ".venv"
                        | "venv"
                        | ".meg"
                        | ".loop"
                        | ".cache"
                        | ".pytest_cache"
                        | ".mypy_cache"
                        | ".ruff_cache"
                ) {
                    continue;
                }
            }
            let Ok(meta) = std::fs::symlink_metadata(&p) else { continue };
            if meta.file_type().is_symlink() {
                continue;
            }
            if meta.is_dir() {
                stack.push(p);
                continue;
            }
            if !meta.is_file() {
                continue;
            }
            if map.len() >= MAX_FILES {
                return None;
            }
            let len = meta.len();
            let digest = if len <= MAX_HASH_BYTES {
                std::fs::read(&p)
                    .map(|b| meg_tools::deliverable_guard::sha256_hex(&b))
                    .unwrap_or_default()
            } else {
                format!("mtime:{:?}", meta.modified().ok())
            };
            map.insert(p, (len, digest));
        }
    }
    Some(map)
}

async fn run_request_detailed(
    request: meg_tui::RunRequest,
    tx: Option<Sender<RunnerMsg>>,
    plane: Option<Arc<meg_core::plane_client::PlaneClient>>,
    extras: RunExtras,
) -> Result<(Vec<meg_core::TurnOutcome>, Option<String>, RunTelemetry)> {
    anyhow::ensure!(
        !request.model.trim().is_empty(),
        "No model selected. Connect a provider and choose /model first."
    );
    let worker = serde_json::json!({"provider":request.provider,"model":request.model,"base_url":request.base_url,"effort":request.effort,"handoff_id":uuid::Uuid::new_v4().to_string()});
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
        let value = tokio::task::spawn_blocking(move || {
            p.call(
                "runtime.catalog",
                serde_json::json!({"workspace":workspace}),
                &cancel,
            )
        })
        .await??;
        catalog = value["catalog"].clone();
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

fn default_max_steps() -> usize {
    std::env::var("MEG_MAX_STEPS")
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .map(|n| n.clamp(1, 200))
        .unwrap_or(40)
}

fn is_verification_command(cmd: &str) -> bool {
    let low = cmd.trim().to_ascii_lowercase();
    if low.starts_with("echo ") || low.starts_with("printf ") {
        return false;
    }
    [
        "pytest",
        "python -m pytest",
        "python -m unittest",
        "cargo test",
        "cargo check",
        "cargo clippy",
        "bun test",
        "npm test",
        "npm run test",
        "pnpm test",
        "yarn test",
        "vitest",
        "jest",
        "go test",
        "dotnet test",
        "ctest",
        "make test",
        "tsc --noemit",
        "mypy",
        "ruff check",
        "eslint",
    ]
    .iter()
    .any(|k| low.contains(k))
}

/// Extract explicit deliverable paths only when the goal in Act mode expresses
/// file creation/editing intent, avoiding false positives on casual prose.
fn extract_finish_deliverables(
    workspace: &Path,
    goal: &str,
    mode: meg_tui::Mode,
) -> Vec<std::path::PathBuf> {
    if mode == meg_tui::Mode::Plan {
        return Vec::new();
    }
    let low = goal.to_ascii_lowercase();
    let has_write_intent = [
        "write ", "create ", "save ", "generate ", "produce ", "output ", "implement ",
        "build ", "edit ", "fix ", "update ", "/app/",
    ]
    .iter()
    .any(|k| low.contains(k));
    if !has_write_intent {
        return Vec::new();
    }
    const KNOWN_EXTS: &[&str] = &[
        "rs", "py", "ts", "tsx", "js", "jsx", "c", "h", "cpp", "hpp", "go", "java", "rb",
        "sh", "ps1", "bat", "cmd", "json", "toml", "yaml", "yml", "md", "txt", "csv",
        "html", "css", "sql", "scm", "xml", "png", "bmp", "bin", "pt", "pth", "pkl",
    ];
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for raw in goal.split(|c: char| c.is_whitespace() || c == ',' || c == '"') {
        let token = raw
            .trim_matches(|c: char| matches!(c, '`' | '\'' | '(' | ')' | '[' | ']' | ':' | ';' | '?' | '!'))
            .trim_end_matches('.');
        if token.is_empty() || token.contains("://") {
            continue;
        }
        let candidate = if let Some(rest) = token.strip_prefix("/app/") {
            rest
        } else {
            token
        };
        if candidate.is_empty() || !candidate.contains('.') {
            continue;
        }
        let Some(ext) = candidate.rsplit('.').next() else {
            continue;
        };
        if !KNOWN_EXTS.contains(&ext.to_ascii_lowercase().as_str()) {
            continue;
        }
        if let Ok(resolved) = meg_tools::fs_tools::resolve(workspace, candidate) {
            if seen.insert(resolved.clone()) {
                out.push(resolved);
            }
        }
    }
    out
}
fn build_tool_specs(
    registry: &ToolRegistry,
    mode: meg_tui::Mode,
    catalog: &serde_json::Value,
    has_plane: bool,
) -> Vec<meg_core::ToolSpec> {
    let mut specs: Vec<meg_core::ToolSpec> = registry
        .tool_schemas()
        .into_iter()
        .filter(|s| {
            mode != meg_tui::Mode::Plan
                || matches!(
                    s.name.as_str(),
                    "read_file" | "search_files" | "grep_files" | "glob_grep" | "list_dir"
                )
        })
        .map(|s| meg_core::ToolSpec {
            name: s.name,
            description: s.description,
            parameters: s.parameters,
        })
        .collect();
    specs.push(meg_core::ToolSpec {
        name: "todo".into(),
        description: "Update the task checklist: {items: [{text, done}]}.".into(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "items": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "text": { "type": "string" },
                            "done": { "type": "boolean" }
                        },
                        "required": ["text", "done"]
                    }
                }
            },
            "required": ["items"]
        }),
    });
    if has_plane {
        specs.push(meg_core::ToolSpec {
            name: "read_skill".into(),
            description: "Load full SKILL.md instructions for an installed skill.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": { "name": { "type": "string" } },
                "required": ["name"]
            }),
        });
        specs.push(meg_core::ToolSpec {
            name: "delegate".into(),
            description: "Run a read-only planner, researcher or reviewer in a separate native worker.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "role": { "type": "string", "enum": ["planner", "researcher", "reviewer"] },
                    "task": { "type": "string" }
                },
                "required": ["role", "task"]
            }),
        });
        if let Some(mcp_tools) = catalog["tools"].as_array() {
            for t in mcp_tools {
                if mode == meg_tui::Mode::Plan && t["readOnly"].as_bool() != Some(true) {
                    continue;
                }
                if let Some(alias) = t["alias"].as_str() {
                    let params = if t["inputSchema"].is_object() {
                        t["inputSchema"].clone()
                    } else {
                        serde_json::json!({ "type": "object", "properties": {} })
                    };
                    let safe_name = sanitize_mcp_tool_name(alias);
                    specs.push(meg_core::ToolSpec {
                        name: safe_name,
                        description: t["description"].as_str().unwrap_or(alias).to_string(),
                        parameters: params,
                    });
                }
            }
        }
    }
    specs
}

fn sanitize_mcp_tool_name(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len().min(64));
    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
            out.push(ch);
        } else if ch == ':' {
            out.push_str("__");
        } else {
            out.push('_');
        }
    }
    if out.is_empty() {
        return "mcp_tool".into();
    }
    if out.len() <= 64 {
        return out;
    }
    let mut h: u32 = 2166136261;
    for b in raw.as_bytes() {
        h = (h ^ (*b as u32)).wrapping_mul(16777619);
    }
    format!("{}_{h:08x}", &out[..55])
}

fn parse_bridge_mode(raw: &str) -> Result<meg_tui::Mode> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "plan" | "readonly" => Ok(meg_tui::Mode::Plan),
        "build" | "act" => Ok(meg_tui::Mode::Build),
        other => anyhow::bail!("Invalid mode '{other}'; expected 'plan' or 'build'"),
    }
}

fn forward_agent_event(tx: &Option<Sender<RunnerMsg>>, value: serde_json::Value) {
    if let Some(tx) = tx {
        if let (Some(id), Some(status)) = (value["id"].as_str(), value["status"].as_str()) {
            let _ = tx.send(RunnerMsg::SubagentState(id.into(), status.into()));
        }
    }
}
fn start_plane() -> Result<Arc<meg_core::plane_client::PlaneClient>> {
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
    meg_core::plane_client::PlaneClient::spawn(&entry, &exe, Some(&builtin))
}

fn builtin_skills_path(entry: &Path, repo: &Path) -> std::path::PathBuf {
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
async fn run_worker() -> Result<()> {
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
fn detect_git_branch(workspace: &Path) -> String {
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

fn meg_tui_clock() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{:02}:{:02} UTC", (secs / 3600) % 24, (secs / 60) % 60)
}

/// Executes a goal with the workspace's saved local/cloud connection settings.
async fn run_goal_headless(workspace: &Path, goal: &str, options: &TuiOptions) -> Result<()> {
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
    let plane = if std::env::var("MEG_NO_PLANE").as_deref() == Ok("1") {
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

/// Compute a deterministic 8-hex-char fingerprint of the last 20 lines of failing output
/// after masking digits so identical bugs produce the same hash.
fn mask_failure_fingerprint(output: &str) -> String {
    let lines: Vec<&str> = output.lines().collect();
    let tail = if lines.len() > 20 {
        &lines[lines.len() - 20..]
    } else {
        &lines[..]
    };
    let mut normalized = String::new();
    for line in tail {
        for ch in line.chars() {
            if ch.is_ascii_digit() {
                normalized.push('#');
            } else {
                normalized.push(ch);
            }
        }
        normalized.push('\n');
    }
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in normalized.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{:08x}", (hash >> 32) as u32)
}

fn run_external_check_cmd(workspace: &std::path::Path, cmd: &str) -> (bool, String) {
    match meg_tools::bash::run(
        workspace,
        &serde_json::json!({ "command": cmd, "timeout_seconds": 60 }),
    ) {
        Ok(out) => (true, out),
        Err(err) => {
            let msg = err.to_string();
            let tail = if msg.len() > 3_000 {
                let start = msg.floor_char_boundary(msg.len() - 3_000);
                msg[start..].to_string()
            } else {
                msg
            };
            (false, tail)
        }
    }
}

/// Loop-2 verified outer work loop (`meg loop --check "<cmd>" "<goal>"`):
/// - Pre-run `check_cannot_fail` verification (must fail before agent starts)
/// - Persists `.loop/state.json` on disk with `tried` dead-end history
/// - Masks digits in failure fingerprints and parks on `stalled` after `max_same` repeats
/// - Signs `.meg/receipt.json` only when external `--check` exits green
async fn run_loop_headless(workspace: &std::path::Path, options: &LoopOptions) -> Result<()> {
    anyhow::ensure!(
        std::env::var("MEG_OFFLINE").as_deref() != Ok("1"),
        "MEG_OFFLINE=1: inference is disabled; no provider was contacted"
    );
    let loop_dir = workspace.join(".loop");
    std::fs::create_dir_all(&loop_dir)?;
    let state_path = loop_dir.join("state.json");

    let mut runs_log: Vec<serde_json::Value> = Vec::new();
    let write_state = |status: &str, runs: &[serde_json::Value]| -> Result<()> {
        let doc = serde_json::json!({
            "goal": options.goal,
            "check_cmd": options.check_cmd,
            "status": status,
            "runs": runs,
        });
        std::fs::write(&state_path, serde_json::to_vec_pretty(&doc)?)?;
        Ok(())
    };

    // 1. Pre-run check_cannot_fail gate: if check already passes before Run 1, abort!
    let (pre_ok, mut last_output) = run_external_check_cmd(workspace, &options.check_cmd);
    if pre_ok {
        write_state("check_cannot_fail", &runs_log)?;
        if options.base.json_output {
            println!(
                "{}",
                serde_json::to_string(&serde_json::json!({
                    "ok": false,
                    "status": "check_cannot_fail",
                    "runs": 0,
                    "state_path": state_path.display().to_string(),
                }))?
            );
        }
        anyhow::bail!(
            "CHECK_CANNOT_FAIL: external check `{}` was already green before any agent edit",
            options.check_cmd
        );
    }

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
    let default_model = std::env::var("MEG_TUI_MODEL")
        .ok()
        .or_else(|| provider.as_ref().map(|p| p.default_model.clone()))
        .unwrap_or_default();
    if options.base.core_six {
        std::env::set_var("MEG_CORE_SIX", "1");
    }
    let plane = if std::env::var("MEG_NO_PLANE").as_deref() == Ok("1") {
        None
    } else {
        Some(start_plane()?)
    };

    let mut recent_fps: Vec<String> = Vec::new();
    for run_idx in 1..=options.max_runs {
        let mut state = WorkstationState {
            cwd: meg_tui::friendly_path(workspace),
            providers: settings.profiles.clone(),
            provider: provider.clone(),
            model: default_model.clone(),
            ..Default::default()
        };
        if let Some(m) = &options.base.model {
            state.select_model(m.clone());
        }
        if let Some(pinned) = &options.base.pinned_model {
            state.select_model(pinned.clone());
            anyhow::ensure!(
                state.model == *pinned,
                "PINNED_MODEL_VIOLATION: expected '{pinned}', resolved '{}'",
                state.model
            );
        }
        if let Some(effort) = &options.base.effort {
            state.select_effort(effort.clone());
        }

        let tried_summary = if runs_log.is_empty() {
            "- nothing yet".to_string()
        } else {
            runs_log
                .iter()
                .map(|r| format!("- run {}: {}", r["n"], r["tried"].as_str().unwrap_or("")))
                .collect::<Vec<_>>()
                .join("\n")
        };
        let iter_goal = format!(
            "{}\n\nExternal check `{}` currently fails with:\n{}\n\nAlready tried (DO NOT REPEAT):\n{}\n\nMake the smallest fix to the implementation (never edit protected tests).",
            options.goal, options.check_cmd, last_output, tried_summary
        );

        let attempt = run_request_detailed(
            state.run_request(iter_goal),
            None,
            plane.clone(),
            RunExtras {
                max_steps: options.base.max_steps,
                verify_cmd: None,
                require_verify: false,
                pair_mode: options.base.pair_mode.clone(),
                executor_model: options.base.executor_model.clone(),
                reviewer_model: options.base.reviewer_model.clone(),
            },
        )
        .await;

        let tried_note = match &attempt {
            Ok((_, Some(ans), _)) => ans.lines().next().unwrap_or("completed").chars().take(180).collect::<String>(),
            Ok((_, None, _)) => "exhausted step budget".into(),
            Err(e) => format!("error: {}", e.to_string().chars().take(160).collect::<String>()),
        };

        let inner_ok = matches!(&attempt, Ok((_, Some(_), _)));
        let (check_ok, out_now) = run_external_check_cmd(workspace, &options.check_cmd);
        let ok_now = check_ok && inner_ok;
        let fp = if ok_now {
            "00000000".to_string()
        } else if !inner_ok {
            "inner_rejected".to_string()
        } else {
            mask_failure_fingerprint(&out_now)
        };
        runs_log.push(serde_json::json!({
            "n": run_idx,
            "ok": ok_now,
            "inner_ok": inner_ok,
            "check_ok": check_ok,
            "failure": fp,
            "tried": tried_note,
        }));

        if ok_now {
            write_state("green", &runs_log)?;
            let receipt = meg_tools::ledger::write_receipt(workspace, "meg-loop", true)?;
            if options.base.json_output {
                println!(
                    "{}",
                    serde_json::to_string(&serde_json::json!({
                        "ok": true,
                        "status": "green",
                        "runs": run_idx,
                        "state_path": state_path.display().to_string(),
                        "receipt": receipt,
                    }))?
                );
            } else {
                println!("MEG LOOP GREEN after {run_idx} run(s).");
            }
            return Ok(());
        }

        last_output = out_now;
        recent_fps.push(fp.clone());
        let stalled = recent_fps.len() >= options.max_same
            && recent_fps[recent_fps.len() - options.max_same..]
                .iter()
                .all(|x| x == &fp);
        if stalled {
            write_state("stalled", &runs_log)?;
            if options.base.json_output {
                println!(
                    "{}",
                    serde_json::to_string(&serde_json::json!({
                        "ok": false,
                        "status": "stalled",
                        "runs": run_idx,
                        "failure_fingerprint": fp,
                        "state_path": state_path.display().to_string(),
                    }))?
                );
            }
            anyhow::bail!(
                "LOOP_STALLED: identical failure fingerprint ({fp}) repeated {} times",
                options.max_same
            );
        }
        write_state("running", &runs_log)?;
    }

    write_state("budget", &runs_log)?;
    if options.base.json_output {
        println!(
            "{}",
            serde_json::to_string(&serde_json::json!({
                "ok": false,
                "status": "budget",
                "runs": options.max_runs,
                "state_path": state_path.display().to_string(),
            }))?
        );
    }
    anyhow::bail!("LOOP_BUDGET_EXHAUSTED: check did not pass within {} run(s)", options.max_runs)
}

/// Open Bring-Your-Own-TUI NDJSON Bridge (`meg ui-bridge`):
/// Emits structured JSON-line events on stdout and accepts JSON-line commands on stdin
/// so users can build or plug in any custom TUI/GUI (Python, Go, JS/TS, Rust, Web)
/// while keeping MEG's Ring-0 sandbox, verification, and multi-model orchestration intact.
async fn run_ui_bridge(workspace: &std::path::Path) -> Result<()> {
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

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let workspace = std::env::current_dir()?;

    match parse_cli_args(&args) {
        CliCommand::Worker => run_worker().await,
        CliCommand::Tui => {
            launch_tui(&workspace)?;
            Ok(())
        }
        CliCommand::Run { goal } => {
            let (_, options) = parse_run_options(&args)?;
            let workspace = options
                .cwd
                .as_deref()
                .unwrap_or(&workspace)
                .canonicalize()?;
            anyhow::ensure!(workspace.is_dir(), "Workspace must be a directory");
            run_goal_headless(&workspace, &goal, &options).await?;
            Ok(())
        }
        CliCommand::Loop => {
            let options = parse_loop_options(&args)?;
            let workspace = options
                .base
                .cwd
                .as_deref()
                .unwrap_or(&workspace)
                .canonicalize()?;
            anyhow::ensure!(workspace.is_dir(), "Workspace must be a directory");
            run_loop_headless(&workspace, &options).await?;
            Ok(())
        }
        CliCommand::UiBridge => {
            let mut ws = workspace;
            if let Some(dir) = args.iter().position(|a| a == "-C" || a == "--cd").and_then(|pos| args.get(pos + 1)) {
                ws = std::path::PathBuf::from(dir).canonicalize()?;
            }
            anyhow::ensure!(ws.is_dir(), "Workspace must be a directory");
            run_ui_bridge(&ws).await?;
            Ok(())
        }
        CliCommand::Help => {
            print_usage();
            Ok(())
        }
        CliCommand::Version => {
            println!("MEG {} (meg-tui)", meg_tui::APP_VERSION);
            Ok(())
        }
        CliCommand::Unknown(cmd) => {
            eprintln!("Unknown command: {cmd}\n");
            print_usage();
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn builtin_skills_follow_installed_sidecar_and_public_source_layout() {
        let root = std::env::temp_dir().join(format!("meg-cli-skills-{}", uuid::Uuid::new_v4()));
        let repo = root.join("source");
        let source_skills = repo.join("meg-plane/skills");
        std::fs::create_dir_all(&source_skills).unwrap();
        for entry in [
            repo.join("meg-plane/src/index.ts"),
            repo.join("meg-plane/dist/plane.js"),
        ] {
            assert_eq!(builtin_skills_path(&entry, &repo), source_skills);
        }
        // The path selected by MEG_PLANE_ENTRY follows the same sidecar rule.
        let installed_entry = root.join("custom-plane/plane.js");
        let installed_skills = installed_entry.parent().unwrap().join("skills");
        std::fs::create_dir_all(&installed_skills).unwrap();
        assert_eq!(
            builtin_skills_path(&installed_entry, &repo),
            installed_skills
        );
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn run_options_preserve_literal_goals_and_apply_session_options() {
        let args = [
            "meg",
            "run",
            "--headless",
            "-m",
            "qwen",
            "--effort",
            "high",
            "-C",
            "project folder",
            "real task",
        ]
        .map(str::to_string);
        let (goal, options) = parse_run_options(&args).unwrap();
        assert_eq!(goal, "real task");
        assert_eq!(options.model.as_deref(), Some("qwen"));
        assert_eq!(options.effort.as_deref(), Some("high"));
        assert_eq!(
            options.cwd.unwrap(),
            std::path::PathBuf::from("project folder")
        );
        let (goal, _) = parse_run_options(
            &["meg", "run", "--headless", "--fix-critical-defect"].map(str::to_string),
        )
        .unwrap();
        assert_eq!(goal, "--fix-critical-defect");
        let (goal, _) =
            parse_run_options(&["meg", "run", "--", "-m", "literal"].map(str::to_string)).unwrap();
        assert_eq!(goal, "-m literal");
    }
    #[test]
    fn incomplete_run_options_fail_before_any_provider_call() {
        for args in [
            vec!["meg", "run"],
            vec!["meg", "run", "-m"],
            vec!["meg", "run", "--effort", "invalid", "task"],
        ] {
            assert!(
                parse_run_options(&args.into_iter().map(str::to_string).collect::<Vec<_>>())
                    .is_err()
            );
        }
        assert!(matches!(
            parse_cli_args(&["meg", "run", "--help"].map(str::to_string)),
            CliCommand::Help
        ));
    }
    #[test]
    fn tui_options_validate_values_and_keep_workspace_paths() {
        let args = [
            "meg",
            "tui",
            "-C",
            "C:\\project with spaces",
            "--resume",
            "latest",
            "-m",
            "fixture",
            "--effort",
            "high",
        ]
        .map(str::to_string);
        let options = parse_tui_options(&args).unwrap();
        assert_eq!(
            options.cwd.unwrap(),
            std::path::PathBuf::from("C:\\project with spaces")
        );
        assert_eq!(options.model.as_deref(), Some("fixture"));
        assert_eq!(options.resume.as_deref(), Some("latest"));
        assert!(parse_tui_options(&["meg", "--effort", "nonsense"].map(str::to_string)).is_err());
        assert!(parse_tui_options(&["meg", "--model"].map(str::to_string)).is_err());
        assert!(parse_tui_options(&["meg", "tui", "--unknown"].map(str::to_string)).is_err());
    }

    #[test]
    fn test_parse_cli_args_bare_invocation_defaults_to_tui() {
        let args = vec!["meg".to_string()];
        assert!(matches!(parse_cli_args(&args), CliCommand::Tui));

        let empty: Vec<String> = vec![];
        assert!(matches!(parse_cli_args(&empty), CliCommand::Tui));
    }

    #[test]
    fn test_parse_cli_args_explicit_tui() {
        let args = vec!["meg".to_string(), "tui".to_string()];
        assert!(matches!(parse_cli_args(&args), CliCommand::Tui));
    }

    #[test]
    fn test_parse_cli_args_run_goal() {
        let args = vec![
            "meg".to_string(),
            "run".to_string(),
            "build landing page".to_string(),
        ];
        match parse_cli_args(&args) {
            CliCommand::Run { goal } => assert_eq!(goal, "build landing page"),
            _ => panic!("expected Run"),
        }
    }

    #[test]
    fn test_parse_cli_args_help_flags() {
        let short = vec!["meg".to_string(), "-h".to_string()];
        assert!(matches!(parse_cli_args(&short), CliCommand::Help));

        let long = vec!["meg".to_string(), "--help".to_string()];
        assert!(matches!(parse_cli_args(&long), CliCommand::Help));
    }

    #[test]
    fn run_options_parse_pinned_model_max_steps_and_json() {
        let args: Vec<String> = [
            "meg",
            "run",
            "--pinned-model",
            "offline-eval-v1",
            "--max-steps",
            "12",
            "--json",
            "--",
            "fix calc.py",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let (goal, opts) = parse_run_options(&args).unwrap();
        assert_eq!(goal, "fix calc.py");
        assert_eq!(opts.pinned_model.as_deref(), Some("offline-eval-v1"));
        assert_eq!(opts.model.as_deref(), Some("offline-eval-v1"));
        assert_eq!(opts.max_steps, Some(12));
        assert!(opts.json_output);

        let bad_steps: Vec<String> = ["meg", "run", "--max-steps", "0", "hi"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert!(parse_run_options(&bad_steps).is_err());
    }

    #[test]
    fn adversarial_l3_reviewer_rejects_negative_prose_and_plan_allows_glob_grep() {
        let (v1, _) = parse_l3_reviewer_verdict("This does not pass review due to missing tests.");
        assert_eq!(v1, "REJECT");
        let (v2, _) = parse_l3_reviewer_verdict("FAIL: tests did not pass");
        assert_eq!(v2, "REJECT");
        let (v3, _) = parse_l3_reviewer_verdict(r#"{"verdict":"PASS","reason":"clean"}"#);
        assert_eq!(v3, "PASS");
        let (v4, _) = parse_l3_reviewer_verdict("VERDICT: PASS\nAll checks verified.");
        assert_eq!(v4, "PASS");

        let dir = std::env::temp_dir().join(format!("meg-plan-coresix-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let reg = meg_tools::ToolRegistry::with_core_six(&dir);
        let specs = build_tool_specs(&reg, meg_tui::Mode::Plan, &serde_json::json!({}), false);
        assert!(
            specs.iter().any(|s| s.name == "glob_grep"),
            "Plan mode must keep glob_grep when --core-six is enabled"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
    #[test]
    fn adversarial_verification_classifier_rejects_decoys() {
        assert!(!is_verification_command("echo test passed"));
        assert!(!is_verification_command("python -c \"open('a.txt','w').write('ok')\""));
        assert!(!is_verification_command("node -e \"console.log('check')\""));
        assert!(is_verification_command("cargo test --locked"));
        assert!(is_verification_command("pytest -q"));
        assert!(is_verification_command("bun test"));
        assert!(is_verification_command("go test ./..."));
    }
    #[test]
    fn adversarial_mcp_names_and_bridge_mode_fail_closed() {
        let safe = sanitize_mcp_tool_name("mcp:github.server:create/issue");
        assert!(!safe.contains(':'));
        assert!(safe.len() <= 64);
        assert!(safe.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'));

        let long_safe = sanitize_mcp_tool_name(&format!("mcp:server:{}", "x".repeat(120)));
        assert!(long_safe.len() <= 64);
        assert!(long_safe.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'));

        assert_eq!(parse_bridge_mode("plan").unwrap(), meg_tui::Mode::Plan);
        assert_eq!(parse_bridge_mode("build").unwrap(), meg_tui::Mode::Build);
        assert_eq!(parse_bridge_mode("act").unwrap(), meg_tui::Mode::Build);
        assert!(parse_bridge_mode("buld").is_err(), "Typo mode must fail closed");
        assert!(parse_bridge_mode("").is_err(), "Empty mode must fail closed");
    }
    #[test]
    fn test_parse_cli_args_unknown_command() {
        let unknown = vec!["meg".to_string(), "random-cmd".to_string()];
        assert!(matches!(parse_cli_args(&unknown), CliCommand::Unknown(_)));
    }
}

#[cfg(test)]
mod native_integration {
    use super::*;
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
}
