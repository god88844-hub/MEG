//! Regressions through the actual dashboard entry point, rather than a mock UI.
use meg_tui::{draw, render_composer, BackgroundTask, StreamState, WorkstationState, DASHBOARD};
use ratatui::{backend::TestBackend, layout::Rect, Terminal};

fn screen(st: &WorkstationState, width: u16, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|f| draw(f, &DASHBOARD, st)).unwrap();
    let buffer = terminal.backend().buffer();
    (0..height)
        .map(|y| {
            (0..width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn dashboard_displays_streaming_response_and_thinking() {
    let mut st = WorkstationState::default();
    let mut stream = StreamState::default();
    stream.push_thinking("Inspecting the workspace");
    stream.push_content("A live response before completion");
    stream.is_thinking_collapsed = false;
    st.stream = Some(stream);
    let output = screen(&st, 160, 40);
    assert!(
        output.contains("A live response before completion"),
        "live content is missing"
    );
    assert!(
        output.contains("Inspecting the workspace"),
        "expanded thinking is missing"
    );
}

#[test]
fn dashboard_wraps_multiline_answers_without_losing_the_end() {
    let mut st = WorkstationState::default();
    st.transcript.push(format!(
        "\u{2713} {} END_OF_ANSWER\nSECOND_LINE",
        "word ".repeat(70)
    ));
    let output = screen(&st, 80, 30);
    assert!(output.contains("END_OF_ANSWER"), "long answer was clipped");
    assert!(
        output.contains("SECOND_LINE"),
        "embedded newlines were lost"
    );
    assert!(
        !output.contains("Background Tasks"),
        "narrow terminals must prioritize chat"
    );
}

#[test]
fn composer_scrolls_to_the_actual_insertion_point() {
    let mut st = WorkstationState::default();
    st.input = format!("{}TAIL", "x".repeat(120));
    st.move_cursor_end();
    let mut terminal = Terminal::new(TestBackend::new(40, 3)).unwrap();
    terminal
        .draw(|f| render_composer(f, Rect::new(0, 0, 40, 3), &st, &DASHBOARD))
        .unwrap();
    let output = (0..40)
        .map(|x| terminal.backend().buffer()[(x, 1)].symbol())
        .collect::<String>();
    assert!(
        output.contains("TAIL"),
        "the visible input and hardware cursor disagree"
    );
}

#[test]
fn background_elapsed_time_does_not_fabricate_completion_percent() {
    let mut task = BackgroundTask::start("tests");
    task.tick(10.0);
    assert_eq!(
        task.progress, 0,
        "time elapsed is not a measured completion percentage"
    );
    assert_eq!(task.secs, 10.0);
}

#[test]
fn dashboard_survives_resizes_with_populated_state() {
    let mut st = WorkstationState::default();
    st.transcript = vec!["\u{276f} hi".into(), "\u{2713} hello".into()];
    st.plan_steps = vec![(false, "Run tests".into())];
    for (width, height) in [
        (0, 0),
        (1, 1),
        (10, 4),
        (40, 10),
        (80, 24),
        (120, 30),
        (190, 46),
    ] {
        screen(&st, width, height);
    }
}

#[test]
fn short_terminal_retains_chat_content() {
    let mut st = WorkstationState::default();
    st.transcript.push("✓ Short terminal answer".into());
    assert!(screen(&st, 40, 10).contains("Short terminal answer"));
}

#[test]
fn palette_can_reach_commands_beyond_the_first_twelve_and_render_them() {
    let mut st = WorkstationState::default();
    st.palette.open();
    for _ in 0..100 {
        st.palette.move_down();
    }
    assert_eq!(st.palette.selected_command(), Some("/exit"));
    for (width, height) in [(20, 8), (60, 15), (120, 30)] {
        assert!(screen(&st, width, height).contains("/exit"));
    }
}

#[test]
fn run_request_tracks_model_and_workspace_selections() {
    let mut st = WorkstationState::default();
    st.cwd = r"C:\chosen-workspace".into();
    st.model = "chosen-model".into();
    let request = st.run_request("inspect this workspace".into());
    assert_eq!(
        request.workspace,
        std::path::PathBuf::from(r"C:\chosen-workspace")
    );
    assert_eq!(request.model, "chosen-model");
    assert_eq!(request.goal, "inspect this workspace");
}

#[test]
fn scrolling_changes_visible_chat_and_returning_to_tail_restores_it() {
    let mut st = WorkstationState::default();
    for i in 0..60 {
        st.transcript.push(format!("✓ Message {i:02}"));
    }
    let tail = screen(&st, 80, 24);
    assert!(tail.contains("Message 59"));
    st.transcript_scroll = 40;
    let earlier = screen(&st, 80, 24);
    assert!(!earlier.contains("Message 59"));
    st.transcript_scroll = 0;
    assert_eq!(screen(&st, 80, 24), tail);
}

#[test]
fn dashboard_works_in_every_theme_including_the_empty_state() {
    for palette in meg_tui::ALL_PALETTES {
        let st = WorkstationState {
            theme: *palette,
            ..Default::default()
        };
        let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
        terminal.draw(|f| draw(f, palette, &st)).unwrap();
        assert_eq!(terminal.backend().buffer()[(0, 0)].bg, palette.chrome_bg);
    }
    let mut st = WorkstationState::default();
    st.theme = DASHBOARD;
    st.open_theme_picker();
    st.commit_active_modal();
    assert_eq!(st.theme, DASHBOARD);
}

#[test]
fn final_answer_is_displayed_once_and_next_response_starts_clean() {
    let mut st = WorkstationState::default();
    st.stream = Some(StreamState::default());
    st.stream
        .as_mut()
        .unwrap()
        .push_content("A complete answer");
    st.finish_response(Some("A complete answer".into()));
    assert_eq!(screen(&st, 120, 30).matches("A complete answer").count(), 1);
    assert!(st.stream.as_ref().unwrap().is_complete);
    st.stream = Some(StreamState::default());
    st.stream.as_mut().unwrap().push_content("The next answer");
    let output = screen(&st, 120, 30);
    assert_eq!(output.matches("A complete answer").count(), 1);
    assert_eq!(output.matches("The next answer").count(), 1);
}

#[test]
fn thinking_is_preserved_before_the_final_answer_and_expandable_after_resume() {
    let mut st = WorkstationState::default();
    st.stream = Some(StreamState::default());
    st.stream
        .as_mut()
        .unwrap()
        .push_thinking("Checked the workspace");
    st.finish_response(Some("Verified answer".into()));
    let collapsed = screen(&st, 120, 30);
    assert!(collapsed.find("Thought").unwrap() < collapsed.find("Verified answer").unwrap());
    assert!(!collapsed.contains("Checked the workspace"));
    st.stream = None;
    st.toggle_thinking_collapse();
    let expanded = screen(&st, 120, 30);
    assert!(expanded.contains("Checked the workspace"));
    assert_eq!(expanded.matches("Verified answer").count(), 1);
}

#[test]
fn thinking_animates_before_tokens_and_in_both_fold_states_then_stops() {
    for folded in [false, true] {
        let mut st = WorkstationState::default();
        st.stream = Some(StreamState::default());
        assert!(screen(&st, 120, 30).contains("⠋ thinking…"));
        st.stream.as_mut().unwrap().elapsed_secs = 0.125;
        assert!(screen(&st, 120, 30).contains("⠙ thinking…"));
        st.stream
            .as_mut()
            .unwrap()
            .push_thinking("Inspecting files");
        st.stream.as_mut().unwrap().is_thinking_collapsed = folded;
        assert!(screen(&st, 120, 30).contains("⠙ thinking…"));
        st.stream.as_mut().unwrap().elapsed_secs = 0.25;
        assert!(screen(&st, 120, 30).contains("⠹ thinking…"));
        st.stream.as_mut().unwrap().push_content("Answer started");
        let output = screen(&st, 120, 30);
        assert!(output.contains("Thought"));
        assert!(!output.contains("thinking…"));
        st.finish_response(Some("Answer started".into()));
        let finished = screen(&st, 120, 30);
        st.stream.as_mut().unwrap().elapsed_secs = 0.5;
        assert_eq!(screen(&st, 120, 30), finished);
    }
    let mut cancelled = WorkstationState::default();
    cancelled.stream = Some(StreamState::default());
    cancelled.finish_response(None);
    assert!(!screen(&cancelled, 120, 30).contains("thinking…"));
}
