use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use meg_core::providers::{ProviderProfile, Secret};
use meg_tui::picker::ActiveModal;
use meg_tui::provider::{ProviderAction, ProviderDialog};
use meg_tui::workspace::{PickPhase, WorkspacePicker};
use meg_tui::{Controller, RunnerMsg, ServiceRunner, WorkstationState};
use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}
fn temp() -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "meg-production-test-interface-{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir(&root).unwrap();
    root
}
fn cleanup(root: PathBuf) {
    assert_eq!(root.parent().unwrap(), std::env::temp_dir());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn any_folder_can_be_opened_and_new_folder_creation_is_explicit() {
    let root = temp();
    let plain = root.join("plain folder");
    std::fs::create_dir(&plain).unwrap();
    let mut picker = WorkspacePicker::open_startup(root.clone());
    assert_eq!(picker.phase, PickPhase::Browse);
    assert!(picker.dirs.contains(&plain));
    picker.selected = picker.dirs.iter().position(|p| p == &plain).unwrap();
    assert_eq!(picker.open_selected().unwrap(), plain);
    picker.begin_create();
    picker.paste("new project");
    assert!(!root.join("new project").exists());
    let created = picker.open_selected().unwrap();
    assert_eq!(created, root.join("new project"));
    assert!(created.is_dir());
    picker.query = "new project".into();
    assert!(picker.open_selected().is_none());
    assert!(picker.error.is_some());
    picker.query = "../escape".into();
    assert!(picker.open_selected().is_none());
    picker.cancel_create();
    assert!(!picker.create_mode);
    assert!(picker.error.is_none());
    cleanup(root);
}

#[test]
fn nvidia_key_paste_opens_the_correct_masked_key_prompt() {
    let mut dialog = ProviderDialog::new(Vec::new(), None);
    dialog.paste("nvapi-this-is-a-generated-fixture-only");
    let editor = dialog.editor.as_ref().unwrap();
    assert_eq!(editor.profile.id, "nvidia");
    assert_eq!(editor.focus, 4);
    assert!(!editor.advanced);
    assert_eq!(
        editor.profile.base_url,
        "https://integrate.api.nvidia.com/v1"
    );
    assert!(!format!("{dialog:?}").contains("nvapi-this-is-a-generated-fixture-only"));
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(65, 14)).unwrap();
    terminal
        .draw(|f| meg_tui::provider::render_provider(f, f.area(), &dialog, &meg_tui::DASHBOARD))
        .unwrap();
    let text = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect::<String>();
    assert!(text.contains("API key"));
    assert!(text.contains("••••••••"));
    assert!(!text.contains("generated-fixture"));
    let ProviderAction::Save(profile, Some(secret)) = dialog.key(key(KeyCode::Enter)) else {
        panic!("Enter connects")
    };
    assert_eq!(profile.id, "nvidia");
    assert_eq!(secret.expose(), "nvapi-this-is-a-generated-fixture-only");
}

#[test]
fn in_session_browser_descends_and_returns_to_the_correct_parent() {
    let root = temp();
    let child = root.join("plain child");
    std::fs::create_dir(&child).unwrap();
    let mut picker = WorkspacePicker::open(root.clone());
    picker.toggle_projects();
    assert_eq!(picker.phase, meg_tui::workspace::PickPhase::Projects);
    picker.toggle_projects();
    assert_eq!(picker.phase, meg_tui::workspace::PickPhase::Browse);
    picker.selected = picker.dirs.iter().position(|p| p == &child).unwrap();
    picker.descend();
    assert_eq!(picker.current_directory(), child);
    picker.parent();
    assert_eq!(picker.current_directory(), root);
    cleanup(root);
}

#[test]
fn asynchronous_model_refresh_preserves_search_and_selected_identity() {
    let mut state = WorkstationState::default();
    state.available_models = vec!["qwen-a".into(), "qwen-b".into()];
    state.open_model_picker();
    let ActiveModal::ModelPicker(picker) = &mut state.active_modal else { panic!("model picker") };
    picker.search_query = "qwen".into();
    picker.filter();
    picker.move_down();
    state.available_models.insert(0, "another-model".into());
    state.open_model_picker();
    let ActiveModal::ModelPicker(picker) = &state.active_modal else { panic!("model picker") };
    assert_eq!(picker.search_query, "qwen");
    assert_eq!(picker.selected_item().unwrap().value.id, "qwen-b");
}

