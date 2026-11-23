//! Adversarial Theme Engine & ANSI Hygiene Stress Test Suite — Challenger 2
//!
//! Empirical stress tests:
//! 1. Live preview theme transitions: rapid theme cycling (hundreds of cycles),
//!    Esc cancel restoring initial snapshot without corrupting composer input.
//! 2. Modal selection commit, re-opening with new snapshot, and filtering edge cases.
//! 3. Hostile ANSI escape inputs: malformed escapes, truncated sequences, raw CSI cursor
//!    movements, OSC 8 hyperlinks, OSC window titles, SGR high-intensity colors.
//! 4. Full buffer cell inspection across all 5 themes: zero ANSI escape bytes (`\x1b` or `?[`)
//!    leak into Ratatui buffer cells across all 5 palettes at multiple terminal dimensions.

use meg_tui::{
    draw, sanitize_to_spans, strip_ansi, WorkstationState, ALL_PALETTES, DARK_SLATE, PHOSPHOR,
    TOKYO_NIGHT,
};
use ratatui::{
    backend::TestBackend,
    buffer::Buffer,
    style::{Color, Modifier, Style},
    Terminal,
};

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

fn assert_strict_no_ansi_leaks(buf: &Buffer, context: &str) {
    let area = buf.area();
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            let symbol = buf[(x, y)].symbol();
            assert!(
                !symbol.contains('\x1b'),
                "[{}] ANSI escape byte '\\x1b' leaked into buffer cell at ({}, {}): {:?}",
                context,
                x,
                y,
                symbol
            );
            assert!(
                !symbol.contains("?["),
                "[{}] Leaked ESC-bracket replacement '?[' detected at ({}, {}): {:?}",
                context,
                x,
                y,
                symbol
            );
            assert!(
                !symbol.contains("^["),
                "[{}] Leaked caret escape indicator '^[' detected at ({}, {}): {:?}",
                context,
                x,
                y,
                symbol
            );
            // Verify no raw control characters below 0x20 in printable symbol
            for ch in symbol.chars() {
                if (ch as u32) < 0x20 && ch != ' ' {
                    panic!(
                        "[{}] Raw control character 0x{:02X} detected in cell at ({}, {}): {:?}",
                        context, ch as u32, x, y, symbol
                    );
                }
            }
        }
    }

    let lines = buffer_to_lines(buf);
    for (i, line) in lines.iter().enumerate() {
        assert!(
            !line.contains('\x1b'),
            "[{}] Reconstructed line {} contains '\\x1b': {}",
            context,
            i,
            line
        );
        assert!(
            !line.contains("?["),
            "[{}] Reconstructed line {} contains '?[': {}",
            context,
            i,
            line
        );
        assert!(
            !line.contains("^["),
            "[{}] Reconstructed line {} contains '^[': {}",
            context,
            i,
            line
        );
    }
}

// ============================================================================
// 1. Live Preview Theme Transitions & Snapshot Restoration
// ============================================================================

#[test]
fn test_adv_live_preview_rapid_theme_cycling() {
    let mut st = WorkstationState::default();
    let initial_input = "const value = compute_hash(&seed); // in-progress code draft";
    st.insert_str(initial_input);
    st.cursor_position = 18;
    st.theme = DARK_SLATE;

    // Open theme picker
    st.open_theme_picker();
    assert!(st.has_active_modal());
    assert_eq!(st.theme_snapshot, Some(DARK_SLATE));
    assert_eq!(st.input, initial_input);
    assert_eq!(st.cursor_position, 18);

    // Rapidly cycle down 250 times
    for cycle in 0..250 {
        st.active_modal_move_down();
        // Input and cursor must NEVER be mutated during modal navigation
        assert_eq!(
            st.input, initial_input,
            "Input corrupted during move_down at cycle {}",
            cycle
        );
        assert_eq!(
            st.cursor_position, 18,
            "Cursor position corrupted during move_down at cycle {}",
            cycle
        );
        assert!(st.has_active_modal());
        assert_eq!(st.theme_snapshot, Some(DARK_SLATE));
        // Active theme must be one of the 5 palettes
        assert!(ALL_PALETTES.contains(&st.theme));
    }

    // Rapidly cycle up 125 times
    for cycle in 0..125 {
        st.active_modal_move_up();
        assert_eq!(
            st.input, initial_input,
            "Input corrupted during move_up at cycle {}",
            cycle
        );
        assert_eq!(
            st.cursor_position, 18,
            "Cursor position corrupted during move_up at cycle {}",
            cycle
        );
        assert!(ALL_PALETTES.contains(&st.theme));
    }

    // Cancel modal via Esc simulation
    st.cancel_active_modal();
    assert!(!st.has_active_modal());
    assert_eq!(st.theme_snapshot, None);
    // Theme MUST be exactly restored to DARK_SLATE
    assert_eq!(st.theme, DARK_SLATE);
    // Composer input and cursor MUST be completely untouched
    assert_eq!(st.input, initial_input);
    assert_eq!(st.cursor_position, 18);
}

