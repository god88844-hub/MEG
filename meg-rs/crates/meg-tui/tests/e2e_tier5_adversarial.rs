//! Tier 5 Rust TUI Core Adversarial Challenge Test Suite
//!
//! Deep white-box stress testing and boundary verification across:
//! 1. Extreme terminal dimensions (0x0, 1x1, 1x50, 50x1, 2x2, 3x3, 500x500)
//! 2. Malicious & chaotic ANSI injection across streaming, pickers, jobs, and state
//! 3. Rapid event sequence bursts (keypress floods in composer and modal pickers)
//! 4. Unicode edge cases (ZWJ sequences, 4-byte emojis, combining accents, RTL, BiDi)
//! 5. Stream interleaving, rapid thinking folding toggles, and hostile telemetry
//! 6. Concurrent job lifecycle transitions and user interrupt handling
//! 7. Massive transcript scale and scroll calculations
//!
//! 100% offline, zero live API calls.

use meg_tui::{
    cursor_screen_pos, default_model_picker, default_theme_picker, draw, job_indicator_height,
    render_chip_bar, render_composer, render_hero, render_job_indicator, render_picker_modal,
    render_streaming_view, render_viewport, JobState, Mode, PickerItem, PickerState, RunnerMsg,
    StreamState, WorkstationState, ALL_PALETTES, DARK_SLATE, PHOSPHOR, TOKYO_NIGHT,
};
use ratatui::{backend::TestBackend, buffer::Buffer, layout::Rect, Terminal};
use std::sync::mpsc::channel;

// ============================================================================
// Helper Functions
// ============================================================================

fn extract_lines(buf: &Buffer) -> Vec<String> {
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

fn assert_strict_no_ansi(buf: &Buffer, context: &str) {
    let area = buf.area();
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            let symbol = buf[(x, y)].symbol();
            assert!(
                !symbol.contains('\x1b'),
                "[{}] Leaked ESC '\\x1b' in cell ({}, {}): {:?}",
                context,
                x,
                y,
                symbol
            );
            assert!(
                !symbol.contains("?["),
                "[{}] Leaked ESC-bracket '?[' in cell ({}, {}): {:?}",
                context,
                x,
                y,
                symbol
            );
            assert!(
                !symbol.contains("^["),
                "[{}] Leaked caret escape '^[' in cell ({}, {}): {:?}",
                context,
                x,
                y,
                symbol
            );
        }
    }
}

// ============================================================================
// Suite 1: Extreme Terminal Dimensions
// ============================================================================

#[test]
fn test_tier5_dimensions_zero_zero_full_draw() {
    let backend = TestBackend::new(0, 0);
    let mut terminal = Terminal::new(backend).unwrap();

    let mut st = WorkstationState::default();
    st.transcript.push("Sample log line".into());
    st.insert_str("test command");
    st.plan_steps.push((true, "Step 1".into()));
    st.status.push(("KEY".into(), "VAL".into()));

    let mut stream = StreamState::default();
    stream.push_thinking("Thinking deeply...");
    stream.push_content("Result text");
    st.stream = Some(stream);

    let mut job = JobState::default();
    job.start("j1".into(), "Active job".into());
    job.push_output("running...".into());
    st.active_job = Some(job);

    for pal in ALL_PALETTES {
        let res = terminal.draw(|f| {
            draw(f, pal, &st);
        });
        assert!(
            res.is_ok(),
            "draw() must not panic on 0x0 buffer: {:?}",
            res.err()
        );
    }
}

#[test]
fn test_tier5_dimensions_one_by_one_all_widgets() {
    let backend = TestBackend::new(1, 1);
    let mut terminal = Terminal::new(backend).unwrap();

    let mut st = WorkstationState::default();
    st.insert_str("input");
    st.transcript.push("line".into());

    let res = terminal.draw(|f| {
        draw(f, &DARK_SLATE, &st);
        render_hero(f, Rect::new(0, 0, 1, 1), &st, &DARK_SLATE);
        render_composer(f, Rect::new(0, 0, 1, 1), &st, &DARK_SLATE);
        render_chip_bar(f, Rect::new(0, 0, 1, 1), &st, &DARK_SLATE);
        render_viewport(f, Rect::new(0, 0, 1, 1), &st, &DARK_SLATE);

        let job = JobState::new("j".into(), "desc".into());
        render_job_indicator(f, Rect::new(0, 0, 1, 1), &job, &DARK_SLATE);

        let picker = default_theme_picker(&DARK_SLATE);
        render_picker_modal(f, Rect::new(0, 0, 1, 1), &picker, &DARK_SLATE);
    });
    assert!(res.is_ok(), "1x1 draw must not panic: {:?}", res.err());
}

