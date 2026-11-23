//! The dashboard's responsive geometry and chrome. Chat rendering lives in viewport.
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph},
    Frame,
};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::{
    render_composer, render_palette, render_picker_modal, render_viewport, render_workspace_picker,
    sanitize_ansi, ActiveModal, Mode, Palette, WorkstationState, APP_VERSION,
};

#[derive(Debug, Clone, Copy)]
pub struct DashboardLayout {
    pub header: Rect,
    pub chat: Rect,
    pub sidebar: Rect,
    pub composer: Rect,
    pub footer: Rect,
}

impl DashboardLayout {
    pub fn new(area: Rect, input: &str) -> Self {
        let margin = u16::from(area.width >= 50);
        let area = Rect::new(
            area.x + margin,
            area.y,
            area.width.saturating_sub(2 * margin),
            area.height,
        );
        let roomy = area.height >= 18;
        let header_height = if roomy && area.width >= 100 {
            4
        } else if area.height >= 10 {
            2
        } else {
            0
        };
        let footer_height = u16::from(area.height >= 10);
        let composer_height = if area.height >= 10 {
            (crate::input_visual_rows(area.width, input).min(4) as u16 + 2)
                .min(area.height / 3)
                .max(3)
        } else {
            area.height.min(1)
        };
        let gap = u16::from(roomy);
        let chunks = Layout::vertical([
            Constraint::Length(header_height),
            Constraint::Length(if header_height >= 4 { 0 } else { gap }),
            Constraint::Min(0),
            Constraint::Length(gap),
            Constraint::Length(composer_height),
            Constraint::Length(gap),
            Constraint::Length(footer_height),
        ])
        .split(area);
        let (chat, sidebar) = if area.width >= 96 && chunks[2].height >= 16 {
            let sidebar_width = (area.width * 27 / 100).clamp(30, 50);
            let body = Layout::horizontal([
                Constraint::Min(1),
                Constraint::Length(2),
                Constraint::Length(sidebar_width),
            ])
            .split(chunks[2]);
            (body[0], body[2])
        } else {
            (chunks[2], Rect::default())
        };
        Self {
            header: chunks[0],
            chat,
            sidebar,
            composer: chunks[4],
            footer: chunks[6],
        }
    }
}

pub fn draw(f: &mut Frame, pal: &Palette, st: &WorkstationState) {
    if let Some(picker) = &st.workspace_picker {
        if picker.fullscreen {
            crate::render_workspace_picker(f, pal, picker);
            return;
        }
    }
    let area = f.area();
    f.render_widget(
        Block::default().style(Style::default().bg(pal.chrome_bg).fg(pal.viewport_fg)),
        area,
    );
    let layout = DashboardLayout::new(area, &st.input);
    render_header(f, layout.header, st, pal);
    render_viewport(f, layout.chat, st, pal);
    render_sidebar(f, layout.sidebar, st, pal);
    render_composer(f, layout.composer, st, pal);
    render_footer(f, layout.footer, st, pal);
    match &st.active_modal {
        ActiveModal::ThemePicker(p) => render_picker_modal(f, area, p, pal),
        ActiveModal::ModelPicker(p) => render_picker_modal(f, area, p, pal),
        ActiveModal::EffortPicker(p) => render_picker_modal(f, area, p, pal),
        ActiveModal::None => {}
    }
    if let Some(picker) = &st.session_picker {
        render_picker_modal(f, area, picker, pal);
    }
    if let Some(picker) = &st.settings_picker {
        render_picker_modal(f, area, picker, pal);
    }
    if let Some(picker) = &st.workspace_picker {
        render_workspace_picker(f, pal, picker);
    }
    if st.palette.open {
        render_palette(f, layout.composer, pal, &st.palette);
    }
    if let Some(dialog) = &st.provider_dialog {
        crate::provider::render_provider(f, area, dialog, pal);
    }
}

/// Clip by terminal cells, preserving whole graphemes and leaving an ellipsis.
pub fn fit_text(text: &str, width: usize) -> String {
    let clean = sanitize_ansi(text).replace(['\n', '\r', '\t'], " ");
    if clean.width() <= width {
        return clean;
    }
    if width == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut used = 0;
    for g in clean.graphemes(true) {
        if used + g.width() > width - 1 {
            break;
        }
        out.push_str(g);
        used += g.width();
    }
    out.push('…');
    out
}

