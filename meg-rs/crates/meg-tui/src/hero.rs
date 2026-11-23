use ratatui::{
    layout::{Alignment, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::{sanitize_ansi, Palette, WorkstationState};

pub const PKG_VERSION: &str = env!("CARGO_PKG_VERSION");

/// The big MEG wordmark, Minimax-Code style: chunky block letters with a
/// gradient from bright accent on top to dimmer at the bottom.
const MEG_BANNER: &[&str] = &[
    "\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2580}\u{2580} \u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2580}\u{2580} \u{2588}\u{2588}     \u{2588}\u{2588} \u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}",
    "\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588} \u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588} \u{2588}\u{2588}     \u{2588}\u{2588} \u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}",
    "\u{2588}\u{2588}     \u{2588}\u{2588} \u{2588}\u{2588}     \u{2588}\u{2588} \u{2588}\u{2588}  \u{2588}  \u{2588}\u{2588} \u{2588}\u{2588}",
    "\u{2588}\u{2588}     \u{2588}\u{2588} \u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588} \u{2588}\u{2588} \u{2588} \u{2588} \u{2588}\u{2588} \u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}",
    "\u{2588}\u{2588}     \u{2588}\u{2588} \u{2588}\u{2588}     \u{2588}\u{2588} \u{2588}\u{2588}  \u{2588}  \u{2588}\u{2588} \u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}",
];

/// Gradient colors for the banner rows (top = brightest).
const BANNER_COLORS: &[(&str, &str)] = &[
    ("#7dd3fc", "#0ea5e9"), // sky-300 -> sky-500
    ("#38bdf8", "#0284c7"), // sky-400 -> sky-600
    ("#0ea5e9", "#0284c7"), // sky-500
    ("#0284c7", "#0369a1"), // sky-600 -> sky-700
    ("#0369a1", "#075985"), // sky-700
];

pub const MIN_BANNER_WIDTH: u16 = 60;
pub const BANNER_LINES: usize = MEG_BANNER.len();

/// Renders the full startup screen: big MEG banner + version + tagline.
pub fn render_hero(frame: &mut Frame, area: Rect, state: &WorkstationState, pal: &Palette) {
    if area.is_empty() || area.width < 30 {
        return;
    }

    let mut lines: Vec<Line> = Vec::new();

    // Spacer above banner
    let top_spacer = (area.height.saturating_sub(BANNER_LINES as u16 + 6) / 2).max(1) as usize;
    for _ in 0..top_spacer {
        lines.push(Line::from(""));
    }

    // Banner: each row uses a gradient color pair
    let banner_width = area.width.saturating_sub(4) as usize;
    for (i, row) in MEG_BANNER.iter().enumerate() {
        let (top_c, _bot_c) = BANNER_COLORS[i.min(BANNER_COLORS.len() - 1)];
        let centered = center_text(row, banner_width);
        lines.push(Line::from(vec![Span::styled(
            centered,
            Style::default().fg(parse_color(top_c)),
        )]));
    }

    // Tagline
    lines.push(Line::from(""));
    let tagline = "local-first agentic coding harness";
    lines.push(Line::from(Span::styled(
        center_text(tagline, banner_width),
        Style::default()
            .fg(pal.chrome_fg)
            .add_modifier(Modifier::DIM),
    )));

    // Version + workspace
    lines.push(Line::from(""));
    let clean_ws = sanitize_ansi(&state.project);
    let info = format!("v{PKG_VERSION}  \u{00b7}  {clean_ws}");
    lines.push(Line::from(Span::styled(
        center_text(&info, banner_width),
        Style::default().fg(pal.accent),
    )));

    // Tips
    lines.push(Line::from(""));
    let tip_text = "type / for commands  \u{00b7}  describe a task to begin";
    lines.push(Line::from(Span::styled(
        center_text(tip_text, banner_width),
        Style::default()
            .fg(pal.chrome_fg)
            .add_modifier(Modifier::DIM),
    )));

    frame.render_widget(Paragraph::new(lines).alignment(Alignment::Center), area);
}

/// The compact 1-line header shown during an active session (not startup).
pub fn render_session_header(
    frame: &mut Frame,
    area: Rect,
    state: &WorkstationState,
    pal: &Palette,
) {
    if area.is_empty() || area.width < 20 {
        return;
    }
    let model = sanitize_ansi(&state.model);
    let ws = sanitize_ansi(&state.project);
    let text = format!(" \u{26a1} MEG \u{00b7} {model} \u{00b7} {ws} ");
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            text,
            Style::default()
                .fg(pal.viewport_fg)
                .add_modifier(Modifier::BOLD),
        ))),
        area,
    );
}

fn center_text(text: &str, width: usize) -> String {
    let len = text.chars().count();
    if len >= width {
        return text.to_string();
    }
    let left = (width - len) / 2;
    format!("{}{}", " ".repeat(left), text)
}

fn parse_color(hex: &str) -> ratatui::style::Color {
    // Parse "#rrggbb" into ratatui Color
    if hex.len() == 7 && hex.starts_with('#') {
        let r = u8::from_str_radix(&hex[1..3], 16).unwrap_or(0);
        let g = u8::from_str_radix(&hex[3..5], 16).unwrap_or(0);
        let b = u8::from_str_radix(&hex[5..7], 16).unwrap_or(0);
        return ratatui::style::Color::Rgb(r, g, b);
    }
    ratatui::style::Color::White
}