#[test]
fn test_adv_live_preview_commit_and_reopen() {
    let mut st = WorkstationState::default();
    st.theme = DARK_SLATE;
    st.input = "draft for commit test".into();
    st.cursor_position = 5;

    // Open theme picker
    st.open_theme_picker();
    assert_eq!(st.theme_snapshot, Some(DARK_SLATE));

    // Navigate to Tokyo Night
    while st.theme.name != "Tokyo Night" {
        st.active_modal_move_down();
    }
    assert_eq!(st.theme, TOKYO_NIGHT);

    // Commit selection via Enter simulation
    st.commit_active_modal();
    assert!(!st.has_active_modal());
    assert_eq!(st.theme_snapshot, None);
    assert_eq!(st.theme, TOKYO_NIGHT);
    assert_eq!(st.input, "draft for commit test");
    assert_eq!(st.cursor_position, 5);
    assert!(
        st.transcript
            .iter()
            .any(|l| l.contains("Theme switched to Tokyo Night")),
        "Transcript should log switch"
    );

    // Re-open theme picker: new snapshot MUST be Tokyo Night (not the old Dark Slate!)
    st.open_theme_picker();
    assert_eq!(st.theme_snapshot, Some(TOKYO_NIGHT));

    // Navigate away to Phosphor
    while st.theme.name != "Phosphor" {
        st.active_modal_move_down();
    }
    assert_eq!(st.theme, PHOSPHOR);

    // Cancel: MUST restore to Tokyo Night!
    st.cancel_active_modal();
    assert!(!st.has_active_modal());
    assert_eq!(st.theme, TOKYO_NIGHT);
    assert_eq!(st.input, "draft for commit test");
}

#[test]
fn test_adv_live_preview_filter_and_empty_search() {
    let mut st = WorkstationState::default();
    st.theme = DARK_SLATE;

    st.open_theme_picker();

    // Type 'p' then 'h' -> "ph" uniquely matches Phosphor
    st.active_modal_type_char('p');
    st.active_modal_type_char('h');
    assert_eq!(st.theme, PHOSPHOR);

    // Backspace twice -> resets filter
    st.active_modal_backspace();
    st.active_modal_backspace();
    assert!(ALL_PALETTES.contains(&st.theme));

    // Type query with 0 matches
    for c in "nonexistent_theme_query_12345".chars() {
        st.active_modal_type_char(c);
    }
    // Theme should remain a valid palette without panicking
    assert!(ALL_PALETTES.contains(&st.theme));

    // Backspacing completely restores all items
    for _ in 0..30 {
        st.active_modal_backspace();
    }
    assert!(ALL_PALETTES.contains(&st.theme));

    st.cancel_active_modal();
    assert_eq!(st.theme, DARK_SLATE);
}

// ============================================================================
// 2. Hostile ANSI Escape Inputs
// ============================================================================

