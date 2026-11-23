use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use meg_tui::{Controller, GoalRunner, RunnerMsg, WorkstationState};
use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

fn temp_workspace() -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!("meg-production-test-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&path).unwrap();
    path
}
fn state(path: &std::path::Path) -> WorkstationState {
    WorkstationState {
        cwd: path.to_string_lossy().into_owned(),
        available_models: vec!["fixture".into()],
        model: "fixture".into(),
        ..Default::default()
    }
}
fn drain(controller: &mut Controller) {
    let start = Instant::now();
    while controller.state.busy {
        controller.poll(0.01).unwrap();
        assert!(
            start.elapsed() < Duration::from_secs(3),
            "Run did not settle"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}
fn key(c: KeyCode) -> Event {
    Event::Key(KeyEvent::new(c, KeyModifiers::NONE))
}

#[test]
fn session_and_unknown_commands_never_invoke_the_agent() {
    let path = temp_workspace();
    let called = Arc::new(Mutex::new(0));
    let count = called.clone();
    let runner: Arc<GoalRunner> = Arc::new(move |_, _| {
        *count.lock().unwrap() += 1;
        Ok(None)
    });
    let mut c = Controller::new(state(&path), Some(runner)).unwrap();
    c.submit_line("/sessions").unwrap();
    assert!(c.state.session_picker.is_some());
    assert!(c.submit_line("/not-a-command").is_err());
    assert!(!c.state.busy);
    assert_eq!(*called.lock().unwrap(), 0);
}
#[test]
fn session_resume_restores_real_conversation_and_next_turn_receives_it() {
    let path = temp_workspace();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let history = seen.clone();
    let runner: Arc<GoalRunner> = Arc::new(move |request, _| {
        history.lock().unwrap().push(request.history.len());
        Ok(Some("remembered reply".into()))
    });
    let mut c = Controller::new(state(&path), Some(runner.clone())).unwrap();
    c.submit_line("remember cobalt").unwrap();
    drain(&mut c);
    let id = c.state.session_id.clone();
    c.submit_line("/new").unwrap();
    assert!(c.state.conversation.is_empty());
    c.submit_line(&format!("/resume {id}")).unwrap();
    assert_eq!(c.state.conversation.len(), 2);
    c.submit_line("what did I say?").unwrap();
    drain(&mut c);
    assert_eq!(*seen.lock().unwrap(), vec![0, 2]);
    drop(c);
    let mut reopened = Controller::new(state(&path), Some(runner)).unwrap();
    reopened.submit_line(&format!("/resume {id}")).unwrap();
    assert_eq!(reopened.state.conversation.len(), 4);
}
#[test]
fn cancel_reaches_worker_and_terminal_state_settles() {
    let path = temp_workspace();
    let runner: Arc<GoalRunner> = Arc::new(|request, _| {
        while !request.cancel.is_cancelled() {
            std::thread::sleep(Duration::from_millis(5));
        }
        anyhow::bail!("Run cancelled")
    });
    let mut c = Controller::in_memory(state(&path), Some(runner));
    c.submit_line("wait").unwrap();
    c.submit_line("/stop").unwrap();
    drain(&mut c);
    assert!(!c.state.busy);
    assert!(c.state.stream.as_ref().unwrap().is_complete);
    assert!(c
        .state
        .transcript
        .iter()
        .any(|s| s.contains("Run cancelled")));
    assert_eq!(c.state.usage.output_tokens, 0);
    assert_eq!(c.state.usage.reported_requests, 0);
}
#[test]
fn local_commands_remain_local_during_running_work_and_followups_queue() {
    let path = temp_workspace();
    let runner: Arc<GoalRunner> = Arc::new(|request, _| {
        while !request.cancel.is_cancelled() {
            std::thread::sleep(Duration::from_millis(5));
        }
        Ok(None)
    });
    let mut c = Controller::in_memory(state(&path), Some(runner));
    c.submit_line("wait").unwrap();
    c.submit_line("/status").unwrap();
    assert!(c.state.busy);
    c.submit_line("follow-up").unwrap();
    assert_eq!(c.state.queued_count, 1);
    assert!(c.submit_line("/new").is_err());
    assert_eq!(c.state.input, "/new");
    c.submit_line("/stop").unwrap();
    drain(&mut c);
    assert_eq!(c.state.queued_count, 0);
}
#[test]
fn paste_is_editable_and_never_submitted_automatically() {
    let path = temp_workspace();
    let mut c = Controller::in_memory(state(&path), None);
    c.handle_event(Event::Paste("hello\r\n世界".into()))
        .unwrap();
    assert_eq!(c.state.input, "hello\n世界");
    assert!(!c.state.busy);
    c.handle_event(key(KeyCode::Backspace)).unwrap();
    assert_eq!(c.state.input, "hello\n世");
}
#[test]
fn picker_search_accepts_j_and_k_as_text() {
    let path = temp_workspace();
    let mut c = Controller::in_memory(state(&path), None);
    c.submit_line("/theme").unwrap();
    c.handle_event(key(KeyCode::Char('j'))).unwrap();
    if let meg_tui::ActiveModal::ThemePicker(p) = &c.state.active_modal {
        assert_eq!(p.search_query, "j");
    } else {
        panic!("Theme picker closed")
    }
}
#[test]
fn failed_tools_remain_failed_and_chunks_are_not_counted_as_tokens() {
    let mut c = Controller::in_memory(WorkstationState::default(), None);
    c.apply(RunnerMsg::StreamStart);
    for _ in 0..5 {
        c.apply(RunnerMsg::ContentChunk("word ".into()));
    }
    assert_eq!(c.state.stream.as_ref().unwrap().token_count, 0);
    c.apply(RunnerMsg::ToolStarted {
        id: "1".into(),
        name: "bash".into(),
        args: "exit 7".into(),
    });
    c.apply(RunnerMsg::ToolFinished {
        id: "1".into(),
        ok: false,
        output: "exit code: 7".into(),
    });
    assert!(c.state.background_tasks[0].failed);
    assert!(!c.state.background_tasks[0].active);
    c.apply(RunnerMsg::Usage {
        input: 45,
        output: 12,
        cached: 10,
    });
    assert_eq!(c.state.usage.output_tokens, 12);
}
#[test]
fn slash_arguments_use_same_editor_and_registry() {
    let path = temp_workspace();
    let mut c = Controller::in_memory(state(&path), None);
    for ch in "/theme Dashboard".chars() {
        c.handle_event(key(KeyCode::Char(ch))).unwrap();
    }
    c.handle_event(key(KeyCode::Enter)).unwrap();
    assert_eq!(c.state.theme, meg_tui::DASHBOARD);
    assert!(c.state.input.is_empty());
}

#[test]
fn completion_preserves_arguments_and_command_menu_preserves_draft() {
    let mut c = Controller::in_memory(WorkstationState::default(), None);
    for ch in "/theme Dashboard".chars() {
        c.handle_event(key(KeyCode::Char(ch))).unwrap();
    }
    c.handle_event(key(KeyCode::Tab)).unwrap();
    assert_eq!(c.state.input, "/theme Dashboard");
    c.state.input = "unfinished draft".into();
    c.state.move_cursor_end();
    c.handle_event(Event::Key(KeyEvent::new(
        KeyCode::Char('p'),
        KeyModifiers::CONTROL,
    )))
    .unwrap();
    c.handle_event(key(KeyCode::Tab)).unwrap();
    assert_eq!(c.state.input, "unfinished draft");
}
#[test]
fn error_pauses_followups_until_explicit_queue_run() {
    let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let calls = count.clone();
    let runner: Arc<GoalRunner> = Arc::new(move |_, _| {
        if calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
            anyhow::bail!("fixture failure")
        }
        Ok(Some("Recovered".into()))
    });
    let mut c = Controller::in_memory(WorkstationState::default(), Some(runner));
    c.submit_line("first").unwrap();
    c.submit_line("second").unwrap();
    drain(&mut c);
    assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert_eq!(c.state.queued_count, 1);
    c.submit_line("/queue run").unwrap();
    drain(&mut c);
    assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 2);
    assert_eq!(c.state.queued_count, 0);
}
#[test]
fn startup_resume_and_overrides_restore_the_requested_session() {
    let path = temp_workspace();
    let mut c = Controller::new(state(&path), None).unwrap();
    c.state.transcript.push("saved marker".into());
    c.submit_line("/todo add saved task").unwrap();
    let id = c.state.session_id.clone();
    drop(c);
    let mut st = state(&path);
    st.startup = meg_tui::StartupOptions {
        resume: Some(id.clone()),
        model: Some("explicit-model".into()),
        effort: Some("high".into()),
    };
    let reopened = Controller::new(st, None).unwrap();
    assert_eq!(reopened.state.session_id, id);
    assert_eq!(reopened.state.model, "explicit-model");
    assert_eq!(reopened.state.effort, "high");
    assert_eq!(reopened.state.plan_steps.len(), 1);
}
#[test]
fn session_store_rejects_invalid_ids_and_corrupt_records() {
    let path = temp_workspace();
    let store = meg_tui::session::SessionStore::open(&path).unwrap();
    assert!(store.load("../../elsewhere").is_err());
    std::fs::write(
        store
            .root
            .join("sessions")
            .join(format!("{}.json", uuid::Uuid::new_v4())),
        "{ broken",
    )
    .unwrap();
    assert!(store.list().unwrap().is_empty());
}
#[test]
fn effort_uses_only_advertised_sibling_models() {
    let mut st = WorkstationState {
        model: "fixture-medium".into(),
        available_models: vec!["fixture-medium".into(), "fixture-high".into()],
        ..Default::default()
    };
    st.select_effort("high".into());
    assert_eq!(st.model, "fixture-high");
    st.select_effort("max".into());
    assert_eq!(st.model, "fixture-high");
    assert_eq!(st.effort, "max");
}
#[test]
fn lone_carriage_returns_in_paste_become_newlines() {
    let mut c = Controller::in_memory(WorkstationState::default(), None);
    c.handle_event(Event::Paste("a\rb\u{7}".into())).unwrap();
    assert_eq!(c.state.input, "a\nb");
}

