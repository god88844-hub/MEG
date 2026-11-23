//! E2E Tests for Feature 4 (F4): Theme Engine & ANSI Hygiene
//!
//! Covers Tier 1 (Feature Coverage) and Tier 2 (Boundary & Corner Cases).
//! 100% offline, zero live API calls, asserting on Ratatui TestBackend buffers.

mod common;

use common::*;
use ratatui::layout::Rect;
use ratatui::style::Style;

// ============================================================================
// Tier 1: Feature Coverage (Category-Partition)
// ============================================================================

#[test]
fn t1_theme_five_palettes_exist() {
    assert_eq!(ALL_PALETTES.len(), 5);

    let names: Vec<&str> = ALL_PALETTES.iter().map(|p| p.name).collect();
    assert!(names.contains(&"Classic"));
    assert!(names.contains(&"Phosphor"));
    assert!(names.contains(&"Dark Slate"));
    assert!(names.contains(&"Dracula"));
    assert!(names.contains(&"Tokyo Night"));

    // Verify all palettes have distinct background colors
    for i in 0..ALL_PALETTES.len() {
        for j in (i + 1)..ALL_PALETTES.len() {
            assert_ne!(
                ALL_PALETTES[i].viewport_bg, ALL_PALETTES[j].viewport_bg,
                "Palettes {} and {} must differ in viewport_bg",
                ALL_PALETTES[i].name, ALL_PALETTES[j].name
            );
        }
    }
}

#[test]
fn t1_theme_palette_colors_applied() {
    let mut harness = TestHarness::new(80, 10);
    let state = ComposerState::default();

    // Render with Classic (light chrome)
    let buf_classic = harness.draw(|f| {
        let area = Rect::new(0, 0, 80, 5);
        render_composer(f, area, &state, &CLASSIC);
    });

    // Render with Tokyo Night (dark blue/neon chrome)
    let buf_tokyo = harness.draw(|f| {
        let area = Rect::new(0, 0, 80, 5);
        render_composer(f, area, &state, &TOKYO_NIGHT);
    });

    // Outer border color should reflect panel_border of each theme
    let border_classic = buf_classic[(0, 0)].fg;
    let border_tokyo = buf_tokyo[(0, 0)].fg;
    assert_eq!(border_classic, CLASSIC.panel_border);
    assert_eq!(border_tokyo, TOKYO_NIGHT.panel_border);
    assert_ne!(border_classic, border_tokyo);

    assert_no_ansi_leaks(&buf_classic);
    assert_no_ansi_leaks(&buf_tokyo);
}

#[test]
fn t1_ansi_sanitization_color() {
    // String containing ANSI color escape codes from external subprocess
    let raw_input = "\x1b[32m✔ Build successful\x1b[0m: \x1b[1;34m12 tests passed\x1b[0m";
    let stripped = strip_ansi(raw_input);
    assert_eq!(stripped, "✔ Build successful: 12 tests passed");
    assert!(!stripped.contains('\x1b'));

    // Test conversion to styled spans
    let spans = sanitize_to_spans(raw_input, Style::default());
    assert_eq!(spans.len(), 3);
    assert_eq!(spans[0].content, "✔ Build successful");
    assert_eq!(spans[1].content, ": ");
    assert_eq!(spans[2].content, "12 tests passed");

    for span in &spans {
        assert!(!span.content.contains('\x1b'));
    }
}

#[test]
fn t1_ansi_sanitization_cursor() {
    // Strings containing dangerous terminal control sequences:
    // clear line (\x1b[2K), cursor move (\x1b[10;20H), hide cursor (\x1b[?25l)
    let hostile_input = "\x1b[2K\x1b[10;20HHello\x1b[?25l World\x1b[?25h";
    let stripped = strip_ansi(hostile_input);
    assert_eq!(stripped, "Hello World");
    assert!(!stripped.contains('\x1b'));

    // Check with Ratatui TestBackend rendering
    let mut harness = TestHarness::new(80, 10);
    let buf = harness.draw(|f| {
        let spans = sanitize_to_spans(hostile_input, Style::default());
        let para = ratatui::widgets::Paragraph::new(ratatui::text::Line::from(spans));
        f.render_widget(para, Rect::new(0, 0, 80, 1));
    });

    let line0 = buffer_line(&buf, 0);
    assert!(line0.contains("Hello World"));
    assert_no_ansi_leaks(&buf);
}