#[test]
fn test_tier5_dimensions_permutation_matrix() {
    let dimensions: &[(u16, u16)] = &[
        (0, 0),
        (1, 1),
        (1, 50),
        (50, 1),
        (2, 2),
        (3, 3),
        (10, 2),
        (20, 5),
        (51, 4),
        (52, 5),
        (80, 24),
        (120, 40),
        (300, 100),
    ];

    let mut st = WorkstationState::default();
    st.insert_str("multi\nline\ninput\nfor\ntesting");
    st.transcript.push("Alpha".into());
    st.transcript.push("Beta".into());
    st.plan_steps.push((true, "Step done".into()));
    st.plan_steps.push((false, "Step pending".into()));
    st.status.push(("STATUS".into(), "RUNNING".into()));

    let mut job = JobState::default();
    job.start("job-perm".into(), "Stress job".into());
    job.push_output("line 1".into());
    job.push_output("line 2".into());
    st.active_job = Some(job);

    for &(w, h) in dimensions {
        let backend = TestBackend::new(w, h);
        let mut terminal = Terminal::new(backend).unwrap();
        let res = terminal.draw(|f| {
            draw(f, &TOKYO_NIGHT, &st);
        });
        assert!(res.is_ok(), "draw() failed on {}x{}: {:?}", w, h, res.err());
    }
}

/// Adversarial Challenge Bug Probe 1:
/// Validates whether `cursor_screen_pos` clamps cursor x when `input` is empty on narrow width.
#[test]
fn test_tier5_dimensions_cursor_screen_pos_narrow_bounds() {
    let st = WorkstationState::default();
    let area = Rect::new(0, 0, 2, 3);
    let (cx, _cy) = cursor_screen_pos(area, &st);
    assert!(
        cx < area.right(),
        "BUG: cursor x ({}) exceeds area right boundary ({}) when input is empty on narrow terminal!",
        cx, area.right()
    );
}

// ============================================================================
// Suite 2: Malicious & Chaotic ANSI Injection
// ============================================================================

#[test]
fn test_tier5_ansi_malicious_bombs_in_streaming() {
    let mut st = WorkstationState::default();
    let mut stream = StreamState::default();

    // Chaotic and hostile ANSI payloads
    let hostile_payloads = [
        "\x1b[38;2;255;0;0mRGB Red\x1b[0m",
        "\x1b[48;5;196m256 Color BG\x1b[0m",
        "\x1b[1;2;3;4;5;7;8;9mAll SGR Modifiers\x1b[0m",
        "\x1b[2J\x1b[HScreen Clear Bomb\x1b[10;20H",
        "\x1b]8;;https://malicious.example.com/exploit\x07Click Here\x1b]8;;\x07",
        "\x1b]0;Evil Window Title\x07",
        "\x1b[?1049h\x1b[?25lAlternate Screen Switch",
        "\x1b[6nDevice Status Report Query",
        "\x1b[9999999999999999999999;8888888888888888888888mGiant Param",
        "\x1b[;;;mEmpty Params",
        "\x1b[Truncated Escape Sequence",
        "\x1b",
        "\x1b[31",
        "\x1b]8;;unclosed-url",
    ];

    for payload in &hostile_payloads {
        stream.push_thinking(&format!("Thinking: {payload}\n"));
        stream.push_content(&format!("Content: {payload}\n"));
    }
    st.stream = Some(stream);

    let backend = TestBackend::new(100, 30);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| {
            draw(f, &DARK_SLATE, &st);
        })
        .unwrap();

    let buf = terminal.backend().buffer().clone();
    assert_strict_no_ansi(&buf, "Streaming View ANSI Injection");
}