fn friendly_model(id: &str) -> String {
    if id.is_empty() {
        return "Select a model".into();
    }
    let base = ["-medium", "-high", "-low", "-max", "-xhigh"]
        .iter()
        .find_map(|suffix| id.strip_suffix(suffix))
        .unwrap_or(id);
    base.split('-')
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(c) => format!("{}{}", c.to_uppercase(), chars.as_str()),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn render_header(f: &mut Frame, area: Rect, st: &WorkstationState, pal: &Palette) {
    if area.is_empty() {
        return;
    }
    let bold = Style::default()
        .fg(pal.titlebar_fg)
        .add_modifier(Modifier::BOLD);
    let dim = Style::default().fg(pal.chrome_fg);
    let active = usize::from(st.busy)
        + st.subagents
            .iter()
            .filter(|(_, status)| status == "running")
            .count();
    let mode = if st.mode == Mode::Build {
        "Act"
    } else {
        st.mode.label()
    };
    let status_color = if st.online { pal.success } else { pal.warning };
    let row = if area.height >= 4 { area.y + 1 } else { area.y };
    if area.height >= 4 {
        let logo = ["█▄ ▄█  █▀▀  █▀▀", "█ ▀ █  █▀   █ ▄", "█   █  █▄▄  █▄█"];
        for (i, text) in logo.iter().enumerate() {
            f.render_widget(
                Paragraph::new(*text).style(bold),
                Rect::new(area.x + 1, area.y + i as u16, 17, 1),
            );
        }
    } else {
        f.render_widget(
            Paragraph::new(" MEG").style(bold),
            Rect::new(area.x, row, area.width.min(6), 1),
        );
    }
    let start = if area.height >= 4 {
        if area.width >= 140 {
            area.width * 31 / 100
        } else {
            21
        }
    } else {
        7
    };
    let available = area.width.saturating_sub(start);
    let show_details = available >= 75;
    let tail = if show_details { 48 } else { 15 };
    let model_width = available.saturating_sub(tail);
    let mut spans = vec![
        Span::styled("│  ", dim),
        Span::styled("Model: ", bold),
        Span::styled(
            fit_text(
                &friendly_model(&st.model),
                model_width.saturating_sub(10) as usize,
            ),
            Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
        ),
    ];
    let used = spans.iter().map(Span::width).sum::<usize>();
    spans.push(Span::raw(
        " ".repeat((model_width as usize).saturating_sub(used)),
    ));
    if show_details {
        spans.push(Span::styled(format!("│  Agents: {active:<3}  "), bold));
    }
    spans.push(Span::styled("│  ", dim));
    spans.push(Span::styled(
        format!("● {mode}"),
        Style::default()
            .fg(status_color)
            .add_modifier(Modifier::BOLD),
    ));
    if !st.online {
        spans.push(Span::styled(" · offline", dim));
    }
    if show_details {
        spans.push(Span::styled(format!("    │  v{APP_VERSION}"), bold));
    }
    f.render_widget(
        Paragraph::new(Line::from(spans)),
        Rect::new(area.x + start.min(area.width), row, available, 1),
    );
    if area.height > 1 {
        f.render_widget(
            Paragraph::new("─".repeat(area.width as usize))
                .style(Style::default().fg(pal.panel_border)),
            Rect::new(area.x, area.bottom() - 1, area.width, 1),
        );
    }
}

/// Rounded panel with its title on a separate row and a subtle header rule.
pub(crate) fn panel(f: &mut Frame, area: Rect, title: &str, pal: &Palette) -> Rect {
    if area.is_empty() {
        return Rect::default();
    }
    if area.height < 3 {
        return area;
    }
    let mut block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .style(Style::default().bg(pal.viewport_bg))
        .border_style(Style::default().fg(pal.panel_border));
    if area.height < 7 {
        block = block.title(Span::styled(
            format!(" {title} "),
            Style::default()
                .fg(pal.panel_title)
                .add_modifier(Modifier::BOLD),
        ));
        let inner = block.inner(area);
        f.render_widget(block, area);
        return inner;
    }
    let inner = block.inner(area);
    f.render_widget(block, area);
    if inner.is_empty() {
        return inner;
    }
    f.render_widget(
        Paragraph::new(format!(
            " {}",
            fit_text(
                &format!(
                    "{}  {title}",
                    if title == "Chat / Agent" {
                        "▱"
                    } else if title.starts_with("Todo") {
                        "☑"
                    } else if title.starts_with("Background") {
                        "≋"
                    } else {
                        "♟"
                    }
                ),
                inner.width.saturating_sub(2) as usize
            )
        ))
        .style(
            Style::default()
                .fg(pal.panel_title)
                .add_modifier(Modifier::BOLD),
        ),
        Rect::new(inner.x, inner.y, inner.width, 1),
    );
    if inner.height < 3 {
        return Rect::new(inner.x, inner.bottom(), inner.width, 0);
    }
    f.render_widget(
        Paragraph::new("─".repeat(inner.width as usize))
            .style(Style::default().fg(pal.panel_border)),
        Rect::new(inner.x, inner.y + 1, inner.width, 1),
    );
    Rect::new(
        inner.x + 1,
        inner.y + 2,
        inner.width.saturating_sub(2),
        inner.height.saturating_sub(2),
    )
}

fn render_sidebar(f: &mut Frame, area: Rect, st: &WorkstationState, pal: &Palette) {
    if area.is_empty() {
        return;
    }
    // Respect content density while keeping space for each of the three panels.
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(38),
            Constraint::Length(1),
            Constraint::Percentage(29),
            Constraint::Length(1),
            Constraint::Min(4),
        ])
        .split(area);
    let done = st.plan_steps.iter().filter(|(done, _)| *done).count();
    let todo = panel(
        f,
        sections[0],
        &format!("Todo ({done}/{})", st.plan_steps.len()),
        pal,
    );
    if st.plan_steps.is_empty() {
        empty(f, todo, "No task items", "Use /todo add <task>.", pal);
    } else {
        let capacity = todo.height as usize;
        let visible = capacity.saturating_sub(usize::from(st.plan_steps.len() > capacity));
        let mut lines = Vec::new();
        for (done, desc) in st.plan_steps.iter().take(visible) {
            lines.push(Line::from(vec![
                Span::styled(
                    if *done { "☑  " } else { "☐  " },
                    Style::default().fg(if *done { pal.success } else { pal.viewport_fg }),
                ),
                Span::styled(
                    fit_text(desc, todo.width.saturating_sub(3) as usize),
                    Style::default().fg(pal.viewport_fg),
                ),
            ]));
        }
        if st.plan_steps.len() > visible {
            lines.push(Line::styled(
                format!("   +{} more", st.plan_steps.len() - visible),
                Style::default().fg(pal.chrome_fg),
            ));
        }
        f.render_widget(Paragraph::new(lines), todo);
    }
    let active = st.background_tasks.len()
        + usize::from(st.active_job.as_ref().is_some_and(|j| j.is_active));
    let bg = panel(f, sections[2], &format!("Background Tasks ({active})"), pal);
    let mut tasks: Vec<(String, u8, f32, bool, bool)> = Vec::new();
    if let Some(job) = st.active_job.as_ref().filter(|j| j.is_active) {
        tasks.push((job.description.clone(), 0, job.elapsed_secs, true, false));
    }
    let mut recent: Vec<_> = st.background_tasks.iter().collect();
    recent.sort_by_key(|t| !t.active);
    tasks.extend(recent.into_iter().map(|t| {
        (
            t.name.clone(),
            t.progress.min(100),
            t.secs,
            t.active,
            t.failed,
        )
    }));
    if tasks.is_empty() {
        empty(
            f,
            bg,
            "No background work",
            "Tool activity appears here.",
            pal,
        );
    } else {
        let mut lines = Vec::new();
        let capacity = (bg.height / 2) as usize;
        for (name, progress, secs, active, failed) in tasks.iter().take(capacity) {
            let color = if *failed {
                pal.error
            } else if *active {
                pal.viewport_fg
            } else {
                pal.success
            };
            lines.push(Line::styled(
                format!(
                    "{} {}",
                    if *failed {
                        "✗"
                    } else if *active {
                        "▪"
                    } else {
                        "✓"
                    },
                    fit_text(name, bg.width.saturating_sub(2) as usize)
                ),
                Style::default().fg(color),
            ));
            if !*active {
                lines.push(Line::styled(
                    format!("  {} · {secs:.1}s", if *failed { "Failed" } else { "Done" }),
                    Style::default().fg(color),
                ));
            } else if *progress > 0 {
                let width = bg.width.saturating_sub(8) as usize;
                let filled = (width * *progress as usize / 100).min(width);
                lines.push(Line::from(vec![
                    Span::styled(
                        format!("  {}", "█".repeat(filled)),
                        Style::default().fg(if *active { pal.accent } else { pal.success }),
                    ),
                    Span::styled(
                        "░".repeat(width - filled),
                        Style::default().fg(pal.chrome_fg),
                    ),
                    Span::styled(
                        format!(" {progress:>3}%"),
                        Style::default().fg(pal.viewport_fg),
                    ),
                ]));
            } else {
                let frames = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
                let spinner = frames[((*secs * 10.0) as usize) % frames.len()];
                lines.push(Line::styled(
                    format!("  {spinner} Running · {secs:.1}s"),
                    Style::default().fg(pal.accent),
                ));
            }
        }
        f.render_widget(Paragraph::new(lines), bg);
    }
    let sub = panel(
        f,
        sections[4],
        &format!("Subagents ({})", st.subagents.len()),
        pal,
    );
    if st.subagents.is_empty() {
        empty(
            f,
            sub,
            "No delegated workers attached",
            "/agent shows runtime details.",
            pal,
        );
    } else {
        let capacity = sub.height as usize;
        let visible = capacity.saturating_sub(usize::from(st.subagents.len() > capacity));
        let mut lines: Vec<_> = st
            .subagents
            .iter()
            .take(visible)
            .map(|(name, status)| {
                let color = match status.as_str() {
                    "running" | "done" | "completed" => pal.success,
                    "failed" => pal.error,
                    _ => pal.chrome_fg,
                };
                let right = fit_text(status, 9);
                let name_width = sub.width.saturating_sub(right.width() as u16 + 4) as usize;
                let name = fit_text(name, name_width);
                Line::from(vec![
                    Span::styled(
                        if status == "running" { "● " } else { "○ " },
                        Style::default().fg(color),
                    ),
                    Span::styled(
                        format!(
                            "{name}{}",
                            " ".repeat(name_width.saturating_sub(name.width()) + 2)
                        ),
                        Style::default().fg(pal.viewport_fg),
                    ),
                    Span::styled(right, Style::default().fg(color)),
                ])
            })
            .collect();
        if st.subagents.len() > visible {
            lines.push(Line::styled(
                format!("  +{} more", st.subagents.len() - visible),
                Style::default().fg(pal.chrome_fg),
            ));
        }
        f.render_widget(Paragraph::new(lines), sub);
    }
}

