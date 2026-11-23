#![allow(dead_code, unused_imports, unused_variables)]

//! Shared test harness and specification interfaces for MEG Harness TUI E2E tests.
//!
//! Provides Ratatui `TestBackend` helpers, buffer assertions, ANSI sanitization,
//! theme palettes, hero banner, rail/band composer, modal picker overlays,
//! streaming & thinking displays, and job execution status indicators.

use ratatui::{
    backend::TestBackend,
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame, Terminal,
};
use std::sync::mpsc::{channel, Receiver, Sender};

// ============================================================================
// 1. Theme Palettes (Specification 6)
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThemePalette {
    pub name: &'static str,
    pub chrome_bg: Color,
    pub chrome_fg: Color,
    pub titlebar_bg: Color,
    pub titlebar_fg: Color,
    pub panel_border: Color,
    pub panel_title: Color,
    pub viewport_bg: Color,
    pub viewport_fg: Color,
    pub accent: Color,
    pub success: Color,
    pub error: Color,
    pub warning: Color,
}

pub const CLASSIC: ThemePalette = ThemePalette {
    name: "Classic",
    chrome_bg: Color::Rgb(192, 192, 192),
    chrome_fg: Color::Black,
    titlebar_bg: Color::Rgb(0, 0, 128),
    titlebar_fg: Color::White,
    panel_border: Color::Rgb(128, 128, 128),
    panel_title: Color::Rgb(0, 0, 128),
    viewport_bg: Color::White,
    viewport_fg: Color::Black,
    accent: Color::Rgb(0, 90, 156),
    success: Color::Rgb(0, 115, 0),
    error: Color::Rgb(160, 0, 0),
    warning: Color::Rgb(180, 100, 0),
};

pub const PHOSPHOR: ThemePalette = ThemePalette {
    name: "Phosphor",
    chrome_bg: Color::Rgb(4, 20, 10),
    chrome_fg: Color::Rgb(0, 255, 65),
    titlebar_bg: Color::Rgb(0, 59, 0),
    titlebar_fg: Color::Rgb(127, 255, 159),
    panel_border: Color::Rgb(0, 77, 26),
    panel_title: Color::Rgb(51, 255, 51),
    viewport_bg: Color::Rgb(1, 13, 5),
    viewport_fg: Color::Rgb(0, 255, 65),
    accent: Color::Rgb(51, 255, 51),
    success: Color::Rgb(51, 255, 51),
    error: Color::Rgb(255, 51, 51),
    warning: Color::Rgb(255, 204, 0),
};

pub const DARK_SLATE: ThemePalette = ThemePalette {
    name: "Dark Slate",
    chrome_bg: Color::Rgb(24, 24, 37),
    chrome_fg: Color::Rgb(166, 173, 200),
    titlebar_bg: Color::Rgb(30, 30, 46),
    titlebar_fg: Color::Rgb(205, 214, 244),
    panel_border: Color::Rgb(69, 71, 90),
    panel_title: Color::Rgb(137, 180, 250),
    viewport_bg: Color::Rgb(30, 30, 46),
    viewport_fg: Color::Rgb(205, 214, 244),
    accent: Color::Rgb(137, 180, 250),
    success: Color::Rgb(166, 227, 161),
    error: Color::Rgb(243, 139, 168),
    warning: Color::Rgb(250, 179, 135),
};

pub const DRACULA: ThemePalette = ThemePalette {
    name: "Dracula",
    chrome_bg: Color::Rgb(33, 34, 44),
    chrome_fg: Color::Rgb(98, 114, 164),
    titlebar_bg: Color::Rgb(40, 42, 54),
    titlebar_fg: Color::Rgb(248, 248, 242),
    panel_border: Color::Rgb(68, 71, 90),
    panel_title: Color::Rgb(189, 147, 249),
    viewport_bg: Color::Rgb(40, 42, 54),
    viewport_fg: Color::Rgb(248, 248, 242),
    accent: Color::Rgb(189, 147, 249),
    success: Color::Rgb(80, 250, 123),
    error: Color::Rgb(255, 85, 85),
    warning: Color::Rgb(255, 184, 108),
};

pub const TOKYO_NIGHT: ThemePalette = ThemePalette {
    name: "Tokyo Night",
    chrome_bg: Color::Rgb(22, 22, 30),
    chrome_fg: Color::Rgb(86, 95, 137),
    titlebar_bg: Color::Rgb(26, 27, 38),
    titlebar_fg: Color::Rgb(192, 202, 245),
    panel_border: Color::Rgb(41, 164, 232),
    panel_title: Color::Rgb(122, 162, 247),
    viewport_bg: Color::Rgb(26, 27, 38),
    viewport_fg: Color::Rgb(192, 202, 245),
    accent: Color::Rgb(122, 162, 247),
    success: Color::Rgb(158, 206, 106),
    error: Color::Rgb(247, 118, 142),
    warning: Color::Rgb(224, 175, 104),
};

