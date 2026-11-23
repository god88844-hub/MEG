//! E2E Tests for Feature 5 & 6 (F5 & F6): Streaming, Thinking Display, & Active Job Execution
//!
//! Covers Tier 1 (10 tests) and Tier 2 (10 tests).
//! 100% offline, zero live API calls, asserting on Ratatui TestBackend buffers.

mod common;

use common::*;
use ratatui::layout::Rect;
use std::sync::mpsc::channel;

// ============================================================================
// Tier 1: Feature Coverage (Category-Partition)
// ============================================================================

// --- F5: Live Token Streaming & Thinking Display ---

#[test]
fn t1_stream_thinking_chunks() {
    let mut harness = TestHarness::new(80, 20);
    let mut state = StreamState::default();
    let pal = DARK_SLATE;

    state.push_thinking("Let's analyze the problem step-by-step.\nFirst, check ratatui layout.\n");
    state.elapsed_secs = 2.4;
    state.tok_per_sec = 48.0;
    state.token_count = 115;

    let buf = harness.draw(|f| {
        let area = Rect::new(0, 0, 80, 10);
        render_streaming_view(f, area, &state, &pal);
    });

    let text = buffer_lines(&buf).join("\n");
    assert!(text.contains("Thinking"), "Must render thinking header");
    assert!(text.contains("step-by-step"), "Thinking text rendered");
    assert!(text.contains("2.4s"), "Thinking timer rendered");
    assert!(text.contains("48.0 tok/s"), "Speed metric rendered");
    assert_no_ansi_leaks(&buf);
}

#[test]
fn t1_stream_content_chunks() {
    let mut harness = TestHarness::new(80, 20);
    let mut state = StreamState::default();
    let pal = TOKYO_NIGHT;

    state.push_content("# Analysis Complete\n");
    state.push_content("The ratatui layout has been updated with docked chips.\n");

    let buf = harness.draw(|f| {
        let area = Rect::new(0, 0, 80, 10);
        render_streaming_view(f, area, &state, &pal);
    });

    let text = buffer_lines(&buf).join("\n");
    assert!(text.contains("Analysis Complete"), "Content chunk rendered");
    assert!(text.contains("docked chips"), "Incremental content chunk rendered");
    assert_no_ansi_leaks(&buf);
}

#[test]
fn t1_stream_metrics_tok_sec() {
    let mut state = StreamState::default();
    state.token_count = 521;
    state.elapsed_secs = 10.0;
    state.tok_per_sec = StreamState::calculate_rate(state.token_count, state.elapsed_secs);

    assert!((state.tok_per_sec - 52.1).abs() < 0.05);

    let mut harness = TestHarness::new(80, 10);
    state.push_thinking("Calculating rate test");
    let pal = CLASSIC;
    let buf = harness.draw(|f| {
        let area = Rect::new(0, 0, 80, 8);
        render_streaming_view(f, area, &state, &pal);
    });

    let text = buffer_lines(&buf).join("\n");
    assert!(text.contains("52.1 tok/s"), "Rate metric formatted and rendered");
    assert_no_ansi_leaks(&buf);
}

#[test]
fn t1_stream_metrics_elapsed() {
    let mut state = StreamState::default();
    state.elapsed_secs = 4.2;
    assert_eq!(state.formatted_elapsed(), "4.2s");

    state.elapsed_secs = 75.0; // 1m 15s
    assert_eq!(state.formatted_elapsed(), "1m 15s");

    state.elapsed_secs = 3600.0; // 60m 0s
    assert_eq!(state.formatted_elapsed(), "60m 0s");
}

#[test]
fn t1_stream_collapse_toggle() {
    let mut harness = TestHarness::new(80, 15);
    let mut state = StreamState::default();
    let pal = DRACULA;

    state.push_thinking("Long thinking log\nwith multiple lines of reasoning\n");
    state.elapsed_secs = 3.5;
    state.token_count = 142;

    // Default: expanded
    assert!(!state.is_thinking_collapsed);
    let buf_expanded = harness.draw(|f| {
        let area = Rect::new(0, 0, 80, 10);
        render_streaming_view(f, area, &state, &pal);
    });
    let text_exp = buffer_lines(&buf_expanded).join("\n");
    assert!(text_exp.contains("multiple lines of reasoning"), "Expanded shows details");

    // Toggle: collapsed (Ctrl+T)
    state.toggle_thinking_collapse();
    assert!(state.is_thinking_collapsed);

    let buf_collapsed = harness.draw(|f| {
        let area = Rect::new(0, 0, 80, 10);
        render_streaming_view(f, area, &state, &pal);
    });
    let text_col = buffer_lines(&buf_collapsed).join("\n");
    assert!(text_col.contains("Thought for 3.5s (142 tokens)"), "Collapsed badge shows summary");
    assert!(!text_col.contains("multiple lines of reasoning"), "Collapsed hides detail lines");

    assert_no_ansi_leaks(&buf_expanded);
    assert_no_ansi_leaks(&buf_collapsed);
}

// --- F6: Active Job Execution & Status Indicator ---