/// Adversarial Challenge Bug Probe 2:
/// Probes whether `render_picker_modal` sanitizes ANSI escapes in picker items.
#[test]
fn test_tier5_ansi_injection_in_picker_search_and_items() {
    let hostile_items = vec![
        PickerItem {
            label: "Model \x1b[31mRed\x1b[0m".into(),
            detail: "Detail \x1b]8;;http://bad.org\x07link\x1b]8;;\x07".into(),
            meta: "\x1b[1mBold\x1b[0m".into(),
            value: "m1".to_string(),
        },
        PickerItem {
            label: "\x1b[2J\x1b[HClearBomb".into(),
            detail: "\x1b[?25hCursorOn".into(),
            meta: "\x1b[?1049lLeaveAlt".into(),
            value: "m2".to_string(),
        },
    ];

    let mut picker = PickerState::new("Hostile Picker".into(), hostile_items);
    picker.type_char('\x1b');
    picker.type_char('[');
    picker.type_char('3');
    picker.type_char('1');
    picker.type_char('m');

    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();

    terminal
        .draw(|f| {
            render_picker_modal(f, Rect::new(0, 0, 80, 24), &picker, &PHOSPHOR);
        })
        .unwrap();

    let buf = terminal.backend().buffer().clone();
    assert_strict_no_ansi(&buf, "Picker Search & Items");
}

/// Adversarial Challenge Bug Probe 3:
/// Probes whether `state.model` and `state.project` leak ANSI into hero banner and chips.
#[test]
fn test_tier5_ansi_injection_across_entire_workstation_state() {
    let mut st = WorkstationState::default();
    st.model = "\x1b[33mamber-model\x1b[0m".into();
    st.effort = "\x1b[35mmax-effort\x1b[0m".into();
    st.project = "\x1b[36m~/work/repo\x1b[0m".into();
    st.input = "run \x1b[31mtest\x1b[0m --flag".into();
    st.transcript
        .push("\u{276f} prompt \x1b[32mOK\x1b[0m".into());
    st.transcript
        .push("\u{2713} done \x1b[34mBlue\x1b[0m".into());
    st.plan_steps.push((true, "\x1b[1mPlan 1\x1b[0m".into()));
    st.status
        .push(("\x1b[31mK\x1b[0m".into(), "\x1b[32mV\x1b[0m".into()));

    let mut job = JobState::default();
    job.start("j-ansi".into(), "\x1b[31mJob Desc\x1b[0m".into());
    job.push_output("\x1b[33mJob output\x1b[0m".into());
    st.active_job = Some(job);

    for pal in ALL_PALETTES {
        let backend = TestBackend::new(100, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                draw(f, pal, &st);
            })
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        assert_strict_no_ansi(&buf, &format!("Full Workstation under {}", pal.name));
    }
}

// ============================================================================
// Suite 3: Rapid Event Sequence Bursts (Keypress Floods)
// ============================================================================

#[test]
fn test_tier5_rapid_burst_composer_fuzzing() {
    let mut st = WorkstationState::default();

    // 10,000 rapid chaotic operations on composer
    for i in 0..10_000 {
        match i % 10 {
            0 => st.insert_char(('a' as u8 + (i % 26) as u8) as char),
            1 => st.insert_str(" fn test() { "),
            2 => st.insert_char('\n'),
            3 => {
                st.delete_backspace();
            }
            4 => {
                st.delete_char();
            }
            5 => st.move_cursor_left(),
            6 => st.move_cursor_right(),
            7 => st.move_cursor_word_left(),
            8 => st.move_cursor_word_right(),
            9 => {
                if i % 20 == 0 {
                    st.move_cursor_home();
                } else {
                    st.move_cursor_end();
                }
            }
            _ => unreachable!(),
        }

        // Structural invariants that must NEVER be violated
        assert!(
            st.cursor_position <= st.input.chars().count(),
            "Invariant violation at iteration {}: cursor_pos {} > chars {}",
            i,
            st.cursor_position,
            st.input.chars().count()
        );
    }

    // Verify submit cleans up cleanly
    let submitted = st.commit_input();
    assert!(submitted.is_some());
    assert_eq!(st.cursor_position, 0);
    assert!(st.input.is_empty());
}