pub const ALL_PALETTES: [&ThemePalette; 5] = [
    &CLASSIC,
    &PHOSPHOR,
    &DARK_SLATE,
    &DRACULA,
    &TOKYO_NIGHT,
];

pub fn get_palette_by_name(name: &str) -> &'static ThemePalette {
    match name.to_lowercase().as_str() {
        "classic" => &CLASSIC,
        "phosphor" => &PHOSPHOR,
        "dark slate" | "darkslate" | "slate" => &DARK_SLATE,
        "dracula" => &DRACULA,
        "tokyo night" | "tokyonight" => &TOKYO_NIGHT,
        _ => &DARK_SLATE, // safe default
    }
}

// ============================================================================
// 2. ANSI Sanitizer (Specification 6)
// ============================================================================

/// Strips all raw ANSI escape sequences, OSC hyperlinks, and control codes.
pub fn strip_ansi(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            if let Some(&next) = chars.peek() {
                if next == '[' {
                    chars.next(); // consume '['
                    // CSI sequence: consume parameter bytes (0x30-0x3F)
                    while let Some(&b) = chars.peek() {
                        if (b as u32) >= 0x30 && (b as u32) <= 0x3F {
                            chars.next();
                        } else {
                            break;
                        }
                    }
                    // Optional intermediate bytes (0x20-0x2F except space)
                    while let Some(&b) = chars.peek() {
                        if (b as u32) > 0x20 && (b as u32) <= 0x2F {
                            chars.next();
                        } else {
                            break;
                        }
                    }
                    // Final byte must be 0x40-0x7E (e.g. 'm', 'H', 'J')
                    if let Some(&b) = chars.peek() {
                        if (b as u32) >= 0x40 && (b as u32) <= 0x7E {
                            chars.next();
                        }
                    }
                } else if next == ']' {
                    chars.next(); // consume ']'
                    // OSC sequence: consume until BEL (\x07) or ST (\x1b\)
                    while let Some(&b) = chars.peek() {
                        chars.next();
                        if b == '\x07' {
                            break;
                        }
                        if b == '\x1b' && chars.peek() == Some(&'\\') {
                            chars.next();
                            break;
                        }
                    }
                } else {
                    chars.next();
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Parses ANSI color sequences into Ratatui `Span`s, strictly sanitizing control sequences.
pub fn sanitize_to_spans(input: &str, base_style: Style) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut cur_text = String::new();
    let mut cur_style = base_style;
    let mut chars = input.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '\x1b' {
            if let Some(&next) = chars.peek() {
                if next == '[' {
                    chars.next(); // consume '['
                    let mut code = String::new();
                    // Parameter bytes
                    while let Some(&b) = chars.peek() {
                        if (b as u32) >= 0x30 && (b as u32) <= 0x3F {
                            code.push(b);
                            chars.next();
                        } else {
                            break;
                        }
                    }
                    // Optional intermediate bytes
                    while let Some(&b) = chars.peek() {
                        if (b as u32) > 0x20 && (b as u32) <= 0x2F {
                            code.push(b);
                            chars.next();
                        } else {
                            break;
                        }
                    }
                    // Final command byte
                    if let Some(&b) = chars.peek() {
                        if (b as u32) >= 0x40 && (b as u32) <= 0x7E {
                            code.push(b);
                            chars.next();
                        }
                    }
                    if !cur_text.is_empty() {
                        spans.push(Span::styled(std::mem::take(&mut cur_text), cur_style));
                    }
                    // Handle common SGR color codes
                    if code.ends_with('m') {
                        let inner = &code[..code.len() - 1];
                        for part in inner.split(';') {
                            match part {
                                "0" | "" => cur_style = base_style,
                                "1" => cur_style = cur_style.add_modifier(Modifier::BOLD),
                                "31" => cur_style = cur_style.fg(Color::Red),
                                "32" => cur_style = cur_style.fg(Color::Green),
                                "33" => cur_style = cur_style.fg(Color::Yellow),
                                "34" => cur_style = cur_style.fg(Color::Blue),
                                "35" => cur_style = cur_style.fg(Color::Magenta),
                                "36" => cur_style = cur_style.fg(Color::Cyan),
                                "37" => cur_style = cur_style.fg(Color::White),
                                _ => {}
                            }
                        }
                    }
                } else if next == ']' {
                    chars.next(); // OSC
                    while let Some(&b) = chars.peek() {
                        chars.next();
                        if b == '\x07' || (b == '\x1b' && chars.peek() == Some(&'\\')) {
                            if b == '\x1b' {
                                chars.next();
                            }
                            break;
                        }
                    }
                }
            }
        } else {
            cur_text.push(c);
        }
    }
    if !cur_text.is_empty() {
        spans.push(Span::styled(cur_text, cur_style));
    }
    spans
}

// ============================================================================
// 3. Compact Hero Banner (Specification 1)
// ============================================================================

#[derive(Debug, Clone)]
pub struct HeroState {
    pub version: String,
    pub model: String,
    pub workspace: String,
    pub tips: Vec<String>,
}

impl Default for HeroState {
    fn default() -> Self {
        Self {
            version: "v0.8.0".into(),
            model: "qwen2.5-coder:7b".into(),
            workspace: "~/MEG".into(),
            tips: vec!["Tab toggles Plan/Build".into(), "? for shortcuts".into()],
        }
    }
}

pub fn render_hero(frame: &mut Frame, area: Rect, state: &HeroState, pal: &ThemePalette) {
    if area.width < 52 || area.height < 5 {
        // Responsive 1-line fallback
        let truncated_model = if state.model.len() > 20 {
            format!("{}...", &state.model[..17])
        } else {
            state.model.clone()
        };
        let fallback_line = Line::from(vec![
            Span::styled(" \u{26a1} MEG ", Style::default().fg(pal.titlebar_fg).bg(pal.titlebar_bg).add_modifier(Modifier::BOLD)),
            Span::styled(format!(" {} \u{00b7} {} \u{00b7} {} ", state.version, truncated_model, state.workspace),
                Style::default().fg(pal.chrome_fg).bg(pal.chrome_bg)),
        ]);
        frame.render_widget(Paragraph::new(fallback_line), area);
        return;
    }

    // 5-line rounded card
    let model_display = if state.model.len() > 30 {
        format!("{}...", &state.model[..27])
    } else {
        state.model.clone()
    };

    let title_line = format!(" \u{26a1} MEG Polyglot Harness ({}) ", state.version);
    let card_block = Block::default()
        .borders(Borders::ALL)
        .border_type(ratatui::widgets::BorderType::Rounded)
        .border_style(Style::default().fg(pal.panel_border))
        .title(Span::styled(title_line, Style::default().fg(pal.panel_title).add_modifier(Modifier::BOLD)));

    let inner_lines = vec![
        Line::from(vec![
            Span::styled("  Model:     ", Style::default().fg(pal.chrome_fg)),
            Span::styled(format!("{:<28}", model_display), Style::default().fg(pal.viewport_fg).add_modifier(Modifier::BOLD)),
            Span::styled(" \u{2022}  /model to switch", Style::default().fg(pal.accent)),
        ]),
        Line::from(vec![
            Span::styled("  Workspace: ", Style::default().fg(pal.chrome_fg)),
            Span::styled(format!("{:<28}", state.workspace), Style::default().fg(pal.viewport_fg)),
            Span::styled(" \u{2022}  /theme to restyle", Style::default().fg(pal.accent)),
        ]),
        Line::from(vec![
            Span::styled("  Tips:      ", Style::default().fg(pal.chrome_fg)),
            Span::styled(format!("{:<28}", state.tips.first().map(|s| s.as_str()).unwrap_or("")), Style::default().fg(pal.viewport_fg)),
            Span::styled(" \u{2022}  ? for shortcuts", Style::default().fg(pal.accent)),
        ]),
    ];

    frame.render_widget(Paragraph::new(inner_lines).block(card_block), area);
}

/// Renders 2-line half-block micro wordmark
pub fn render_micro_wordmark(frame: &mut Frame, area: Rect, pal: &ThemePalette) {
    if area.height < 2 || area.width < 14 {
        return;
    }
    // Micro half-block MEG logo (2 lines)
    let line1 = Span::styled("\u{2588}\u{2584}   \u{2584}\u{2588} \u{2588}\u{2588}\u{2588}\u{2588} \u{2584}\u{2588}\u{2588}\u{2588}\u{2588}\u{2584}", Style::default().fg(pal.accent));
    let line2 = Span::styled("\u{2588} \u{2580}\u{2580} \u{2588} \u{2588}\u{2584}\u{2584}  \u{2588}\u{2584}\u{2584}\u{2584}\u{2588}", Style::default().fg(pal.accent));
    let para = Paragraph::new(vec![Line::from(line1), Line::from(line2)]);
    frame.render_widget(para, area);
}

// ============================================================================
// 4. Rail / Band Composer & Docked Chip Bar (Specification 2)
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComposerMode {
    Build,
    Plan,
}

impl ComposerMode {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Build => "[BUILD]",
            Self::Plan => "[PLAN]",
        }
    }

    pub fn bg_color(&self) -> Color {
        match self {
            Self::Build => Color::Rgb(2, 132, 199), // Blue
            Self::Plan => Color::Rgb(217, 119, 6),  // Amber
        }
    }

    pub fn next(&self) -> Self {
        match self {
            Self::Build => Self::Plan,
            Self::Plan => Self::Build,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ComposerState {
    pub input: String,
    pub cursor_pos: usize,
    pub history: Vec<String>,
    pub history_index: Option<usize>,
    pub draft_input: Option<String>,
    pub mode: ComposerMode,
    pub model: String,
    pub effort: String,
    pub context_pct: u8,
    pub context_tokens: usize,
    pub max_lines: u16,
}

impl Default for ComposerState {
    fn default() -> Self {
        Self {
            input: String::new(),
            cursor_pos: 0,
            history: Vec::new(),
            history_index: None,
            draft_input: None,
            mode: ComposerMode::Build,
            model: "qwen2.5-coder:7b".into(),
            effort: "high".into(),
            context_pct: 6,
            context_tokens: 12400,
            max_lines: 5,
        }
    }
}

impl ComposerState {
    pub fn type_char(&mut self, c: char) {
        let char_idx = self.cursor_pos;
        if char_idx >= self.input.chars().count() {
            self.input.push(c);
        } else {
            let byte_idx = self.input.char_indices().nth(char_idx).map(|(i, _)| i).unwrap_or(self.input.len());
            self.input.insert(byte_idx, c);
        }
        self.cursor_pos += 1;
    }

    pub fn type_str(&mut self, s: &str) {
        for c in s.chars() {
            self.type_char(c);
        }
    }

    pub fn backspace(&mut self) {
        if self.cursor_pos > 0 {
            self.cursor_pos -= 1;
            let byte_idx = self.input.char_indices().nth(self.cursor_pos).map(|(i, _)| i).unwrap_or(0);
            if byte_idx < self.input.len() {
                self.input.remove(byte_idx);
            }
        }
    }

    pub fn delete(&mut self) {
        let total_chars = self.input.chars().count();
        if self.cursor_pos < total_chars {
            let byte_idx = self.input.char_indices().nth(self.cursor_pos).map(|(i, _)| i).unwrap_or(0);
            if byte_idx < self.input.len() {
                self.input.remove(byte_idx);
            }
        }
    }

    pub fn move_left(&mut self) {
        if self.cursor_pos > 0 {
            self.cursor_pos -= 1;
        }
    }

    pub fn move_right(&mut self) {
        if self.cursor_pos < self.input.chars().count() {
            self.cursor_pos += 1;
        }
    }

    pub fn move_home(&mut self) {
        self.cursor_pos = 0;
    }

    pub fn move_end(&mut self) {
        self.cursor_pos = self.input.chars().count();
    }

    pub fn history_up(&mut self) {
        if self.history.is_empty() {
            return;
        }
        if self.history_index.is_none() {
            self.draft_input = Some(self.input.clone());
            let last_idx = self.history.len() - 1;
            self.history_index = Some(last_idx);
            self.input = self.history[last_idx].clone();
            self.cursor_pos = self.input.chars().count();
        } else if let Some(idx) = self.history_index {
            if idx > 0 {
                let new_idx = idx - 1;
                self.history_index = Some(new_idx);
                self.input = self.history[new_idx].clone();
                self.cursor_pos = self.input.chars().count();
            }
        }
    }

    pub fn history_down(&mut self) {
        if let Some(idx) = self.history_index {
            if idx + 1 < self.history.len() {
                let new_idx = idx + 1;
                self.history_index = Some(new_idx);
                self.input = self.history[new_idx].clone();
                self.cursor_pos = self.input.chars().count();
            } else {
                self.history_index = None;
                self.input = self.draft_input.take().unwrap_or_default();
                self.cursor_pos = self.input.chars().count();
            }
        }
    }

    pub fn cycle_mode(&mut self) {
        self.mode = self.mode.next();
    }

    pub fn submit(&mut self) -> Option<String> {
        let trimmed = self.input.trim().to_string();
        if trimmed.is_empty() {
            return None;
        }
        self.history.push(self.input.clone());
        self.history_index = None;
        self.draft_input = None;
        self.cursor_pos = 0;
        let submitted = std::mem::take(&mut self.input);
        Some(submitted)
    }

    pub fn calculate_height(&self) -> u16 {
        let input_lines = self.input.split('\n').count().max(1);
        let clamped_lines = (input_lines as u16).min(self.max_lines);
        // Container: top border + input lines + bottom border = clamped_lines + 2
        clamped_lines + 2
    }
}

pub fn render_composer(frame: &mut Frame, area: Rect, state: &ComposerState, pal: &ThemePalette) {
    if area.height < 3 || area.width < 20 {
        return;
    }

    // Test-side mirror of the 2026-11-20 crate composer: full rounded
    // border, `>` prompt, no divider, no chip bar (header/statusbar own it).
    let outer_block = Block::default()
        .borders(Borders::ALL)
        .border_type(ratatui::widgets::BorderType::Rounded)
        .border_style(Style::default().fg(pal.panel_border));

    let inner_area = outer_block.inner(area);
    frame.render_widget(outer_block, area);

    if inner_area.height < 1 {
        return;
    }

    let placeholder = "Ask MEG anything\u{2026} (/ for commands, @ for files, Ctrl+P for menu)";
    let input_lines: Vec<&str> = if state.input.is_empty() {
        vec![placeholder]
    } else {
        state.input.split('\n').collect()
    };

    let mut rendered_lines = Vec::new();
    for line in input_lines.iter() {
        let is_placeholder = state.input.is_empty();
        let text_style = if is_placeholder {
            Style::default().fg(pal.panel_border)
        } else {
            Style::default().fg(pal.viewport_fg)
        };

        rendered_lines.push(Line::from(vec![
            Span::styled(" > ", Style::default().fg(pal.accent).add_modifier(Modifier::BOLD)),
            Span::styled(*line, text_style),
        ]));
        if rendered_lines.len() >= 4 {
            break; // limit display to max lines
        }
    }
    frame.render_widget(
        Paragraph::new(rendered_lines).style(Style::default().bg(pal.chrome_bg)),
        inner_area,
    );
}


// ============================================================================
// 5. Interactive Modal Pickers (Specification 3)
// ============================================================================

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PickerItem<T: Clone> {
    pub label: String,
    pub detail: String,
    pub meta: String,
    pub value: T,
}

#[derive(Debug, Clone)]
pub struct PickerState<T: Clone> {
    pub title: String,
    pub items: Vec<PickerItem<T>>,
    pub filtered_indices: Vec<usize>,
    pub selected_index: usize,
    pub search_query: String,
    pub is_open: bool,
    pub max_visible_items: usize,
    pub scroll_offset: usize,
}

impl<T: Clone> PickerState<T> {
    pub fn new(title: String, items: Vec<PickerItem<T>>) -> Self {
        let count = items.len();
        Self {
            title,
            items,
            filtered_indices: (0..count).collect(),
            selected_index: 0,
            search_query: String::new(),
            is_open: true,
            max_visible_items: 8,
            scroll_offset: 0,
        }
    }

    pub fn filter(&mut self) {
        let q = self.search_query.to_lowercase();
        self.filtered_indices = self
            .items
            .iter()
            .enumerate()
            .filter(|(_, it)| {
                q.is_empty()
                    || it.label.to_lowercase().contains(&q)
                    || it.detail.to_lowercase().contains(&q)
                    || it.meta.to_lowercase().contains(&q)
            })
            .map(|(i, _)| i)
            .collect();

        if self.filtered_indices.is_empty() {
            self.selected_index = 0;
            self.scroll_offset = 0;
        } else if self.selected_index >= self.filtered_indices.len() {
            self.selected_index = self.filtered_indices.len() - 1;
        }
    }

    pub fn type_char(&mut self, c: char) {
        self.search_query.push(c);
        self.filter();
    }

    pub fn backspace(&mut self) {
        self.search_query.pop();
        self.filter();
    }

    pub fn move_up(&mut self) {
        if self.filtered_indices.is_empty() {
            return;
        }
        if self.selected_index == 0 {
            self.selected_index = self.filtered_indices.len() - 1;
        } else {
            self.selected_index -= 1;
        }
        self.adjust_scroll();
    }

    pub fn move_down(&mut self) {
        if self.filtered_indices.is_empty() {
            return;
        }
        if self.selected_index + 1 >= self.filtered_indices.len() {
            self.selected_index = 0;
        } else {
            self.selected_index += 1;
        }
        self.adjust_scroll();
    }

    fn adjust_scroll(&mut self) {
        if self.selected_index < self.scroll_offset {
            self.scroll_offset = self.selected_index;
        } else if self.selected_index >= self.scroll_offset + self.max_visible_items {
            self.scroll_offset = self.selected_index + 1 - self.max_visible_items;
        }
    }

    pub fn selected_item(&self) -> Option<&PickerItem<T>> {
        let item_idx = *self.filtered_indices.get(self.selected_index)?;
        self.items.get(item_idx)
    }

    pub fn commit_selection(&mut self) -> Option<T> {
        let item = self.selected_item()?.value.clone();
        self.is_open = false;
        Some(item)
    }

    pub fn cancel(&mut self) {
        self.is_open = false;
    }
}

pub fn render_picker_modal<T: Clone>(
    frame: &mut Frame,
    area: Rect,
    state: &PickerState<T>,
    pal: &ThemePalette,
) {
    if !state.is_open {
        return;
    }

    // Modal dimensions: 64 wide, 14 high, centered
    let width = 64.min(area.width.saturating_sub(4));
    let height = 14.min(area.height.saturating_sub(2));
    let x = (area.width.saturating_sub(width)) / 2;
    let y = (area.height.saturating_sub(height)) / 2;
    let modal_area = Rect::new(x, y, width, height);

    frame.render_widget(Clear, modal_area);

    let modal_block = Block::default()
        .borders(Borders::ALL)
        .border_type(ratatui::widgets::BorderType::Rounded)
        .border_style(Style::default().fg(pal.accent))
        .title(Span::styled(
            format!(" \u{2728} {} ", state.title),
            Style::default().fg(pal.titlebar_fg).bg(pal.titlebar_bg).add_modifier(Modifier::BOLD),
        ));
    frame.render_widget(modal_block, modal_area);

    let inner = Rect::new(modal_area.x + 1, modal_area.y + 1, modal_area.width - 2, modal_area.height - 2);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // search input
            Constraint::Length(1), // separator rule
            Constraint::Min(3),    // item list
            Constraint::Length(1), // footer
        ])
        .split(inner);

    // Search query row
    let search_line = Line::from(vec![
        Span::styled(" \u{1f50d} Search: ", Style::default().fg(pal.chrome_fg)),
        Span::styled(&state.search_query, Style::default().fg(pal.viewport_fg).add_modifier(Modifier::BOLD)),
        Span::styled("\u{258c}", Style::default().fg(pal.accent)), // cursor block
    ]);
    frame.render_widget(Paragraph::new(search_line), chunks[0]);

    // Separator
    let sep = Line::from(Span::styled(
        "\u{2500}".repeat(chunks[1].width as usize),
        Style::default().fg(pal.panel_border),
    ));
    frame.render_widget(Paragraph::new(sep), chunks[1]);

    // Items list
    if state.filtered_indices.is_empty() {
        let empty_msg = Line::from(Span::styled(
            "   No matching items found",
            Style::default().fg(pal.panel_border),
        ));
        frame.render_widget(Paragraph::new(empty_msg), chunks[2]);
    } else {
        let mut lines = Vec::new();
        let end_idx = (state.scroll_offset + state.max_visible_items).min(state.filtered_indices.len());
        for (pos, &actual_idx) in state.filtered_indices[state.scroll_offset..end_idx].iter().enumerate() {
            let list_idx = state.scroll_offset + pos;
            let item = &state.items[actual_idx];
            let is_selected = list_idx == state.selected_index;

            let (prefix, style) = if is_selected {
                (
                    " > ",
                    Style::default().fg(pal.titlebar_fg).bg(pal.accent).add_modifier(Modifier::BOLD),
                )
            } else {
                ("   ", Style::default().fg(pal.viewport_fg))
            };

            lines.push(Line::from(vec![
                Span::styled(prefix, style),
                Span::styled(format!("{:<24}", item.label), style),
                Span::styled(format!("{:<20}", item.detail), style),
                Span::styled(&item.meta, style),
            ]));
        }
        frame.render_widget(Paragraph::new(lines), chunks[2]);
    }

    // Footer
    let count_str = if state.filtered_indices.is_empty() {
        "(0/0)".to_string()
    } else {
        format!("({}/{})", state.selected_index + 1, state.filtered_indices.len())
    };
    let footer_line = Line::from(vec![
        Span::styled(
            format!(" \u{2191}/\u{2193} Navigate \u{2022} Enter Select \u{2022} Esc Cancel   {}", count_str),
            Style::default().fg(pal.chrome_fg),
        ),
    ]);
    frame.render_widget(Paragraph::new(footer_line), chunks[3]);
}

