//! E2E Tests for Feature 1 (F1): Compact Hero Banner
//!
//! Covers Tier 1 (Feature Coverage) and Tier 2 (Boundary & Corner Cases).
//! 100% offline, zero live API calls, asserting on Ratatui TestBackend buffers.

mod common;

use common::*;
use ratatui::layout::Rect;

// ============================================================================
// Tier 1: Feature Coverage (Category-Partition)
// ============================================================================

#[test]
fn t1_hero_standard_render() {
    let mut harness = TestHarness::new(80, 24);
    let state = HeroState::default();
    let pal = DARK_SLATE;

    let buf = harness.draw(|f| {
        let area = Rect::new(0, 0, 80, 5);
        render_hero(f, area, &state, &pal);
    });

    let lines = buffer_lines(&buf);
    // Line 0: top border with title
    assert!(lines[0].contains('╭'), "Expected rounded top-left corner");
    assert!(lines[0].contains("MEG Polyglot Harness"), "Expected banner title");
    assert!(lines[0].contains('╮'), "Expected rounded top-right corner");

    // Line 1: Model metadata
    assert!(lines[1].contains("Model:"), "Expected Model key");
    assert!(lines[1].contains("qwen2.5-coder:7b"), "Expected active model");

    // Line 2: Workspace metadata
    assert!(lines[2].contains("Workspace:"), "Expected Workspace key");
    assert!(lines[2].contains("~/MEG"), "Expected workspace path");

    // Line 3: Shortcuts/tips
    assert!(lines[3].contains("Tips:"), "Expected Tips key");

    // Line 4: Bottom border
    assert!(lines[4].contains('╰'), "Expected rounded bottom-left corner");
    assert!(lines[4].contains('╯'), "Expected rounded bottom-right corner");

    assert_no_ansi_leaks(&buf);
}

#[test]
fn t1_hero_half_block_render() {
    let mut harness = TestHarness::new(80, 10);
    let pal = PHOSPHOR;

    let buf = harness.draw(|f| {
        let area = Rect::new(2, 1, 20, 2);
        render_micro_wordmark(f, area, &pal);
    });

    let lines = buffer_lines(&buf);
    // Half-block MEG logo occupies 2 lines
    assert!(lines[1].contains('\u{2588}'), "Line 1 must contain upper half block glyphs");
    assert!(lines[2].contains('\u{2588}'), "Line 2 must contain lower half block glyphs");
    assert_no_ansi_leaks(&buf);
}

#[test]
fn t1_hero_metadata_display() {
    let mut harness = TestHarness::new(80, 10);
    let state = HeroState {
        version: "v0.9.1-alpha".into(),
        model: "claude-sonnet-4-6".into(),
        workspace: "c:\\Projects\\AI\\meg-polyglot".into(),
        tips: vec!["Tab toggles Plan/Act".into()],
    };
    let pal = CLASSIC;

    let buf = harness.draw(|f| {
        let area = Rect::new(0, 0, 80, 5);
        render_hero(f, area, &state, &pal);
    });

    let text = buffer_lines(&buf).join("\n");
    assert!(text.contains("v0.9.1-alpha"), "Custom version missing");
    assert!(text.contains("claude-sonnet-4-6"), "Custom model missing");
    assert!(text.contains("c:\\Projects\\AI\\meg-polyglot"), "Custom workspace missing");
    assert_no_ansi_leaks(&buf);
}

#[test]
fn t1_hero_shortcuts_display() {
    let mut harness = TestHarness::new(80, 10);
    let state = HeroState::default();
    let pal = DRACULA;

    let buf = harness.draw(|f| {
        let area = Rect::new(0, 0, 80, 5);
        render_hero(f, area, &state, &pal);
    });

    let text = buffer_lines(&buf).join("\n");
    assert!(text.contains("/model to switch"), "Expected /model hint");
    assert!(text.contains("/theme to restyle"), "Expected /theme hint");
    assert!(text.contains("? for shortcuts"), "Expected shortcuts hint");
    assert_no_ansi_leaks(&buf);
}

