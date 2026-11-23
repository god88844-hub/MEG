//! The `/` command palette (2026-11-20) — the Cline/Zed-style popup.
//!
//! Typing `/` as the first character (or Ctrl+P) opens a floating bordered
//! list above the composer: essential commands filtered by what you typed,
//! arrow keys to move, Enter to run, Esc to close. Descriptions sit
//! right-aligned and dim, matching the reference screenshots.

use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
    Frame,
};

use crate::theme::Palette;

/// The essential command surface — ported from the Python registry's
/// ESSENTIAL_GROUPS so both harnesses expose the same commands.
pub use crate::commands::ESSENTIAL_COMMANDS;

/// State of the floating `/` palette.
#[derive(Debug, Default, Clone)]
pub struct CommandPalette {
    pub extra: Vec<(String, String)>,
    pub open: bool,
    /// The raw query, including the leading `/` the user typed.
    pub query: String,
    pub selected: usize,
}

impl CommandPalette {
    pub fn open(&mut self) {
        self.open = true;
        self.query = "/".to_string();
        self.selected = 0;
    }

    pub fn close(&mut self) {
        self.open = false;
        self.query.clear();
        self.selected = 0;
    }

    /// Feed a character into the query (called for every keystroke while the
    /// palette is open). Backspace is handled by the caller.
    pub fn push_char(&mut self, c: char) {
        self.query.push(c);
        self.selected = 0;
    }

    pub fn backspace(&mut self) {
        if self.query.len() > 1 {
            self.query.pop();
            self.selected = 0;
        } else {
            self.close();
        }
    }

    /// Commands matching the current query, in table order.
    pub fn matches(&self) -> Vec<(usize, &str, &str)> {
        let q = self
            .query
            .split_whitespace()
            .next()
            .unwrap_or_default()
            .trim_start_matches('/')
            .to_lowercase();
        ESSENTIAL_COMMANDS
            .iter()
            .map(|(name, desc)| (*name, *desc))
            .chain(
                self.extra
                    .iter()
                    .map(|(name, desc)| (name.as_str(), desc.as_str())),
            )
            .enumerate()
            .filter(|(_, (name, desc))| {
                let name = name.trim_start_matches('/').to_lowercase();
                let at_file = name == "file";
                if q.is_empty() {
                    return true;
                }
                name.starts_with(&q) || (!at_file && desc.to_lowercase().contains(&q))
            })
            .map(|(i, (n, d))| (i, n, d))
            .collect()
    }

    pub fn move_up(&mut self) {
        let n = self.matches().len();
        if n > 0 {
            self.selected = self.selected.saturating_sub(1).min(n.saturating_sub(1));
        }
    }

    pub fn move_down(&mut self) {
        let n = self.matches().len();
        if n > 0 {
            self.selected = (self.selected + 1).min(n.saturating_sub(1));
        }
    }

    /// The highlighted command, if any.
    pub fn selected_command(&self) -> Option<&str> {
        self.matches().get(self.selected).map(|(_, name, _)| *name)
    }

    /// Total commands matching (for the "N more" hints like the reference).
    pub fn total_matches(&self) -> usize {
        self.matches().len()
    }
}

/// Keep the selected command in a viewport bounded by the actual terminal.
pub fn render_palette(frame: &mut Frame, composer_area: Rect, pal: &Palette, cp: &CommandPalette) {
    let screen = frame.area();
    if screen.width < 4 || screen.height < 3 {
        return;
    }
    let matches = cp.matches();
    let height = (matches.len() as u16 + 4).clamp(5, 16).min(screen.height);
    let width = composer_area.width.min(96).min(screen.width);
    let x = composer_area.x.min(screen.right().saturating_sub(width));
    let y = composer_area.y.saturating_sub(height).max(screen.y);
    let area = Rect::new(x, y, width, height);
    frame.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .style(Style::default().bg(pal.chrome_bg))
        .border_style(Style::default().fg(pal.accent))
        .title(Span::styled(
            " Commands ",
            Style::default()
                .fg(pal.viewport_fg)
                .add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.is_empty() {
        return;
    }
    frame.render_widget(
        Paragraph::new(crate::fit_text(&cp.query, inner.width as usize))
            .style(Style::default().fg(pal.accent)),
        Rect::new(inner.x, inner.y, inner.width, 1),
    );
    let capacity = inner.height.saturating_sub(2) as usize;
    let start = cp.selected.saturating_sub(capacity.saturating_sub(1));
    let lines: Vec<_> = matches
        .iter()
        .skip(start)
        .take(capacity)
        .enumerate()
        .map(|(i, (_, name, desc))| {
            let selected = i + start == cp.selected;
            let style = if selected {
                Style::default()
                    .fg(pal.chrome_bg)
                    .bg(pal.accent)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(pal.viewport_fg)
            };
            let text = if inner.width >= 50 {
                format!(" {} {name:<11} {desc}", if selected { "›" } else { " " })
            } else {
                format!(" {} {name}", if selected { "›" } else { " " })
            };
            Line::styled(
                format!(
                    "{:<width$}",
                    crate::fit_text(&text, inner.width as usize),
                    width = inner.width as usize
                ),
                style,
            )
        })
        .collect();
    if matches.is_empty() {
        frame.render_widget(
            Paragraph::new(" No matching commands").style(Style::default().fg(pal.chrome_fg)),
            Rect::new(inner.x, inner.y + 1, inner.width, capacity as u16),
        );
    } else {
        frame.render_widget(
            Paragraph::new(lines),
            Rect::new(inner.x, inner.y + 1, inner.width, capacity as u16),
        );
    }
    let footer = format!(
        " ↑↓ select · Enter run · Esc close  {}/{}",
        if matches.is_empty() {
            0
        } else {
            cp.selected + 1
        },
        matches.len()
    );
    frame.render_widget(
        Paragraph::new(crate::fit_text(&footer, inner.width as usize))
            .style(Style::default().fg(pal.chrome_fg)),
        Rect::new(inner.x, inner.bottom() - 1, inner.width, 1),
    );
}
