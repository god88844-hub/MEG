//! `/ui pro` — Titanium Dark Dual-TUI Frontend:
//! - Obsidian/Titanium palette with user `~/.meg/ui.json` RGB overrides
//! - Live Single-Model vs. Multi-Model Orchestration HUD (Antigravity / Devin Fusion / L3 Reviewer)
//! - Collapsible left-accented ToolCards & ANSI-safe transcript viewport
//! - Right Inspector Rail: Orchestration/Token HUD, Plan/Subagent tree, and Background Jobs
//! - Responsive layout collapsing cleanly on narrow/short terminals without panicking

use crate::{
    dashboard::fit_text,
    viewport::sanitize_ansi,
    render_palette, render_picker_modal, render_workspace_picker,
    ui_config::UiFrontend,
    ActiveModal, Palette, WorkstationState,
};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph, Wrap},
    Frame,
};
use unicode_width::UnicodeWidthStr;

/// Titanium Dark palette for `/ui pro` (kept separate from `ALL_PALETTES` to preserve Classic tests).
pub const TITANIUM_DARK: Palette = Palette {
    name: "Titanium Dark",
    chrome_bg: Color::Rgb(11, 14, 20),
    chrome_fg: Color::Rgb(148, 163, 184),
    titlebar_bg: Color::Rgb(17, 22, 32),
    titlebar_fg: Color::Rgb(241, 245, 249),
    panel_border: Color::Rgb(46, 55, 72),
    panel_title: Color::Rgb(56, 189, 248),
    viewport_bg: Color::Rgb(15, 19, 28),
    viewport_fg: Color::Rgb(226, 232, 240),
    accent: Color::Rgb(56, 189, 248),
    success: Color::Rgb(52, 211, 153),
    error: Color::Rgb(248, 113, 113),
    warning: Color::Rgb(251, 191, 36),
};

pub struct ProFrontend;

impl UiFrontend for ProFrontend {
    fn name(&self) -> &'static str {
        "pro"
    }

    fn draw(&self, f: &mut Frame, pal: &Palette, st: &WorkstationState) {
        draw_pro(f, pal, st);
    }
}

pub fn effective_pro_palette(pal: &Palette, st: &WorkstationState) -> Palette {
    let base = if st.theme_snapshot.is_some()
        || st.ui_config.theme_name.is_some()
        || *pal != crate::DARK_SLATE
    {
        *pal
    } else {
        TITANIUM_DARK
    };
    st.ui_config.colors.apply(base)
}

fn orchestration_badge(st: &WorkstationState) -> String {
    match (&st.pair_mode, &st.executor_model, &st.reviewer_model) {
        (Some(mode), Some(exec), Some(rev)) => {
            let short = if mode == "antigravity_orchestrator" {
                "MULTI"
            } else {
                "FUSION"
            };
            format!("{short} {} <-> {} | rev:{rev}", st.model, exec)
        }
        (Some(mode), Some(exec), None) => {
            let short = if mode == "antigravity_orchestrator" {
                "MULTI"
            } else {
                "FUSION"
            };
            format!("{short} {} <-> {}", st.model, exec)
        }
        (_, _, Some(rev)) => format!("SINGLE {} | rev:{rev}", st.model),
        _ => format!("SINGLE {}", st.model),
    }
}

fn render_pro_header(f: &mut Frame, area: Rect, st: &WorkstationState, pal: &Palette) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let w = area.width as usize;
    let status_color = if st.busy {
        pal.warning
    } else if st.online {
        pal.success
    } else {
        pal.chrome_fg
    };
    let status_dot = if st.busy { "● RUNNING" } else { "● READY" };
    let ws_label = if st.cwd.is_empty() {
        "~".to_string()
    } else {
        std::path::Path::new(&st.cwd)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| st.cwd.clone())
    };
    let branch = if st.git_branch.is_empty() {
        String::new()
    } else {
        format!(" ({})", sanitize_ansi(&st.git_branch))
    };
    let orch = orchestration_badge(st);
    let pct = st.context_percent.min(100);
    let filled = (usize::from(pct) * 8) / 100;
    let bar = format!("{}{}", "█".repeat(filled), "░".repeat(8 - filled));

    let line1 = Line::from(vec![
        Span::styled(
            " MEG PRO ",
            Style::default()
                .bg(pal.accent)
                .fg(pal.chrome_bg)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        Span::styled(
            status_dot,
            Style::default().fg(status_color).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" │ ", Style::default().fg(pal.panel_border)),
        Span::styled(
            fit_text(&format!("{ws_label}{branch}"), w.saturating_sub(26).max(8)),
            Style::default().fg(pal.titlebar_fg),
        ),
    ]);

    let mut lines = vec![line1];
    if area.height >= 2 {
        let meta = format!(
            " {} │ mode:{} │ effort:{} │ ctx {} {}%",
            fit_text(&orch, w.saturating_sub(42).max(12)),
            st.mode.label(),
            sanitize_ansi(&st.effort),
            bar,
            pct
        );
        lines.push(Line::from(Span::styled(
            fit_text(&meta, w),
            Style::default().fg(pal.chrome_fg),
        )));
    }

    f.render_widget(
        Paragraph::new(lines).style(Style::default().bg(pal.titlebar_bg).fg(pal.titlebar_fg)),
        area,
    );
}

