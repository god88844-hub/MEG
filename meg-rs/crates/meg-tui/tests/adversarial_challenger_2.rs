//! Adversarial Challenge Test Suite — Challenger 2 (TUI Input & State Challenger)
//!
//! Empirical verification of:
//! 1. UTF-8 safety (CJK, 4-byte emojis, combining accents, zero-width chars, RTL)
//! 2. History draft preservation, modification during recall, and Esc cancellation
//! 3. Mode cycling (Build <-> Plan <-> Act)
//! 4. Hardware cursor positioning coordinates and terminal boundary clamping

use meg_tui::{cursor_screen_pos, render_composer, Mode, WorkstationState, DARK_SLATE};
use ratatui::{
    backend::TestBackend,
    layout::{Position, Rect},
    Terminal,
};

// ============================================================================
// 1. UTF-8 Safety Adversarial Tests
// ============================================================================

#[test]
fn adv_utf8_cjk_insert_delete_cursor() {
    let mut st = WorkstationState::default();

    // Insert 3-byte CJK Chinese, Japanese, and Korean
    let sample = "中文\u{30c6}\u{30b9}\u{30c8}\u{d55c}\u{ae00}"; // "中文テスト한글" (7 CJK characters, 21 bytes)
    st.insert_str(sample);

    assert_eq!(st.cursor_position, 7);
    assert_eq!(st.input.chars().count(), 7);
    assert_eq!(st.input.len(), 21);

    // Navigate cursor through every CJK character backwards
    for i in (0..7).rev() {
        st.move_cursor_left();
        assert_eq!(st.cursor_position, i);
    }
    // Cannot move left past 0
    st.move_cursor_left();
    assert_eq!(st.cursor_position, 0);

    // Insert character at beginning
    st.insert_char('前');
    assert_eq!(st.cursor_position, 1);
    assert!(st.input.starts_with('前'));

    // Move to middle and insert
    st.cursor_position = 3;
    st.insert_str("中间");
    assert_eq!(st.cursor_position, 5);

    // Delete characters from middle
    let before_del = st.input.clone();
    let char_count_before = before_del.chars().count();
    let ok = st.delete_char();
    assert!(ok);
    assert_eq!(st.input.chars().count(), char_count_before - 1);

    // Backspace from middle
    let char_count_before_bs = st.input.chars().count();
    let ok_bs = st.delete_backspace();
    assert!(ok_bs);
    assert_eq!(st.input.chars().count(), char_count_before_bs - 1);

    // Move cursor to end and delete all remaining characters via backspace
    st.move_cursor_end();
    while st.delete_backspace() {}
    assert_eq!(st.input, "");
    assert_eq!(st.cursor_position, 0);

    // Subsequent backspace on empty returns false and doesn't panic
    assert!(!st.delete_backspace());
    assert!(!st.delete_char());
}

#[test]
fn adv_utf8_emojis_4byte_boundaries() {
    let mut st = WorkstationState::default();

    // 4-byte UTF-8 emojis
    st.insert_str("🚀🦀🔥💡🎉");
    assert_eq!(st.cursor_position, 5); // 5 characters
    assert_eq!(st.input.len(), 20); // 20 bytes (4 bytes each)

    // Navigate backwards
    st.move_cursor_left(); // before 🎉
    assert_eq!(st.cursor_position, 4);

    // Delete '🎉' with delete_char
    assert!(st.delete_char());
    assert_eq!(st.input, "🚀🦀🔥💡");
    assert_eq!(st.cursor_position, 4);
    assert_eq!(st.input.len(), 16);

    // Backspace '💡'
    assert!(st.delete_backspace());
    assert_eq!(st.input, "🚀🦀🔥");
    assert_eq!(st.cursor_position, 3);
    assert_eq!(st.input.len(), 12);

    // Insert ASCII between emojis
    st.move_cursor_left(); // between 🦀 and 🔥
    assert_eq!(st.cursor_position, 2);
    st.insert_str(" RUST ");
    assert_eq!(st.input, "🚀🦀 RUST 🔥");
    assert_eq!(st.cursor_position, 8); // 2 + 6 = 8

    // Word navigation across emojis and spaces
    st.move_cursor_word_left(); // to start of "RUST"
    assert_eq!(st.cursor_position, 3);
    st.move_cursor_word_left(); // to start
    assert_eq!(st.cursor_position, 0);
    st.move_cursor_word_right(); // past 🚀🦀
    assert_eq!(st.cursor_position, 3);
    st.move_cursor_word_right(); // past RUST
    assert_eq!(st.cursor_position, 8);
}

