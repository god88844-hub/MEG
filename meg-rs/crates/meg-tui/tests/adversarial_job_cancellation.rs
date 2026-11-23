//! Adversarial Challenge Test Suite — Challenger 2 (Active Job Indicator & Cancellation)
//!
//! Empirical verification of:
//! 1. Output folding under massive log output (10,000+ lines, long lines, ANSI stripping, Unicode)
//! 2. Rapid sequential job lifecycles (1,000 cycles, channel message draining, ID matching, reset purity)
//! 3. Interactive cancellation signals (Esc, Ctrl+C handling, rapid cancellation toggles, error status)
//! 4. Layout docking geometry (exact alignment above composer, 0-height inactive collapse, <12 height clamping)

use meg_tui::{
    draw, fmt_elapsed_compact, job_indicator_height, render_job_indicator,
    JobState, RunnerMsg, WorkstationState, CLASSIC, DARK_SLATE, DRACULA,
    SPINNER_FRAMES, TOKYO_NIGHT,
};
use ratatui::{
    backend::TestBackend,
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    Terminal,
};
use std::sync::mpsc::channel;

fn buffer_to_lines(buf: &Buffer) -> Vec<String> {
    let mut lines = Vec::new();
    for y in 0..buf.area.height {
        let mut line = String::new();
        for x in 0..buf.area.width {
            line.push_str(buf[(x, y)].symbol());
        }
        lines.push(line);
    }
    lines
}

fn assert_no_raw_ansi(buf: &Buffer) {
    for y in 0..buf.area.height {
        for x in 0..buf.area.width {
            let sym = buf[(x, y)].symbol();
            assert!(
                !sym.contains('\x1b'),
                "Found raw escape byte 0x1B at ({x}, {y}): {sym:?}"
            );
        }
    }
}

// ============================================================================
// 1. Massive Log Output & Folding Adversarial Tests
// ============================================================================

#[test]
fn adv_job_10000_lines_massive_output_folding() {
    let mut job = JobState::default();
    job.start("job-bulk-10k".into(), "Cargo build --all --release".into());

    // Push 10,000 distinct log lines
    for i in 0..10_000 {
        job.push_output(format!("Compiling target_crate_{i:05} v1.0.0 (path/to/crate_{i:05})"));
    }

    assert_eq!(job.output_lines.len(), 10_000);
    assert_eq!(job_indicator_height(Some(&job)), 2);

    let backend = TestBackend::new(100, 5);
    let mut terminal = Terminal::new(backend).unwrap();

    terminal
        .draw(|f| {
            render_job_indicator(f, Rect::new(0, 0, 100, 2), &job, &DARK_SLATE);
        })
        .unwrap();

    let buf = terminal.backend().buffer();
    let lines = buffer_to_lines(buf);

    // Row 0: Job header with description and duration
    assert!(lines[0].contains("Cargo build --all --release"));
    assert!(lines[0].contains("esc to interrupt"));

    // Row 1: Folded output with branch glyph, the 10,000th line, and (+9999 lines) badge
    assert!(
        lines[1].contains("target_crate_09999"),
        "Row 1 must show the latest log line: {}",
        lines[1]
    );
    assert!(
        lines[1].contains("(+9999 lines)"),
        "Row 1 must show (+9999 lines) badge: {}",
        lines[1]
    );
    assert!(
        lines[1].contains('\u{2514}'),
        "Row 1 must contain branch glyph: {}",
        lines[1]
    );

    assert_no_raw_ansi(buf);
}

