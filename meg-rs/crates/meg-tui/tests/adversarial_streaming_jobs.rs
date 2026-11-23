//! Adversarial Test Suite for Milestone 3 (M3_STREAMING_JOBS)
//!
//! Empirical stress tests, fuzzers, and boundary verifications for:
//! 1. High-frequency token bursts, empty chunks, interleaved thinking/content chunks, auto-collapsing.
//! 2. Division-by-zero resilience, negative elapsed times, NaN/Inf handling in telemetry.
//! 3. Ctrl+T toggle behavior when pinned vs unpinned.
//! 4. Non-blocking channel draining and event loop frame pacing.
//! 5. Layout stress with simultaneous active job + streaming on extreme terminal dimensions.
//!
//! 100% offline, zero live API calls.

mod common;

use common::*;
use meg_tui::{
    fmt_elapsed_compact, job_indicator_height, render_job_indicator, render_streaming_view,
    render_viewport, JobState, RunnerMsg, StreamState, WorkstationState, DARK_SLATE, DRACULA,
    PHOSPHOR, TOKYO_NIGHT,
};
use ratatui::layout::Rect;
use std::sync::mpsc::channel;

// ============================================================================
// 1. High-Frequency Token Bursts, Empty Chunks, Interleaving, & Auto-Collapsing
// ============================================================================

#[test]
fn adv_stream_burst_50k_tokens_throughput() {
    let (tx, rx) = channel::<RunnerMsg>();
    let mut state = StreamState::default();

    // Send 50,000 rapid token chunks
    for i in 0..50_000 {
        tx.send(RunnerMsg::ContentChunk(format!("t{i} "))).unwrap();
    }
    drop(tx);

    let mut drained = 0;
    while let Ok(msg) = rx.try_recv() {
        if let RunnerMsg::ContentChunk(c) = msg {
            state.push_content(&c);
            drained += 1;
        }
    }

    assert_eq!(drained, 50_000, "Must drain all 50,000 tokens without loss");
    assert!(
        state.content_buffer.len() > 250_000,
        "Buffer must contain all accumulated tokens"
    );
}

#[test]
fn adv_stream_empty_chunks_flood() {
    let (tx, rx) = channel::<RunnerMsg>();
    let mut state = StreamState::default();

    // Flood with alternating empty thinking and content chunks
    for _ in 0..10_000 {
        tx.send(RunnerMsg::ThinkingChunk(String::new())).unwrap();
        tx.send(RunnerMsg::ContentChunk(String::new())).unwrap();
    }
    drop(tx);

    let mut t_count = 0;
    let mut c_count = 0;
    while let Ok(msg) = rx.try_recv() {
        match msg {
            RunnerMsg::ThinkingChunk(t) => {
                state.push_thinking(&t);
                t_count += 1;
            }
            RunnerMsg::ContentChunk(c) => {
                state.push_content(&c);
                c_count += 1;
            }
            _ => {}
        }
    }

    assert_eq!(t_count, 10_000);
    assert_eq!(c_count, 10_000);
    assert_eq!(state.thinking_buffer, "");
    assert_eq!(state.content_buffer, "");
    // Thinking was empty, so push_content should not set is_thinking_collapsed to true
    assert!(!state.is_thinking_collapsed);

    // Verify rendering empty stream does not panic or render phantom lines
    let mut harness = TestHarness::new(80, 10);
    let pal = DARK_SLATE;
    let buf = harness.draw(|f| {
        render_streaming_view(f, Rect::new(0, 0, 80, 10), &state, &pal);
    });
    assert_no_ansi_leaks(&buf);
    let text = buffer_lines(&buf).join("");
    assert!(text.trim().is_empty());
}

#[test]
fn adv_stream_interleaved_thinking_and_content_rapid() {
    let mut state = StreamState::default();

    // Phase 1: Thinking block 1
    state.push_thinking("Thinking step 1...\n");
    assert!(!state.is_thinking_collapsed);

    // Phase 2: Content 1 -> should auto-collapse thinking
    state.push_content("Result 1: Computed 42.\n");
    assert!(
        state.is_thinking_collapsed,
        "Arrival of content must auto-collapse thinking"
    );

    // Phase 3: Thinking block 2 (interleaved multi-step model)
    state.push_thinking("Thinking step 2...\n");
    // Remains collapsed unless user toggles
    assert!(state.is_thinking_collapsed);

    // Phase 4: Content 2
    state.push_content("Result 2: Verified.\n");
    assert!(state.is_thinking_collapsed);

    assert_eq!(
        state.thinking_buffer,
        "Thinking step 1...\nThinking step 2...\n"
    );
    assert_eq!(
        state.content_buffer,
        "Result 1: Computed 42.\nResult 2: Verified.\n"
    );
}

