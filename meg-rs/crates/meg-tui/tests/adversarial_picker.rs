//! Adversarial Challenge Test Suite — Milestone 2 Modal Pickers & Themes
//!
//! Empirical stress-testing of `PickerState<T>`, `render_picker_modal`, and `ActiveModal`:
//! 1. Wrap-around navigation with 0 items, 1 item, and 100+ items.
//! 2. Searching with characters matching nothing, special characters, unicode, and repeated backspaces.
//! 3. Extreme screen sizes: width < 10, height < 5, 0x0 area (must never panic).
//! 4. Scroll offset adjustments when moving up/down past visible bounds.
//! 5. Deep scroll filtering reduction and bounds preservation.
//! 6. Active modal lifecycle and input isolation.

use meg_tui::{
    default_model_picker, default_theme_picker, render_picker_modal, ActiveModal, PickerItem,
    PickerState, WorkstationState, CLASSIC, DARK_SLATE, DRACULA, PHOSPHOR, TOKYO_NIGHT,
};
use ratatui::{backend::TestBackend, layout::Rect, Terminal};

// ============================================================================
// Helper: buffer text extraction
// ============================================================================

fn extract_buffer_lines(terminal: &Terminal<TestBackend>, width: u16, height: u16) -> Vec<String> {
    let buf = terminal.backend().buffer();
    let mut lines = Vec::with_capacity(height as usize);
    for y in 0..height {
        let mut line = String::with_capacity(width as usize);
        for x in 0..width {
            line.push_str(buf[(x, y)].symbol());
        }
        lines.push(line);
    }
    lines
}

// ============================================================================
// 1. Wrap-Around Navigation Edge Cases (0 items, 1 item, 100+ items)
// ============================================================================

#[test]
fn adv_picker_navigation_wraparound_0_items() {
    let mut picker: PickerState<String> = PickerState::new("Empty Picker".into(), vec![]);
    assert_eq!(picker.items.len(), 0);
    assert_eq!(picker.filtered_indices.len(), 0);
    assert_eq!(picker.selected_index, 0);
    assert_eq!(picker.scroll_offset, 0);

    // Navigation on 0 items must never panic or underflow
    picker.move_down();
    assert_eq!(picker.selected_index, 0);
    assert_eq!(picker.scroll_offset, 0);

    picker.move_up();
    assert_eq!(picker.selected_index, 0);
    assert_eq!(picker.scroll_offset, 0);

    // Selected item and commit must be safely None
    assert_eq!(picker.selected_item(), None);
    assert_eq!(picker.commit_selection(), None);
    assert!(picker.is_open, "Failed commit on empty list must not close modal");

    // Rendering 0 items must show empty state gracefully
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| {
            render_picker_modal(f, Rect::new(0, 0, 80, 24), &picker, &DARK_SLATE);
        })
        .unwrap();

    let lines = extract_buffer_lines(&terminal, 80, 24);
    assert!(
        lines.iter().any(|l| l.contains("No matching items found")),
        "Empty picker must show placeholder message"
    );
    assert!(
        lines.iter().any(|l| l.contains("(0/0)")),
        "Footer counter must indicate (0/0)"
    );
}

#[test]
fn adv_picker_navigation_wraparound_1_item() {
    let items = vec![PickerItem {
        label: "solitary-item".into(),
        detail: "only choice".into(),
        meta: "singleton".into(),
        value: 42,
    }];
    let mut picker = PickerState::new("Solo Picker".into(), items);
    assert_eq!(picker.filtered_indices.len(), 1);
    assert_eq!(picker.selected_index, 0);
    assert_eq!(picker.scroll_offset, 0);

    // Repeated move_down must wrap around back to 0
    for _ in 0..100 {
        picker.move_down();
        assert_eq!(picker.selected_index, 0);
        assert_eq!(picker.scroll_offset, 0);
    }

    // Repeated move_up must wrap around back to 0
    for _ in 0..100 {
        picker.move_up();
        assert_eq!(picker.selected_index, 0);
        assert_eq!(picker.scroll_offset, 0);
    }

    assert_eq!(picker.selected_item().unwrap().value, 42);

    // Render test
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| {
            render_picker_modal(f, Rect::new(0, 0, 80, 24), &picker, &PHOSPHOR);
        })
        .unwrap();

    let lines = extract_buffer_lines(&terminal, 80, 24);
    assert!(
        lines.iter().any(|l| l.contains("solitary-item")),
        "Singleton item must be rendered"
    );
    assert!(
        lines.iter().any(|l| l.contains("(1/1)")),
        "Counter must show (1/1)"
    );

    // Commit selection
    assert_eq!(picker.commit_selection(), Some(42));
    assert!(!picker.is_open);
}

