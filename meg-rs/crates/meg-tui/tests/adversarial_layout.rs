//! Adversarial Layout & Extreme Dimension Stress Harness for `meg-tui`.
//!
//! Rigorous stress testing against:
//! 1. Extreme terminal dimensions: 0x0, 1x1, 10x1, 51x4, 52x5, 80x1, 80x2, 200x50, 0x10, 10x0.
//! 2. Height-1 and Height-2 composer: verifies borders are not crushed, text is visible.
//! 3. Hero card responsive fallback at <52 cols and <5 rows.
//! 4. Chip bar responsive truncation under narrow viewports.
//! 5. Full `draw()` skeleton rendering under all dimension permutations.

use meg_tui::{
    cursor_screen_pos, draw, render_chip_bar, render_composer, render_hero,
    render_viewport, WorkstationState, DARK_SLATE,
};
use ratatui::{backend::TestBackend, buffer::Buffer, layout::Rect, Terminal};

fn buffer_to_lines(buf: &Buffer) -> Vec<String> {
    let mut lines = Vec::new();
    let area = buf.area();
    for y in area.top()..area.bottom() {
        let mut line_str = String::new();
        for x in area.left()..area.right() {
            line_str.push_str(buf[(x, y)].symbol());
        }
        lines.push(line_str);
    }
    lines
}

fn create_terminal(width: u16, height: u16) -> Terminal<TestBackend> {
    let backend = TestBackend::new(width, height);
    Terminal::new(backend).expect("Failed to initialize TestBackend")
}

// ============================================================================
// 1. Extreme Terminal Dimensions
// ============================================================================

#[test]
fn test_extreme_dimensions_full_draw() {
    let test_dimensions = [
        (0, 0),
        (1, 1),
        (10, 1),
        (51, 4),
        (52, 5),
        (80, 1),
        (80, 2),
        (200, 50),
        (0, 10),
        (10, 0),
        (51, 5),
        (52, 4),
    ];

    let state = WorkstationState::default();
    let pal = DARK_SLATE;

    for &(w, h) in &test_dimensions {
        let mut terminal = create_terminal(w, h);
        let res = terminal.draw(|f| {
            draw(f, &pal, &state);
        });
        assert!(
            res.is_ok(),
            "Full draw() failed on dimension {}x{}: {:?}",
            w, h, res.err()
        );
    }
}

#[test]
fn test_extreme_dimensions_hero_direct() {
    let test_dimensions = [
        (0, 0),
        (1, 1),
        (10, 1),
        (51, 4),
        (52, 5),
        (80, 1),
        (80, 2),
        (200, 50),
    ];

    let state = WorkstationState::default();
    let pal = DARK_SLATE;

    for &(w, h) in &test_dimensions {
        let mut terminal = create_terminal(w, h);
        let res = terminal.draw(|f| {
            render_hero(f, Rect::new(0, 0, w, h), &state, &pal);
        });
        assert!(
            res.is_ok(),
            "render_hero_state failed on dimension {}x{}: {:?}",
            w, h, res.err()
        );
    }
}

#[test]
fn test_extreme_dimensions_composer_direct() {
    let test_dimensions = [
        (0, 0),
        (1, 1),
        (10, 1),
        (51, 4),
        (52, 5),
        (80, 1),
        (80, 2),
        (200, 50),
    ];

    let mut state = WorkstationState::default();
    state.input = "adversarial test string".to_string();
    state.cursor_position = 5;
    let pal = DARK_SLATE;

    for &(w, h) in &test_dimensions {
        let mut terminal = create_terminal(w, h);
        let res = terminal.draw(|f| {
            render_composer(f, Rect::new(0, 0, w, h), &state, &pal);
        });
        assert!(
            res.is_ok(),
            "render_composer failed on dimension {}x{}: {:?}",
            w, h, res.err()
        );
    }
}

// ============================================================================
// 2. Height-1 and Height-2 Composer Verification
// ============================================================================

