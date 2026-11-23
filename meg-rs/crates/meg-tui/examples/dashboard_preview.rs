//! Offline visual fixture using the production renderer, including a cell snapshot for review.
use meg_tui::{BackgroundTask, StreamState, WorkstationState, DASHBOARD};
use ratatui::{backend::TestBackend, style::Color, Terminal};

fn rgb(color: Color) -> String {
    match color {
        Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
        Color::Black => "#000000".into(),
        Color::White => "#ffffff".into(),
        _ => "#e6edf3".into(),
    }
}

fn main() -> anyhow::Result<()> {
    let args: Vec<_> = std::env::args().collect();
    let mut st = WorkstationState {
        theme: DASHBOARD,
        online: true,
        project: "MEG".into(),
        cwd: r"C:\projects\MEG".into(),
        git_branch: "main".into(),
        model: "gemini-3.8-flash-medium".into(),
        ..Default::default()
    };
    st.transcript = vec!["❯ hi".into(), "✓ Hello! How can I help you today?".into()];
    st.plan_steps = vec![
        (true, "Set up project harness".into()),
        (true, "Integrate core agents".into()),
        (false, "Implement task planner".into()),
        (false, "Add theme support".into()),
        (false, "Run test suite".into()),
        (false, "Update documentation".into()),
        (true, "Clean up and commit".into()),
    ];
    let mut indexing = BackgroundTask::start("Indexing workspace…");
    indexing.progress = 63;
    let mut tests = BackgroundTask::start("Running tests…");
    tests.progress = 28;
    st.background_tasks = vec![indexing, tests];
    st.subagents = vec![
        ("planner".into(), "running".into()),
        ("executor".into(), "idle".into()),
        ("researcher".into(), "idle".into()),
        ("reviewer".into(), "running".into()),
    ];
    if args.iter().any(|arg| arg == "--thinking") {
        let mut stream = StreamState::default();
        stream
            .push_thinking("Interpreting request ✓\nChecking workspace ✓\nPreparing next action…");
        st.stream = Some(stream);
    }
    if args.iter().any(|arg| arg == "--empty") {
        st.transcript.clear();
        st.plan_steps.clear();
        st.background_tasks.clear();
        st.subagents.clear();
    }
    if args.iter().any(|arg| arg == "--palette") {
        st.palette.open();
        for _ in 0..24 {
            st.palette.move_down();
        }
    }
    if args.iter().any(|arg| arg == "--model-picker") {
        st.open_model_picker();
    }
    if args.get(1).map(String::as_str) == Some("--snapshot") {
        let path = args
            .get(2)
            .ok_or_else(|| anyhow::anyhow!("snapshot path required"))?;
        let width = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(160);
        let height = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(44);
        let mut terminal = Terminal::new(TestBackend::new(width, height))?;
        terminal.draw(|f| meg_tui::draw(f, &st.theme, &st))?;
        let buffer = terminal.backend().buffer();
        let mut cells = Vec::new();
        for y in 0..height {
            for x in 0..width {
                let cell = &buffer[(x, y)];
                cells.push(serde_json::json!({"x": x, "y": y, "text": cell.symbol(), "fg": rgb(cell.fg), "bg": rgb(cell.bg), "bold": cell.modifier.contains(ratatui::style::Modifier::BOLD)}));
            }
        }
        std::fs::write(
            path,
            serde_json::to_string(
                &serde_json::json!({"width": width, "height": height, "cells": cells}),
            )?,
        )?;
        println!("Saved actual dashboard frame: {width}x{height}");
    } else {
        meg_tui::run_app(st, None)?;
    }
    Ok(())
}