// ============================================================================
// 6. Live Token Streaming & Thinking Display (Specification 4)
// ============================================================================

#[derive(Debug, Clone)]
pub enum RunnerMsg {
    Line(String),
    ThinkingChunk(String),
    ContentChunk(String),
    StreamMetrics { tok_per_sec: f32, elapsed_secs: f32, token_count: usize },
    JobStart { id: String, description: String },
    JobProgress { id: String, status: String },
    JobDone { id: String },
    PlanStep(bool, String),
    Done(Option<String>),
}

#[derive(Debug, Clone)]
pub struct StreamState {
    pub thinking_buffer: String,
    pub content_buffer: String,
    pub is_thinking_collapsed: bool,
    pub tok_per_sec: f32,
    pub elapsed_secs: f32,
    pub token_count: usize,
    pub is_complete: bool,
}

impl Default for StreamState {
    fn default() -> Self {
        Self {
            thinking_buffer: String::new(),
            content_buffer: String::new(),
            is_thinking_collapsed: false,
            tok_per_sec: 0.0,
            elapsed_secs: 0.0,
            token_count: 0,
            is_complete: false,
        }
    }
}

impl StreamState {
    pub fn push_thinking(&mut self, chunk: &str) {
        self.thinking_buffer.push_str(chunk);
    }

