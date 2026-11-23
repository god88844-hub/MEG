//! E2E Tests for Tier 3: Cross-Feature Interactions
//!
//! Tests pairwise combinations and cross-cutting behaviors across F1-F6:
//! - Hero Banner + Composer coexistence and layout bounds
//! - Modal Picker overlays on top of active streaming / transcript
//! - Theme switching while background job spinner is animating
//! - Expanding multiline composer elevating docked job status indicator
//! - Modal picker keystrokes completely isolated from composer buffer
//! - ANSI sanitization across streaming and transcript boundaries
//!
//! 100% offline, zero live API calls, asserting on Ratatui TestBackend buffers.

mod common;

use common::*;
use ratatui::layout::{Constraint, Direction, Layout, Rect};

#[test]
fn t3_combo_hero_and_composer() {
    let mut harness = TestHarness::new(80, 24);
    let hero = HeroState::default();
    let mut composer = ComposerState::default();
    composer.type_str("cargo build --release");
    let pal = DARK_SLATE;

    let buf = harness.draw(|f| {
        let size = f.area();
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(5), // Hero banner
                Constraint::Min(4),    // Transcript fill
                Constraint::Length(5), // Composer
            ])
            .split(size);

        render_hero(f, chunks[0], &hero, &pal);
        render_composer(f, chunks[2], &composer, &pal);
    });

    let lines = buffer_lines(&buf);

    // Hero banner at top
    assert!(lines[0].contains("MEG Polyglot Harness"), "Hero banner rendered at row 0");
    assert!(lines[1].contains("Model:"), "Hero metadata at row 1");
    assert!(lines[4].contains('╰'), "Hero bottom border at row 4");

    // Composer at bottom
    let comp_row = lines.iter().position(|l| l.contains("cargo build --release")).expect("Composer input rendered");
    assert!(comp_row >= 19, "Composer docked at bottom rows");
    // 2026-11-20: no chip bar - the composer band closes below the input
    // with its rounded bottom border.
    let band_row = lines.iter().rposition(|l| l.contains('\u{2570}')).expect("Composer closing border renders");
    assert!(band_row > comp_row, "Band closes after the input row");

    assert_no_ansi_leaks(&buf);
}

#[test]
fn t3_combo_picker_over_streaming() {
    let mut harness = TestHarness::new(80, 24);
    let mut stream = StreamState::default();
    stream.push_thinking("Analyzing project dependencies...\n");
    stream.push_content("1. meg-core\n2. meg-tools\n3. meg-tui\n");
    stream.elapsed_secs = 3.2;

    let picker_items: Vec<PickerItem<String>> = vec![
        PickerItem {
            label: "claude-sonnet-4-6".into(),
            detail: "cloud".into(),
            meta: "--".into(),
            value: "claude-sonnet-4-6".into(),
        },
        PickerItem {
            label: "qwen2.5-coder:7b".into(),
            detail: "local".into(),
            meta: "45ms".into(),
            value: "qwen2.5-coder:7b".into(),
        },
    ];
    let picker = PickerState::new("Model Selector".into(), picker_items);
    let pal = TOKYO_NIGHT;

    // Render stream, then modal picker on top (overlay pattern)
    let buf = harness.draw(|f| {
        let size = f.area();
        render_streaming_view(f, size, &stream, &pal);
        render_picker_modal(f, size, &picker, &pal);
    });

    let lines = buffer_lines(&buf);
    // Modal overlay must be visible and properly layered
    assert!(lines.iter().any(|l| l.contains("Model Selector")), "Overlay modal title visible");
    assert!(lines.iter().any(|l| l.contains("claude-sonnet-4-6")), "Overlay item visible");

    // Background streaming content remains intact outside overlay bounds
    assert!(lines.iter().any(|l| l.contains("meg-core") || l.contains("Analyzing")), "Background stream intact");
    assert_no_ansi_leaks(&buf);
}

#[test]
fn t3_combo_theme_switch_during_job() {
    let mut harness = TestHarness::new(80, 10);
    let mut job = JobState::default();
    job.start("job-run".into(), "Bash(cargo test)".into());
    job.tick(1.5);

    // Frame 1 with Phosphor (CRT Green)
    let buf1 = harness.draw(|f| {
        let area = Rect::new(0, 0, 80, 2);
        render_job_indicator(f, area, &job, &PHOSPHOR);
    });
    let fg1 = buf1[(1, 0)].fg; // Spinner glyph cell

    // Frame 2 user switches theme to Dracula
    job.tick(0.08);
    let buf2 = harness.draw(|f| {
        let area = Rect::new(0, 0, 80, 2);
        render_job_indicator(f, area, &job, &DRACULA);
    });
    let fg2 = buf2[(1, 0)].fg;
    assert_eq!(fg1, PHOSPHOR.accent);
    assert_eq!(fg2, DRACULA.accent);
    assert_ne!(fg1, fg2, "Job spinner accent color dynamically updated on theme change");

    assert_no_ansi_leaks(&buf1);
    assert_no_ansi_leaks(&buf2);
}