#[test]
fn adv_stream_auto_collapse_pinned_vs_unpinned_lifecycle() {
    let mut state = StreamState::default();

    // 1. Initially unpinned
    assert!(!state.user_pinned_thinking);
    assert!(!state.is_thinking_collapsed);

    state.push_thinking("Initial reasoning trace...\n");
    assert!(!state.is_thinking_collapsed);

    // Content triggers auto-collapse
    state.push_content("First answer word\n");
    assert!(state.is_thinking_collapsed);
    assert!(!state.user_pinned_thinking);

    // 2. User presses Ctrl+T to expand
    state.toggle_thinking_collapse();
    assert!(!state.is_thinking_collapsed, "Now expanded");
    assert!(
        state.user_pinned_thinking,
        "Expanding must set user_pinned_thinking = true"
    );

    // 3. Subsequent content arrives while pinned open -> MUST NOT auto-collapse
    for i in 0..100 {
        state.push_content(&format!("chunk {i}\n"));
        assert!(
            !state.is_thinking_collapsed,
            "Pinned thinking must remain open upon content arrival"
        );
    }

    // 4. User presses Ctrl+T again to collapse
    state.toggle_thinking_collapse();
    assert!(state.is_thinking_collapsed, "Now collapsed");
    assert!(
        !state.user_pinned_thinking,
        "Collapsing must unpin user_pinned_thinking"
    );

    // 5. Subsequent content arrives while unpinned -> remains collapsed
    state.push_content("more content\n");
    assert!(state.is_thinking_collapsed);
}

#[test]
fn adv_stream_thinking_with_newlines_and_ansi_bomb() {
    let mut state = StreamState::default();
    let hostile_ansi = "\x1b[31;1mDangerous \x1b[2J\x1b[H\x1b[0;32mANSI \x1b]8;;http://evil.com\x07Link\x1b]8;;\x07\x1b[m";
    state.push_thinking(&format!("Line 1: {hostile_ansi}\nLine 2: Normal\n"));
    for i in 3..=25 {
        state.push_thinking(&format!("Line {i}: extra thought step\n"));
    }
    state.elapsed_secs = 5.0;
    state.token_count = 350;
    state.tok_per_sec = 70.0;

    let mut harness = TestHarness::new(100, 20);
    let pal = TOKYO_NIGHT;
    let buf = harness.draw(|f| {
        render_streaming_view(f, Rect::new(0, 0, 100, 15), &state, &pal);
    });

    assert_no_ansi_leaks(&buf);
    let lines = buffer_lines(&buf);

    // Header line
    assert!(lines[0].contains("Thinking"));
    assert!(lines[0].contains("70.0 tok/s"));
    assert!(lines[0].contains("350 tokens"));

    // Verify take(10) bounds thinking lines:
    // Line 0 is header, lines 1..=10 are thoughts, line 11 is divider
    assert!(lines[1].contains("Dangerous ANSI Link"));
    assert!(lines[10].contains("Line 10: extra thought step"));
    // Line 11 is the horizontal rule divider
    assert!(lines[11].contains('\u{2500}'));
    // Line 11.. should NOT contain Line 11 thought step because bounded to 10
    assert!(!lines[11].contains("Line 11"));
}

// ============================================================================
// 2. Adversarial Metric Computations (Div-by-Zero & Float Extremes)
// ============================================================================

#[test]
fn adv_metrics_div_zero_zero_elapsed() {
    let rate = StreamState::calculate_rate(100, 0.0);
    assert_eq!(rate, 0.0);
    assert!(!rate.is_nan());
    assert!(!rate.is_infinite());
}