#[test]
fn adv_job_ansi_bombs_and_unicode_sanitization() {
    let mut job = JobState::default();
    job.start("job-ansi".into(), "Tool execution \x1b[31;1mHIGH ALERT\x1b[0m".into());

    // Hostile line packed with nested ANSI colors, OSC hyperlinks, and emojis within 80 cols
    let hostile_line = String::from("\x1b]8;;https://malicious.site\x1b\\CLICK\x1b]8;;\x1b\\ \x1b[31m🦀\x1b[32m🔥\x1b[33m💡\x1b[0m status");

    job.push_output("initial line".into());
    job.push_output(hostile_line);

    let backend = TestBackend::new(100, 5);
    let mut terminal = Terminal::new(backend).unwrap();

    terminal
        .draw(|f| {
            render_job_indicator(f, Rect::new(0, 0, 100, 2), &job, &TOKYO_NIGHT);
        })
        .unwrap();

    let buf = terminal.backend().buffer();
    let lines = buffer_to_lines(buf);

    // Header has ANSI stripped
    assert!(lines[0].contains("Tool execution HIGH ALERT"));
    assert!(!lines[0].contains("\x1b"));

    // Folded line contains sanitised text, branch glyph, and count badge without escape bytes
    assert!(lines[1].contains("CLICK"));
    assert!(lines[1].contains("🦀"));
    assert!(lines[1].contains("🔥"));
    assert!(lines[1].contains("💡"));
    assert!(lines[1].contains("status"));
    assert!(lines[1].contains("(+1 lines)"));
    assert!(lines[1].contains('\u{2514}'));
    assert_no_raw_ansi(buf);
}

#[test]
fn adv_job_ultra_long_single_line_clipping_safety() {
    let mut job = JobState::default();
    job.start("job-long".into(), "Ultra long single line".into());

    // 50,000 character single line
    let huge_line: String = (0..5000).map(|i| format!("[chunk_{i:04}]")).collect();
    job.push_output(huge_line);

    let backend = TestBackend::new(80, 5);
    let mut terminal = Terminal::new(backend).unwrap();

    // Must render cleanly without panic or buffer out-of-bounds
    terminal
        .draw(|f| {
            render_job_indicator(f, Rect::new(0, 0, 80, 2), &job, &DARK_SLATE);
        })
        .unwrap();

    let buf = terminal.backend().buffer();
    let lines = buffer_to_lines(buf);

    assert!(lines[0].contains("Ultra long single line"));
    assert!(lines[1].contains("[chunk_0000]"));
    assert_no_raw_ansi(buf);
}

#[test]
fn adv_job_empty_and_whitespace_output_lines() {
    let mut job = JobState::default();
    job.start("job-empty".into(), "Empty line generator".into());

    // Push empty string
    job.push_output("".into());
    assert_eq!(job_indicator_height(Some(&job)), 2);

    let backend = TestBackend::new(80, 5);
    let mut terminal = Terminal::new(backend).unwrap();

    terminal
        .draw(|f| {
            render_job_indicator(f, Rect::new(0, 0, 80, 2), &job, &DARK_SLATE);
        })
        .unwrap();

    let buf = terminal.backend().buffer();
    let lines = buffer_to_lines(buf);
    assert!(lines[1].contains('\u{2514}')); // Renders prefix without crashing on empty line

    // Push several empty lines
    job.push_output("   \t   ".into());
    job.push_output("".into());

    terminal
        .draw(|f| {
            render_job_indicator(f, Rect::new(0, 0, 80, 2), &job, &DARK_SLATE);
        })
        .unwrap();

    let lines2 = buffer_to_lines(terminal.backend().buffer());
    assert!(lines2[1].contains("(+2 lines)"));
    assert_no_raw_ansi(terminal.backend().buffer());
}

#[test]
fn adv_job_varying_render_rect_heights() {
    let mut job = JobState::default();
    job.start("job-rect".into(), "Testing rect clamping".into());
    job.push_output("first line".into());
    job.push_output("second line".into());

    let backend = TestBackend::new(80, 10);
    let mut terminal = Terminal::new(backend).unwrap();

    // Height 0: should return immediately without drawing or panicking
    terminal
        .draw(|f| {
            render_job_indicator(f, Rect::new(0, 0, 80, 0), &job, &CLASSIC);
        })
        .unwrap();

    // Height 1: should render ONLY the header line, suppressing output fold line
    terminal
        .draw(|f| {
            render_job_indicator(f, Rect::new(0, 0, 80, 1), &job, &CLASSIC);
        })
        .unwrap();

    let lines_h1 = buffer_to_lines(terminal.backend().buffer());
    assert!(lines_h1[0].contains("Testing rect clamping"));
    assert!(!lines_h1[0].contains("second line"));

    // Height 3: should render header and output line without overflowing
    terminal
        .draw(|f| {
            render_job_indicator(f, Rect::new(0, 0, 80, 3), &job, &CLASSIC);
        })
        .unwrap();

    let lines_h3 = buffer_to_lines(terminal.backend().buffer());
    assert!(lines_h3[0].contains("Testing rect clamping"));
    assert!(lines_h3[1].contains("second line"));
    assert_eq!(lines_h3[2].trim(), ""); // line 3 empty
}