#[test]
fn adv_picker_navigation_wraparound_100_plus_items() {
    const TOTAL_ITEMS: usize = 150;
    let items: Vec<PickerItem<usize>> = (0..TOTAL_ITEMS)
        .map(|i| PickerItem {
            label: format!("item-{:03}", i),
            detail: format!("payload-data-{:03}", i),
            meta: format!("cat-{}", i % 10),
            value: i,
        })
        .collect();

    let mut picker = PickerState::new("Massive Picker".into(), items);
    picker.max_visible_items = 8;

    assert_eq!(picker.selected_index, 0);
    assert_eq!(picker.scroll_offset, 0);

    // Wraparound up from 0 -> must jump to last item (149)
    picker.move_up();
    assert_eq!(picker.selected_index, TOTAL_ITEMS - 1);
    // Scroll offset must adjust so last item is visible in window
    let expected_offset = TOTAL_ITEMS - picker.max_visible_items;
    assert_eq!(
        picker.scroll_offset, expected_offset,
        "Scroll offset must adjust to bottom on upward wrap"
    );
    assert_eq!(picker.selected_item().unwrap().value, TOTAL_ITEMS - 1);

    // Wraparound down from 149 -> must jump back to 0
    picker.move_down();
    assert_eq!(picker.selected_index, 0);
    assert_eq!(
        picker.scroll_offset, 0,
        "Scroll offset must snap to 0 on downward wrap"
    );
    assert_eq!(picker.selected_item().unwrap().value, 0);

    // Rapid alternating wrap-around stress test (50 cycles)
    for _ in 0..50 {
        picker.move_up();
        assert_eq!(picker.selected_index, TOTAL_ITEMS - 1);
        assert_eq!(picker.scroll_offset, expected_offset);

        picker.move_down();
        assert_eq!(picker.selected_index, 0);
        assert_eq!(picker.scroll_offset, 0);
    }
}

// ============================================================================
// 2. Search & Filtering Edge Cases (Special Chars, Unicode, Repeated Backspaces)
// ============================================================================

#[test]
fn adv_picker_search_no_match() {
    let items = vec![
        PickerItem {
            label: "alpha".into(),
            detail: "first".into(),
            meta: "greek".into(),
            value: 1,
        },
        PickerItem {
            label: "beta".into(),
            detail: "second".into(),
            meta: "greek".into(),
            value: 2,
        },
    ];
    let mut picker = PickerState::new("Search Test".into(), items);

    for c in "nonexistent_query_xyz".chars() {
        picker.type_char(c);
    }
    assert_eq!(picker.filtered_indices.len(), 0);
    assert_eq!(picker.selected_index, 0);
    assert_eq!(picker.scroll_offset, 0);
    assert_eq!(picker.selected_item(), None);
    assert_eq!(picker.commit_selection(), None);

    // Navigation on zero matches does not panic
    picker.move_up();
    picker.move_down();
    assert_eq!(picker.selected_index, 0);
}

#[test]
fn adv_picker_search_special_characters() {
    let items = vec![
        PickerItem {
            label: "regex-meta-item (.*+?^$|#)".into(),
            detail: "contains [brackets] & {braces}".into(),
            meta: "special/chars\\path".into(),
            value: "special-1".to_string(),
        },
        PickerItem {
            label: "quotes-item \"double\" and 'single'".into(),
            detail: "semicolon; injection && echo `hi`".into(),
            meta: "$PATH | grep <test>".into(),
            value: "special-2".to_string(),
        },
        PickerItem {
            label: "plain-item".into(),
            detail: "normal content".into(),
            meta: "none".into(),
            value: "plain".to_string(),
        },
    ];
    let mut picker = PickerState::new("Special Chars".into(), items);

    // Search with regex metacharacters: must perform literal substring match, not regex
    for c in "[brackets]".chars() {
        picker.type_char(c);
    }
    assert_eq!(picker.filtered_indices.len(), 1);
    assert_eq!(picker.selected_item().unwrap().value, "special-1");

    // Clear search
    for _ in 0.."[brackets]".len() {
        picker.backspace();
    }
    assert_eq!(picker.filtered_indices.len(), 3);

    // Search with shell metacharacters
    for c in "&& echo".chars() {
        picker.type_char(c);
    }
    assert_eq!(picker.filtered_indices.len(), 1);
    assert_eq!(picker.selected_item().unwrap().value, "special-2");

    // Render verification
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| {
            render_picker_modal(f, Rect::new(0, 0, 80, 24), &picker, &DRACULA);
        })
        .unwrap();
    let lines = extract_buffer_lines(&terminal, 80, 24);
    assert!(lines.iter().any(|l| l.contains("quotes-item")));
}