#[test]
fn adv_metrics_div_zero_sub_millisecond() {
    // 0.00005 is less than the 0.0001 epsilon threshold
    let rate = StreamState::calculate_rate(50, 0.00005);
    assert_eq!(rate, 0.0);
    assert!(!rate.is_nan());
    assert!(!rate.is_infinite());

    // Exactly 0.0001 boundary
    let rate_exact = StreamState::calculate_rate(50, 0.0001);
    assert_eq!(rate_exact, 0.0);

    // Slightly above threshold: 0.0002s with 1 token
    let rate_above = StreamState::calculate_rate(1, 0.0002);
    assert_eq!(rate_above, 5000.0);
}

#[test]
fn adv_metrics_negative_elapsed_secs() {
    let rate = StreamState::calculate_rate(100, -5.0);
    // Since -5.0 <= 0.0001, calculate_rate must safely return 0.0
    assert_eq!(rate, 0.0);
    assert!(!rate.is_nan());
    assert!(!rate.is_infinite());
}

#[test]
fn adv_metrics_zero_tokens_positive_time() {
    let rate = StreamState::calculate_rate(0, 15.0);
    assert_eq!(rate, 0.0);
    assert!(!rate.is_nan());
    assert!(!rate.is_infinite());
}

#[test]
fn adv_metrics_massive_token_counts() {
    let rate = StreamState::calculate_rate(usize::MAX, 100.0);
    assert!(!rate.is_nan());
    assert!(rate > 0.0);
}

#[test]
fn adv_formatted_elapsed_time_boundaries() {
    let mut state = StreamState::default();

    state.elapsed_secs = 0.0;
    assert_eq!(state.formatted_elapsed(), "0.0s");

    state.elapsed_secs = 0.04;
    assert_eq!(state.formatted_elapsed(), "0.0s");

    state.elapsed_secs = 0.06;
    assert_eq!(state.formatted_elapsed(), "0.1s");

    state.elapsed_secs = 59.94;
    assert_eq!(state.formatted_elapsed(), "59.9s");

    state.elapsed_secs = 59.96;
    // Under 60.0, rounds to 60.0s
    assert_eq!(state.formatted_elapsed(), "60.0s");

    state.elapsed_secs = 60.0;
    assert_eq!(state.formatted_elapsed(), "1m 0s");

    state.elapsed_secs = 65.5;
    assert_eq!(state.formatted_elapsed(), "1m 5s");

    state.elapsed_secs = 3599.0;
    assert_eq!(state.formatted_elapsed(), "59m 59s");

    state.elapsed_secs = 3600.0;
    assert_eq!(state.formatted_elapsed(), "60m 0s");

    state.elapsed_secs = 7265.0; // 121m 5s
    assert_eq!(state.formatted_elapsed(), "121m 5s");
}

#[test]
fn adv_job_compact_elapsed_boundaries() {
    assert_eq!(fmt_elapsed_compact(0), "0s");
    assert_eq!(fmt_elapsed_compact(59), "59s");
    assert_eq!(fmt_elapsed_compact(60), "1m 00s");
    assert_eq!(fmt_elapsed_compact(61), "1m 01s");
    assert_eq!(fmt_elapsed_compact(3599), "59m 59s");
    assert_eq!(fmt_elapsed_compact(3600), "1h 00m");
    assert_eq!(fmt_elapsed_compact(3665), "1h 01m");
    assert_eq!(fmt_elapsed_compact(86400), "24h 00m");
}

// ============================================================================
// 3. Adversarial Job Lifecycle & Stress
// ============================================================================

#[test]
fn adv_job_output_overflow_100k_lines() {
    let mut job = JobState::default();
    job.start("job-stress".into(), "Fuzzing test runner".into());

    for i in 0..100_000 {
        job.push_output(format!("Log line {i}"));
    }

    assert_eq!(job.output_lines.len(), 100_000);
    assert_eq!(job_indicator_height(Some(&job)), 2);

    let mut harness = TestHarness::new(100, 5);
    let pal = DARK_SLATE;
    let buf = harness.draw(|f| {
        render_job_indicator(f, Rect::new(0, 0, 100, 2), &job, &pal);
    });

    assert_no_ansi_leaks(&buf);
    let lines = buffer_lines(&buf);
    assert!(lines[0].contains("Fuzzing test runner"));
    assert!(lines[1].contains("Log line 99999"));
    assert!(lines[1].contains("(+99999 lines)"));
    assert!(lines[1].contains('\u{2514}'));
}