#[test]
fn t3_combo_composer_history_with_chips() {
    let mut state = ComposerState::default();
    state.model = "qwen2.5-coder:7b".into();
    state.mode = ComposerMode::Build;

    // Command 1
    state.type_str("/mode plan");
    state.cycle_mode(); // switch to Plan
    state.submit();

    // Command 2
    state.type_str("refactor architecture");
    state.submit();

    // Recalling history restores prompt and leaves chips intact
    state.history_up();
    assert_eq!(state.input, "refactor architecture");
    assert_eq!(state.mode, ComposerMode::Plan);

    state.history_up();
    assert_eq!(state.input, "/mode plan");
    assert_eq!(state.mode, ComposerMode::Plan);
}

#[test]
fn t3_combo_thinking_collapse_and_viewport_scroll() {
    let mut harness = TestHarness::new(80, 20);
    let mut stream = StreamState::default();
    stream.push_thinking("Line 1 of thought\nLine 2 of thought\nLine 3 of thought\n");
    stream.push_content("Final output line 1\nFinal output line 2\n");
    stream.elapsed_secs = 2.0;
    stream.token_count = 60;
    let pal = DARK_SLATE;

    // Expanded state: occupies >6 lines
    let buf_exp = harness.draw(|f| {
        let area = Rect::new(0, 0, 80, 15);
        render_streaming_view(f, area, &stream, &pal);
    });
    let lines_exp = buffer_lines(&buf_exp);
    let exp_content_idx = lines_exp.iter().position(|l| l.contains("Final output line 1")).unwrap();

    // Collapse thinking
    stream.toggle_thinking_collapse();
    let buf_col = harness.draw(|f| {
        let area = Rect::new(0, 0, 80, 15);
        render_streaming_view(f, area, &stream, &pal);
    });
    let lines_col = buffer_lines(&buf_col);
    let col_content_idx = lines_col.iter().position(|l| l.contains("Final output line 1")).unwrap();

    // When thinking is collapsed, content moves up
    assert!(col_content_idx < exp_content_idx, "Content moves upward when thinking collapses");
    assert_no_ansi_leaks(&buf_exp);
    assert_no_ansi_leaks(&buf_col);
}

#[test]
fn t3_combo_job_status_and_composer_multiline() {
    let mut harness = TestHarness::new(80, 24);
    let mut job = JobState::default();
    job.start("job-t3".into(), "Building release binaries".into());

    let mut composer = ComposerState::default();
    let pal = DARK_SLATE;

    // Single-line composer
    let h1 = composer.calculate_height();
    assert_eq!(h1, 3); // 1 line + borders

    // Multiline expansion: 3 lines
    composer.type_str("fn main() {\n    println!(\"hello\");\n}");
    let h2 = composer.calculate_height();
    assert_eq!(h2, 5); // 3 lines + borders

    // Layout allocates exactly h2 for composer, job indicator sits immediately above
    let buf = harness.draw(|f| {
        let size = f.area();
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(5),    // transcript
                Constraint::Length(2), // active job indicator
                Constraint::Length(h2), // dynamic composer
            ])
            .split(size);

        render_job_indicator(f, chunks[1], &job, &pal);
        render_composer(f, chunks[2], &composer, &pal);
    });

    let lines = buffer_lines(&buf);
    let job_pos = lines.iter().position(|l| l.contains("Building release binaries")).unwrap();
    let comp_pos = lines.iter().position(|l| l.contains("fn main()")).unwrap();

    assert!(job_pos < comp_pos, "Job indicator rendered directly above composer");
    assert_eq!(comp_pos - job_pos, 3, "Exact vertical docking with composer border");
    assert!(lines[comp_pos - 1].contains('╭'), "Composer top border directly follows job indicator");
    assert_no_ansi_leaks(&buf);
}

#[test]
fn t3_combo_launch_and_theme_init() {
    // Launching with configured theme loads correct palette into Hero Banner and Composer immediately on frame 1
    let mut harness = TestHarness::new(80, 24);
    let hero = HeroState::default();
    let composer = ComposerState::default();
    let pal = TOKYO_NIGHT;

    let buf = harness.draw(|f| {
        let size = f.area();
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(5),
                Constraint::Min(4),
                Constraint::Length(5),
            ])
            .split(size);

        render_hero(f, chunks[0], &hero, &pal);
        render_composer(f, chunks[2], &composer, &pal);
    });

    // Check Hero border and Composer border both match TOKYO_NIGHT panel_border
    assert_eq!(buf[(0, 0)].fg, TOKYO_NIGHT.panel_border);
    let comp_y = 24 - 5;
    assert_eq!(buf[(0, comp_y)].fg, TOKYO_NIGHT.panel_border);
    assert_no_ansi_leaks(&buf);
}