#[test]
fn test_height_1_composer_does_not_crush_borders_and_text_is_visible() {
    let mut terminal = create_terminal(80, 1);
    let mut state = WorkstationState::default();
    state.input = "find layout bugs".into();
    state.cursor_position = 16;
    let pal = DARK_SLATE;

    terminal
        .draw(|f| {
            render_composer(f, Rect::new(0, 0, 80, 1), &state, &pal);
        })
        .unwrap();

    let buf = terminal.backend().buffer().clone();
    let lines = buffer_to_lines(&buf);
    assert_eq!(lines.len(), 1);

    let row = &lines[0];
    // 1. Accent rail must be present
    assert!(
        row.contains('>'),
        "Height-1 composer must contain accent rail '>', got: {row:?}"
    );
    // 2. Text must be visible
    assert!(
        row.contains("find layout bugs"),
        "Height-1 composer must visibly display input text, got: {row:?}"
    );
    // 3. No box-drawing border characters should crush the single line
    assert!(
        !row.contains('╭') && !row.contains('─') && !row.contains('╰') && !row.contains('│'),
        "Height-1 composer must not render box borders that crush text, got: {row:?}"
    );
}

#[test]
fn test_height_1_composer_placeholder_visible() {
    let mut terminal = create_terminal(80, 1);
    let state = WorkstationState::default(); // empty input
    let pal = DARK_SLATE;

    terminal
        .draw(|f| {
            render_composer(f, Rect::new(0, 0, 80, 1), &state, &pal);
        })
        .unwrap();

    let buf = terminal.backend().buffer().clone();
    let lines = buffer_to_lines(&buf);
    let row = &lines[0];

    assert!(
        row.contains('>'),
        "Expected accent rail in empty height-1 composer, got: {row:?}"
    );
    assert!(
        row.contains("Ask MEG anything"),
        "Expected placeholder text in empty height-1 composer, got: {row:?}"
    );
}

#[test]
fn test_height_2_composer_borderless_band() {
    let mut terminal = create_terminal(80, 2);
    let mut state = WorkstationState::default();
    state.input = "line 1".into();
    let pal = DARK_SLATE;

    terminal
        .draw(|f| {
            render_composer(f, Rect::new(0, 0, 80, 2), &state, &pal);
        })
        .unwrap();

    let buf = terminal.backend().buffer().clone();
    let lines = buffer_to_lines(&buf);
    assert_eq!(lines.len(), 2);

    // Height 2 is still < 3, so defensive borderless band applies
    let row0 = &lines[0];
    assert!(
        row0.contains('>') && row0.contains("line 1"),
        "Height-2 composer row 0 must contain rail and input: {row0:?}"
    );
    assert!(
        !row0.contains('╭'),
        "Height-2 composer must not contain rounded top border"
    );
}

#[test]
fn test_height_1_composer_cursor_position() {
    let mut state = WorkstationState::default();
    state.input = "test".into();
    state.cursor_position = 3;

    let area = Rect::new(0, 0, 80, 1);
    let (cx, cy) = cursor_screen_pos(area, &state);

    // In height < 3: cursor_x = (area.x + 3 + cursor_position).min(right - 1)
    // 0 + 3 + 3 = 6
    assert_eq!(cx, 6);
    assert_eq!(cy, 0);

    // Extreme narrow terminal cursor bound check
    let narrow_area = Rect::new(0, 0, 5, 1);
    let (ncx, ncy) = cursor_screen_pos(narrow_area, &state);
    assert_eq!(ncx, 4); // area.right() is 5, right - 1 = 4
    assert_eq!(ncy, 0);
}

// ============================================================================
// 3. Hero Card Responsive Fallback at <52 cols
// ============================================================================