#[test]
fn adv_job_empty_and_whitespace_output_lines() {
    let mut job = JobState::default();
    job.start("job-ws".into(), "Empty line test".into());
    job.push_output(String::new());
    job.push_output("   \t\n   ".into());

    let mut harness = TestHarness::new(80, 5);
    let pal = PHOSPHOR;
    let buf = harness.draw(|f| {
        render_job_indicator(f, Rect::new(0, 0, 80, 2), &job, &pal);
    });

    assert_no_ansi_leaks(&buf);
    let lines = buffer_lines(&buf);
    assert!(lines[0].contains("Empty line test"));
    assert!(lines[1].contains("(+1 lines)"));
}

#[test]
fn adv_job_ansi_injection_in_description_and_output() {
    let mut job = JobState::default();
    let hostile_desc = "Task \x1b[31;1mDanger\x1b[0m \x1b[2K\x1b[1;1H";
    let hostile_out = "Output \x1b[32mSuccess\x1b[0m \x1b]8;;http://evil.com\x07Link\x1b]8;;\x07";
    job.start("job-ansi".into(), hostile_desc.into());
    job.push_output(hostile_out.into());

    let mut harness = TestHarness::new(80, 5);
    let pal = DRACULA;
    let buf = harness.draw(|f| {
        render_job_indicator(f, Rect::new(0, 0, 80, 2), &job, &pal);
    });

    assert_no_ansi_leaks(&buf);
    let lines = buffer_lines(&buf);
    assert!(lines[0].contains("Task Danger"));
    assert!(lines[1].contains("Output Success Link"));
}

#[test]
fn adv_job_rapid_lifecycle_churn() {
    let mut job = JobState::default();

    for i in 0..500 {
        job.start(format!("job-{i}"), format!("Job #{i}"));
        assert!(job.is_active);
        assert_eq!(job_indicator_height(Some(&job)), 1);

        job.push_output(format!("Output {i}"));
        assert_eq!(job_indicator_height(Some(&job)), 2);

        job.tick(0.05);

        if i % 3 == 0 {
            job.cancel();
            assert!(job.is_cancelled);
            assert_eq!(job_indicator_height(Some(&job)), 1);
        } else if i % 3 == 1 {
            job.fail(format!("Error {i}"));
            assert!(job.is_failed);
            assert_eq!(job_indicator_height(Some(&job)), 1);
        } else {
            job.complete();
            assert!(!job.is_active);
            assert_eq!(job_indicator_height(Some(&job)), 0);
        }
    }
}

// ============================================================================
// 4. Event Loop Non-Blocking Draining & Workstation Integration
// ============================================================================