#[test]
fn test_adv_hostile_ansi_malformed_and_truncated() {
    let huge_param = format!("\x1b[{}mHugeParam", "9".repeat(1000));
    let test_cases = vec![
        // Bare escapes
        "\x1b",
        "\x1b[",
        "\x1b]",
        "\x1b[?",
        "\x1b[3",
        "\x1b[38",
        "\x1b[38;",
        "\x1b[38;5",
        "\x1b[38;5;",
        "\x1b[38;2",
        "\x1b[38;2;255",
        "\x1b[38;2;255;0",
        "\x1b[38;2;255;0;",
        // Chained / repeated escapes
        "\x1b\x1b",
        "\x1b\x1b[31mDoubleEscape",
        "\x1b[\x1b[32mNestedEscape",
        "\x1b[m",
        "\x1b[;m",
        "\x1b[;;;;;;m",
        "\x1b[0;0;0;0;0;0;0;0;0;0;0;0;0;0;0;0;0;0;0;0mLongEmptyParams",
        // Text with embedded truncated escape at tail
        "Prefix text \x1b",
        "Prefix text \x1b[",
        "Prefix text \x1b[31",
        // Massive parameter list to stress parser
        &huge_param,
        // Trailing semicolons and unexpected bytes
        "\x1b[1;2;3;4;5;6;7;8;9;mTrailingSemi",
        "\x1b[31@InvalidCommandByte",
        "\x1b[31`AnotherInvalidByte",
        // Unterminated OSC
        "\x1b]8;;https://crates.io",
        "\x1b]8;;https://crates.io unterminated text without bel or st",
        "\x1b]0;Title without terminator",
        // Empty OSC
        "\x1b]\x07",
        "\x1b]\x1b\\",
    ];

    for (idx, &case) in test_cases.iter().enumerate() {
        let stripped = strip_ansi(case);
        assert!(
            !stripped.contains('\x1b'),
            "strip_ansi leaked '\\x1b' on case #{}: {:?}",
            idx,
            case
        );
        assert!(
            !stripped.contains("?["),
            "strip_ansi leaked '?[' on case #{}: {:?}",
            idx,
            case
        );

        let spans = sanitize_to_spans(case, Style::default());
        for span in &spans {
            assert!(
                !span.content.contains('\x1b'),
                "sanitize_to_spans leaked '\\x1b' in span on case #{}: {:?}",
                idx,
                case
            );
            assert!(
                !span.content.contains("?["),
                "sanitize_to_spans leaked '?[' in span on case #{}: {:?}",
                idx,
                case
            );
        }
    }
}

#[test]
fn test_adv_hostile_ansi_csi_cursor_movements() {
    let hostile_csi = vec![
        "\x1b[0K",     // Clear line from cursor to end
        "\x1b[1K",     // Clear line from cursor to start
        "\x1b[2K",     // Clear entire line
        "\x1b[0J",     // Clear display from cursor down
        "\x1b[1J",     // Clear display from cursor up
        "\x1b[2J",     // Clear full display
        "\x1b[3J",     // Clear scrollback
        "\x1b[10;20H", // Move cursor to (10, 20)
        "\x1b[10;20f", // Move cursor to (10, 20)
        "\x1b[5A",     // Move cursor up 5
        "\x1b[5B",     // Move cursor down 5
        "\x1b[5C",     // Move cursor right 5
        "\x1b[5D",     // Move cursor left 5
        "\x1b[5E",     // Move cursor to start of next line 5 down
        "\x1b[5F",     // Move cursor to start of prev line 5 up
        "\x1b[10G",    // Move cursor to col 10
        "\x1b[6n",     // DSR Device Status Report
        "\x1b[?25h",   // Show cursor
        "\x1b[?25l",   // Hide cursor
        "\x1b[?1049h", // Alternate screen buffer
        "\x1b[?1049l", // Normal screen buffer
        "\x1b[s",      // Save cursor
        "\x1b[u",      // Restore cursor
        "\x1b[20S",    // Scroll up 20 lines
        "\x1b[20T",    // Scroll down 20 lines
    ];

    for csi in hostile_csi {
        let input = format!("Pre{}Mid{}Post", csi, csi);
        let stripped = strip_ansi(&input);
        assert_eq!(
            stripped, "PreMidPost",
            "CSI sequence {:?} was not cleanly stripped",
            csi
        );

        let spans = sanitize_to_spans(&input, Style::default());
        let joined: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(
            joined, "PreMidPost",
            "CSI sequence {:?} leaked into styled spans",
            csi
        );
    }
}

#[test]
fn test_adv_hostile_ansi_osc_hyperlinks_and_titles() {
    // OSC 8 Hyperlink with BEL (\x07)
    let link_bel = "Check out \x1b]8;;https://github.com/astral-sh/uv\x07uv package manager\x1b]8;;\x07 for speed";
    assert_eq!(
        strip_ansi(link_bel),
        "Check out uv package manager for speed"
    );

    // OSC 8 Hyperlink with ST (\x1b\)
    let link_st = "Visit \x1b]8;;https://ratatui.rs\x1b\\Ratatui Documentation\x1b]8;;\x1b\\ now";
    assert_eq!(strip_ansi(link_st), "Visit Ratatui Documentation now");

    // OSC 0 and 2 Window Titles with BEL and ST
    let title_bel = "\x1b]0;Evil Window Title\x07Actual content line";
    assert_eq!(strip_ansi(title_bel), "Actual content line");

    let title_st = "\x1b]2;Compromised Window Title\x1b\\Secure line";
    assert_eq!(strip_ansi(title_st), "Secure line");

    // Combined consecutive OSC commands
    let complex_osc =
        "\x1b]0;Title 1\x07\x1b]2;Title 2\x1b\\\x1b]8;;https://meg.ai\x07MEG\x1b]8;;\x07 Engine";
    assert_eq!(strip_ansi(complex_osc), "MEG Engine");
}