    pub fn push_content(&mut self, chunk: &str) {
        self.content_buffer.push_str(chunk);
    }

    pub fn toggle_thinking_collapse(&mut self) {
        self.is_thinking_collapsed = !self.is_thinking_collapsed;
    }

    pub fn formatted_elapsed(&self) -> String {
        if self.elapsed_secs < 60.0 {
            format!("{:.1}s", self.elapsed_secs)
        } else {
            let mins = (self.elapsed_secs / 60.0).floor() as u64;
            let secs = (self.elapsed_secs % 60.0) as u64;
            format!("{}m {}s", mins, secs)
        }
    }

    pub fn calculate_rate(token_count: usize, elapsed_secs: f32) -> f32 {
        if elapsed_secs <= 0.0001 {
            0.0
        } else {
            token_count as f32 / elapsed_secs
        }
    }
}

pub fn render_streaming_view(
    frame: &mut Frame,
    area: Rect,
    state: &StreamState,
    pal: &ThemePalette,
) {
    let mut lines = Vec::new();

    // Thinking region
    if !state.thinking_buffer.is_empty() {
        let duration_str = state.formatted_elapsed();
        let rate_str = format!("{:.1} tok/s", state.tok_per_sec);

        if state.is_thinking_collapsed {
            let collapsed_line = Line::from(vec![
                Span::styled(
                    format!(" * Thought for {} ({} tokens) [Ctrl+T to expand] * ", duration_str, state.token_count),
                    Style::default().fg(pal.chrome_fg).add_modifier(Modifier::ITALIC),
                ),
            ]);
            lines.push(collapsed_line);
        } else {
            let header = Line::from(vec![
                Span::styled(" \u{1f9e0} Thinking ", Style::default().fg(pal.accent).add_modifier(Modifier::BOLD)),
                Span::styled(format!("({} \u{2022} {} \u{2022} {} tokens) ", duration_str, rate_str, state.token_count), Style::default().fg(pal.chrome_fg)),
                Span::styled("[Ctrl+T to collapse] ", Style::default().fg(pal.panel_border)),
            ]);
            lines.push(header);

            for t_line in state.thinking_buffer.lines().take(10) {
                lines.push(Line::from(Span::styled(
                    format!("   {}", t_line),
                    Style::default().fg(pal.chrome_fg).add_modifier(Modifier::ITALIC),
                )));
            }

            lines.push(Line::from(Span::styled(
                "\u{2500}".repeat(area.width as usize),
                Style::default().fg(pal.panel_border),
            )));
        }
    }

    // Content region
    for c_line in state.content_buffer.lines() {
        lines.push(Line::from(Span::styled(c_line, Style::default().fg(pal.viewport_fg))));
    }

    frame.render_widget(Paragraph::new(lines), area);
}

