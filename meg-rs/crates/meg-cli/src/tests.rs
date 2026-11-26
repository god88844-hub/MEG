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

#[test]
fn start_plane_defaults_to_pure_rust_native_services_without_bun() {
    let plane = start_plane().expect("native services must initialize without Bun");
    assert!(plane.is_native(), "default plane must be pure-Rust NativeServices");
    let cancel = meg_core::CancellationToken::default();
    let ws = std::env::temp_dir().join(format!("meg-native-default-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&ws).unwrap();
    let cat = plane
        .call(
            "runtime.catalog",
            serde_json::json!({"workspace": ws.display().to_string()}),
            &cancel,
        )
        .unwrap();
    let skills = cat["catalog"]["skills"].as_array().unwrap();
    assert!(
        skills.iter().any(|s| s["name"] == "tdd-discipline"),
        "embedded builtin skills must be available in pure-Rust mode"
    );
    let _ = std::fs::remove_dir_all(&ws);
}

#[test]
fn nested_agents_and_claude_guidance_discovered_in_order() {
    let ws = std::env::temp_dir().join(format!("meg-b05-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(ws.join("sub/pkg")).unwrap();
    std::fs::create_dir_all(ws.join("node_modules/ignored")).unwrap();
    std::fs::write(ws.join("AGENTS.md"), "ROOT_RULE").unwrap();
    std::fs::write(ws.join("sub/CLAUDE.md"), "SUB_CLAUDE_RULE").unwrap();
    std::fs::write(ws.join("sub/pkg/AGENTS.md"), "PKG_RULE").unwrap();
    std::fs::write(ws.join("node_modules/ignored/AGENTS.md"), "SHOULD_IGNORE").unwrap();

    let combined = crate::engine::collect_workspace_guidance(&ws);
    assert!(combined.contains("ROOT_RULE"), "{combined}");
    assert!(combined.contains("SUB_CLAUDE_RULE"), "{combined}");
    assert!(combined.contains("PKG_RULE"), "{combined}");
    assert!(!combined.contains("SHOULD_IGNORE"), "{combined}");
    assert!(combined.find("ROOT_RULE").unwrap() < combined.find("PKG_RULE").unwrap());
    let _ = std::fs::remove_dir_all(&ws);
}

#[test]
fn deferred_mcp_loading_withholds_large_catalogs_and_activates_on_demand() {
    let dir = std::env::temp_dir().join(format!("meg-defer-mcp-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let reg = meg_tools::ToolRegistry::with_core_six(&dir);

    let mut tools = Vec::new();
    for i in 0..10 {
        tools.push(serde_json::json!({
            "alias": format!("mcp:github:tool_{i}"),
            "description": format!("GitHub helper action {i}"),
            "readOnly": i % 2 == 0,
            "inputSchema": {
                "type": "object",
                "properties": { "repo": { "type": "string" } },
                "required": ["repo"]
            }
        }));
    }
    let catalog = serde_json::json!({ "tools": tools });

    // Above threshold (10 > 6): full MCP schemas withheld, single load_mcp_tool exposed
    let deferred_specs = crate::engine::build_tool_specs_with_threshold(
        &reg,
        meg_tui::Mode::default(),
        &catalog,
        true,
        6,
    );
    assert!(
        deferred_specs.iter().any(|s| s.name == "load_mcp_tool"),
        "expected load_mcp_tool when MCP tools exceed threshold"
    );
    assert!(
        !deferred_specs.iter().any(|s| s.name.starts_with("mcp__")),
        "deferred mode must not dump raw MCP schemas up front"
    );

    // At or below threshold (10 <= 10): eager loading includes all 10 MCP tools, no loader
    let eager_specs = crate::engine::build_tool_specs_with_threshold(
        &reg,
        meg_tui::Mode::default(),
        &catalog,
        true,
        10,
    );
    assert!(!eager_specs.iter().any(|s| s.name == "load_mcp_tool"));
    assert_eq!(
        eager_specs.iter().filter(|s| s.name.starts_with("mcp__")).count(),
        10
    );

    // On-demand query loads matching tools and ShimClient deduplicates dynamic registration
    let (out, specs) = crate::engine::resolve_deferred_mcp_tools(
        &catalog,
        meg_tui::Mode::default(),
        &serde_json::json!({ "names": ["mcp:github:tool_2", "mcp__github__tool_3"] }),
    )
    .unwrap();
    assert_eq!(specs.len(), 2, "{out}");
    let mut shim = meg_core::shim_client::ShimClient::with("http://127.0.0.1:1".into(), "m".into());
    shim.tools = deferred_specs;
    assert!(shim.register_dynamic_tool(specs[0].clone()));
    assert!(!shim.register_dynamic_tool(specs[0].clone()), "duplicate load must be ignored");
    assert!(shim.effective_tools().iter().any(|t| t.name == "mcp__github__tool_2"));

    // Plan mode blocks mutating MCP tools (odd indices have readOnly=false)
    let (plan_out, plan_specs) = crate::engine::resolve_deferred_mcp_tools(
        &catalog,
        meg_tui::Mode::Plan,
        &serde_json::json!({ "names": ["mcp:github:tool_1", "mcp:github:tool_2"] }),
    )
    .unwrap();
    assert_eq!(plan_specs.len(), 1, "{plan_out}");
    assert_eq!(plan_specs[0].name, "mcp__github__tool_2");
    assert!(plan_out.contains("mcp:github:tool_1"), "expected blocked_in_plan notice: {plan_out}");

    let _ = std::fs::remove_dir_all(&dir);
}
