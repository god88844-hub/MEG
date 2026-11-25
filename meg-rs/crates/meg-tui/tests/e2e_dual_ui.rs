//! Phase 8 Batch 1 (P8-01 + P8-02) E2E Tests:
//! - Dual-TUI `/ui classic` vs `/ui pro` Titanium Dark switching
//! - Pluggable `UiFrontend` trait & custom `~/.meg/ui.json` RGB overrides
//! - Live `/pair` single-model vs. multi-model orchestration in `RunRequest`
//! - Responsive narrow/extreme sizes (`0x0`, `1x1`, `40x12`, `140x40`) & zero ANSI leaks

use meg_tui::{
    draw, effective_pro_palette, ClassicFrontend, Controller, PaletteOverrides, ProFrontend,
    UiFrontend, UiMode, UserUiConfig, WorkstationState, TITANIUM_DARK,
};
use ratatui::{backend::TestBackend, style::Color, Terminal};

fn buffer_text(term: &Terminal<TestBackend>) -> String {
    let buf = term.backend().buffer();
    let mut out = String::new();
    for y in 0..buf.area.height {
        for x in 0..buf.area.width {
            out.push_str(buf[(x, y)].symbol());
        }
        out.push('\n');
    }
    out
}

#[test]
fn dual_tui_switches_between_classic_and_pro_titanium_and_renders_orchestration_hud() {
    let ws = std::env::temp_dir().join(format!("meg-dual-tui-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&ws).unwrap();

    let state = WorkstationState {
        cwd: ws.to_string_lossy().into_owned(),
        model: "gemini-3.1-pro".into(),
        online: true,
        context_percent: 38,
        transcript: vec![
            "You: Fix the LRU eviction bug".into(),
            "✓ [tool] edit_file lru.py (+2/-1)".into(),
            "Updated popitem(last=False) and verified tests.".into(),
        ],
        plan_steps: vec![(true, "Inspect lru.py".into()), (false, "Run pytest".into())],
        subagents: vec![("reviewer".into(), "ready".into())],
        ..Default::default()
    };
    let mut c = Controller::new(state, None).unwrap();
    assert_eq!(c.state.ui_mode, UiMode::Classic);

    // Switch live to /ui pro and enable multi-model Antigravity orchestration + Cold L3 reviewer
    c.submit_line("/ui pro").unwrap();
    c.submit_line("/pair antigravity_orchestrator qwen-exec claude-critic")
        .unwrap();
    assert_eq!(c.state.ui_mode, UiMode::Pro);

    // RunRequest carries live orchestration fields to meg-cli
    let req = c.state.run_request("Verify tests".into());
    assert_eq!(req.pair_mode.as_deref(), Some("antigravity_orchestrator"));
    assert_eq!(req.executor_model.as_deref(), Some("qwen-exec"));
    assert_eq!(req.reviewer_model.as_deref(), Some("claude-critic"));

    // Render /ui pro at desktop dimensions and verify HUD + Orchestration + ToolCard + Tree
    let backend = TestBackend::new(120, 32);
    let mut term = Terminal::new(backend).unwrap();
    term.draw(|f| draw(f, &c.state.theme, &c.state)).unwrap();
    let rendered = buffer_text(&term);
    assert!(rendered.contains("MEG PRO"), "Expected MEG PRO badge:\n{rendered}");
    assert!(rendered.contains("MULTI"), "Expected orchestration badge:\n{rendered}");
    assert!(rendered.contains("Orchestration"), "Expected right rail:\n{rendered}");
    assert!(rendered.contains("qwen-exec"), "Expected executor model:\n{rendered}");
    assert!(rendered.contains("claude-critic"), "Expected L3 reviewer:\n{rendered}");
    assert!(rendered.contains("edit_file lru.py"), "Expected tool card:\n{rendered}");
    assert!(!rendered.contains('\x1b'), "Zero raw ANSI leaks allowed");

    // Switch back to single-model + /ui classic cleanly
    c.submit_line("/pair off").unwrap();
    c.submit_line("/ui classic").unwrap();
    assert_eq!(c.state.ui_mode, UiMode::Classic);
    assert!(c.state.pair_mode.is_none());
    assert!(c.state.executor_model.is_none());
    let _ = std::fs::remove_dir_all(&ws);
}

#[test]
fn custom_user_ui_json_overrides_palette_and_hot_reloads() {
    let ws = std::env::temp_dir().join(format!("meg-ui-json-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&ws).unwrap();

    let custom = UserUiConfig {
        ui_mode: UiMode::Pro,
        theme_name: None,
        show_sidebar: false,
        compact_tool_cards: false,
        sync_csi_2026: true,
        pair_mode: Some("devin_fusion".into()),
        executor_model: Some("fast-coder".into()),
        reviewer_model: Some("strict-l3".into()),
        roles: Default::default(),
        colors: PaletteOverrides {
            accent: Some("#FF6B00".into()),
            viewport_bg: Some("#101018".into()),
            ..Default::default()
        },
    };
    let path = UserUiConfig::workspace_path(&ws);
    custom.save_to(&path).unwrap();

    let state = WorkstationState {
        cwd: ws.to_string_lossy().into_owned(),
        ..Default::default()
    };
    let mut c = Controller::new(state, None).unwrap();
    c.submit_line("/ui reload").unwrap();

    assert_eq!(c.state.ui_mode, UiMode::Pro);
    assert!(!c.state.ui_config.show_sidebar);
    assert!(!c.state.collapse_tool_cards);
    assert_eq!(c.state.pair_mode.as_deref(), Some("devin_fusion"));
    assert_eq!(c.state.executor_model.as_deref(), Some("fast-coder"));

    let eff = effective_pro_palette(&TITANIUM_DARK, &c.state);
    assert_eq!(eff.accent, Color::Rgb(0xFF, 0x6B, 0x00));
    assert_eq!(eff.viewport_bg, Color::Rgb(0x10, 0x10, 0x18));
    let _ = std::fs::remove_dir_all(&ws);
}

#[test]
fn pro_and_classic_frontends_survive_extreme_dimensions_and_ansi_bombs() {
    let st = WorkstationState {
        ui_mode: UiMode::Pro,
        model: "\x1b[31mhostile-model\x1b[0m".into(),
        transcript: vec![
            "\x1b[2J\x1b[H[tool] bash \x1b]8;;http://evil\x07click\x1b]8;;\x07".into(),
            "Normal output line".into(),
        ],
        ..Default::default()
    };
    let frontends: [&dyn UiFrontend; 2] = [&ClassicFrontend, &ProFrontend];
    for frontend in frontends {
        for (w, h) in [(0, 0), (1, 1), (20, 6), (48, 14), (120, 36)] {
            let backend = TestBackend::new(w, h);
            let mut term = Terminal::new(backend).unwrap();
            term.draw(|f| frontend.draw(f, &st.theme, &st)).unwrap();
            let text = buffer_text(&term);
            assert!(
                !text.contains('\x1b'),
                "Frontend {} leaked ANSI escape at {w}x{h}",
                frontend.name()
            );
        }
    }
}
#[test]
fn ui_popup_live_previews_pro_classic_and_double_ctrl_c_guards_exit() {
    use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
    let ws = std::env::temp_dir().join(format!("meg-ui-popup-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&ws).unwrap();

    let state = WorkstationState {
        cwd: ws.to_string_lossy().into_owned(),
        ..Default::default()
    };
    let mut c = Controller::new(state, None).unwrap();

    // 1. Bare /ui opens the interactive picker modal
    c.submit_line("/ui").unwrap();
    assert!(c.state.ui_picker.is_some());
    assert_eq!(c.state.ui_mode, UiMode::Classic);

    // Down -> Classic, Up -> Pro live previews immediately
    c.handle_event(Event::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)))
        .unwrap();
    assert_eq!(c.state.ui_mode, UiMode::Classic);
    c.handle_event(Event::Key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE)))
        .unwrap();
    assert_eq!(c.state.ui_mode, UiMode::Pro);

    // Esc cancels and restores original mode
    c.handle_event(Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)))
        .unwrap();
    assert!(c.state.ui_picker.is_none());
    assert_eq!(c.state.ui_mode, UiMode::Classic);

    // 2. Live theme switching works inside /ui pro
    c.submit_line("/ui pro").unwrap();
    c.submit_line("/theme Dracula").unwrap();
    let eff = effective_pro_palette(&c.state.theme, &c.state);
    assert_eq!(eff.name, "Dracula");

    // 3. Double Ctrl+C exit guard when idle
    c.handle_event(Event::Key(KeyEvent::new(
        KeyCode::Char('c'),
        KeyModifiers::CONTROL,
    )))
    .unwrap();
    assert!(!c.state.should_quit, "First Ctrl+C should warn, not exit");
    assert!(c.state.notice.contains("Ctrl+C again"));

    c.handle_event(Event::Key(KeyEvent::new(
        KeyCode::Char('c'),
        KeyModifiers::CONTROL,
    )))
    .unwrap();
    assert!(c.state.should_quit, "Second Ctrl+C within 2.5s should exit");

    // 4. /ui custom scaffolds .meg/ui.json
    c.submit_line("/ui custom").unwrap();
    assert!(UserUiConfig::workspace_path(&ws).is_file());
    let _ = std::fs::remove_dir_all(&ws);
}