#[test]
fn adv_picker_search_multibyte_unicode_and_emoji() {
    let items = vec![
        PickerItem {
            label: "🦀 Rust Crab".into(),
            detail: "高速かつメモリ効率的".into(),
            meta: "東京".into(),
            value: "jp",
        },
        PickerItem {
            label: "🚀 Qwen Coder 中文模型".into(),
            detail: "通义千问 代码专家".into(),
            meta: "北京".into(),
            value: "zh",
        },
        PickerItem {
            label: "✨ Hangeul 모델 한국어".into(),
            detail: "초거대 언어 모델 인공지능".into(),
            meta: "서울".into(),
            value: "ko",
        },
        PickerItem {
            label: "🔥 Mixed ZWJ 🧙‍♂️ Wizard".into(),
            detail: "Magical reasoning with café & résumé".into(),
            meta: "accent".into(),
            value: "acc",
        },
    ];
    let mut picker = PickerState::new("Unicode Picker".into(), items);

    // 1. Search with Emoji "🦀"
    picker.type_char('🦀');
    assert_eq!(picker.filtered_indices.len(), 1);
    assert_eq!(picker.selected_item().unwrap().value, "jp");
    picker.backspace();
    assert_eq!(picker.filtered_indices.len(), 4);

    // 2. Search with CJK Kanji "模型"
    for c in "模型".chars() {
        picker.type_char(c);
    }
    assert_eq!(picker.filtered_indices.len(), 1);
    assert_eq!(picker.selected_item().unwrap().value, "zh");
    picker.backspace();
    picker.backspace();
    assert_eq!(picker.filtered_indices.len(), 4);

    // 3. Search with accented character "café"
    for c in "café".chars() {
        picker.type_char(c);
    }
    assert_eq!(picker.filtered_indices.len(), 1);
    assert_eq!(picker.selected_item().unwrap().value, "acc");

    // 4. Backspace multi-byte UTF-8 safely
    picker.backspace(); // removes 'é' (2-byte UTF-8)
    assert_eq!(picker.search_query, "caf");
    assert_eq!(picker.filtered_indices.len(), 1);

    picker.backspace();
    picker.backspace();
    picker.backspace();
    assert_eq!(picker.search_query, "");
    assert_eq!(picker.filtered_indices.len(), 4);

    // Render Unicode buffer
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| {
            render_picker_modal(f, Rect::new(0, 0, 80, 24), &picker, &TOKYO_NIGHT);
        })
        .unwrap();
    let lines = extract_buffer_lines(&terminal, 80, 24);
    assert!(lines.iter().any(|l| l.contains("Rust Crab")));
}

#[test]
fn adv_picker_search_repeated_backspaces() {
    let items = vec![
        PickerItem {
            label: "item-a".into(),
            detail: "det".into(),
            meta: "m".into(),
            value: 1,
        },
        PickerItem {
            label: "item-b".into(),
            detail: "det".into(),
            meta: "m".into(),
            value: 2,
        },
    ];
    let mut picker = PickerState::new("Backspace Fuzz".into(), items);

    // Search query is initially empty: 100 backspaces must be no-ops
    for _ in 0..100 {
        picker.backspace();
    }
    assert_eq!(picker.search_query, "");
    assert_eq!(picker.filtered_indices.len(), 2);
    assert_eq!(picker.selected_index, 0);

    // Type 5 chars
    for c in "hello".chars() {
        picker.type_char(c);
    }
    assert_eq!(picker.search_query, "hello");

    // Pop 200 backspaces (more than query length)
    for _ in 0..200 {
        picker.backspace();
    }
    assert_eq!(picker.search_query, "");
    assert_eq!(picker.filtered_indices.len(), 2);
    assert_eq!(picker.selected_index, 0);
    assert_eq!(picker.scroll_offset, 0);
}

// ============================================================================
// 3. Extreme Screen Dimensions (width < 10, height < 5, 0x0 area, giants)
// ============================================================================