#[test]
fn test_tier5_rapid_burst_modal_picker_fuzzing() {
    let mut picker = default_model_picker("qwen2.5-coder:7b");

    for i in 0..5_000 {
        match i % 6 {
            0 => picker.type_char(('a' as u8 + (i % 26) as u8) as char),
            1 => picker.type_char(' '),
            2 => picker.backspace(),
            3 => picker.move_down(),
            4 => picker.move_up(),
            5 => {
                picker.adjust_scroll();
            }
            _ => unreachable!(),
        }

        if !picker.filtered_indices.is_empty() {
            assert!(
                picker.selected_index < picker.filtered_indices.len(),
                "selected_index {} out of bounds for len {}",
                picker.selected_index,
                picker.filtered_indices.len()
            );
            assert!(
                picker.selected_item().is_some(),
                "selected_item must be Some when filtered_indices not empty"
            );
        } else {
            assert_eq!(picker.selected_index, 0);
            assert_eq!(picker.selected_item(), None);
        }
    }
}

#[test]
fn test_tier5_rapid_burst_mode_cycling_bidirectional() {
    let mut st = WorkstationState::default();
    assert_eq!(st.mode, Mode::Build);

    for _ in 0..3_000 {
        let initial = st.mode;
        st.cycle_mode();
        assert_ne!(st.mode, initial);
        st.cycle_mode_reverse();
        assert_eq!(st.mode, initial, "Forward + Reverse must return to start");
    }
}

// ============================================================================
// Suite 4: Unicode Edge Cases
// ============================================================================

#[test]
fn test_tier5_unicode_complex_zwj_sequences() {
    let mut st = WorkstationState::default();

    // ZWJ family sequences: 👨‍👩‍👧‍👦 (7 codepoints), 🏳️‍🌈 (4 codepoints), 👩🏽‍💻 (5 codepoints)
    let zwj_text = "👨‍👩‍👧‍👦 🏳️‍🌈 👩🏽‍💻";
    st.insert_str(zwj_text);

    let char_count = zwj_text.chars().count();
    assert_eq!(st.cursor_position, char_count);

    // Each family/flag/profession emoji is one editable grapheme.
    for _ in 0..5 {
        st.move_cursor_left();
    }
    assert_eq!(st.cursor_position, 0);

    // Skip the complete family; delete the separator, then the family.
    st.move_cursor_right();
    assert_eq!(st.cursor_position, "👨‍👩‍👧‍👦".chars().count());
    let ok = st.delete_char();
    assert!(ok);
    let ok_bs = st.delete_backspace();
    assert!(ok_bs);
    assert_eq!(st.input, "🏳️‍🌈 👩🏽‍💻");

    // Test cursor screen pos calculation does not panic
    let area = Rect::new(0, 0, 80, 5);
    let (cx, cy) = cursor_screen_pos(area, &st);
    assert!(cx < 80);
    assert!(cy < 5);
}

#[test]
fn test_tier5_unicode_bidi_and_combining_diacritics() {
    let mut st = WorkstationState::default();

    // Combining diacritics: e + acute + grave + circumflex
    let combining = "e\u{0301}\u{0300}\u{0302}";
    st.insert_str(combining);
    assert_eq!(st.cursor_position, 4);

    // RTL Hebrew and Arabic
    let rtl = " עברית العربية ";
    st.insert_str(rtl);

    // BiDi override chars
    let bidi_override = "\u{202e}REVERSED\u{202c}";
    st.insert_str(bidi_override);

    let backend = TestBackend::new(80, 10);
    let mut terminal = Terminal::new(backend).unwrap();
    let res = terminal.draw(|f| {
        render_composer(f, Rect::new(0, 0, 80, 5), &st, &DARK_SLATE);
    });
    assert!(
        res.is_ok(),
        "Rendering combining/RTL/BiDi must not panic: {:?}",
        res.err()
    );
}