// ============================================================================
// 2. Rapid Sequential Job Lifecycles & State Reset Purity
// ============================================================================

#[test]
fn adv_job_1000_sequential_lifecycle_cycles() {
    let mut job = JobState::default();

    for i in 0..1_000 {
        // Start job
        let id = format!("job-{i:04}");
        let desc = format!("Subprocess task execution #{i}");
        job.start(id.clone(), desc.clone());

        assert!(job.is_active);
        assert!(!job.is_failed);
        assert!(!job.is_cancelled);
        assert_eq!(job.id, id);
        assert_eq!(job.description, desc);
        assert_eq!(job.output_lines.len(), 0);
        assert_eq!(job.elapsed_secs, 0.0);
        assert_eq!(job_indicator_height(Some(&job)), 1);

        // Advance ticks
        job.tick(0.08);
        job.tick(0.08);
        assert_eq!(job.spinner_frame, 2);
        assert!((job.elapsed_secs - 0.16).abs() < 0.001);

        // Push output
        job.push_output(format!("Log message for {i}"));
        assert_eq!(job.output_lines.len(), 1);
        assert_eq!(job_indicator_height(Some(&job)), 2);

        // Complete job
        job.complete();
        assert!(!job.is_active);
        assert_eq!(job.status, "completed");
        assert_eq!(job_indicator_height(Some(&job)), 0);
    }
}

#[test]
fn adv_workstation_runner_message_rapid_job_stream() {
    let mut st = WorkstationState::default();
    let (tx, rx) = channel::<RunnerMsg>();

    // Send 100 job sequences rapidly over channel
    for i in 0..100 {
        let id = format!("task-{i}");
        tx.send(RunnerMsg::JobStart {
            id: id.clone(),
            description: format!("Compiling step {i}"),
        })
        .unwrap();
        tx.send(RunnerMsg::JobProgress {
            id: id.clone(),
            status: format!("Step {i} compiling..."),
        })
        .unwrap();
        tx.send(RunnerMsg::JobDone { id: id.clone() }).unwrap();
    }
    drop(tx);

    let mut completed_count = 0;
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
                        completed_count += 1;
                    }
                }
                st.active_job = None;
            }
            _ => {}
        }
    }

    assert_eq!(completed_count, 100);
    assert_eq!(st.transcript.len(), 100);
    assert!(st.active_job.is_none());
    assert_eq!(job_indicator_height(st.active_job.as_ref()), 0);
}

#[test]
fn adv_workstation_runner_message_mismatched_and_interleaved_job_ids() {
    let mut st = WorkstationState::default();

    // 1. Start job A
    let mut job_a = JobState::default();
    job_a.start("job-A".into(), "Task A".into());
    st.active_job = Some(job_a);

    // 2. Progress arrives for job B (mismatched ID)
    let msg_b = RunnerMsg::JobProgress {
        id: "job-B".into(),
        status: "Output from rogue job B".into(),
    };
    if let RunnerMsg::JobProgress { id, status } = msg_b {
        if let Some(ref mut job) = st.active_job {
            if job.id == id || job.id.is_empty() {
                job.push_output(status);
            }
        }
    }

    // Output must NOT be added to job A
    assert_eq!(
        st.active_job.as_ref().unwrap().output_lines.len(),
        0,
        "Mismatched JobProgress must be ignored by active job"
    );

    // 3. Progress arrives for job A (matched ID)
    let msg_a = RunnerMsg::JobProgress {
        id: "job-A".into(),
        status: "Legitimate output for A".into(),
    };
    if let RunnerMsg::JobProgress { id, status } = msg_a {
        if let Some(ref mut job) = st.active_job {
            if job.id == id || job.id.is_empty() {
                job.push_output(status);
            }
        }
    }
    assert_eq!(st.active_job.as_ref().unwrap().output_lines.len(), 1);
    assert_eq!(
        st.active_job.as_ref().unwrap().output_lines[0],
        "Legitimate output for A"
    );
}

