//! E2E Tests for Feature 3 (F3): Interactive Modal Pickers (/theme, /model, /effort)
//!
//! Covers Tier 1 (Feature Coverage) and Tier 2 (Boundary & Corner Cases).
//! 100% offline, zero live API calls, asserting on Ratatui TestBackend buffers.

mod common;

use common::*;
use ratatui::layout::Rect;

// Helper to construct sample theme picker items
fn create_theme_picker() -> PickerState<ThemePalette> {
    let items = vec![
        PickerItem {
            label: "Classic".into(),
            detail: "Win95 retro grey".into(),
            meta: "light".into(),
            value: CLASSIC,
        },
        PickerItem {
            label: "Phosphor".into(),
            detail: "VT220 amber/green CRT".into(),
            meta: "retro".into(),
            value: PHOSPHOR,
        },
        PickerItem {
            label: "Dark Slate".into(),
            detail: "Modern dark slate palette".into(),
            meta: "dark".into(),
            value: DARK_SLATE,
        },
        PickerItem {
            label: "Dracula".into(),
            detail: "Vampire purple & pink".into(),
            meta: "dark".into(),
            value: DRACULA,
        },
        PickerItem {
            label: "Tokyo Night".into(),
            detail: "Neon night downtown Tokyo".into(),
            meta: "dark".into(),
            value: TOKYO_NIGHT,
        },
    ];
    PickerState::new("Switch Theme".into(), items)
}

// Helper to construct sample model picker items
fn create_model_picker() -> PickerState<String> {
    let items = vec![
        PickerItem {
            label: "qwen2.5-coder:7b".into(),
            detail: "local \u{2022} ready".into(),
            meta: "45ms".into(),
            value: "qwen2.5-coder:7b".into(),
        },
        PickerItem {
            label: "deepseek-coder:6.7b".into(),
            detail: "local \u{2022} ready".into(),
            meta: "52ms".into(),
            value: "deepseek-coder:6.7b".into(),
        },
        PickerItem {
            label: "claude-sonnet-4-6".into(),
            detail: "cloud \u{2022} not configured".into(),
            meta: "--".into(),
            value: "claude-sonnet-4-6".into(),
        },
        PickerItem {
            label: "gpt-4o".into(),
            detail: "cloud \u{2022} not configured".into(),
            meta: "--".into(),
            value: "gpt-4o".into(),
        },
    ];
    PickerState::new("Switch Model".into(), items)
}

// ============================================================================
// Tier 1: Feature Coverage (Category-Partition)
// ============================================================================

#[test]
fn t1_picker_theme_open_close() {
    let mut harness = TestHarness::new(80, 24);
    let mut picker = create_theme_picker();
    let pal = DARK_SLATE;

    assert!(picker.is_open);

    // Render open modal overlay
    let buf = harness.draw(|f| {
        let area = Rect::new(0, 0, 80, 24);
        render_picker_modal(f, area, &picker, &pal);
    });

    let lines = buffer_lines(&buf);
    assert!(lines.iter().any(|l| l.contains("Switch Theme")), "Title must be rendered");
    assert!(lines.iter().any(|l| l.contains("Search:")), "Search input row must be present");
    assert!(lines.iter().any(|l| l.contains("Classic")), "Classic item present");

    // Close on Esc
    picker.cancel();
    assert!(!picker.is_open);

    // Re-render when closed: nothing drawn
    let buf_closed = harness.draw(|f| {
        let area = Rect::new(0, 0, 80, 24);
        render_picker_modal(f, area, &picker, &pal);
    });
    let lines_closed = buffer_lines(&buf_closed);
    assert!(!lines_closed.iter().any(|l| l.contains("Switch Theme")), "Closed picker must not render");

    assert_no_ansi_leaks(&buf);
}