#[test]
fn adv_picker_extreme_screen_sizes_never_panic() {
    let picker = default_theme_picker(&DARK_SLATE);
    let pal = DARK_SLATE;

    // Sub-minimum test matrix: widths 0..15, heights 0..8
    let test_dimensions = [
        (0, 0),
        (0, 24),
        (80, 0),
        (1, 1),
        (2, 2),
        (5, 3),
        (9, 4),
        (9, 24), // width < 10
        (80, 4), // height < 5
        (10, 5), // exact minimum accepted boundary
        (11, 5),
        (10, 6),
        (12, 6),
        (14, 7),
        (64, 14), // exact modal preferred size
        (68, 16),
        (200, 100), // very large
        (500, 300), // giant screen
    ];

    for (w, h) in test_dimensions {
        let backend = TestBackend::new(w.max(1), h.max(1));
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|f| {
                let area = Rect::new(0, 0, w, h);
                render_picker_modal(f, area, &picker, &pal);
            })
            .unwrap_or_else(|e| panic!("Failed rendering at {w}x{h}: {e}"));
    }
}

#[test]
fn adv_picker_extreme_screen_offsets_never_panic() {
    let picker = default_model_picker("gpt-4o");
    let pal = CLASSIC;

    // Render within shifted bounding rects
    let shifted_areas = [
        Rect::new(20, 10, 60, 20),
        Rect::new(50, 40, 10, 5),
        Rect::new(100, 100, 80, 24),
        Rect::new(10, 10, 0, 0),
        Rect::new(5, 5, 8, 3),
    ];

    let backend = TestBackend::new(200, 150);
    let mut terminal = Terminal::new(backend).unwrap();

    for area in shifted_areas {
        terminal
            .draw(|f| {
                render_picker_modal(f, area, &picker, &pal);
            })
            .unwrap_or_else(|e| panic!("Failed rendering offset area {area:?}: {e}"));
    }
}

// ============================================================================
// 4. Scroll Offset Adjustments Past Visible Bounds (Full Cycle & Invariant)
// ============================================================================

#[test]
fn adv_picker_scroll_offset_invariants() {
    const TOTAL: usize = 50;
    const VISIBLE: usize = 8;

    let items: Vec<PickerItem<usize>> = (0..TOTAL)
        .map(|i| PickerItem {
            label: format!("item-{:02}", i),
            detail: "detail".into(),
            meta: "tag".into(),
            value: i,
        })
        .collect();

    let mut picker = PickerState::new("Scroll Invariant".into(), items);
    picker.max_visible_items = VISIBLE;

    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();

    // 1. Move down through all items, verifying window invariant at every step
    for step in 0..TOTAL {
        assert_eq!(picker.selected_index, step);

        // Invariant: selected_index must be strictly within [scroll_offset, scroll_offset + VISIBLE)
        assert!(
            picker.selected_index >= picker.scroll_offset,
            "selected_index {} must be >= scroll_offset {}",
            picker.selected_index,
            picker.scroll_offset
        );
        assert!(
            picker.selected_index < picker.scroll_offset + picker.max_visible_items,
            "selected_index {} must be < scroll_offset {} + visible {}",
            picker.selected_index,
            picker.scroll_offset,
            picker.max_visible_items
        );

        // Render and verify active item label is in drawn frame
        terminal
            .draw(|f| {
                render_picker_modal(f, Rect::new(0, 0, 80, 24), &picker, &DARK_SLATE);
            })
            .unwrap();

        let lines = extract_buffer_lines(&terminal, 80, 24);
        let expected_label = format!("item-{:02}", step);
        assert!(
            lines.iter().any(|l| l.contains(&expected_label)),
            "Currently selected item {expected_label} must be visible in viewport at step {step}"
        );

        if step + 1 < TOTAL {
            picker.move_down();
        }
    }

    // Now at last item (49). Scroll offset must be exactly 49 + 1 - 8 = 42.
    assert_eq!(picker.selected_index, TOTAL - 1);
    assert_eq!(picker.scroll_offset, TOTAL - VISIBLE);

    // 2. Move up all the way back to 0, verifying invariant at every step
    for step in (0..TOTAL).rev() {
        assert_eq!(picker.selected_index, step);

        assert!(picker.selected_index >= picker.scroll_offset);
        assert!(picker.selected_index < picker.scroll_offset + picker.max_visible_items);

        if step > 0 {
            picker.move_up();
        }
    }

    assert_eq!(picker.selected_index, 0);
    assert_eq!(picker.scroll_offset, 0);
}

// ============================================================================
// 5. Deep Scroll Filtering Reduction Safety
// ============================================================================