#[test]
fn test_adv_ansi_sgr_full_palette_and_modifiers() {
    // Test standard 30..=37 foregrounds
    let fg_colors = [
        ("30", Color::Black),
        ("31", Color::Red),
        ("32", Color::Green),
        ("33", Color::Yellow),
        ("34", Color::Blue),
        ("35", Color::Magenta),
        ("36", Color::Cyan),
        ("37", Color::White),
    ];
    for (code, color) in fg_colors {
        let input = format!("\x1b[{}mColored\x1b[0m", code);
        let spans = sanitize_to_spans(&input, Style::default());
        assert_eq!(spans[0].content, "Colored");
        assert_eq!(spans[0].style.fg, Some(color));
    }

    // Test high-intensity 90..=97 bright foregrounds
    let bright_fg_colors = [
        ("90", Color::DarkGray),
        ("91", Color::LightRed),
        ("92", Color::LightGreen),
        ("93", Color::LightYellow),
        ("94", Color::LightBlue),
        ("95", Color::LightMagenta),
        ("96", Color::LightCyan),
        ("97", Color::White),
    ];
    for (code, color) in bright_fg_colors {
        let input = format!("\x1b[{}mBrightColored\x1b[0m", code);
        let spans = sanitize_to_spans(&input, Style::default());
        assert_eq!(spans[0].content, "BrightColored");
        assert_eq!(spans[0].style.fg, Some(color));
    }

    // Test standard 40..=47 backgrounds
    let bg_colors = [
        ("40", Color::Black),
        ("41", Color::Red),
        ("42", Color::Green),
        ("43", Color::Yellow),
        ("44", Color::Blue),
        ("45", Color::Magenta),
        ("46", Color::Cyan),
        ("47", Color::White),
    ];
    for (code, color) in bg_colors {
        let input = format!("\x1b[{}mBgColored\x1b[0m", code);
        let spans = sanitize_to_spans(&input, Style::default());
        assert_eq!(spans[0].content, "BgColored");
        assert_eq!(spans[0].style.bg, Some(color));
    }

    // Test high-intensity 100..=107 bright backgrounds
    let bright_bg_colors = [
        ("100", Color::DarkGray),
        ("101", Color::LightRed),
        ("102", Color::LightGreen),
        ("103", Color::LightYellow),
        ("104", Color::LightBlue),
        ("105", Color::LightMagenta),
        ("106", Color::LightCyan),
        ("107", Color::White),
    ];
    for (code, color) in bright_bg_colors {
        let input = format!("\x1b[{}mBrightBgColored\x1b[0m", code);
        let spans = sanitize_to_spans(&input, Style::default());
        assert_eq!(spans[0].content, "BrightBgColored");
        assert_eq!(spans[0].style.bg, Some(color));
    }

    // Test multi-attribute SGR: bold (1) + dim (2) + italic (3) + underline (4) + reverse (7) + bright fg (91) + bright bg (104)
    let combo = "\x1b[1;2;3;4;7;91;104mUltraStyled\x1b[0m";
    let spans = sanitize_to_spans(combo, Style::default());
    assert_eq!(spans[0].content, "UltraStyled");
    assert_eq!(spans[0].style.fg, Some(Color::LightRed));
    assert_eq!(spans[0].style.bg, Some(Color::LightBlue));
    assert!(spans[0].style.add_modifier.contains(Modifier::BOLD));
    assert!(spans[0].style.add_modifier.contains(Modifier::DIM));
    assert!(spans[0].style.add_modifier.contains(Modifier::ITALIC));
    assert!(spans[0].style.add_modifier.contains(Modifier::UNDERLINED));
    assert!(spans[0].style.add_modifier.contains(Modifier::REVERSED));
}

// ============================================================================
// 3. Buffer Cell Leak Verification Across All 5 Themes
// ============================================================================