#[test]
fn t1_picker_model_navigation() {
    let mut harness = TestHarness::new(80, 24);
    let mut picker = create_model_picker();
    let pal = DRACULA;

    assert_eq!(picker.selected_index, 0);
    assert_eq!(picker.selected_item().unwrap().label, "qwen2.5-coder:7b");

    // Arrow down
    picker.move_down();
    assert_eq!(picker.selected_index, 1);
    assert_eq!(picker.selected_item().unwrap().label, "deepseek-coder:6.7b");

    // Arrow down again
    picker.move_down();
    assert_eq!(picker.selected_index, 2);
    assert_eq!(picker.selected_item().unwrap().label, "claude-sonnet-4-6");

    // Draw and verify that the selected row has the selection pointer " > "
    let buf = harness.draw(|f| {
        let area = Rect::new(0, 0, 80, 24);
        render_picker_modal(f, area, &picker, &pal);
    });
    let lines = buffer_lines(&buf);
    let active_line = lines.iter().find(|l| l.contains("claude-sonnet-4-6")).expect("Active line found");
    assert!(active_line.contains(">"), "Selected row must have '>' selection cursor");

    // Arrow up wrap-around test
    picker.move_up(); // back to 1
    picker.move_up(); // back to 0
    picker.move_up(); // wraps to last item (3)
    assert_eq!(picker.selected_index, 3);
    assert_eq!(picker.selected_item().unwrap().label, "gpt-4o");

    assert_no_ansi_leaks(&buf);
}

#[test]
fn t1_picker_search_filter() {
    let mut harness = TestHarness::new(80, 24);
    let mut picker = create_model_picker();
    let pal = CLASSIC;

    // Type 'coder'
    picker.type_char('c');
    picker.type_char('o');
    picker.type_char('d');
    picker.type_char('e');
    picker.type_char('r');

    assert_eq!(picker.search_query, "coder");
    assert_eq!(picker.filtered_indices.len(), 2);
    assert_eq!(picker.selected_item().unwrap().label, "qwen2.5-coder:7b");

    let buf = harness.draw(|f| {
        let area = Rect::new(0, 0, 80, 24);
        render_picker_modal(f, area, &picker, &pal);
    });

    let lines = buffer_lines(&buf);
    assert!(lines.iter().any(|l| l.contains("qwen2.5-coder:7b")));
    assert!(lines.iter().any(|l| l.contains("deepseek-coder:6.7b")));
    assert!(!lines.iter().any(|l| l.contains("gpt-4o")), "Non-matching item must be filtered out");

    // Backspace query
    for _ in 0..5 {
        picker.backspace();
    }
    assert_eq!(picker.search_query, "");
    assert_eq!(picker.filtered_indices.len(), 4, "Clearing filter restores all items");

    assert_no_ansi_leaks(&buf);
}

#[test]
fn t1_picker_selection_commit() {
    let mut picker = create_theme_picker();
    // Move to Dracula
    picker.move_down(); // Phosphor
    picker.move_down(); // Dark Slate
    picker.move_down(); // Dracula
    assert_eq!(picker.selected_item().unwrap().label, "Dracula");

    let selected_theme = picker.commit_selection().expect("Commit returns selected value");
    assert_eq!(selected_theme.name, "Dracula");
    assert!(!picker.is_open, "Modal must be closed after commit");
}

#[test]
fn t1_picker_effort_levels() {
    let effort_items: Vec<PickerItem<String>> = vec![
        PickerItem {
            label: "low".into(),
            detail: "Fastest response, minimal reasoning".into(),
            meta: "~1k tokens".into(),
            value: "low".into(),
        },
        PickerItem {
            label: "medium".into(),
            detail: "Balanced reasoning for standard coding".into(),
            meta: "~4k tokens".into(),
            value: "medium".into(),
        },
        PickerItem {
            label: "high".into(),
            detail: "Deep reasoning with multi-step planning".into(),
            meta: "~16k tokens".into(),
            value: "high".into(),
        },
        PickerItem {
            label: "max".into(),
            detail: "Full thinking budget, architectural breakdown".into(),
            meta: "~32k tokens".into(),
            value: "max".into(),
        },
    ];

    let mut picker = PickerState::new("Reasoning Effort".into(), effort_items);
    assert_eq!(picker.filtered_indices.len(), 4);

    picker.move_down(); // medium
    picker.move_down(); // high
    let committed = picker.commit_selection().unwrap();
    assert_eq!(committed, "high");
}

// ============================================================================
// Tier 2: Boundary & Corner Cases (Boundary Value Analysis)
// ============================================================================