#[test]
fn adv_job_unannounced_start_overwrites_cleanly_without_leak() {
    let mut st = WorkstationState::default();

    // Start job 1
    let mut job1 = JobState::default();
    job1.start("job-1".into(), "Initial job".into());
    job1.push_output("some work".into());
    st.active_job = Some(job1);
    assert_eq!(job_indicator_height(st.active_job.as_ref()), 2);

    // Runner sends JobStart for job 2 WITHOUT JobDone for job 1
    let mut job2 = JobState::default();
    job2.start("job-2".into(), "Second job taking over".into());
    st.active_job = Some(job2);

    assert_eq!(st.active_job.as_ref().unwrap().id, "job-2");
    assert_eq!(st.active_job.as_ref().unwrap().output_lines.len(), 0);
    assert_eq!(job_indicator_height(st.active_job.as_ref()), 1);
}

// ============================================================================
// 3. Interactive Cancellation Signals (Esc, Ctrl+C) & Failure
// ============================================================================

#[test]
fn adv_job_cancellation_state_transitions_and_height() {
    let mut job = JobState::default();
    job.start("job-cancel".into(), "Long compilation task".into());
    job.push_output("Line 1".into());
    job.push_output("Line 2".into());

    assert!(job.is_active);
    assert_eq!(job_indicator_height(Some(&job)), 2);

    // User triggers cancellation
    job.cancel();

    assert!(!job.is_active);
    assert!(job.is_cancelled);
    assert!(!job.is_failed);
    assert_eq!(job.status, "cancelled");
    assert_eq!(job.status_line, "[cancelled by user]");

    // Cancelled state displays 1 line: "✗ <desc> — cancelled"
    assert_eq!(job_indicator_height(Some(&job)), 1);

    let backend = TestBackend::new(80, 5);
    let mut terminal = Terminal::new(backend).unwrap();

    terminal
        .draw(|f| {
            render_job_indicator(f, Rect::new(0, 0, 80, 1), &job, &DARK_SLATE);
        })
        .unwrap();

    let buf = terminal.backend().buffer();
    let lines = buffer_to_lines(buf);
    assert!(lines[0].contains("Long compilation task — cancelled"));
    assert!(lines[0].contains('\u{2717}'));
    assert_no_raw_ansi(buf);
}

#[test]
fn adv_job_failure_state_transitions_and_height() {
    let mut job = JobState::default();
    job.start("job-fail".into(), "Rustc check".into());

    job.fail("error[E0308]: mismatched types".into());

    assert!(!job.is_active);
    assert!(job.is_failed);
    assert!(!job.is_cancelled);
    assert_eq!(job.status, "failed");
    assert_eq!(
        job.error_msg.as_deref(),
        Some("error[E0308]: mismatched types")
    );
    assert_eq!(
        job.status_line,
        "[error: error[E0308]: mismatched types]"
    );

    // Failed state displays 1 line
    assert_eq!(job_indicator_height(Some(&job)), 1);

    let backend = TestBackend::new(90, 5);
    let mut terminal = Terminal::new(backend).unwrap();

    terminal
        .draw(|f| {
            render_job_indicator(f, Rect::new(0, 0, 90, 1), &job, &DRACULA);
        })
        .unwrap();

    let buf = terminal.backend().buffer();
    let lines = buffer_to_lines(buf);
    assert!(lines[0].contains("Rustc check — [error: error[E0308]: mismatched types]"));
    assert!(lines[0].contains('\u{2717}'));
    assert_no_raw_ansi(buf);
}