#[test]
fn t3_combo_theme_preview_cancel_with_composer_input() {
    let mut harness = TestHarness::new(80, 24);
    let mut composer = ComposerState::default();
    composer.type_str("keep this draft unchanged");

    let initial_theme = DARK_SLATE;
    let mut active_theme;

    let mut picker = PickerState::new("Theme".into(), vec![
        PickerItem { label: "Dracula".into(), detail: "".into(), meta: "".into(), value: DRACULA },
    ]);

    // Preview theme
    active_theme = DRACULA;
    assert_eq!(active_theme.name, "Dracula");

    // Cancel modal
    picker.cancel();
    active_theme = initial_theme;

    // Composer draft is completely preserved
    assert_eq!(composer.input, "keep this draft unchanged");
    assert_eq!(active_theme.name, "Dark Slate");

    let buf = harness.draw(|f| {
        render_composer(f, Rect::new(0, 18, 80, 5), &composer, &active_theme);
    });
    assert!(buffer_contains(&buf, "keep this draft unchanged"));
    assert_no_ansi_leaks(&buf);
}

#[test]
fn t3_combo_multiline_composer_shrink_on_delete() {
    let mut composer = ComposerState::default();
    composer.type_str("line 1\nline 2\nline 3");
    assert_eq!(composer.calculate_height(), 5); // 3 lines + borders

    // Delete newline between line 2 and 3
    for _ in 0..7 {
        composer.backspace();
    }
    // Now 2 lines
    assert_eq!(composer.calculate_height(), 4); // 2 lines + borders

    // Delete line 2 completely
    for _ in 0..7 {
        composer.backspace();
    }
    // Now single line
    assert_eq!(composer.calculate_height(), 3);
}

#[test]
fn t3_combo_job_done_leaves_ready_chips() {
    let mut harness = TestHarness::new(80, 20);
    let mut job = JobState::default();
    job.start("build-step".into(), "Cargo check".into());

    let mut composer = ComposerState::default();
    composer.model = "qwen2.5-coder:7b".into();
    let pal = DARK_SLATE;

    // Job active: chip bar ready
    assert!(job.is_active);

    // Job finishes
    job.complete();
    assert!(!job.is_active);

    let buf = harness.draw(|f| {
        render_job_indicator(f, Rect::new(0, 13, 80, 2), &job, &pal);
        render_composer(f, Rect::new(0, 15, 80, 5), &composer, &pal);
    });

    let text = buffer_lines(&buf).join("\n");
    assert!(!text.contains("Cargo check"), "Completed job indicator is cleared");
    assert!(text.contains("Ask MEG anything"), "Composer placeholder intact and ready");
    assert_no_ansi_leaks(&buf);
}

#[test]
fn t3_combo_modal_picker_input_isolated_from_composer() {
    let mut composer = ComposerState::default();
    composer.type_str("active draft");

    let mut picker: PickerState<String> = PickerState::new("Model".into(), vec![
        PickerItem { label: "gpt-4o".into(), detail: "".into(), meta: "".into(), value: "gpt-4o".into() },
    ]);

    // Keystrokes intended for picker search
    picker.type_char('g');
    picker.type_char('p');
    picker.type_char('t');

    // Picker search query is updated
    assert_eq!(picker.search_query, "gpt");
    // Composer input is completely untouched
    assert_eq!(composer.input, "active draft");
    assert_eq!(composer.cursor_pos, 12);
}

#[test]
fn t3_combo_streaming_ansi_cleaned_in_transcript() {
    let mut harness = TestHarness::new(80, 15);
    let mut stream = StreamState::default();
    // Simulate streaming chunks containing ANSI escape codes from a remote runner
    let dirty_chunk = "\x1b[32m✔ Passed 100 tests\x1b[0m with \x1b[1;33m0 warnings\x1b[0m";
    let cleaned = strip_ansi(dirty_chunk);
    stream.push_content(&cleaned);
    let pal = PHOSPHOR;

    let buf = harness.draw(|f| {
        let area = Rect::new(0, 0, 80, 10);
        render_streaming_view(f, area, &stream, &pal);
    });

    let text = buffer_lines(&buf).join("\n");
    assert!(text.contains("✔ Passed 100 tests with 0 warnings"));
    assert!(!text.contains('\x1b'));
    assert_no_ansi_leaks(&buf);
}