// ============================================================================
// 7. Active Job Execution & Status Indicator (Specification 5)
// ============================================================================

pub const SPINNER_FRAMES: [char; 10] = ['\u{280b}', '\u{2819}', '\u{2839}', '\u{2838}', '\u{283c}', '\u{2834}', '\u{2826}', '\u{2827}', '\u{2807}', '\u{280f}'];

#[derive(Debug, Clone)]
pub struct JobState {
    pub id: String,
    pub description: String,
    pub status_line: String,
    pub output_lines: Vec<String>,
    pub spinner_tick: usize,
    pub elapsed_secs: f32,
    pub is_active: bool,
    pub is_failed: bool,
    pub is_cancelled: bool,
}

impl Default for JobState {
    fn default() -> Self {
        Self {
            id: String::new(),
            description: String::new(),
            status_line: String::new(),
            output_lines: Vec::new(),
            spinner_tick: 0,
            elapsed_secs: 0.0,
            is_active: false,
            is_failed: false,
            is_cancelled: false,
        }
    }
}

impl JobState {
    pub fn start(&mut self, id: String, desc: String) {
        self.id = id;
        self.description = desc;
        self.status_line.clear();
        self.output_lines.clear();
        self.spinner_tick = 0;
        self.elapsed_secs = 0.0;
        self.is_active = true;
        self.is_failed = false;
        self.is_cancelled = false;
    }