fn is_tool_card_line(raw: &str) -> bool {
    let trimmed = raw.trim_start();
    trimmed.starts_with("[tool]")
        || trimmed.starts_with("Tool ")
        || trimmed.starts_with("▸ ")
        || trimmed.starts_with("✓ ")
        || trimmed.starts_with("✗ ")
}

fn render_pro_transcript(f: &mut Frame, area: Rect, st: &WorkstationState, pal: &Palette) {
    if area.width < 3 || area.height < 3 {
        return;
    }
    let title = if st.collapse_tool_cards {
        " Transcript [cards:compact] "
    } else {
        " Transcript [cards:expanded] "
    };
    let block = Block::default()
        .title(Span::styled(
            title,
            Style::default().fg(pal.panel_title).add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(pal.panel_border))
        .style(Style::default().bg(pal.viewport_bg).fg(pal.viewport_fg));

    let inner = block.inner(area);
    f.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let w = inner.width as usize;
    let mut lines: Vec<Line> = Vec::new();

    if st.transcript.is_empty() && st.stream.is_none() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  MEG",
            Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(Span::styled(
            "  Code, plan, verify, and run single or multi-model workflows in your terminal.",
            Style::default().fg(pal.chrome_fg),
        )));
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  Quick commands: /ui · /theme · /model · /roles · /help",
            Style::default().fg(pal.chrome_fg),
        )));
    } else {
        let mut collapsed_run = 0usize;
        for entry in &st.transcript {
            let clean = sanitize_ansi(entry);
            if is_tool_card_line(&clean) {
                if st.collapse_tool_cards {
                    collapsed_run += 1;
                    let card_color = if clean.contains("✗") || clean.to_ascii_lowercase().contains("fail") {
                        pal.error
                    } else if clean.contains("✓") {
                        pal.success
                    } else {
                        pal.accent
                    };
                    lines.push(Line::from(vec![
                        Span::styled("▎ ", Style::default().fg(card_color).add_modifier(Modifier::BOLD)),
                        Span::styled(
                            fit_text(&clean, w.saturating_sub(3)),
                            Style::default().fg(pal.chrome_fg),
                        ),
                    ]));
                } else {
                    lines.push(Line::from(vec![
                        Span::styled("╭─ ", Style::default().fg(pal.accent)),
                        Span::styled(
                            fit_text(&clean, w.saturating_sub(4)),
                            Style::default().fg(pal.viewport_fg),
                        ),
                    ]));
                }
            } else {
                collapsed_run = 0;
                for sub in clean.lines() {
                    let style = if sub.starts_with("You:") || sub.starts_with("> ") {
                        Style::default().fg(pal.accent).add_modifier(Modifier::BOLD)
                    } else if sub.starts_with("Error:") || sub.contains("BLOCKED") {
                        Style::default().fg(pal.error)
                    } else {
                        Style::default().fg(pal.viewport_fg)
                    };
                    lines.push(Line::from(Span::styled(sub.to_string(), style)));
                }
            }
        }
        let _ = collapsed_run;
    }

    if let Some(stream) = &st.stream {
        if !stream.thinking_buffer.is_empty() {
            let think_preview = sanitize_ansi(&stream.thinking_buffer);
            lines.push(Line::from(vec![
                Span::styled("◈ thinking: ", Style::default().fg(pal.warning)),
                Span::styled(
                    fit_text(&think_preview, w.saturating_sub(13)),
                    Style::default().fg(pal.chrome_fg),
                ),
            ]));
        }
        if !stream.content_buffer.is_empty() {
            for sub in sanitize_ansi(&stream.content_buffer).lines() {
                lines.push(Line::from(Span::styled(
                    sub.to_string(),
                    Style::default().fg(pal.viewport_fg),
                )));
            }
        }
    }

    let total = lines.len();
    let visible = inner.height as usize;
    let max_scroll = total.saturating_sub(visible);
    let scroll_from_bottom = usize::from(st.transcript_scroll).min(max_scroll);
    let top_row = max_scroll.saturating_sub(scroll_from_bottom);

    f.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .scroll((top_row.min(u16::MAX as usize) as u16, 0))
            .style(Style::default().bg(pal.viewport_bg).fg(pal.viewport_fg)),
        inner,
    );
}