#[test]
fn adv_utf8_combining_and_zwj_sequences() {
    let mut st = WorkstationState::default();

    // Combining characters: e + combining acute accent
    st.insert_str("e\u{0301}"); // é as decomposed form
    assert_eq!(st.cursor_position, 2);
    assert_eq!(st.input.chars().count(), 2);

    // Editing removes the complete visible grapheme, preserving its shape.
    assert!(st.delete_backspace());
    assert_eq!(st.input, "");
    assert_eq!(st.cursor_position, 0);

    // ZWJ Emoji sequence: Man + ZWJ + Laptop = Technologist (👨‍💻)
    let tech_emoji = "👨\u{200D}💻";
    st.input.clear();
    st.cursor_position = 0;
    st.insert_str(tech_emoji);
    assert_eq!(st.cursor_position, 3);
    assert_eq!(st.input.chars().count(), 3);

    // The displayed emoji is one editing unit, including its ZWJ.
    st.move_cursor_left();
    assert_eq!(st.cursor_position, 0);
    st.move_cursor_right();
    assert_eq!(st.cursor_position, 3);
    assert!(st.delete_backspace());
    assert_eq!(st.input, "");
    assert_eq!(st.cursor_position, 0);
}

#[test]
fn adv_utf8_rtl_and_zero_width_chars() {
    let mut st = WorkstationState::default();

    // Arabic and Hebrew RTL strings
    let rtl_arabic = "مرحبا";
    let rtl_hebrew = "שלום";
    st.insert_str(rtl_arabic);
    st.insert_char(' ');
    st.insert_str(rtl_hebrew);

    let total_chars = st.input.chars().count();
    assert_eq!(st.cursor_position, total_chars);

    // Zero-width space (\u{200B}) insertion
    st.insert_char('\u{200B}');
    assert_eq!(st.cursor_position, total_chars + 1);

    // Home and End
    st.move_cursor_home();
    assert_eq!(st.cursor_position, 0);
    st.move_cursor_end();
    assert_eq!(st.cursor_position, total_chars + 1);

    // Clear
    while st.delete_backspace() {}
    assert!(st.input.is_empty());
}

#[test]
fn adv_utf8_stress_fuzz_operations() {
    let mut st = WorkstationState::default();
    let characters = [
        'a', 'Z', '9', ' ', '\n', 'é', 'ñ', 'ü', 'ç', 'ø', '中', '文', '日', '本', '語', '🚀',
        '🦀', '🔥', '🎉', '\u{0301}', '\u{200B}', '\u{200D}',
    ];

    // Pseudorandom deterministic walk
    let mut lcg: u64 = 0xDEADBEEF;
    let mut next_rand = || -> u64 {
        lcg = lcg
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        lcg
    };

    for _ in 0..2000 {
        let op = next_rand() % 9;
        match op {
            0 => {
                let c = characters[(next_rand() as usize) % characters.len()];
                st.insert_char(c);
            }
            1 => {
                st.delete_backspace();
            }
            2 => {
                st.delete_char();
            }
            3 => {
                st.move_cursor_left();
            }
            4 => {
                st.move_cursor_right();
            }
            5 => {
                st.move_cursor_home();
            }
            6 => {
                st.move_cursor_end();
            }
            7 => {
                st.move_cursor_word_left();
            }
            8 => {
                st.move_cursor_word_right();
            }
            _ => unreachable!(),
        }

        // Invariant: cursor_position must always be <= input.chars().count()
        let char_count = st.input.chars().count();
        assert!(
            st.cursor_position <= char_count,
            "Invariant violation: cursor {} > len {}",
            st.cursor_position,
            char_count
        );
        // Invariant: input must always be valid UTF-8 (guaranteed by Rust, but verify slice indexing)
        assert!(std::str::from_utf8(st.input.as_bytes()).is_ok());
    }
}

// ============================================================================
// 2. History Recall & Draft Preservation Adversarial Tests
// ============================================================================