#[test]
fn test_hero_responsive_fallback_exact_boundary() {
    let pal = DARK_SLATE;
    let state = WorkstationState::default();

    // 1. At 51 cols the banner (43 cols) fits: blocks render after the spacer.
    {
        let mut term = create_terminal(51, 5);
        term.draw(|f| {
            render_hero(f, Rect::new(0, 0, 51, 5), &state, &pal);
        })
        .unwrap();
        let buf = term.backend().buffer().clone();
        let lines = buffer_to_lines(&buf);
        assert!(
            lines.join("
").contains('█'),
            "51 cols must render banner blocks, got: {:?}",
            &lines[..3.min(lines.len())]
        );
    }

    // 2. Below 30 cols the hero renders nothing (no clipped garbage).
    {
        let mut term = create_terminal(10, 1);
        term.draw(|f| {
            render_hero(f, Rect::new(0, 0, 10, 1), &state, &pal);
        })
        .unwrap();
        let buf = term.backend().buffer().clone();
        let lines = buffer_to_lines(&buf);
        assert!(
            lines[0].trim().is_empty(),
            "Hero must render nothing at 10 cols, got: {:?}",
            lines[0]
        );
    }
}

#[test]
fn test_hero_extreme_narrow_rendering() {
    let pal = DARK_SLATE;
    let state = WorkstationState::default();

    // 10x1 terminal
    let mut term = create_terminal(10, 1);
    term.draw(|f| {
        render_hero(f, Rect::new(0, 0, 10, 1), &state, &pal);
    })
    .unwrap();
    let buf = term.backend().buffer().clone();
    let lines = buffer_to_lines(&buf);
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].chars().count(), 10);
    // Below 30 cols the hero renders nothing (no clipped garbage).
    assert!(
        lines[0].trim().is_empty(),
        "Hero must render nothing at 10 cols, got: {:?}",
        lines[0]
    );

    // 1x1 terminal
    let mut term1 = create_terminal(1, 1);
    term1
        .draw(|f| {
            render_hero(f, Rect::new(0, 0, 1, 1), &state, &pal);
        })
        .unwrap();
    let buf1 = term1.backend().buffer().clone();
    let lines1 = buffer_to_lines(&buf1);
    assert_eq!(lines1.len(), 1);
    assert_eq!(lines1[0].chars().count(), 1);
}

// ============================================================================
// 4. Chips Bar Responsive Truncation
// ============================================================================

#[test]
fn test_chips_bar_responsive_truncation() {
    let pal = DARK_SLATE;
    let state = WorkstationState::default();

    // Extra Wide (100 cols) -> full hints fit comfortably
    {
        let mut term = create_terminal(100, 1);
        term.draw(|f| {
            render_chip_bar(f, Rect::new(0, 0, 100, 1), &state, &pal);
        })
        .unwrap();
        let lines = buffer_to_lines(term.backend().buffer());
        assert!(lines[0].contains("[BUILD]"), "Must contain mode badge");
        assert!(
            lines[0].contains("? help · / cmds · Tab mode · Ctrl+C quit"),
            "Extra wide chip bar must contain full shortcut hints, got: {:?}",
            lines[0]
        );
        assert!(lines[0].contains('╯'), "Must contain right corner");
    }

    // Standard 80 cols -> compact hints fit (left spans ~44 chars + compact ~16 chars = 60 chars < 80)
    {
        let mut term = create_terminal(80, 1);
        term.draw(|f| {
            render_chip_bar(f, Rect::new(0, 0, 80, 1), &state, &pal);
        })
        .unwrap();
        let lines = buffer_to_lines(term.backend().buffer());
        assert!(lines[0].contains("[BUILD]"));
        assert!(
            lines[0].contains("? help · / cmds"),
            "80 cols chip bar must contain compact shortcut hints, got: {:?}",
            lines[0]
        );
        assert!(lines[0].contains('╯'), "Must contain right corner");
    }

    // Narrow 40 cols -> hints omitted when width < left_spans + compact_hints
    {
        let mut term = create_terminal(40, 1);
        term.draw(|f| {
            render_chip_bar(f, Rect::new(0, 0, 40, 1), &state, &pal);
        })
        .unwrap();
        let lines = buffer_to_lines(term.backend().buffer());
        assert!(lines[0].contains("[BUILD]"));
        assert!(
            !lines[0].contains("? help"),
            "Narrow chip bar must omit hints when space is tight, got: {:?}",
            lines[0]
        );
    }

    // Extremely narrow (10 cols, 1 col) -> no panic
    for &w in &[10, 5, 2, 1] {
        let mut term = create_terminal(w, 1);
        let res = term.draw(|f| {
            render_chip_bar(f, Rect::new(0, 0, w, 1), &state, &pal);
        });
        assert!(res.is_ok(), "Chip bar crashed on width {w}");
    }
}

// ============================================================================
// 5. Viewport ANSI and Auto-scroll Stress
// ============================================================================