#[test]
fn adv_event_loop_draining_all_runner_messages() {
    let (tx, rx) = channel::<RunnerMsg>();
    let mut st = WorkstationState::default();

    // Send a stream of all message types in rapid succession
    tx.send(RunnerMsg::PlanStep(false, "Step 1: Check build".into())).unwrap();
    tx.send(RunnerMsg::JobStart {
        id: "job-1".into(),
        description: "Compiling crates".into(),
    }).unwrap();
    tx.send(RunnerMsg::JobProgress {
        id: "job-1".into(),
        status: "compiling meg-tui...".into(),
    }).unwrap();
    tx.send(RunnerMsg::ThinkingChunk("Planning response... ".into())).unwrap();
    tx.send(RunnerMsg::ThinkingChunk("Analyzed layout.".into())).unwrap();
    tx.send(RunnerMsg::StreamMetrics {
        tok_per_sec: 42.5,
        elapsed_secs: 1.2,
        token_count: 51,
    }).unwrap();
    tx.send(RunnerMsg::ContentChunk("Here is the answer: ".into())).unwrap();
    tx.send(RunnerMsg::ContentChunk("success.".into())).unwrap();
    tx.send(RunnerMsg::JobDone { id: "job-1".into() }).unwrap();
    tx.send(RunnerMsg::PlanStep(true, "Step 1: Check build".into())).unwrap();
    tx.send(RunnerMsg::Done(Some("All tasks completed.".into()))).unwrap();
    drop(tx);

    // Simulate run_app draining loop
    while let Ok(msg) = rx.try_recv() {
        match msg {
            RunnerMsg::Line(l) => st.transcript.push(l),
            RunnerMsg::ThinkingChunk(t) => {
                let stream = st.stream.get_or_insert_with(StreamState::default);
                stream.push_thinking(&t);
                stream.token_count += 1;
            }
            RunnerMsg::ContentChunk(c) => {
                let stream = st.stream.get_or_insert_with(StreamState::default);
                stream.push_content(&c);
                stream.token_count += 1;
            }
            RunnerMsg::StreamMetrics {
                tok_per_sec,
                elapsed_secs,
                token_count,
            } => {
                let stream = st.stream.get_or_insert_with(StreamState::default);
                stream.tok_per_sec = tok_per_sec;
                stream.elapsed_secs = elapsed_secs;
                stream.token_count = token_count;
            }
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
                st.active_job = None;
            }
            RunnerMsg::PlanStep(done, desc) => st.plan_steps.push((done, desc)),
            RunnerMsg::Done(final_answer) => {
                st.state_label = "\u{25cf} READY".into();
                if let Some(ref mut stream) = st.stream {
                    stream.is_complete = true;
                    if !stream.user_pinned_thinking {
                        stream.is_thinking_collapsed = true;
                    }
                }
                st.active_job = None;
                if let Some(a) = final_answer {
                    st.transcript.push(format!("\u{2713} {a}"));
                }
            }
            _ => {}
        }
    }

    assert_eq!(st.plan_steps.len(), 2);
    assert!(st.active_job.is_none(), "Completed job is cleared");

    let stream = st.stream.as_ref().unwrap();
    assert_eq!(
        stream.thinking_buffer,
        "Planning response... Analyzed layout."
    );
    assert_eq!(stream.content_buffer, "Here is the answer: success.");
    assert!(
        stream.is_thinking_collapsed,
        "Unpinned thinking auto-collapsed on Done"
    );
    assert_eq!(stream.tok_per_sec, 42.5);
    assert_eq!(stream.elapsed_secs, 1.2);

    let transcript_text = st.transcript.join("\n");
    assert!(transcript_text.contains("Job completed: Compiling crates"));
    assert!(transcript_text.contains("All tasks completed."));
}

// ============================================================================
// 5. Layout Stress with Simultaneous Active Job & Stream on Extreme Dimensions
// ============================================================================

#[test]
fn adv_simultaneous_stream_and_job_extreme_small_screen() {
    let mut harness = TestHarness::new(30, 6);
    let mut st = WorkstationState::default();

    let mut job = JobState::default();
    job.start("j-min".into(), "Cargo check".into());
    job.push_output("Checking...".into());
    st.active_job = Some(job);

    let mut stream = StreamState::default();
    stream.push_thinking("Thinking deeply...\n");
    stream.push_content("Result line\n");
    stream.user_pinned_thinking = true;
    stream.is_thinking_collapsed = false;
    st.stream = Some(stream);

    let pal = DARK_SLATE;

    // Draw on small dimensions: 30x6
    let buf = harness.draw(|f| {
        meg_tui::draw(f, &pal, &st);
    });
    assert_no_ansi_leaks(&buf);

    // Draw on even smaller: 20x4
    let mut tiny_harness = TestHarness::new(20, 4);
    let buf_tiny = tiny_harness.draw(|f| {
        meg_tui::draw(f, &pal, &st);
    });
    assert_no_ansi_leaks(&buf_tiny);
}

#[test]
fn adv_viewport_zero_and_single_width_divider() {
    let mut harness = TestHarness::new(1, 10);
    let mut st = WorkstationState::default();
    let mut stream = StreamState::default();
    stream.push_thinking("Thinking step\n");
    stream.user_pinned_thinking = true;
    stream.is_thinking_collapsed = false;
    st.stream = Some(stream);

    let pal = TOKYO_NIGHT;

    // 1-column wide rendering
    let buf = harness.draw(|f| {
        render_viewport(f, Rect::new(0, 0, 1, 10), &st, &pal);
    });
    assert_no_ansi_leaks(&buf);

    // 0-width rendering
    let mut zero_harness = TestHarness::new(80, 10);
    let buf_zero = zero_harness.draw(|f| {
        render_viewport(f, Rect::new(0, 0, 0, 10), &st, &pal);
    });
    assert_no_ansi_leaks(&buf_zero);
}