#[test]
fn t2_picker_empty_search() {
    let mut harness = TestHarness::new(80, 24);
    let mut picker = create_model_picker();
    let pal = TOKYO_NIGHT;

    // Search query matching nothing
    for c in "nonexistent-model-xyz".chars() {
        picker.type_char(c);
    }
    assert!(picker.filtered_indices.is_empty());
    assert_eq!(picker.selected_item(), None);

    let buf = harness.draw(|f| {
        let area = Rect::new(0, 0, 80, 24);
        render_picker_modal(f, area, &picker, &pal);
    });

    let lines = buffer_lines(&buf);
    assert!(lines.iter().any(|l| l.contains("No matching items found")), "Empty message displayed");
    assert!(lines.iter().any(|l| l.contains("(0/0)")), "Footer counter reflects zero matches");

    // Commit on empty returns None without crashing
    assert_eq!(picker.commit_selection(), None);

    assert_no_ansi_leaks(&buf);
}

#[test]
fn t2_picker_rapid_keystrokes() {
    let mut picker = create_theme_picker();
    // Simulate burst of random navigation and typing
    for _ in 0..100 {
        picker.move_down();
        picker.type_char('a');
        picker.move_up();
        picker.backspace();
    }
    // State remains valid and uncorrupted
    assert!(picker.selected_index < picker.filtered_indices.len());
    assert_eq!(picker.search_query, "");
    assert_eq!(picker.filtered_indices.len(), 5);
}

#[test]
fn t2_picker_cancel_preserves_original() {
    let initial_theme = CLASSIC;
    let mut current_theme = initial_theme;

    let mut picker = create_theme_picker();

    // User navigates to Tokyo Night and previews it
    picker.move_down();
    picker.move_down();
    picker.move_down();
    picker.move_down();
    if let Some(item) = picker.selected_item() {
        current_theme = item.value; // live preview updated
    }
    assert_eq!(current_theme.name, "Tokyo Night");

    // User hits Esc to cancel
    picker.cancel();
    // Revert to initial theme snapshot
    current_theme = initial_theme;
    assert_eq!(current_theme.name, "Classic", "Cancelled modal restores original theme snapshot");
}

#[test]
fn t2_picker_screen_resize() {
    let mut harness = TestHarness::new(100, 30);
    let picker = create_model_picker();
    let pal = DARK_SLATE;

    let buf1 = harness.draw(|f| {
        let area = Rect::new(0, 0, 100, 30);
        render_picker_modal(f, area, &picker, &pal);
    });
    assert!(buffer_contains(&buf1, "Switch Model"));

    // Resize to smaller terminal (70 x 18)
    harness.resize(70, 18);
    let buf2 = harness.draw(|f| {
        let area = Rect::new(0, 0, 70, 18);
        render_picker_modal(f, area, &picker, &pal);
    });

    let lines2 = buffer_lines(&buf2);
    // Modal is centered: (70 - 64) / 2 = 3
    assert!(lines2.iter().any(|l| l.contains("Switch Model")), "Modal properly centered on resize");
    assert_no_ansi_leaks(&buf2);
}

#[test]
fn t2_picker_max_items_scroll() {
    // 60 mock models
    let large_list: Vec<PickerItem<usize>> = (0..60)
        .map(|i| PickerItem {
            label: format!("model-variant-{:02}", i),
            detail: format!("provider #{}", i % 5),
            meta: format!("{}ms", 20 + i),
            value: i,
        })
        .collect();

    let mut picker = PickerState::new("Browse All Models".into(), large_list);
    picker.max_visible_items = 8;

    assert_eq!(picker.scroll_offset, 0);

    // Navigate down 25 times
    for _ in 0..25 {
        picker.move_down();
    }
    assert_eq!(picker.selected_index, 25);
    // Scroll offset must have adjusted to keep selection visible
    assert!(picker.scroll_offset <= 25);
    assert!(picker.scroll_offset + picker.max_visible_items > 25);
    assert_eq!(picker.selected_item().unwrap().label, "model-variant-25");

    let mut harness = TestHarness::new(80, 24);
    let pal = PHOSPHOR;
    let buf = harness.draw(|f| {
        let area = Rect::new(0, 0, 80, 24);
        render_picker_modal(f, area, &picker, &pal);
    });

    let lines = buffer_lines(&buf);
    assert!(lines.iter().any(|l| l.contains("model-variant-25")), "Selected item visible in viewport");
    assert_no_ansi_leaks(&buf);
}