#[test]
fn startup_path_paste_survives_multiple_console_bursts_without_creating_state_early() {
    let root = temp_workspace();
    let selected = root.join("selected workspace");
    std::fs::create_dir(&selected).unwrap();
    let mut st = state(&root);
    st.workspace_picker = Some(meg_tui::WorkspacePicker::open_startup(root.clone()));
    let mut controller = Controller::new(st, None).unwrap();
    assert!(!root.join(".meg").exists());
    let path = selected.to_string_lossy();
    let midpoint = path.len() / 2;
    controller
        .handle_event(Event::Paste(path[..midpoint].into()))
        .unwrap();
    controller
        .handle_event(Event::Paste(path[midpoint..].into()))
        .unwrap();
    assert_eq!(
        controller.state.workspace_picker.as_ref().unwrap().query,
        path
    );
    controller.handle_event(key(KeyCode::Enter)).unwrap();
    assert!(controller.state.workspace_picker.is_none());
    assert!(selected.join(".meg/tui").exists());
}

#[test]
fn workspace_preferences_restore_provider_and_model_together() {
    let root = temp_workspace();
    let mut st = state(&root);
    let mut profile = meg_core::providers::ProviderProfile::environment();
    profile.id = uuid::Uuid::new_v4().to_string();
    st.model = "cloud-model".into();
    st.provider = Some(profile.clone());
    st.providers = vec![profile.clone()];
    let store = meg_tui::session::SessionStore::open(&root).unwrap();
    store.save_preferences(&st).unwrap();
    st.provider = None;
    st.model = "wrong-model".into();
    store.restore_preferences(&mut st).unwrap();
    assert_eq!(st.provider.as_ref().unwrap().id, profile.id);
    assert_eq!(st.model, "cloud-model");
    st.providers.clear();
    store.restore_preferences(&mut st).unwrap();
    assert!(st.provider.is_none());
    assert!(st.model.is_empty());
}

#[test]
fn retired_provider_preferences_do_not_override_the_current_active_cloud_connection() {
    let root = temp_workspace();
    let store = meg_tui::session::SessionStore::open(&root).unwrap();
    std::fs::write(
        store.root.join("preferences.json"),
        r#"{"provider_id":"local-shim","model":"retired-model","theme":"Dashboard"}"#,
    )
    .unwrap();
    let mut active = meg_core::providers::ProviderProfile::environment();
    active.id = "active-cloud".into();
    let mut st = state(&root);
    st.model = "current-cloud-model".into();
    st.provider = Some(active.clone());
    st.providers = vec![active];
    store.restore_preferences(&mut st).unwrap();
    assert_eq!(st.provider.unwrap().id, "active-cloud");
    assert_eq!(st.model, "current-cloud-model");
}