#[test]
fn t1_job_indicator_render() {
    let mut harness = TestHarness::new(80, 10);
    let mut job = JobState::default();
    job.start("job-1".into(), "Bash(cargo check)".into());
    job.elapsed_secs = 2.1;
    let pal = DARK_SLATE;

    let buf = harness.draw(|f| {
        let area = Rect::new(0, 0, 80, 2);
        render_job_indicator(f, area, &job, &pal);
    });

    let lines = buffer_lines(&buf);
    assert!(lines[0].contains("Bash(cargo check)"), "Action name rendered");
    assert!(lines[0].contains("2.1s"), "Duration rendered");
    assert!(lines[0].contains("esc to interrupt"), "Cancellation hint rendered");
    assert_no_ansi_leaks(&buf);
}

#[test]
fn t1_job_spinner_animation() {
    let mut job = JobState::default();
    job.start("job-1".into(), "Tool(search)".into());

    let glyph0 = job.spinner_glyph();
    assert_eq!(glyph0, SPINNER_FRAMES[0]);

    job.tick(0.08);
    let glyph1 = job.spinner_glyph();
    assert_eq!(glyph1, SPINNER_FRAMES[1]);
    assert_ne!(glyph0, glyph1);

    // Advance 9 more ticks to cycle
    for _ in 0..9 {
        job.tick(0.08);
    }
    assert_eq!(job.spinner_glyph(), SPINNER_FRAMES[0], "Spinner wraps around");
}

#[test]
fn t1_job_duration_timer() {
    let mut job = JobState::default();
    job.start("job-1".into(), "Edit(src/lib.rs)".into());

    assert_eq!(job.formatted_duration(), "0.0s");
    job.tick(1.24);
    assert_eq!(job.formatted_duration(), "1.2s");
    job.tick(0.56);
    assert_eq!(job.formatted_duration(), "1.8s");
}

#[test]
fn t1_job_folding_output() {
    let mut harness = TestHarness::new(80, 10);
    let mut job = JobState::default();
    job.start("job-1".into(), "Bash(npm test)".into());
    job.push_output("PASS tests/router.test.ts".into());
    job.push_output("PASS tests/mcp.test.ts".into());
    job.push_output("PASS tests/session.test.ts (22 passed)".into());
    let pal = TOKYO_NIGHT;

    let buf = harness.draw(|f| {
        let area = Rect::new(0, 0, 80, 3);
        render_job_indicator(f, area, &job, &pal);
    });

    let lines = buffer_lines(&buf);
    // Line 1 should be folded output showing last line + (+2 lines)
    assert!(lines[1].contains("tests/session.test.ts"), "Latest output line shown");
    assert!(lines[1].contains("(+2 lines)"), "Fold count badge shown");
    assert!(lines[1].contains('\u{2514}'), "Branch icon rendered");
    assert_no_ansi_leaks(&buf);
}

#[test]
fn t1_job_cancellation() {
    let mut harness = TestHarness::new(80, 10);
    let mut job = JobState::default();
    job.start("job-1".into(), "Bash(long-build)".into());
    assert!(job.is_active);

    // Cancel on Esc/Ctrl+C
    job.cancel();
    assert!(!job.is_active);
    assert!(job.is_cancelled);

    let pal = CLASSIC;
    let buf = harness.draw(|f| {
        let area = Rect::new(0, 0, 80, 2);
        render_job_indicator(f, area, &job, &pal);
    });

    let text = buffer_lines(&buf).join("\n");
    assert!(text.contains("cancelled"), "Cancelled status displayed");
    assert_no_ansi_leaks(&buf);
}

// ============================================================================
// Tier 2: Boundary & Corner Cases (Boundary Value Analysis)
// ============================================================================

// --- F5 Boundaries ---

#[test]
fn t2_stream_zero_tokens() {
    let mut harness = TestHarness::new(80, 10);
    let mut state = StreamState::default();
    state.is_complete = true;
    let pal = DARK_SLATE;

    let buf = harness.draw(|f| {
        let area = Rect::new(0, 0, 80, 5);
        render_streaming_view(f, area, &state, &pal);
    });

    // Zero tokens stream finishes cleanly without rendering artifacts or panic
    assert_no_ansi_leaks(&buf);
}

#[test]
fn t2_stream_rapid_burst() {
    let (tx, rx) = channel::<RunnerMsg>();
    let state = std::sync::Arc::new(std::sync::Mutex::new(StreamState::default()));

    // Push 10,000 token chunks across channel rapidly
    for i in 0..10_000 {
        tx.send(RunnerMsg::ContentChunk(format!("word{} ", i))).unwrap();
    }
    drop(tx);

    let mut st = state.lock().unwrap();
    let mut count = 0;
    while let Ok(msg) = rx.try_recv() {
        if let RunnerMsg::ContentChunk(c) = msg {
            st.push_content(&c);
            count += 1;
        }
    }
    assert_eq!(count, 10_000);
    assert!(st.content_buffer.len() > 50_000);
}

