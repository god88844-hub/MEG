//! Advertised commands are exercised through the production dispatcher.
//! HTTP, MCP transports and native workers have separate protocol/E2E checks.
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use meg_tui::commands::COMMANDS;
use meg_tui::{Controller, GoalRunner, RunnerMsg, ServiceRunner, WorkstationState};
use std::{
    collections::HashSet,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

fn settle(c: &mut Controller) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        c.poll(0.01).unwrap();
        if !c.state.busy && !c.state.background_tasks.iter().any(|t| t.active) {
            break;
        }
        assert!(Instant::now() < deadline, "command did not settle");
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn escape(c: &mut Controller) {
    c.handle_event(Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)))
        .unwrap();
}
fn run(c: &mut Controller, seen: &mut HashSet<String>, line: &str) {
    seen.insert(line.split_whitespace().next().unwrap().into());
    c.submit_line(line)
        .unwrap_or_else(|e| panic!("{line}: {e}"));
    settle(c);
}

#[test]
fn every_advertised_command_reaches_its_implementation_and_local_commands_stay_local() {
    let root = std::env::temp_dir().join(format!(
        "meg-production-test-command-matrix-{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let goals = Arc::new(Mutex::new(Vec::new()));
    let captured = goals.clone();
    let runner: Arc<GoalRunner> = Arc::new(move |request, tx| {
        captured.lock().unwrap().push(request.goal);
        tx.send(RunnerMsg::StreamStart).unwrap();
        tx.send(RunnerMsg::Usage {
            input: 11,
            output: 4,
            cached: 2,
        })
        .unwrap();
        Ok(Some("Contract answer with evidence".into()))
    });
    let catalog = serde_json::json!({"skills":[{"name":"fixture","description":"Fixture workflow","body":"Follow fixture evidence"}],"commands":[],"tools":[],"plugins":[],"servers":[]});
    let services: Arc<ServiceRunner> = Arc::new(move |request, _, tx| {
        Ok(match request.method.as_str() {
            "provider.models" => {
                serde_json::json!({"provider_id":request.params["id"],"models":["fixture"],"message":"Fixture connected"})
            }
            "runtime.catalog" | "plugins.command" | "mcp.command" => {
                serde_json::json!({"catalog":catalog,"message":"Fixture catalog operation completed"})
            }
            "agents.run" => {
                tx.send(RunnerMsg::SubagentState(
                    "contract-worker".into(),
                    "completed".into(),
                ))
                .unwrap();
                serde_json::json!({"answer":"Delegate evidence","role":"researcher","usage":{"input_tokens":3,"output_tokens":2,"cached_tokens":0,"requests":1,"reported_requests":1}})
            }
            method => panic!("Unexpected service: {method}"),
        })
    });
    let mut profile = meg_core::providers::ProviderProfile::environment();
    profile.id = uuid::Uuid::new_v4().to_string();
    profile.name = "Fixture".into();
    profile.default_model = "fixture".into();
    profile.key_env = None;
    let provider_root = root.join("private-config");
    meg_core::providers::ProviderStore {
        root: provider_root.clone(),
    }
    .upsert(profile.clone(), None)
    .unwrap();
    let state = WorkstationState {
        cwd: root.to_string_lossy().into_owned(),
        model: "fixture".into(),
        provider: Some(profile.clone()),
        providers: vec![profile],
        provider_root: Some(provider_root),
        available_models: vec!["fixture".into()],
        ..Default::default()
    };
    let mut c = Controller::new(state, Some(runner))
        .unwrap()
        .with_services(Some(services));
    let mut seen = HashSet::new();
    for command in [
        "/help",
        "/stats",
        "/tools",
        "/config",
        "/permissions readonly",
        "/permissions act",
        "/status",
        "/doctor",
        "/context",
        "/memory add command-matrix-evidence",
        "/memory search evidence",
        "/todo add Check dispatch",
        "/todo done 1",
        "/todo clear",
        "/queue",
        "/queue clear",
        "/stop",
        "/theme Dashboard",
        "/effort high",
        "/model fixture",
    ] {
        run(&mut c, &mut seen, command);
    }
    assert_eq!(
        goals.lock().unwrap().len(),
        0,
        "local commands must not invoke the model"
    );
    assert!(root.join(".meg/tui/notes.json").is_file());
    assert!(c.state.plan_steps.is_empty());
    for command in [
        "/model",
        "/effort",
        "/theme",
        "/provider",
        "/provider add openrouter",
        "/workspace",
        "/sessions",
    ] {
        run(&mut c, &mut seen, command);
        escape(&mut c);
    }
    run(&mut c, &mut seen, "/skills refresh");
    let deadline = Instant::now() + Duration::from_secs(5);
    while c.state.extensions["skills"].as_array().is_none() {
        c.poll(0.01).unwrap();
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(2));
    }
    for command in [
        "/skills",
        "/plugins list",
        "/mcp list",
        "/agent researcher inspect fixture",
    ] {
        run(&mut c, &mut seen, command);
    }
    assert_eq!(
        goals.lock().unwrap().len(),
        0,
        "extension administration must remain local"
    );
    for command in [
        "/init",
        "/review inspect fixture",
        "/verify fixture",
        "/code fixture",
        "/plan fixture",
        "/skills fixture inspect",
    ] {
        run(&mut c, &mut seen, command);
    }
    assert_eq!(goals.lock().unwrap().len(), 6);
    assert!(goals.lock().unwrap()[0].contains("AGENTS.md"));
    assert_eq!(c.state.mode, meg_tui::Mode::Plan);
    run(&mut c, &mut seen, "/compact");
    assert_eq!(c.state.conversation.len(), 9);
    assert!(std::fs::read_dir(root.join(".meg/tui")).unwrap().any(|e| e
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with("context-")));
    let initial = c.state.session_id.clone();
    run(&mut c, &mut seen, "/fork");
    assert_ne!(initial, c.state.session_id);
    run(&mut c, &mut seen, &format!("/resume {initial}"));
    assert_eq!(c.state.session_id, initial);
    run(&mut c, &mut seen, "/export evidence.txt");
    assert!(std::fs::read_to_string(root.join("evidence.txt"))
        .unwrap()
        .contains("Contract answer"));
    let target = root.join("undo-fixture.txt");
    std::fs::write(&target, "before").unwrap();
    let checkpoint = meg_tools::journal::FileCheckpoint::capture(
        &root,
        &serde_json::json!({"path":"undo-fixture.txt"}),
    )
    .unwrap();
    std::fs::write(&target, "after").unwrap();
    checkpoint.commit(&root).unwrap();
    run(&mut c, &mut seen, "/undo");
    assert_eq!(std::fs::read_to_string(target).unwrap(), "before");
    assert!(std::process::Command::new("git")
        .args(["init", "--quiet"])
        .arg(&root)
        .status()
        .unwrap()
        .success());
    run(&mut c, &mut seen, "/diff");
    let messages = c.state.conversation.len();
    run(&mut c, &mut seen, "/clear");
    assert!(c.state.transcript.is_empty());
    assert_eq!(c.state.conversation.len(), messages);
    run(&mut c, &mut seen, "/new");
    assert!(c.state.conversation.is_empty());
    // No assistant means /copy must fail locally without replacing the OS clipboard.
    seen.insert("/copy".into());
    assert!(c
        .submit_line("/copy")
        .unwrap_err()
        .to_string()
        .contains("No assistant reply"));
    assert!(!c.state.busy);
    run(&mut c, &mut seen, "/settings");
    assert!(c.state.settings_picker.is_some());
    escape(&mut c);
    run(&mut c, &mut seen, "/exit");
    assert!(c.state.should_quit);
    let missing: Vec<_> = COMMANDS
        .iter()
        .filter(|s| !seen.contains(s.name))
        .map(|s| s.name)
        .collect();
    assert!(
        missing.is_empty(),
        "Advertised commands without dispatch coverage: {missing:?}"
    );
    println!(
        "Command coverage: {}/{} advertised commands",
        seen.len(),
        COMMANDS.len()
    );
}