#[test]
fn an_opaque_key_stays_masked_until_its_service_is_chosen() {
    let mut dialog = ProviderDialog::new(Vec::new(), None);
    let fake = "sk-an-opaque-generated-fixture-key";
    dialog.paste(fake);
    assert!(dialog.editor.is_none());
    assert!(dialog.query.is_empty());
    assert!(!format!("{dialog:?}").contains(fake));
    for c in "deepseek".chars() {
        dialog.key(key(KeyCode::Char(c)));
    }
    dialog.key(key(KeyCode::Enter));
    let editor = dialog.editor.as_ref().unwrap();
    assert_eq!(editor.profile.id, "deepseek");
    assert_eq!(editor.key.expose(), fake);
    assert_eq!(editor.profile.base_url, "https://api.deepseek.com");
}

#[test]
fn identical_model_ids_keep_their_provider_and_settings_use_real_controls() {
    let root = temp();
    let profile = |id: &str| ProviderProfile {
        id: id.into(),
        name: format!("Fixture {id}"),
        kind: meg_core::providers::ProviderKind::OpenAi,
        base_url: format!("http://127.0.0.1:1/{id}/v1"),
        default_model: String::new(),
        key_env: None,
        credential_scope: meg_core::providers::credential_scope(&meg_core::providers::config_dir()),
        send_effort: false,
    };
    let first = profile("first");
    let second = profile("second");
    let state = WorkstationState {
        cwd: root.to_string_lossy().into(),
        providers: vec![first.clone(), second],
        provider: Some(first),
        ..Default::default()
    };
    let services: Arc<ServiceRunner> = Arc::new(|request, _, _| {
        assert_eq!(request.method, "provider.models");
        let id = request.params["id"].as_str().unwrap();
        let models = if id == "first" || id == "second" {
            vec!["shared-model"]
        } else {
            Vec::new()
        };
        Ok(
            serde_json::json!({"provider_id":id,"base_url":request.params["base_url"],"models":models,"message":"Fixture catalog"}),
        )
    });
    let mut controller = Controller::new(state, None)
        .unwrap()
        .with_services(Some(services));
    controller.submit_line("/model refresh").unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while controller
        .state
        .provider_models
        .values()
        .any(|p| p.status == meg_tui::model_catalog::DiscoveryStatus::Checking)
    {
        controller.poll(0.01).unwrap();
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(2));
    }
    controller.state.open_model_picker();
    let ActiveModal::ModelPicker(picker) = &controller.state.active_modal else {
        panic!()
    };
    let matching: Vec<_> = picker
        .items
        .iter()
        .filter(|i| i.value.id == "shared-model")
        .collect();
    assert_eq!(matching.len(), 2);
    assert_ne!(matching[0].value.provider_id, matching[1].value.provider_id);
    for c in "Fixture second".chars() {
        controller
            .handle_event(Event::Key(key(KeyCode::Char(c))))
            .unwrap();
    }
    controller
        .handle_event(Event::Key(key(KeyCode::Enter)))
        .unwrap();
    assert_eq!(controller.state.provider.as_ref().unwrap().id, "second");
    assert_eq!(controller.state.model, "shared-model");
    controller.submit_line("/settings").unwrap();
    assert!(controller.state.settings_picker.is_some());
    controller
        .handle_event(Event::Key(key(KeyCode::Enter)))
        .unwrap();
    assert!(controller.state.provider_dialog.is_some());
    controller.apply(RunnerMsg::StreamStart);
    assert!(
        controller
            .state
            .stream
            .as_ref()
            .unwrap()
            .is_thinking_collapsed
    );
    drop(controller);
    cleanup(root);
}

#[test]
fn key_debug_output_is_redacted() {
    assert_eq!(
        format!("{:?}", Secret::new("fixture-private-value".into())),
        "[redacted]"
    );
}