#[test]
fn t1_theme_live_preview() {
    let mut current_palette = DARK_SLATE;
    let mut harness = TestHarness::new(80, 10);
    let state = HeroState::default();

    // Render frame 1 in initial palette
    let buf1 = harness.draw(|f| {
        let area = Rect::new(0, 0, 80, 5);
        render_hero(f, area, &state, &current_palette);
    });
    assert_eq!(buf1[(0, 0)].fg, DARK_SLATE.panel_border);

    // Live preview event: user highlights Phosphor
    current_palette = PHOSPHOR;
    let buf2 = harness.draw(|f| {
        let area = Rect::new(0, 0, 80, 5);
        render_hero(f, area, &state, &current_palette);
    });
    assert_eq!(buf2[(0, 0)].fg, PHOSPHOR.panel_border);
    assert_ne!(buf1[(0, 0)].fg, buf2[(0, 0)].fg);

    assert_no_ansi_leaks(&buf1);
    assert_no_ansi_leaks(&buf2);
}

// ============================================================================
// Tier 2: Boundary & Corner Cases (Boundary Value Analysis)
// ============================================================================

#[test]
fn t2_ansi_malformed_escapes() {
    // Malformed, incomplete, or truncated escape sequences
    let truncated = "Testing \x1b[38;5; and truncated \x1b[ and end \x1b";
    let cleaned = strip_ansi(truncated);
    assert!(!cleaned.contains('\x1b'));
    assert!(cleaned.contains("Testing"));
    assert!(cleaned.contains("and truncated"));

    let spans = sanitize_to_spans(truncated, Style::default());
    for s in &spans {
        assert!(!s.content.contains('\x1b'));
    }
}

#[test]
fn t2_ansi_nested_sgr_codes() {
    // Heavily nested and chained SGR codes: bold + underline + color + reset
    let complex_sgr = "\x1b[1m\x1b[4m\x1b[31mRed Bold Underline\x1b[0m\x1b[32m Green Normal\x1b[0m";
    let cleaned = strip_ansi(complex_sgr);
    assert_eq!(cleaned, "Red Bold Underline Green Normal");

    let spans = sanitize_to_spans(complex_sgr, Style::default());
    assert_eq!(spans[0].content, "Red Bold Underline");
    assert_eq!(spans[1].content, " Green Normal");
}

#[test]
fn t2_theme_invalid_palette_name() {
    // Requesting unknown palette falls back to default DARK_SLATE safely
    let p1 = get_palette_by_name("unknown-palette");
    assert_eq!(p1.name, "Dark Slate");

    let p2 = get_palette_by_name("solarized-invalid");
    assert_eq!(p2.name, "Dark Slate");

    // Case-insensitivity
    let p3 = get_palette_by_name("dRaCuLa");
    assert_eq!(p3.name, "Dracula");

    let p4 = get_palette_by_name("phosphor");
    assert_eq!(p4.name, "Phosphor");
}

#[test]
fn t2_theme_extreme_contrast() {
    // Phosphor: high-contrast green CRT (fg must not match bg)
    assert_ne!(PHOSPHOR.viewport_bg, PHOSPHOR.viewport_fg);
    assert_ne!(PHOSPHOR.chrome_bg, PHOSPHOR.chrome_fg);

    // Classic: high-contrast retro white/black
    assert_ne!(CLASSIC.viewport_bg, CLASSIC.viewport_fg);
    assert_ne!(CLASSIC.titlebar_bg, CLASSIC.titlebar_fg);
}

#[test]
fn t2_ansi_osc_hyperlinks() {
    // OSC 8 hyperlink sequences: \x1b]8;;http://example.com\x07Link Text\x1b]8;;\x07
    let osc_bel = "Visit \x1b]8;;https://github.com/meg\x07MEG Repository\x1b]8;;\x07 for details";
    let cleaned_bel = strip_ansi(osc_bel);
    assert_eq!(cleaned_bel, "Visit MEG Repository for details");
    assert!(!cleaned_bel.contains('\x1b'));

    // OSC 8 with ST (\x1b\) terminator
    let osc_st = "Docs: \x1b]8;;https://docs.meg.local\x1b\\Documentation\x1b]8;;\x1b\\ here";
    let cleaned_st = strip_ansi(osc_st);
    assert_eq!(cleaned_st, "Docs: Documentation here");
    assert!(!cleaned_st.contains('\x1b'));
}
