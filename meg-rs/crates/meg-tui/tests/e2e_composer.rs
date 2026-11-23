//! E2E Tests for Feature 2 (F2): Rail / Band Composer & Docked Chip Bar
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
fn t1_composer_accent_rail() {
    let mut harness = TestHarness::new(80, 10);
    let mut state = ComposerState::default();
    state.type_str("cargo test --all");
    let pal = DARK_SLATE;

    let buf = harness.draw(|f| {
        let area = Rect::new(0, 0, 80, 5);
        render_composer(f, area, &state, &pal);
    });

    let lines = buffer_lines(&buf);
    // Line 1 should be the input row with left accent rail
    assert!(lines[1].contains('>'), "Input row must contain left accent rail (>)");
    assert!(lines[1].contains("cargo test --all"), "Input row must contain input text");

    assert_no_ansi_leaks(&buf);
}

#[test]
fn t1_composer_single_line_input() {
    let mut state = ComposerState::default();
    assert_eq!(state.cursor_pos, 0);
    assert!(state.input.is_empty());

    state.type_str("explain the architecture");
    assert_eq!(state.input, "explain the architecture");
    assert_eq!(state.cursor_pos, 24);

    state.backspace();
    assert_eq!(state.input, "explain the architectur");
    assert_eq!(state.cursor_pos, 23);

    state.type_char('e');
    assert_eq!(state.input, "explain the architecture");
    assert_eq!(state.cursor_pos, 24);
}

#[test]
fn t1_composer_multiline_expand() {
    let mut harness = TestHarness::new(80, 10);
    let mut state = ComposerState::default();

    // Single line height: 1 + 2 = 5
    assert_eq!(state.calculate_height(), 3); // 1 line + 2

    // Multiline input: 3 lines
    state.type_str("def process_data(items):\n    # transform items\n    return [x * 2 for x in items]");
    assert_eq!(state.calculate_height(), 5); // 3 lines + 2

    let pal = CLASSIC;
    let buf = harness.draw(|f| {
        let area = Rect::new(0, 0, 80, 7);
        render_composer(f, area, &state, &pal);
    });

    let lines = buffer_lines(&buf);
    assert!(lines[1].contains("def process_data"), "Row 1 rendered");
    assert!(lines[2].contains("# transform items"), "Row 2 rendered");
    assert!(lines[3].contains("return [x * 2 for x in items]"), "Row 3 rendered");

    // All input rows must have the left accent rail
    assert!(lines[1].contains('>'), "Row 1 has accent rail");
    assert!(lines[2].contains('>'), "Row 2 has accent rail");
    assert!(lines[3].contains('>'), "Row 3 has accent rail");

    assert_no_ansi_leaks(&buf);
}