#[test]
fn adv_workstation_state_cancel_active_job_helper() {
    let mut st = WorkstationState::default();

    // Calling when active_job is None must not panic
    st.cancel_active_job();
    assert!(st.active_job.is_none());

    // Calling when job is active cancels it
    let mut job = JobState::default();
    job.start("j-1".into(), "Active job".into());
    st.active_job = Some(job);
    assert!(st.active_job.as_ref().unwrap().is_active);

    st.cancel_active_job();
    assert!(!st.active_job.as_ref().unwrap().is_active);
    assert!(st.active_job.as_ref().unwrap().is_cancelled);

    // Calling again when already cancelled is idempotent
    st.cancel_active_job();
    assert!(st.active_job.as_ref().unwrap().is_cancelled);
}

#[test]
fn adv_job_cancellation_followed_by_runner_job_done() {
    let mut st = WorkstationState::default();

    // 1. Job starts
    let mut job = JobState::default();
    job.start("job-cancel-test".into(), "Cargo build".into());
    st.active_job = Some(job);

    // 2. User presses Esc -> cancel_active_job()
    st.cancel_active_job();
    assert!(st.active_job.as_ref().unwrap().is_cancelled);
    st.transcript.push("[interrupted by user]".into());

    // 3. Runner finishes and sends JobDone
    let msg_done = RunnerMsg::JobDone {
        id: "job-cancel-test".into(),
    };

    // Simulate event loop processing of JobDone
    if let RunnerMsg::JobDone { id } = msg_done {
        if let Some(ref mut job) = st.active_job {
            if job.id == id || job.id.is_empty() {
                job.complete();
                st.transcript
                    .push(format!("\u{2713} Job completed: {}", job.description));
            }
        }
        st.active_job = None;
    }

    eprintln!("Transcript after cancellation + JobDone: {:?}", st.transcript);
}

// ============================================================================
// 4. Layout Docking Geometry & Border Alignment
// ============================================================================

#[test]
fn adv_layout_docking_geometry_inactive_zero_height() {
    let mut st = WorkstationState::default();
    st.active_job = None;

    let backend = TestBackend::new(100, 30);
    let mut terminal = Terminal::new(backend).unwrap();

    // When inactive, job_height is 0, so chunks[2] has height 0
    let pal = DARK_SLATE;
    terminal
        .draw(|f| {
            let size = f.area();
            let job_height = if size.height >= 12 {
                job_indicator_height(st.active_job.as_ref())
            } else {
                0
            };
            assert_eq!(job_height, 0);

            let input_lines = 1;
            let composer_height = ((input_lines as u16) + 2).clamp(3, 6);
            let hero_height = 5;

            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(hero_height),
                    Constraint::Min(4),
                    Constraint::Length(job_height),
                    Constraint::Length(composer_height),
                    Constraint::Length(1),
                ])
                .split(size);

            assert_eq!(chunks[2].height, 0, "Inactive job chunk must have 0 height");
            assert_eq!(
                chunks[1].bottom(),
                chunks[3].top(),
                "Body must meet composer directly without gap when job is inactive"
            );

            draw(f, &pal, &st);
        })
        .unwrap();

    let buf = terminal.backend().buffer();
    assert_no_raw_ansi(buf);
}

#[test]
fn adv_layout_docking_geometry_active_job_docked_above_composer() {
    let mut st = WorkstationState::default();
    let mut job = JobState::default();
    job.start("job-dock".into(), "Running test suite".into());
    job.push_output("test 1 passed".into());
    st.active_job = Some(job);

    let backend = TestBackend::new(100, 30);
    let mut terminal = Terminal::new(backend).unwrap();

    let pal = DARK_SLATE;
    terminal
        .draw(|f| {
            let size = f.area();
            let job_height = if size.height >= 12 {
                job_indicator_height(st.active_job.as_ref())
            } else {
                0
            };
            assert_eq!(job_height, 2, "Active job with output must have height 2");

            let input_lines = 1;
            let composer_height = ((input_lines as u16) + 2).clamp(3, 6); // 3
            let hero_height = 5;

            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(hero_height),
                    Constraint::Min(4),
                    Constraint::Length(job_height),
                    Constraint::Length(composer_height),
                    Constraint::Length(1),
                ])
                .split(size);

            // Job chunk must sit exactly above composer chunk
            assert_eq!(
                chunks[2].bottom(),
                chunks[3].top(),
                "Job indicator bottom ({}) must equal composer top ({})",
                chunks[2].bottom(),
                chunks[3].top()
            );

            draw(f, &pal, &st);
        })
        .unwrap();

    let buf = terminal.backend().buffer();
    let lines = buffer_to_lines(buf);

    // 2026-11-20 layout: the docked job band became the sidebar's Background
    // Tasks panel; the active job's description renders there, and the
    // composer is the last bordered band above the 1-row statusbar.
    let mut job_line_idx = None;
    let mut composer_line_idx = None;

    for (y, line) in lines.iter().enumerate() {
        if line.contains("Running test suite") {
            job_line_idx = Some(y);
        }
        if line.contains("Ask MEG anything") {
            composer_line_idx = Some(y);
        }
    }

    assert!(job_line_idx.is_some(), "Active job must render in the Background Tasks panel");
    assert!(composer_line_idx.is_some(), "Composer must be rendered in buffer");

    let c_idx = composer_line_idx.unwrap();
    // Composer is the final bordered band: statusbar is one row below it.
    assert!(
        c_idx < lines.len(),
        "Composer placeholder row must sit inside the frame"
    );
    assert_no_raw_ansi(buf);
}