#[test]
fn test_viewport_extreme_dimensions() {
    let pal = DARK_SLATE;
    let mut state = WorkstationState::default();
    state.transcript.push("❯ prompt line 1".into());
    state.transcript.push("✓ step completed".into());
    state.transcript.push("\x1b[32mGreen output line\x1b[0m".into());

    for &(w, h) in &[(0, 0), (1, 1), (10, 1), (50, 2), (80, 10)] {
        let mut term = create_terminal(w, h);
        let res = term.draw(|f| {
            render_viewport(f, Rect::new(0, 0, w, h), &state, &pal);
        });
        assert!(res.is_ok(), "Viewport crashed on dimension {}x{}", w, h);
    }
}

#[test]
fn test_render_hero_wrapper() {
    let state = WorkstationState::default();
    let pal = DARK_SLATE;
    let mut term = create_terminal(80, 5);
    let res = term.draw(|f| {
        render_hero(f, Rect::new(0, 0, 80, 5), &state, &pal);
    });
    assert!(res.is_ok());
}


// ============================================================================
// 6. Adversarial Stress & Chaos Inputs
// ============================================================================

#[test]
fn test_adversarial_massive_multiline_input() {
    let mut state = WorkstationState::default();
    // 100 lines with varying lengths
    let mut huge_text = String::new();
    for i in 0..100 {
        huge_text.push_str(&format!("Line {i}: {}\n", "x".repeat(i * 3)));
    }
    state.input = huge_text;
    state.cursor_position = 500;
    let pal = DARK_SLATE;

    // Test across normal and constrained terminals
    for &(w, h) in &[(80, 24), (80, 1), (10, 5), (51, 4)] {
        let mut term = create_terminal(w, h);
        let res = term.draw(|f| {
            draw(f, &pal, &state);
        });
        assert!(res.is_ok(), "Failed to draw massive multiline at {}x{}", w, h);
    }
}

#[test]
fn test_adversarial_zwj_and_combining_unicode_cursor() {
    let mut state = WorkstationState::default();
    // Complex unicode sequences: ZWJ emoji, combining accents, RTL markers
    state.input = "MEG \u{1F469}\u{200D}\u{1F4BB} \u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}\u{200D}\u{1F466} e\u{0301} \u{200E}".into();
    let char_count = state.input.chars().count();

    // Verify cursor stepping across each unicode char boundary does not panic
    for pos in 0..=char_count {
        state.cursor_position = pos;
        let (cx, cy) = cursor_screen_pos(Rect::new(0, 0, 80, 5), &state);
        assert!(cx < 80, "Cursor x {cx} must not exceed width 80");
        assert!(cy < 5, "Cursor y {cy} must not exceed height 5");

        let (cx1, cy1) = cursor_screen_pos(Rect::new(0, 0, 80, 1), &state);
        assert!(cx1 < 80, "Height-1 cursor x {cx1} must not exceed width 80");
        assert_eq!(cy1, 0);
    }

    // Verify deletion of ZWJ characters does not panic
    state.cursor_position = char_count;
    while state.delete_backspace() {}
    assert!(state.input.is_empty());
}

#[test]
fn test_adversarial_rapid_terminal_resize_sequence() {
    let pal = DARK_SLATE;
    let state = WorkstationState::default();
    let sequence = [
        (80, 24),
        (0, 0),
        (1, 1),
        (200, 60),
        (10, 1),
        (51, 4),
        (52, 5),
        (80, 1),
        (0, 10),
        (10, 0),
        (80, 24),
    ];

    for &(w, h) in &sequence {
        let mut term = create_terminal(w, h);
        let res = term.draw(|f| {
            draw(f, &pal, &state);
        });
        assert!(res.is_ok(), "Resize draw failed at {}x{}", w, h);
    }
}

#[test]
fn test_adversarial_deep_transcript_scroll() {
    let pal = DARK_SLATE;
    let mut state = WorkstationState::default();
    for i in 0..2000 {
        state.transcript.push(format!("Transcript entry index {i} with some log details"));
    }

    let mut term = create_terminal(80, 24);
    let res = term.draw(|f| {
        render_viewport(f, Rect::new(0, 0, 80, 24), &state, &pal);
    });
    assert!(res.is_ok(), "Deep transcript render failed");
}
