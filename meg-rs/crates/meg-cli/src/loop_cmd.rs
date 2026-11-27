use anyhow::Result;
use meg_tui::WorkstationState;
use crate::cli::LoopOptions;
use crate::runner::{RunExtras, run_request_detailed, start_plane};

/// Compute a deterministic 8-hex-char fingerprint of the last 20 lines of failing output
/// after masking digits so identical bugs produce the same hash.
pub(crate) fn mask_failure_fingerprint(output: &str) -> String {
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


pub(crate) fn run_external_check_cmd(workspace: &std::path::Path, cmd: &str) -> (bool, String) {
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
pub(crate) async fn run_loop_headless(workspace: &std::path::Path, options: &LoopOptions) -> Result<()> {
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
        let (canonical, exit_code) = match status {
            "green" => ("GREEN", crate::runner::EXIT_CODE_GREEN),
            "stalled" => ("STALL", crate::runner::EXIT_CODE_STALL),
            "budget" => ("BUDGET", crate::runner::EXIT_CODE_BUDGET),
            "check_cannot_fail" => ("CHECK_CANNOT_FAIL", crate::runner::EXIT_CODE_CHECK_CANNOT_FAIL),
            _ => ("RUNNING", crate::runner::EXIT_CODE_GREEN),
        };
        let _ = crate::runner::write_meg_state(
            workspace,
            canonical,
            exit_code,
            serde_json::json!({
                "mode": "loop",
                "goal": options.goal,
                "check_cmd": options.check_cmd,
                "runs": runs.len(),
                "loop_state_path": state_path.display().to_string(),
            }),
        );
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
    if options.base.delegent {
        std::env::set_var("MEG_DELEGENT", "1");
    }
    if let Some(spec) = &options.base.subagent_model {
        std::env::set_var("MEG_SUBAGENT_MODEL", spec);
    }
    let plane = if std::env::var("MEG_NO_PLANE").as_deref() == Ok("1")
        || std::env::var("MEG_PLANE_DISABLED").as_deref() == Ok("1")
    {
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