    pub fn tick(&mut self, delta_secs: f32) {
        self.spinner_tick = (self.spinner_tick + 1) % SPINNER_FRAMES.len();
        self.elapsed_secs += delta_secs;
    }

    pub fn spinner_glyph(&self) -> char {
        SPINNER_FRAMES[self.spinner_tick % SPINNER_FRAMES.len()]
    }

    pub fn formatted_duration(&self) -> String {
        format!("{:.1}s", self.elapsed_secs)
    }

    pub fn cancel(&mut self) {
        self.is_cancelled = true;
        self.is_active = false;
        self.status_line = "[cancelled by user]".into();
    }

    pub fn fail(&mut self, err: String) {
        self.is_failed = true;
        self.is_active = false;
        self.status_line = format!("[error: {}]", err);
    }

    pub fn complete(&mut self) {
        self.is_active = false;
    }

    pub fn push_output(&mut self, line: String) {
        self.output_lines.push(line);
    }
}

pub fn render_job_indicator(
    frame: &mut Frame,
    area: Rect,
    state: &JobState,
    pal: &ThemePalette,
) {
    if !state.is_active && !state.is_failed && !state.is_cancelled {
        return;
    }

    let mut lines = Vec::new();
    if state.is_active {
        let glyph = state.spinner_glyph();
        let header = Line::from(vec![
            Span::styled(format!(" {} ", glyph), Style::default().fg(pal.accent).add_modifier(Modifier::BOLD)),
            Span::styled(&state.description, Style::default().fg(pal.viewport_fg).add_modifier(Modifier::BOLD)),
            Span::styled(format!(" ({} \u{2022} esc to interrupt)", state.formatted_duration()), Style::default().fg(pal.chrome_fg)),
        ]);
        lines.push(header);

        if let Some(last_output) = state.output_lines.last() {
            let count = state.output_lines.len();
            let folded_msg = if count > 1 {
                format!("  \u{2514} {} (+{} lines)", last_output, count - 1)
            } else {
                format!("  \u{2514} {}", last_output)
            };
            lines.push(Line::from(Span::styled(folded_msg, Style::default().fg(pal.chrome_fg))));
        }
    } else if state.is_cancelled {
        lines.push(Line::from(vec![
            Span::styled(" \u{2717} ", Style::default().fg(pal.warning)),
            Span::styled(format!("{} \u{2014} cancelled", state.description), Style::default().fg(pal.warning)),
        ]));
    } else if state.is_failed {
        lines.push(Line::from(vec![
            Span::styled(" \u{2717} ", Style::default().fg(pal.error)),
            Span::styled(format!("{} \u{2014} {}", state.description, state.status_line), Style::default().fg(pal.error)),
        ]));
    }

    frame.render_widget(Paragraph::new(lines), area);
}

