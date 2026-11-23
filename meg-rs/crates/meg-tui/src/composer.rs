//! Shared input projection: wrapping, caret, scrolling and vertical movement
//! all use the same terminal-cell and grapheme measurements.
use crate::{Palette, WorkstationState};
use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph},
    Frame,
};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;
pub const PLACEHOLDER_TEXT: &str = "Ask MEG anything…";

struct InputLines {
    rows: Vec<String>,
    positions: Vec<(usize, usize, usize)>,
}
fn project(input: &str, columns: usize) -> InputLines {
    let safe = crate::sanitize_ansi(input);
    let input = safe.as_str();
    let columns = columns.max(1);
    let mut rows = vec![String::new()];
    let mut positions = vec![(0, 0, 0)];
    let (mut row, mut col, mut chars) = (0, 0, 0);
    for grapheme in input.graphemes(true) {
        if grapheme == "\n" {
            row += 1;
            col = 0;
            rows.push(String::new());
        } else {
            let width = grapheme.width();
            if col + width > columns || col == columns {
                row += 1;
                col = 0;
                rows.push(String::new());
                if let Some(p) = positions.last_mut() {
                    *p = (chars, row, col);
                }
            }
            rows[row].push_str(grapheme);
            col += width;
        }
        chars += grapheme.chars().count();
        positions.push((chars, row, col));
    }
    if col >= columns {
        row += 1;
        rows.push(String::new());
        if let Some(p) = positions.last_mut() {
            *p = (chars, row, 0);
        }
    }
    InputLines { rows, positions }
}
fn caret(lines: &InputLines, cursor: usize) -> (usize, usize) {
    let position = lines
        .positions
        .iter()
        .rev()
        .find(|p| p.0 <= cursor)
        .unwrap_or(&lines.positions[0]);
    (position.1, position.2)
}
pub fn input_visual_rows(width: u16, input: &str) -> usize {
    project(input, width.saturating_sub(4) as usize).rows.len()
}
pub fn vertical_cursor(input: &str, cursor: usize, width: u16, down: bool) -> Option<usize> {
    let lines = project(input, width.saturating_sub(4) as usize);
    let (row, col) = caret(&lines, cursor);
    let target = if down { row + 1 } else { row.checked_sub(1)? };
    lines
        .positions
        .iter()
        .filter(|p| p.1 == target && p.2 <= col)
        .max_by_key(|p| p.2)
        .map(|p| p.0)
}
fn input_area(area: Rect) -> (Rect, &'static str) {
    if area.height >= 3 && area.width >= 4 {
        (
            Rect::new(area.x + 1, area.y + 1, area.width - 2, area.height - 2),
            "> ",
        )
    } else {
        (area, " > ")
    }
}
pub fn cursor_screen_pos(area: Rect, state: &WorkstationState) -> (u16, u16) {
    if area.is_empty() {
        return (area.x, area.y);
    }
    let (inner, prefix) = input_area(area);
    let lines = project(
        &state.input,
        inner.width.saturating_sub(prefix.len() as u16) as usize,
    );
    let (row, col) = caret(
        &lines,
        state.cursor_position.min(state.input.chars().count()),
    );
    let top = row.saturating_sub(inner.height.saturating_sub(1) as usize);
    (
        (inner.x as usize + prefix.len() + col).min(inner.right().saturating_sub(1) as usize)
            as u16,
        (inner.y as usize + row - top).min(inner.bottom().saturating_sub(1) as usize) as u16,
    )
}
pub fn render_composer(frame: &mut Frame, area: Rect, state: &WorkstationState, pal: &Palette) {
    if area.is_empty() {
        return;
    }
    let (inner, prefix) = input_area(area);
    if inner != area {
        frame.render_widget(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .style(Style::default().bg(pal.chrome_bg))
                .border_style(Style::default().fg(pal.panel_border)),
            area,
        );
    }
    let projected = project(
        &state.input,
        inner.width.saturating_sub(prefix.len() as u16) as usize,
    );
    let (row, _) = caret(&projected, state.cursor_position);
    let top = row.saturating_sub(inner.height.saturating_sub(1) as usize);
    let lines = if state.input.is_empty() {
        vec![Line::from(vec![
            Span::styled(
                prefix,
                Style::default()
                    .fg(pal.viewport_fg)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(PLACEHOLDER_TEXT, Style::default().fg(pal.chrome_fg)),
        ])]
    } else {
        projected
            .rows
            .iter()
            .skip(top)
            .take(inner.height as usize)
            .map(|text| {
                Line::from(vec![
                    Span::styled(prefix, Style::default().fg(pal.accent)),
                    Span::styled(text.clone(), Style::default().fg(pal.viewport_fg)),
                ])
            })
            .collect()
    };
    frame.render_widget(
        Paragraph::new(lines).style(Style::default().bg(pal.chrome_bg)),
        inner,
    );
    if state.provider_dialog.is_none()
        && !state.has_active_modal()
        && !state.palette.open
        && state.session_picker.is_none()
        && state.workspace_picker.is_none()
    {
        frame.set_cursor_position(cursor_screen_pos(area, state));
    }
}