#[test]
fn adv_picker_deep_scroll_query_reduction_safety() {
    let items: Vec<PickerItem<usize>> = (0..100)
        .map(|i| PickerItem {
            label: if i == 95 || i == 97 {
                format!("target-special-{i}")
            } else {
                format!("standard-item-{i}")
            },
            detail: "desc".into(),
            meta: "cat".into(),
            value: i,
        })
        .collect();

    let mut picker = PickerState::new("Reduction Test".into(), items);
    picker.max_visible_items = 8;

    // Scroll down to index 95
    for _ in 0..95 {
        picker.move_down();
    }
    assert_eq!(picker.selected_index, 95);
    assert_eq!(picker.scroll_offset, 95 + 1 - 8); // 88

    // Now type a query that reduces the list from 100 items down to only 2 matches ("target")
    for c in "target".chars() {
        picker.type_char(c);
    }
    assert_eq!(picker.filtered_indices.len(), 2);
    // Selected index must be safely clamped without out-of-bounds index
    assert!(
        picker.selected_index < 2,
        "selected_index must be clamped to filtered len (was {})",
        picker.selected_index
    );
    // Scroll offset must be clamped within range
    assert!(
        picker.scroll_offset <= picker.selected_index,
        "scroll_offset {} must be <= selected_index {}",
        picker.scroll_offset,
        picker.selected_index
    );

    // Rendering this state: item 97 (selected at index 1) is visible.
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| {
            render_picker_modal(f, Rect::new(0, 0, 80, 24), &picker, &DARK_SLATE);
        })
        .unwrap();

    let lines = extract_buffer_lines(&terminal, 80, 24);
    assert!(lines.iter().any(|l| l.contains("target-special-97")));

    // Moving up to index 0 adjusts scroll_offset to 0, making target-special-95 visible
    picker.move_up();
    assert_eq!(picker.selected_index, 0);
    assert_eq!(picker.scroll_offset, 0);

    terminal
        .draw(|f| {
            render_picker_modal(f, Rect::new(0, 0, 80, 24), &picker, &DARK_SLATE);
        })
        .unwrap();
    let lines_after = extract_buffer_lines(&terminal, 80, 24);
    assert!(lines_after.iter().any(|l| l.contains("target-special-95")));

    // Clear the query: restores all 100 items
    for _ in 0.."target".len() {
        picker.backspace();
    }
    assert_eq!(picker.filtered_indices.len(), 100);
    assert!(picker.selected_index < 100);
}

// ============================================================================
// 6. Active Modal Lifecycle & Input Isolation Stress Test
// ============================================================================

#[test]
fn adv_workstation_modal_lifecycle_and_input_isolation() {
    let mut st = WorkstationState::default();
    st.input = "git commit -m 'wip'".into();
    st.cursor_position = st.input.chars().count();
    let initial_input = st.input.clone();
    let initial_cursor = st.cursor_position;

    // 1. Open Theme Picker
    st.open_theme_picker();
    assert!(st.has_active_modal());
    assert!(matches!(st.active_modal, ActiveModal::ThemePicker(_)));
    assert_eq!(st.theme_snapshot.unwrap().name, DARK_SLATE.name);

    // Simulate modal typing and navigation
    st.active_modal_type_char('p');
    st.active_modal_type_char('h');
    st.active_modal_type_char('o');
    st.active_modal_type_char('s');
    // Input in workstation composer must be completely untouched!
    assert_eq!(st.input, initial_input);
    assert_eq!(st.cursor_position, initial_cursor);

    // Cancel theme modal on Esc -> restores Dark Slate
    st.cancel_active_modal();
    assert!(!st.has_active_modal());
    assert_eq!(st.theme.name, DARK_SLATE.name);
    assert_eq!(st.input, initial_input);

    // 2. Open Model Picker, navigate and commit
    st.open_model_picker();
    assert!(st.has_active_modal());
    assert!(matches!(st.active_modal, ActiveModal::ModelPicker(_)));
    st.active_modal_move_down(); // move down to deepseek or second model
    st.commit_active_modal();
    assert!(!st.has_active_modal());
    assert_ne!(st.model, "qwen2.5-coder:7b");
    assert_eq!(st.input, initial_input);

    // 3. Open Effort Picker, navigate and commit
    st.open_effort_picker();
    assert!(st.has_active_modal());
    st.active_modal_move_up(); // wrap around to max or prev
    st.commit_active_modal();
    assert!(!st.has_active_modal());
    assert_eq!(st.input, initial_input);
}