#[test]
fn t1_hero_border_styling() {
    let mut harness = TestHarness::new(80, 10);
    let state = HeroState::default();
    let pal = TOKYO_NIGHT;

    let buf = harness.draw(|f| {
        let area = Rect::new(0, 0, 80, 5);
        render_hero(f, area, &state, &pal);
    });

    // Check border styling on corner cell (0, 0)
    let corner_cell = &buf[(0, 0)];
    assert_eq!(corner_cell.symbol(), "╭");
    assert_eq!(corner_cell.fg, pal.panel_border, "Border fg must match theme panel_border");
    assert_no_ansi_leaks(&buf);
}

// ============================================================================
// Tier 2: Boundary & Corner Cases (Boundary Value Analysis)
// ============================================================================

#[test]
fn t2_hero_narrow_terminal() {
    // Narrow terminal (< 52 cols) triggers responsive 1-line fallback
    let mut harness = TestHarness::new(40, 24);
    let state = HeroState::default();
    let pal = DARK_SLATE;

    let buf = harness.draw(|f| {
        let area = Rect::new(0, 0, 40, 1);
        render_hero(f, area, &state, &pal);
    });

    let line0 = buffer_line(&buf, 0);
    assert!(line0.contains("MEG"), "Fallback must contain MEG mark");
    assert!(line0.contains(state.version.as_str()), "Fallback must contain version");
    // Ensure no panic and no cell wrapping outside column 40
    assert_no_ansi_leaks(&buf);
}

#[test]
fn t2_hero_short_terminal() {
    // Short terminal height (< 5 rows) triggers responsive fallback
    let mut harness = TestHarness::new(80, 3);
    let state = HeroState::default();
    let pal = PHOSPHOR;

    let buf = harness.draw(|f| {
        let area = Rect::new(0, 0, 80, 3);
        render_hero(f, area, &state, &pal);
    });

    let line0 = buffer_line(&buf, 0);
    assert!(line0.contains("MEG"), "Short height fallback must render on line 0");
    assert_no_ansi_leaks(&buf);
}

#[test]
fn t2_hero_empty_workspace_str() {
    let mut harness = TestHarness::new(80, 10);
    let state = HeroState {
        version: "v0.8.0".into(),
        model: "test-model".into(),
        workspace: "".into(), // empty string
        tips: vec![],
    };
    let pal = CLASSIC;

    let buf = harness.draw(|f| {
        let area = Rect::new(0, 0, 80, 5);
        render_hero(f, area, &state, &pal);
    });

    let text = buffer_lines(&buf).join("\n");
    assert!(text.contains("Workspace:"), "Must still render Workspace row cleanly");
    assert_no_ansi_leaks(&buf);
}

#[test]
fn t2_hero_long_model_name() {
    let mut harness = TestHarness::new(80, 10);
    let long_model = "org-very-deep-reasoning-model-v3-ultra-large-context-extended-quantum-checkpoint-epoch-99";
    let state = HeroState {
        version: "v0.8.0".into(),
        model: long_model.into(),
        workspace: "~/repo".into(),
        tips: vec!["Tip".into()],
    };
    let pal = DARK_SLATE;

    let buf = harness.draw(|f| {
        let area = Rect::new(0, 0, 80, 5);
        render_hero(f, area, &state, &pal);
    });

    let line1 = buffer_line(&buf, 1);
    assert!(line1.contains("..."), "Model name over 30 chars must be truncated with ellipsis");
    assert_no_ansi_leaks(&buf);
}

#[test]
fn t2_hero_unicode_handling() {
    let mut harness = TestHarness::new(80, 10);
    let state = HeroState {
        version: "v0.8.0".into(),
        model: "🤖-ai-coder".into(),
        workspace: "c:\\ユーザー\\プロジェクト\\MEG🚀".into(),
        tips: vec!["Unicode tips: ⚡".into()],
    };
    let pal = TOKYO_NIGHT;

    let buf = harness.draw(|f| {
        let area = Rect::new(0, 0, 80, 5);
        render_hero(f, area, &state, &pal);
    });

    let text = buffer_lines(&buf).join("\n");
    assert!(text.contains("ai-coder"), "Model name rendered cleanly");
    assert!(text.contains("MEG"), "Workspace and brand rendered cleanly");
    assert!(text.contains("Unicode tips:"), "Unicode tip rendered cleanly");
    assert_no_ansi_leaks(&buf);
}