#[test]
fn t1_composer_docked_chips() {
    // 2026-11-20: the docked chip bar was removed — the composer is one
    // clean bordered band; header/statusbar own the model+mode chips.
    let mut state = ComposerState::default();
    let pal = DARK_SLATE;
    let mut harness = TestHarness::new(80, 6);

    let buf = harness.draw(|f| {
        let area = Rect::new(0, 0, 80, 4);
        render_composer(f, area, &state, &pal);
    });

    let lines = buffer_lines(&buf);
    let composer_text = lines.join("
");
    assert!(composer_text.contains("Ask MEG anything"), "placeholder visible, got: {composer_text:?}");
    assert!(composer_text.contains('>'), "prompt prefix present");
    assert!(!composer_text.contains("[BUILD]"), "chip bar removed from composer");
    assert!(!composer_text.contains("? help"), "shortcut hint removed from composer");

    // Tab toggles the two modes (Act removed).
    state.cycle_mode();
    assert_eq!(state.mode, ComposerMode::Plan);
    assert_eq!(state.mode.label(), "[PLAN]");
    state.cycle_mode();
    assert_eq!(state.mode, ComposerMode::Build);
    assert_eq!(state.mode.label(), "[BUILD]");

    assert_no_ansi_leaks(&buf);
}

#[test]
fn t1_composer_history_recall() {
    let mut state = ComposerState::default();
    state.type_str("first command");
    let cmd1 = state.submit().unwrap();
    assert_eq!(cmd1, "first command");

    state.type_str("second command");
    let cmd2 = state.submit().unwrap();
    assert_eq!(cmd2, "second command");

    // Typing a draft that is not yet submitted
    state.type_str("wip draft");
    assert_eq!(state.input, "wip draft");

    // History UP: recalls "second command"
    state.history_up();
    assert_eq!(state.input, "second command");
    assert_eq!(state.cursor_pos, 14);

    // History UP: recalls "first command"
    state.history_up();
    assert_eq!(state.input, "first command");
    assert_eq!(state.cursor_pos, 13);

    // History UP at top: stays at "first command"
    state.history_up();
    assert_eq!(state.input, "first command");

    // History DOWN: returns to "second command"
    state.history_down();
    assert_eq!(state.input, "second command");

    // History DOWN at bottom: restores "wip draft"
    state.history_down();
    assert_eq!(state.input, "wip draft");
    assert_eq!(state.history_index, None);
}

// ============================================================================
// Tier 2: Boundary & Corner Cases (Boundary Value Analysis)
// ============================================================================

#[test]
fn t2_composer_empty_enter() {
    let mut state = ComposerState::default();
    // Submitting whitespace or empty input produces None
    assert_eq!(state.submit(), None);

    state.type_str("    \t  \n  ");
    assert_eq!(state.submit(), None);
    assert!(state.history.is_empty(), "Empty submissions must not be pushed to history");
}

#[test]
fn t2_composer_max_height_overflow() {
    let mut state = ComposerState::default();
    state.max_lines = 5;

    // Paste 50 lines of code
    let long_paste = (0..50).map(|i| format!("line {i}")).collect::<Vec<_>>().join("\n");
    state.type_str(&long_paste);

    // Height must be clamped to max_lines (5) + 2 = 9
    assert_eq!(state.calculate_height(), 7); // 5 lines + 2

    let mut harness = TestHarness::new(80, 20);
    let pal = DARK_SLATE;
    let buf = harness.draw(|f| {
        let area = Rect::new(0, 0, 80, 9);
        render_composer(f, area, &state, &pal);
    });

    let lines = buffer_lines(&buf);
    assert!(lines[1].contains("line 0"), "First lines rendered cleanly");
    assert_no_ansi_leaks(&buf);
}

#[test]
fn t2_composer_cursor_boundaries() {
    let mut state = ComposerState::default();
    // Left arrow at start does not underflow
    state.move_left();
    assert_eq!(state.cursor_pos, 0);

    state.backspace();
    assert_eq!(state.cursor_pos, 0);

    state.type_str("abc");
    assert_eq!(state.cursor_pos, 3);

    // Right arrow at end does not exceed string length
    state.move_right();
    assert_eq!(state.cursor_pos, 3);

    // Home / End navigation
    state.move_home();
    assert_eq!(state.cursor_pos, 0);

    state.delete(); // deletes 'a'
    assert_eq!(state.input, "bc");
    assert_eq!(state.cursor_pos, 0);

    state.move_end();
    assert_eq!(state.cursor_pos, 2);
    state.delete(); // delete at end does nothing
    assert_eq!(state.input, "bc");
}

#[test]
fn t2_composer_history_empty() {
    let mut state = ComposerState::default();
    // History up/down when history is empty does nothing safely
    state.history_up();
    assert_eq!(state.input, "");
    assert_eq!(state.history_index, None);

    state.history_down();
    assert_eq!(state.input, "");
    assert_eq!(state.history_index, None);
}

#[test]
fn t2_composer_multibyte_utf8() {
    let mut state = ComposerState::default();
    // Multi-byte CJK and emojis: '日'(1) '本'(2) '語'(3) ' '(4) '🚀'(5) ' '(6) 'c'(7) 'o'(8) 'd'(9) 'i'(10) 'n'(11) 'g'(12)
    state.type_str("日本語 🚀 coding");
    assert_eq!(state.cursor_pos, 12); // 12 unicode characters

    // Move left 7 positions: should land on space after emoji (pos 5)
    for _ in 0..7 {
        state.move_left();
    }
    assert_eq!(state.cursor_pos, 5);

    // Backspace deletes the emoji 🚀 (char at pos 4)
    state.backspace();
    assert_eq!(state.input, "日本語  coding");
    assert_eq!(state.cursor_pos, 4);

    // Backspace deletes space, then '語'
    state.backspace();
    state.backspace();
    assert_eq!(state.input, "日本 coding");
    assert_eq!(state.cursor_pos, 2);
}