#[test]
fn test_adv_buffer_zero_ansi_leak_across_all_five_themes() {
    let hostile_transcript = vec![
        "> \x1b[31;1mUser hostile command\x1b[0m with \x1b[?25l cursor hide".to_string(),
        "✓ \x1b]8;;https://example.com\x07Completed Step with Hyperlink\x1b]8;;\x07 \x1b[2K\x1b[10;20H".to_string(),
        "Agent stream: \x1b[38;2;255;0;0mRGB sequence\x1b[m and \x1b[1;92mBright Green\x1b[0m and \x1b]0;Injected Title\x07Done\x1b".to_string(),
        "Truncated escape test: \x1b[31; and \x1b[ and end \x1b".to_string(),
        "Nested styles: \x1b[1m\x1b[4m\x1b[33mBold Underline Yellow\x1b[0m normal text".to_string(),
    ];

    let dimensions = [(80, 24), (120, 30), (60, 15)];

    for &pal in ALL_PALETTES {
        for &(width, height) in &dimensions {
            let backend = TestBackend::new(width, height);
            let mut terminal = Terminal::new(backend).unwrap();

            let mut st = WorkstationState::default();
            st.project = "meg-tui-adversarial".into();
            st.model = "qwen2.5-coder:7b".into();
            st.theme = pal;
            st.transcript = hostile_transcript.clone();
            st.input = "let x = hostile_filter_test();".into();
            st.plan_steps = vec![
                (true, "Setup completed step".into()),
                (false, "Run test suite".into()),
            ];
            st.status = vec![
                ("STATUS".into(), "ONLINE READY".into()),
                ("LOAD".into(), "0.42".into()),
            ];

            terminal
                .draw(|f| {
                    draw(f, &st.theme, &st);
                })
                .unwrap();

            let buffer = terminal.backend().buffer().clone();
            let context_label = format!("Theme={} Dims={}x{}", pal.name, width, height);

            // Assert zero escape leaks in buffer cells and lines
            assert_strict_no_ansi_leaks(&buffer, &context_label);

            // Both the compact wordmark and full logo use the selected theme.
            let header = meg_tui::DashboardLayout::new(buffer.area, &st.input).header;
            if !header.is_empty() {
                let logo_y = header.y + u16::from(header.height >= 4);
                let cell_1_1 = &buffer[(header.x + 1, logo_y)];
                assert_ne!(cell_1_1.symbol(), " ", "must inspect a logo glyph");
                assert_eq!(
                    cell_1_1.fg, pal.titlebar_fg,
                    "[{}] Header banner color mismatch",
                    context_label
                );
            }
        }
    }
}

#[test]
fn test_adv_modal_active_full_draw_buffer_inspection_all_themes() {
    let dimensions = [(80, 24), (100, 28)];

    for &pal in ALL_PALETTES {
        for &(width, height) in &dimensions {
            let backend = TestBackend::new(width, height);
            let mut terminal = Terminal::new(backend).unwrap();

            let mut st = WorkstationState::default();
            st.theme = pal;
            st.open_theme_picker();

            // Type some search characters
            st.active_modal_type_char('d');
            st.active_modal_type_char('r');

            terminal
                .draw(|f| {
                    draw(f, &st.theme, &st);
                })
                .unwrap();

            let buffer = terminal.backend().buffer().clone();
            let context_label = format!("ActiveModal Theme={} Dims={}x{}", pal.name, width, height);

            assert_strict_no_ansi_leaks(&buffer, &context_label);

            let lines = buffer_to_lines(&buffer);
            assert!(
                lines.iter().any(|l| l.contains("Switch Theme")),
                "[{}] Modal title must be visible",
                context_label
            );
            assert!(
                lines.iter().any(|l| l.contains("Search:")),
                "[{}] Modal search bar must be visible",
                context_label
            );
        }
    }
}

#[test]
fn test_adv_ansi_leak_in_plan_steps_and_status() {
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();

    let mut st = WorkstationState::default();
    st.theme = DARK_SLATE;
    st.plan_steps = vec![
        (
            true,
            "Setup \x1b[32mOK\x1b[0m \x1b]8;;url\x07Link\x1b]8;;\x07".into(),
        ),
        (false, "Run \x1b[31mFAIL\x1b[0m \x1b[?25h".into()),
    ];
    st.status = vec![
        ("STATUS".into(), "ONLINE \x1b[1;32mREADY\x1b[0m".into()),
        ("LOAD".into(), "0.42 \x1b[?25l".into()),
    ];

    terminal
        .draw(|f| {
            draw(f, &st.theme, &st);
        })
        .unwrap();

    let buffer = terminal.backend().buffer().clone();
    assert_strict_no_ansi_leaks(&buffer, "PlanSteps and Status Panes");
}

#[test]
fn test_adv_ansi_leak_in_composer_input() {
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();

    let mut st = WorkstationState::default();
    st.theme = DARK_SLATE;
    st.input = "echo \x1b[31mEvil\x1b[0m".into();

    terminal
        .draw(|f| {
            draw(f, &st.theme, &st);
        })
        .unwrap();

    let buffer = terminal.backend().buffer().clone();
    assert_strict_no_ansi_leaks(&buffer, "Composer Input");
}
