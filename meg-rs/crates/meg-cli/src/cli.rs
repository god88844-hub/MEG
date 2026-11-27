use anyhow::Context;
use anyhow::Result;

pub(crate) fn parse_cli_args(args: &[String]) -> CliCommand {
    if args.get(1).is_some_and(|s| s == "tui")
        && args.iter().skip(2).any(|s| s == "--help" || s == "-h")
    {
        return CliCommand::Help;
    }
    match args.get(1).map(|s| s.as_str()) {
        None | Some("tui") => CliCommand::Tui,
        Some(
            "-C" | "--cd" | "-m" | "--model" | "--effort" | "--resume" | "--continue"
            | "--no-picker" | "--workspace-picker" | "--delegent" | "--subagent-model",
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


pub(crate) enum CliCommand {
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
pub(crate) struct TuiOptions {
    pub(crate) cwd: Option<std::path::PathBuf>,
    pub(crate) model: Option<String>,
    pub(crate) pinned_model: Option<String>,
    pub(crate) effort: Option<String>,
    pub(crate) resume: Option<String>,
    pub(crate) picker: Option<bool>,
    pub(crate) max_steps: Option<usize>,
    pub(crate) json_output: bool,
    pub(crate) core_six: bool,
    pub(crate) pair_mode: Option<String>,
    pub(crate) executor_model: Option<String>,
    pub(crate) verify_cmd: Option<String>,
    pub(crate) require_verify: bool,
    pub(crate) reviewer_model: Option<String>,
    pub(crate) subagent_model: Option<String>,
    pub(crate) delegent: bool,
    pub(crate) ui_mode: Option<String>,
}


pub(crate) fn parse_tui_options(args: &[String]) -> Result<TuiOptions> {
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
            "--delegent" => options.delegent = true,
            "-C" | "--cd" | "-m" | "--model" | "--effort" | "--resume" | "--ui" | "--pair-mode"
            | "--executor-model" | "--reviewer-model" | "--subagent-model" => {
                i += 1;
                let value = args
                    .get(i)
                    .ok_or_else(|| anyhow::anyhow!("{flag} requires a value"))?
                    .clone();
                match flag {
                    "-C" | "--cd" => options.cwd = Some(value.into()),
                    "-m" | "--model" => options.model = Some(value),
                    "--ui" => options.ui_mode = Some(value),
                    "--pair-mode" => options.pair_mode = Some(value),
                    "--executor-model" => options.executor_model = Some(value),
                    "--reviewer-model" => options.reviewer_model = Some(value),
                    "--subagent-model" => options.subagent_model = Some(value),
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
pub(crate) fn parse_run_options(args: &[String]) -> Result<(String, TuiOptions)> {
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
            "--delegent" => {
                options.delegent = true;
                i += 1;
            }
            "-C" | "--cd" | "-m" | "--model" | "--pinned-model" | "--effort" | "--max-steps" | "--pair-mode" | "--executor-model" | "--verify-cmd" | "--reviewer-model" | "--subagent-model" => {
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
                    "--subagent-model" => options.subagent_model = Some(value.clone()),
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


pub(crate) struct LoopOptions {
    pub(crate) goal: String,
    pub(crate) check_cmd: String,
    pub(crate) max_runs: usize,
    pub(crate) max_same: usize,
    pub(crate) base: TuiOptions,
}


pub(crate) fn parse_loop_options(args: &[String]) -> Result<LoopOptions> {
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
            "--delegent" => {
                base.delegent = true;
                i += 1;
            }
            "-C" | "--cd" | "-m" | "--model" | "--pinned-model" | "--effort" | "--max-steps"
            | "--pair-mode" | "--executor-model" | "--reviewer-model" | "--subagent-model"
            | "--check" | "--max-runs" | "--max-same" => {
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
                    "--subagent-model" => base.subagent_model = Some(value.clone()),
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


pub(crate) fn print_usage() {
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