#[test]
fn t2_stream_extremely_long_thought() {
    let mut state = StreamState::default();
    // Simulate a massive 10,000-word thinking block
    let long_thought: String = (0..500).map(|i| format!("thought line number {i}\n")).collect();
    state.push_thinking(&long_thought);
    state.elapsed_secs = 45.2;
    state.token_count = 8500;

    let mut harness = TestHarness::new(80, 15);
    let pal = PHOSPHOR;
    let buf = harness.draw(|f| {
        let area = Rect::new(0, 0, 80, 15);
        render_streaming_view(f, area, &state, &pal);
    });

    // Take(10) bounds prevents layout explosion
    assert_no_ansi_leaks(&buf);
}

#[test]
fn t2_stream_channel_disconnect() {
    let (tx, rx) = channel::<RunnerMsg>();
    tx.send(RunnerMsg::ThinkingChunk("thinking...".into())).unwrap();
    // Producer disconnects unexpectedly mid-stream
    drop(tx);

    let mut state = StreamState::default();
    while let Ok(msg) = rx.try_recv() {
        if let RunnerMsg::ThinkingChunk(t) = msg {
            state.push_thinking(&t);
        }
    }
    assert_eq!(state.thinking_buffer, "thinking...");
    // Receiving from disconnected channel returns Err cleanly without panic
    assert!(rx.try_recv().is_err());
}

#[test]
fn t2_stream_tok_sec_divide_by_zero() {
    // 0.0s elapsed should return 0.0 instead of NaN or Infinity
    let rate = StreamState::calculate_rate(100, 0.0);
    assert_eq!(rate, 0.0);
    assert!(!rate.is_nan());
    assert!(!rate.is_infinite());

    // Microsecond duration (0.00001s) protected against float blowup
    let micro_rate = StreamState::calculate_rate(5, 0.00005);
    assert_eq!(micro_rate, 0.0);
}

// --- F6 Boundaries ---

#[test]
fn t2_job_immediate_completion() {
    let mut harness = TestHarness::new(80, 10);
    let mut job = JobState::default();
    // Job starts and immediately completes in 0ms
    job.start("quick-1".into(), "Stat(config.json)".into());
    job.complete();

    let pal = DARK_SLATE;
    let buf = harness.draw(|f| {
        let area = Rect::new(0, 0, 80, 2);
        render_job_indicator(f, area, &job, &pal);
    });

    // Inactive job renders nothing (leaves no dangling UI artifacts)
    let text = buffer_lines(&buf).join("");
    assert!(text.trim().is_empty(), "Inactive job must not leave lingering text");
    assert_no_ansi_leaks(&buf);
}

#[test]
fn t2_job_massive_output() {
    let mut harness = TestHarness::new(80, 10);
    let mut job = JobState::default();
    job.start("job-bulk".into(), "Cargo build --verbose".into());

    // Push 5,000 log lines
    for i in 0..5000 {
        job.push_output(format!("Compiling crate_{i} v0.1.0"));
    }

    let pal = TOKYO_NIGHT;
    let buf = harness.draw(|f| {
        let area = Rect::new(0, 0, 80, 3);
        render_job_indicator(f, area, &job, &pal);
    });

    let lines = buffer_lines(&buf);
    assert!(lines[1].contains("Compiling crate_4999"));
    assert!(lines[1].contains("(+4999 lines)"), "Correctly folds 5000 lines");
    assert_no_ansi_leaks(&buf);
}

#[test]
fn t2_job_multiple_sequential() {
    let mut harness = TestHarness::new(80, 10);
    let mut job = JobState::default();
    let pal = CLASSIC;

    for i in 0..10 {
        job.start(format!("job-{i}"), format!("Step {i} execution"));
        job.push_output(format!("Output for step {i}"));
        job.tick(0.1);
        job.complete();
    }

    let buf = harness.draw(|f| {
        let area = Rect::new(0, 0, 80, 2);
        render_job_indicator(f, area, &job, &pal);
    });

    // Cleaned up after all 10 jobs finished
    assert_no_ansi_leaks(&buf);
}

#[test]
fn t2_job_failure_exit_status() {
    let mut harness = TestHarness::new(80, 10);
    let mut job = JobState::default();
    job.start("test-job".into(), "Bash(pytest)".into());
    job.fail("exit code 1: 3 tests failed".into());

    let pal = DRACULA;
    let buf = harness.draw(|f| {
        let area = Rect::new(0, 0, 80, 2);
        render_job_indicator(f, area, &job, &pal);
    });

    let text = buffer_lines(&buf).join("\n");
    assert!(text.contains("exit code 1: 3 tests failed"), "Error message rendered");
    assert!(text.contains('\u{2717}'), "Error symbol rendered");
    assert_no_ansi_leaks(&buf);
}

#[test]
fn t2_job_cancel_during_flush() {
    let mut job = JobState::default();
    job.start("flush-job".into(), "HeavyIO(write)".into());

    for i in 0..100 {
        job.push_output(format!("chunk {i}"));
        if i == 50 {
            job.cancel();
            break;
        }
    }

    assert!(job.is_cancelled);
    assert!(!job.is_active);
    assert_eq!(job.status_line, "[cancelled by user]");
}