#[test]
fn adv_history_draft_faithfully_restored() {
    let mut st = WorkstationState::default();
    st.history = vec!["echo 1".into(), "echo 2".into(), "echo 3".into()];

    let draft = "uncommitted WIP draft: analyze memory leaks";
    st.insert_str(draft);
    assert_eq!(st.input, draft);

    // Up recalls echo 3
    st.history_up();
    assert_eq!(st.input, "echo 3");
    assert_eq!(st.history_index, Some(2));

    // Up recalls echo 2
    st.history_up();
    assert_eq!(st.input, "echo 2");
    assert_eq!(st.history_index, Some(1));

    // Up recalls echo 1
    st.history_up();
    assert_eq!(st.input, "echo 1");
    assert_eq!(st.history_index, Some(0));

    // Up at top: stays at echo 1
    st.history_up();
    assert_eq!(st.input, "echo 1");
    assert_eq!(st.history_index, Some(0));

    // Down recalls echo 2
    st.history_down();
    assert_eq!(st.input, "echo 2");
    assert_eq!(st.history_index, Some(1));

    // Down recalls echo 3
    st.history_down();
    assert_eq!(st.input, "echo 3");
    assert_eq!(st.history_index, Some(2));

    // Down at bottom: RESTORES original uncommitted draft
    st.history_down();
    assert_eq!(st.input, draft);
    assert_eq!(st.history_index, None);

    // Extra down at bottom: stays at draft
    st.history_down();
    assert_eq!(st.input, draft);
    assert_eq!(st.history_index, None);
}

#[test]
fn adv_history_modified_item_then_restore_draft() {
    let mut st = WorkstationState::default();
    st.history = vec!["git log --oneline".into(), "cargo test -p meg-tui".into()];

    let draft = "meg run --fast";
    st.insert_str(draft);

    // Recall latest item
    st.history_up();
    assert_eq!(st.input, "cargo test -p meg-tui");
    assert_eq!(st.history_index, Some(1));

    // User modifies recalled item
    st.insert_str(" -- --nocapture");
    assert_eq!(st.input, "cargo test -p meg-tui -- --nocapture");

    // Recall previous item
    st.history_up();
    assert_eq!(st.input, "git log --oneline");
    assert_eq!(st.history_index, Some(0));

    // Down back through history
    st.history_down();
    assert_eq!(st.input, "cargo test -p meg-tui"); // original stored item recalled

    // Down to draft
    st.history_down();
    assert_eq!(st.input, draft); // Draft is STILL faithfully intact!
    assert_eq!(st.history_index, None);
}

#[test]
fn adv_history_esc_cancel_restores_draft() {
    let mut st = WorkstationState::default();
    st.history = vec!["python test.py".into(), "bun run build".into()];

    let draft = "custom build script";
    st.insert_str(draft);

    // Up into history
    st.history_up();
    assert_eq!(st.input, "bun run build");
    assert_eq!(st.history_index, Some(1));

    // Cancel history (Esc key)
    st.cancel_history();
    assert_eq!(st.input, draft);
    assert_eq!(st.history_index, None);

    // Cancel history when not browsing history does nothing
    st.cancel_history();
    assert_eq!(st.input, draft);
    assert_eq!(st.history_index, None);
}

#[test]
fn adv_history_empty_and_consecutive_duplicates() {
    let mut st = WorkstationState::default();

    // Up / down on empty history
    st.history_up();
    assert!(st.input.is_empty());
    assert_eq!(st.history_index, None);
    st.history_down();
    assert!(st.input.is_empty());
    assert_eq!(st.history_index, None);

    // Consecutive identical submissions deduplicated
    st.insert_str("ls -la");
    let cmd1 = st.commit_input();
    assert_eq!(cmd1, Some("ls -la".into()));
    assert_eq!(st.history.len(), 1);

    st.insert_str("ls -la");
    let cmd2 = st.commit_input();
    assert_eq!(cmd2, Some("ls -la".into()));
    assert_eq!(st.history.len(), 1); // Deduplicated!

    st.insert_str("different command");
    let cmd3 = st.commit_input();
    assert_eq!(cmd3, Some("different command".into()));
    assert_eq!(st.history.len(), 2);

    // Whitespace only submission is ignored
    st.insert_str("   \t  \n  ");
    assert_eq!(st.commit_input(), None);
    assert_eq!(st.history.len(), 2);
}

// ============================================================================
// 3. Mode Cycling Adversarial Tests
// ============================================================================

#[test]
fn adv_mode_bidirectional_cycling_and_state() {
    let mut st = WorkstationState::default();
    assert_eq!(st.mode, Mode::Build);
    assert_eq!(st.mode.label(), "BUILD");
    assert_eq!(st.mode.badge(), "[BUILD]");

    // 2026-11-20: two modes only (Act removed) — Tab toggles Build/Plan.
    for _ in 0..6 {
        st.cycle_mode();
        assert_eq!(st.mode, Mode::Plan);
        st.cycle_mode();
        assert_eq!(st.mode, Mode::Build);
    }
    for _ in 0..6 {
        st.cycle_mode_reverse();
        assert_eq!(st.mode, Mode::Plan);
        st.cycle_mode_reverse();
        assert_eq!(st.mode, Mode::Build);
    }

    // Distinct colors per mode
    let c_build = Mode::Build.color();
    let c_plan = Mode::Plan.color();
    assert_ne!(c_build, c_plan);
}