// ============================================================================
// 8. Test Backend Helpers & Assertions
// ============================================================================

pub struct TestHarness {
    pub terminal: Terminal<TestBackend>,
    pub width: u16,
    pub height: u16,
}

impl TestHarness {
    pub fn new(width: u16, height: u16) -> Self {
        let backend = TestBackend::new(width, height);
        let terminal = Terminal::new(backend).expect("failed to create terminal");
        Self { terminal, width, height }
    }

    pub fn draw<F>(&mut self, render_fn: F) -> Buffer
    where
        F: FnOnce(&mut Frame),
    {
        self.terminal.draw(render_fn).expect("draw failed");
        self.terminal.backend().buffer().clone()
    }

    pub fn resize(&mut self, width: u16, height: u16) {
        self.width = width;
        self.height = height;
        let backend = TestBackend::new(width, height);
        self.terminal = Terminal::new(backend).expect("failed to resize terminal");
    }
}

pub fn buffer_lines(buf: &Buffer) -> Vec<String> {
    let mut lines = Vec::new();
    let area = buf.area();
    for y in area.top()..area.bottom() {
        let mut line_str = String::new();
        for x in area.left()..area.right() {
            line_str.push_str(buf[(x, y)].symbol());
        }
        lines.push(line_str);
    }
    lines
}

pub fn buffer_line(buf: &Buffer, y: u16) -> String {
    let area = buf.area();
    let mut line_str = String::new();
    for x in area.left()..area.right() {
        line_str.push_str(buf[(x, y)].symbol());
    }
    line_str
}

pub fn buffer_contains(buf: &Buffer, text: &str) -> bool {
    let lines = buffer_lines(buf);
    lines.iter().any(|line| line.contains(text))
}

pub fn assert_no_ansi_leaks(buf: &Buffer) {
    let area = buf.area();
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            let symbol = buf[(x, y)].symbol();
            assert!(
                !symbol.contains('\x1b'),
                "ANSI escape leak detected at ({}, {}): {:?}",
                x, y, symbol
            );
            assert!(
                !symbol.contains("?["),
                "Leaked ESC-bracket replacement detected at ({}, {}): {:?}",
                x, y, symbol
            );
        }
    }
}