fn empty(f: &mut Frame, area: Rect, title: &str, hint: &str, pal: &Palette) {
    f.render_widget(
        Paragraph::new(vec![
            Line::raw(title),
            Line::raw(fit_text(hint, area.width as usize)),
        ])
        .style(Style::default().fg(pal.chrome_fg)),
        area,
    );
}

fn render_footer(f: &mut Frame, area: Rect, st: &WorkstationState, pal: &Palette) {
    if area.is_empty() {
        return;
    }
    if !st.notice.is_empty() {
        f.render_widget(
            Paragraph::new(fit_text(&st.notice, area.width as usize))
                .style(Style::default().fg(pal.warning)),
            area,
        );
        return;
    }
    let hint = if area.width >= 110 {
        "/ commands   Ctrl+T thinking   PgUp/PgDn scroll"
    } else {
        "/ commands"
    };
    let hint_width = hint.width() as u16;
    let left_width = area.width.saturating_sub(hint_width + 3);
    let branch = if st.git_branch.is_empty() {
        String::new()
    } else {
        format!("   ⎇ git: {}", fit_text(&st.git_branch, 22))
    };
    let path = if st.cwd.is_empty() {
        &st.project
    } else {
        &st.cwd
    };
    let path_width = (left_width as usize).saturating_sub(branch.width());
    let line = Line::from(vec![
        Span::styled(
            fit_text(path, path_width),
            Style::default().fg(pal.viewport_fg),
        ),
        Span::styled(branch, Style::default().fg(pal.accent)),
    ]);
    f.render_widget(
        Paragraph::new(line),
        Rect::new(area.x, area.y, left_width, 1),
    );
    f.render_widget(
        Paragraph::new(hint).style(Style::default().fg(pal.chrome_fg)),
        Rect::new(
            area.right().saturating_sub(hint_width.min(area.width)),
            area.y,
            hint_width.min(area.width),
            1,
        ),
    );
}