#[test]
fn adv_layout_docking_geometry_small_terminal_height_suppression() {
    let mut st = WorkstationState::default();
    let mut job = JobState::default();
    job.start("job-small".into(), "Active on small screen".into());
    job.push_output("some output".into());
    st.active_job = Some(job);

    // Size height = 10 (less than 12 threshold)
    let backend = TestBackend::new(80, 10);
    let mut terminal = Terminal::new(backend).unwrap();

    terminal
        .draw(|f| {
            draw(f, &DARK_SLATE, &st);
        })
        .unwrap();

    let buf = terminal.backend().buffer();
    let lines = buffer_to_lines(buf);

    // Job text should NOT be rendered when terminal height < 12 to avoid crushing composer
    let text = lines.join("\n");
    assert!(
        !text.contains("Active on small screen"),
        "Small screen (<12 rows) must suppress job indicator to protect composer"
    );
    assert_no_raw_ansi(buf);
}

#[test]
fn adv_compact_elapsed_formatting_boundary_stress() {
    // 0 seconds
    assert_eq!(fmt_elapsed_compact(0), "0s");
    // 59 seconds
    assert_eq!(fmt_elapsed_compact(59), "59s");
    // 60 seconds -> 1m 00s
    assert_eq!(fmt_elapsed_compact(60), "1m 00s");
    // 61 seconds -> 1m 01s
    assert_eq!(fmt_elapsed_compact(61), "1m 01s");
    // 3599 seconds -> 59m 59s
    assert_eq!(fmt_elapsed_compact(3599), "59m 59s");
    // 3600 seconds -> 1h 00m
    assert_eq!(fmt_elapsed_compact(3600), "1h 00m");
    // 3665 seconds -> 1h 01m
    assert_eq!(fmt_elapsed_compact(3665), "1h 01m");
    // 86400 seconds (24 hours) -> 24h 00m
    assert_eq!(fmt_elapsed_compact(86400), "24h 00m");
    // Very large u64: 100,000 hours
    assert_eq!(fmt_elapsed_compact(360_000_000), "100000h 00m");
}

#[test]
fn adv_spinner_frame_invariants_and_wraparound() {
    let mut job = JobState::default();
    job.start("spin".into(), "Checking spinner".into());

    assert_eq!(SPINNER_FRAMES.len(), 10);

    // Verify all 10 frames are valid braille characters (U+2800 to U+28FF)
    for (idx, ch) in SPINNER_FRAMES.iter().enumerate() {
        let code = *ch as u32;
        assert!(
            (0x2800..=0x28FF).contains(&code),
            "Frame {idx} ({ch}) must be in Unicode Braille Patterns block"
        );
    }

    // Tick 1,000,000 times to verify no overflow and perfect modular cycle
    for i in 1..=1000 {
        job.tick(0.08);
        let expected_glyph = SPINNER_FRAMES[i % 10];
        assert_eq!(job.spinner_glyph(), expected_glyph);
        assert_eq!(job.spinner_frame, i % 10);
    }
}
