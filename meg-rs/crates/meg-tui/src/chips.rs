use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::{Palette, WorkstationState};

/// Harness execution stance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    #[default]
    Build,
    Plan,
}

impl Mode {
    pub fn next(self) -> Self {
        match self {
            Mode::Build => Mode::Plan,
            Mode::Plan => Mode::Build,
        }
    }

    pub fn prev(self) -> Self {
        self.next()
    }

    pub fn label(self) -> &'static str {
        match self {
            Mode::Build => "BUILD",
            Mode::Plan => "PLAN",
        }
    }

    pub fn badge(self) -> &'static str {
        match self {
            Mode::Build => "[BUILD]",
            Mode::Plan => "[PLAN]",
        }
    }

    pub fn color(self) -> Color {
        match self {
            Mode::Build => Color::Rgb(2, 132, 199), // Blue
            Mode::Plan => Color::Rgb(217, 119, 6),  // Amber
        }
    }

    pub fn fg_color(self) -> Color {
        match self {
            Mode::Build => Color::White,
            Mode::Plan => Color::Black,
        }
    }
}

pub type ComposerMode = Mode;

/// Build the chip bar as a styled Line for a given available width.
pub fn build_chip_line(state: &WorkstationState, pal: &Palette, width: u16) -> Line<'static> {
    let mode = state.mode;
    let mode_span = Span::styled(
        mode.badge(),
        Style::default()
            .fg(mode.fg_color())
            .bg(mode.color())
            .add_modifier(Modifier::BOLD),
    );

    let is_busy = state.state_label.contains("WORKING");
    let dot_color = if is_busy { pal.warning } else { pal.success };
    let dot_span = Span::styled(
        " ● ",
        Style::default().fg(dot_color).add_modifier(Modifier::BOLD),
    );

    let raw_model = if state.model.is_empty() {
        "not selected"
    } else {
        &state.model
    };
    let clean_model = crate::sanitize_ansi(raw_model);
    let model_span = Span::styled(
        clean_model,
        Style::default()
            .fg(pal.viewport_fg)
            .add_modifier(Modifier::BOLD),
    );

    let sep1 = Span::styled(" • ", Style::default().fg(pal.chrome_fg));
    let raw_effort = if state.effort.is_empty() {
        "high"
    } else {
        &state.effort
    };
    let clean_effort = crate::sanitize_ansi(raw_effort);
    let effort_span = Span::styled(
        clean_effort,
        Style::default()
            .fg(Color::Rgb(234, 88, 12))
            .add_modifier(Modifier::BOLD),
    );

    let sep2 = Span::styled(" • ", Style::default().fg(pal.chrome_fg));
    let ctx_pct = state.context_percent;
    let ctx_color = if ctx_pct > 90 {
        pal.error
    } else if ctx_pct > 70 {
        pal.warning
    } else {
        pal.chrome_fg
    };
    let ctx_span = Span::styled(
        format!("ctx {ctx_pct}%"),
        Style::default()
            .fg(ctx_color)
            .add_modifier(if ctx_pct > 90 {
                Modifier::BOLD
            } else {
                Modifier::empty()
            }),
    );

    let mut left_spans = vec![
        Span::styled("╰ ", Style::default().fg(pal.panel_border)),
        mode_span,
        dot_span,
        model_span,
        sep1,
        effort_span,
        sep2,
        ctx_span,
    ];

    let full_hints = "? help · / cmds · Tab mode · Ctrl+C quit ";
    let compact_hints = "? help · / cmds ";
    let right_border = Span::styled("╯", Style::default().fg(pal.panel_border));

    let left_len: usize = left_spans.iter().map(|s| s.content.chars().count()).sum();
    let total_width = width as usize;

    if total_width > left_len + full_hints.chars().count() {
        let space_count = total_width.saturating_sub(left_len + full_hints.chars().count() + 1);
        left_spans.push(Span::raw(" ".repeat(space_count)));
        left_spans.push(Span::styled(full_hints, Style::default().fg(pal.chrome_fg)));
        left_spans.push(right_border);
    } else if total_width > left_len + compact_hints.chars().count() {
        let space_count = total_width.saturating_sub(left_len + compact_hints.chars().count() + 1);
        left_spans.push(Span::raw(" ".repeat(space_count)));
        left_spans.push(Span::styled(
            compact_hints,
            Style::default().fg(pal.chrome_fg),
        ));
        left_spans.push(right_border);
    } else {
        let space_count = total_width.saturating_sub(left_len + 1);
        left_spans.push(Span::raw(" ".repeat(space_count)));
        left_spans.push(right_border);
    }

    Line::from(left_spans)
}

/// Renders the docked chip bar attached to the composer.
pub fn render_chip_bar(frame: &mut Frame, area: Rect, state: &WorkstationState, pal: &Palette) {
    if area.is_empty() {
        return;
    }
    let line = build_chip_line(state, pal, area.width);
    frame.render_widget(
        Paragraph::new(line).style(Style::default().bg(pal.viewport_bg)),
        area,
    );
}