fn render_pro_rail(f: &mut Frame, area: Rect, st: &WorkstationState, pal: &Palette) {
    if area.width < 12 || area.height < 6 {
        return;
    }
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(7),
            Constraint::Min(4),
            Constraint::Length(6),
        ])
        .split(area);

    let rw = area.width.saturating_sub(3) as usize;

    // 1. Orchestration & Usage HUD
    let orch_block = Block::default()
        .title(Span::styled(
            " Orchestration ",
            Style::default().fg(pal.panel_title).add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(pal.panel_border))
        .style(Style::default().bg(pal.chrome_bg).fg(pal.viewport_fg));
    let mode_str = match st.pair_mode.as_deref() { Some("antigravity_orchestrator") => "multi-model", Some("devin_fusion") => "dual-fusion", Some(other) => other, None => "single-model" };
    let exec_str = st.executor_model.as_deref().unwrap_or("-");
    let rev_str = st.reviewer_model.as_deref().unwrap_or("-");
    let orch_lines = vec![
        Line::from(Span::styled(
            fit_text(&format!("Mode: {mode_str}"), rw),
            Style::default().fg(pal.accent),
        )),
        Line::from(Span::styled(
            fit_text(&format!("Plan: {}", st.model), rw),
            Style::default().fg(pal.viewport_fg),
        )),
        Line::from(Span::styled(
            fit_text(&format!("Build: {exec_str}"), rw),
            Style::default().fg(pal.chrome_fg),
        )),
        Line::from(Span::styled(
            fit_text(&format!("Rev  : {rev_str}"), rw),
            Style::default().fg(pal.success),
        )),
        Line::from(Span::styled(
            fit_text(
                &format!(
                    "Tok : {}in/{}out ({}c)",
                    st.usage.input_tokens, st.usage.output_tokens, st.usage.cached_tokens
                ),
                rw,
            ),
            Style::default().fg(pal.chrome_fg),
        )),
    ];
    f.render_widget(Paragraph::new(orch_lines).block(orch_block), chunks[0]);

    // 2. Plan & Subagent Tree
    let tree_block = Block::default()
        .title(Span::styled(
            " Plan & Agents ",
            Style::default().fg(pal.panel_title).add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(pal.panel_border))
        .style(Style::default().bg(pal.chrome_bg).fg(pal.viewport_fg));
    let mut tree_lines: Vec<Line> = Vec::new();
    for (idx, (done, step)) in st.plan_steps.iter().enumerate() {
        let prefix = if idx + 1 == st.plan_steps.len() && st.subagents.is_empty() {
            "└─"
        } else {
            "├─"
        };
        let mark = if *done { "✓" } else { "○" };
        let color = if *done { pal.success } else { pal.viewport_fg };
        tree_lines.push(Line::from(Span::styled(
            fit_text(&format!("{prefix} {mark} {step}"), rw),
            Style::default().fg(color),
        )));
    }
    for (idx, (agent, state)) in st.subagents.iter().enumerate() {
        let prefix = if idx + 1 == st.subagents.len() { "└─" } else { "├─" };
        tree_lines.push(Line::from(Span::styled(
            fit_text(&format!("{prefix} @{agent} [{state}]"), rw),
            Style::default().fg(pal.accent),
        )));
    }
    if tree_lines.is_empty() {
        tree_lines.push(Line::from(Span::styled(
            "  No active plan or subagents",
            Style::default().fg(pal.chrome_fg),
        )));
    }
    f.render_widget(Paragraph::new(tree_lines).block(tree_block), chunks[1]);

    // 3. Background Jobs
    let jobs_block = Block::default()
        .title(Span::styled(
            " Jobs ",
            Style::default().fg(pal.panel_title).add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(pal.panel_border))
        .style(Style::default().bg(pal.chrome_bg).fg(pal.viewport_fg));
    let mut job_lines: Vec<Line> = Vec::new();
    for t in st.background_tasks.iter().rev().take(4) {
        let (sym, color) = if t.failed {
            ("✗", pal.error)
        } else if t.active {
            ("◐", pal.warning)
        } else {
            ("✓", pal.success)
        };
        job_lines.push(Line::from(Span::styled(
            fit_text(&format!("{sym} {} ({:.1}s)", t.name, t.secs), rw),
            Style::default().fg(color),
        )));
    }
    if job_lines.is_empty() {
        job_lines.push(Line::from(Span::styled(
            "  Idle",
            Style::default().fg(pal.chrome_fg),
        )));
    }
    f.render_widget(Paragraph::new(job_lines).block(jobs_block), chunks[2]);
}

fn render_pro_composer(f: &mut Frame, area: Rect, st: &WorkstationState, pal: &Palette) {
    if area.width < 3 || area.height < 2 {
        return;
    }
    let queue_badge = if st.queued_count > 0 {
        format!(" [queued:{}] ", st.queued_count)
    } else {
        String::new()
    };
    let title = format!(" {} ›{queue_badge} ", st.mode.label());
    let block = Block::default()
        .title(Span::styled(
            title,
            Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(pal.accent))
        .style(Style::default().bg(pal.viewport_bg).fg(pal.viewport_fg));

    let inner = block.inner(area);
    let display = if st.input.is_empty() {
        Span::styled(
            "Ask MEG anything or type /ui, /theme, /roles, /help...",
            Style::default().fg(pal.chrome_fg),
        )
    } else {
        Span::styled(sanitize_ansi(&st.input), Style::default().fg(pal.viewport_fg))
    };
    f.render_widget(Paragraph::new(Line::from(display)).block(block), area);

    if inner.width > 0 && inner.height > 0 && matches!(st.active_modal, ActiveModal::None) {
        let prefix_len = st.cursor_position.min(st.input.len());
        let prefix = sanitize_ansi(&st.input[..prefix_len]);
        let col_offset = prefix.width().min(inner.width.saturating_sub(1) as usize) as u16;
        f.set_cursor_position((inner.x + col_offset, inner.y));
    }
}

fn render_pro_footer(f: &mut Frame, area: Rect, st: &WorkstationState, pal: &Palette) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let w = area.width as usize;
    let text = if !st.notice.is_empty() {
        format!(" {} ", sanitize_ansi(&st.notice))
    } else {
        " Tab mode │ /ui layout │ /theme colors │ /roles models │ Ctrl+C x2 exit ".to_string()
    };
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            fit_text(&text, w),
            Style::default().fg(pal.chrome_fg),
        )))
        .style(Style::default().bg(pal.titlebar_bg)),
        area,
    );
}