// ============================================================================
// Suite 5: Stream Interleaving & Thinking Display Folding
// ============================================================================

#[test]
fn test_tier5_stream_rapid_interleaving_and_folding_toggles() {
    let mut stream = StreamState::default();

    for i in 0..500 {
        stream.push_thinking(&format!("Reasoning chunk {i}...\n"));
        // Every 50 chunks, push content
        if i % 50 == 49 {
            let was_pinned = stream.user_pinned_thinking;
            stream.push_content(&format!("Answer chunk {i}\n"));
            if !was_pinned {
                assert!(
                    stream.is_thinking_collapsed,
                    "Unpinned thinking must auto-collapse on content"
                );
            } else {
                assert!(
                    !stream.is_thinking_collapsed,
                    "Pinned thinking must remain expanded on content"
                );
            }
        }
        // Rapid toggle
        if i % 25 == 0 {
            stream.toggle_thinking_collapse();
        }
    }

    assert!(!stream.thinking_buffer.is_empty());
    assert!(!stream.content_buffer.is_empty());

    let backend = TestBackend::new(80, 20);
    let mut terminal = Terminal::new(backend).unwrap();

    // Draw in both collapsed and uncollapsed states
    stream.is_thinking_collapsed = true;
    let res1 = terminal.draw(|f| {
        render_streaming_view(f, Rect::new(0, 0, 80, 20), &stream, &DARK_SLATE);
    });
    assert!(res1.is_ok());

    stream.is_thinking_collapsed = false;
    let res2 = terminal.draw(|f| {
        render_streaming_view(f, Rect::new(0, 0, 80, 20), &stream, &DARK_SLATE);
    });
    assert!(res2.is_ok());
}

#[test]
fn test_tier5_stream_hostile_telemetry_metrics() {
    let mut stream = StreamState::default();
    stream.push_thinking("Thinking...\n");
    stream.push_content("Content...\n");

    let hostile_metrics: &[(f32, f32, usize)] = &[
        (f32::NAN, f32::NAN, 0),
        (f32::INFINITY, f32::INFINITY, usize::MAX),
        (f32::NEG_INFINITY, -100.0, 0),
        (0.0, 0.0, 0),
        (1e6, 86400.0, 10_000_000),
    ];

    let backend = TestBackend::new(80, 15);
    let mut terminal = Terminal::new(backend).unwrap();

    for &(rate, elapsed, tokens) in hostile_metrics {
        stream.tok_per_sec = rate;
        stream.elapsed_secs = elapsed;
        stream.token_count = tokens;

        let formatted = stream.formatted_elapsed();
        assert!(!formatted.is_empty());

        let res = terminal.draw(|f| {
            render_streaming_view(f, Rect::new(0, 0, 80, 15), &stream, &DARK_SLATE);
        });
        assert!(
            res.is_ok(),
            "Telemetry draw failed on ({}, {}, {}): {:?}",
            rate,
            elapsed,
            tokens,
            res.err()
        );
    }
}

// ============================================================================
// Suite 6: Concurrent Job Lifecycle & Interrupt Handling
// ============================================================================

#[test]
fn test_tier5_job_concurrent_lifecycle_interleaving() {
    let (tx, rx) = channel::<RunnerMsg>();
    let mut st = WorkstationState::default();

    let msgs = vec![
        RunnerMsg::JobStart {
            id: "job-A".into(),
            description: "Build kernel".into(),
        },
        RunnerMsg::JobProgress {
            id: "job-A".into(),
            status: "compiling src/lib.rs".into(),
        },
        RunnerMsg::JobStart {
            id: "job-B".into(),
            description: "Run solver".into(),
        },
        RunnerMsg::JobProgress {
            id: "job-A".into(),
            status: "orphan update for job-A".into(),
        },
        RunnerMsg::JobProgress {
            id: "job-B".into(),
            status: "solving SAT clauses".into(),
        },
        RunnerMsg::JobDone { id: "job-A".into() }, // Mismatched done
        RunnerMsg::JobDone { id: "job-B".into() }, // Proper done
    ];

    for msg in msgs {
        tx.send(msg).unwrap();
    }
    drop(tx);

    while let Ok(msg) = rx.try_recv() {
        match msg {
            RunnerMsg::JobStart { id, description } => {
                let mut job = JobState::default();
                job.start(id, description);
                st.active_job = Some(job);
            }
            RunnerMsg::JobProgress { id, status } => {
                if let Some(ref mut job) = st.active_job {
                    if job.id == id || job.id.is_empty() {
                        job.push_output(status);
                    }
                }
            }
            RunnerMsg::JobDone { id } => {
                if let Some(ref mut job) = st.active_job {
                    if job.id == id || job.id.is_empty() {
                        job.complete();
                        st.transcript
                            .push(format!("\u{2713} Job completed: {}", job.description));
                    }
                }
                if let Some(ref job) = st.active_job {
                    if !job.is_active {
                        st.active_job = None;
                    }
                }
            }
            _ => {}
        }
    }

    assert!(st.active_job.is_none(), "All jobs completed cleanly");
    assert!(st.transcript.iter().any(|l| l.contains("Run solver")));
}