// ============================================================================
// 4. Hardware Cursor Positioning Coordinates Adversarial Tests
// ============================================================================

#[test]
fn adv_cursor_screen_position_hardware_coordinates() {
    let area = Rect::new(5, 10, 80, 5);
    let mut st = WorkstationState::default();

    // 1. Empty input
    // inner_input: x = 5 + 1 (left border) = 6, y = 10 + 1 (top border) = 11
    // empty input cursor = inner_input.x + 2 = 6 + 2 = 8, y = 11
    let (cx, cy) = cursor_screen_pos(area, &st);
    assert_eq!(cx, 8);
    assert_eq!(cy, 11);

    // 2. ASCII input: "test" (4 chars)
    st.insert_str("test");
    let (cx, cy) = cursor_screen_pos(area, &st);
    assert_eq!(cx, 8 + 4); // 12
    assert_eq!(cy, 11);

    // 3. Move cursor to index 2 (between 'e' and 's')
    st.cursor_position = 2;
    let (cx, cy) = cursor_screen_pos(area, &st);
    assert_eq!(cx, 8 + 2); // 10
    assert_eq!(cy, 11);
}

#[test]
fn adv_cursor_screen_position_multiline_and_cjk_width() {
    let area = Rect::new(0, 0, 80, 6);
    let mut st = WorkstationState::default();

    // CJK wide characters: "中文" (2 characters, but 4 display columns)
    st.insert_str("中文");
    // inner_input.x = 1. Left accent rail "▌ " = 2 cols. CJK width = 4.
    // cx = 1 + 2 + 4 = 7. cy = 1.
    let (cx, cy) = cursor_screen_pos(area, &st);
    assert_eq!(cx, 7);
    assert_eq!(cy, 1);

    // Move cursor between '中' (width 2) and '文'
    st.move_cursor_left();
    assert_eq!(st.cursor_position, 1);
    let (cx, cy) = cursor_screen_pos(area, &st);
    assert_eq!(cx, 1 + 2 + 2); // 5
    assert_eq!(cy, 1);

    // Multiline: "line 1\nline 2"
    st.input = "line 1\nline 2".into();
    st.cursor_position = 13; // at end of "line 2" (6 + 1 + 6 = 13)
    let (cx, cy) = cursor_screen_pos(area, &st);
    assert_eq!(cx, 1 + 2 + 6); // 9
    assert_eq!(cy, 1 + 1); // line 1 -> y = 2
}

#[test]
fn adv_cursor_screen_position_boundary_clamping() {
    let area = Rect::new(0, 0, 40, 4);
    let mut st = WorkstationState::default();

    // Input longer than container width (100 characters in 40-col area)
    let long_text = "a".repeat(100);
    st.insert_str(&long_text);

    let (cx, cy) = cursor_screen_pos(area, &st);
    // inner_input: x = 1, width = 38, right() = 39.
    // Clamped to inner_input.right() - 1 = 38.
    assert!(
        cx <= 38,
        "Hardware cursor X ({cx}) must be clamped inside inner width"
    );
    assert!(
        cy <= 2,
        "Hardware cursor Y ({cy}) must be clamped inside input area"
    );

    // Height < 3 fallback clamping
    let short_area = Rect::new(0, 0, 40, 1);
    let (scx, scy) = cursor_screen_pos(short_area, &st);
    assert!(
        scx <= 39,
        "Short area cursor X must not exceed area.right - 1"
    );
    assert_eq!(scy, 0, "Short area cursor Y must match area.y");
}

#[test]
fn adv_render_composer_hardware_cursor_integration() {
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut st = WorkstationState::default();
    st.insert_str("cargo check");

    terminal
        .draw(|f| {
            let area = Rect::new(0, 15, 80, 5);
            render_composer(f, area, &st, &DARK_SLATE);
        })
        .unwrap();

    // Hardware cursor: x = 0 + border(1) + rail(2) + "cargo check"(11) = 14, y = 15 + border(1) = 16
    terminal
        .backend_mut()
        .assert_cursor_position(Position::new(14, 16));
}