/// Primary draw entry point for `/ui pro`.
pub fn draw_pro(f: &mut Frame, pal: &Palette, st: &WorkstationState) {
    let eff = effective_pro_palette(pal, st);
    if let Some(picker) = &st.workspace_picker {
        if picker.fullscreen {
            render_workspace_picker(f, &eff, picker);
            return;
        }
    }
    let area = f.area();
    if area.width == 0 || area.height == 0 {
        return;
    }
    f.render_widget(
        Block::default().style(Style::default().bg(eff.chrome_bg).fg(eff.viewport_fg)),
        area,
    );

    let header_h = if area.height >= 12 { 2 } else { 1 };
    let composer_h = if area.height >= 10 { 3 } else { 2 };
    let footer_h = if area.height >= 6 { 1 } else { 0 };

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(header_h),
            Constraint::Min(1),
            Constraint::Length(composer_h),
            Constraint::Length(footer_h),
        ])
        .split(area);

    render_pro_header(f, rows[0], st, &eff);

    if st.ui_config.show_sidebar && rows[1].width >= 76 && rows[1].height >= 8 {
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Min(44), Constraint::Length(32)])
            .split(rows[1]);
        render_pro_transcript(f, cols[0], st, &eff);
        render_pro_rail(f, cols[1], st, &eff);
    } else {
        render_pro_transcript(f, rows[1], st, &eff);
    }

    render_pro_composer(f, rows[2], st, &eff);
    if footer_h > 0 {
        render_pro_footer(f, rows[3], st, &eff);
    }

    // Overlays (identical modal stack as Classic so all pickers & dialogs work seamlessly)
    match &st.active_modal {
        ActiveModal::ThemePicker(p) => render_picker_modal(f, area, p, &eff),
        ActiveModal::ModelPicker(p) => render_picker_modal(f, area, p, &eff),
        ActiveModal::EffortPicker(p) => render_picker_modal(f, area, p, &eff),
        ActiveModal::None => {}
    }
    if let Some(picker) = &st.session_picker {
        render_picker_modal(f, area, picker, &eff);
    }
    if let Some(picker) = &st.settings_picker {
        render_picker_modal(f, area, picker, &eff);
    }
    if let Some(picker) = &st.ui_picker {
        render_picker_modal(f, area, picker, &eff);
    }
    if let Some(picker) = &st.workspace_picker {
        render_workspace_picker(f, &eff, picker);
    }
    if st.palette.open {
        render_palette(f, rows[2], &eff, &st.palette);
    }
    if let Some(dialog) = &st.provider_dialog {
        crate::provider::render_provider(f, area, dialog, &eff);
    }
}