#[test]
fn test_tier5_job_user_interrupt_and_cancellation_rendering() {
    let mut st = WorkstationState::default();
    let mut job = JobState::default();
    job.start("job-cancel".into(), "Cargo check --all".into());
    for i in 0..100 {
        job.push_output(format!("Checking crate {i}..."));
    }
    st.active_job = Some(job);

    assert_eq!(job_indicator_height(st.active_job.as_ref()), 2);

    st.cancel_active_job();

    assert_eq!(job_indicator_height(st.active_job.as_ref()), 1);
    assert!(!st.active_job.as_ref().unwrap().is_active);
    assert!(st.active_job.as_ref().unwrap().is_cancelled);

    let backend = TestBackend::new(80, 10);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| {
            render_job_indicator(
                f,
                Rect::new(0, 0, 80, 1),
                st.active_job.as_ref().unwrap(),
                &DARK_SLATE,
            );
        })
        .unwrap();

    let lines = extract_lines(terminal.backend().buffer());
    assert!(lines.iter().any(|l| l.contains("cancelled")));
    assert!(lines.iter().any(|l| l.contains('\u{2717}')));
}

// ============================================================================
// Suite 7: Massive Transcript Scale & Scroll Calculations
// ============================================================================

/// Adversarial Challenge Bug Probe 4:
/// Probes whether `render_viewport` scroll calculations wrap modulo 65536 on massive transcripts.
#[test]
fn test_tier5_transcript_overflow_65536_lines_scroll_drift() {
    let mut st = WorkstationState::default();
    // Populate transcript with 65,540 lines (exceeding u16::MAX = 65,535)
    for i in 0..65_540 {
        st.transcript.push(format!("Row {i}"));
    }

    let backend = TestBackend::new(80, 20);
    let mut terminal = Terminal::new(backend).unwrap();

    terminal
        .draw(|f| {
            render_viewport(f, Rect::new(0, 0, 80, 20), &st, &DARK_SLATE);
        })
        .unwrap();

    let lines = extract_lines(terminal.backend().buffer());
    let shows_latest = lines
        .iter()
        .any(|l| l.contains("Row 65539") || l.contains("Row 65538"));
    assert!(
        shows_latest,
        "BUG: Viewport total_lines cast truncation! 65,540 lines wrapped modulo 65536, scrolling to top instead of bottom! Visible lines: {:?}",
        &lines[0..5.min(lines.len())]
    );
}

#[test]
fn test_tier5_transcript_massive_scale_scrolling() {
    let mut st = WorkstationState::default();
    for i in 0..10_000 {
        st.transcript.push(format!("Log message row {i:06}"));
    }

    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();

    let res = terminal.draw(|f| {
        render_viewport(f, Rect::new(0, 0, 80, 24), &st, &DARK_SLATE);
    });
    assert!(
        res.is_ok(),
        "10k lines viewport draw must succeed: {:?}",
        res.err()
    );

    let lines = extract_lines(terminal.backend().buffer());
    assert!(
        lines.iter().any(|l| l.contains("Log message row 00999")),
        "Auto-scroll must show bottom of 10k lines"
    );
}
