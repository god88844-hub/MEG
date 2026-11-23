//! Modal picker overlays for interactive theme, model, and effort selection.
//!
//! Provides generic `PickerState<T>` and `PickerItem<T>`, concrete `ModelItem` and
//! `EffortLevel` domain types, `ActiveModal` enum, and Ratatui overlay rendering.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
    Frame,
};

use crate::theme::{Palette, CLASSIC, DARK_SLATE, DASHBOARD, DRACULA, PHOSPHOR, TOKYO_NIGHT};

// ============================================================================
// 1. Generic Picker Item & State
// ============================================================================

/// An individual item displayed within a modal picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PickerItem<T: Clone> {
    pub label: String,
    pub detail: String,
    pub meta: String,
    pub value: T,
}

/// Generic state managing item filtering, selection cursor, and scrolling.
#[derive(Debug, Clone)]
pub struct PickerState<T: Clone> {
    pub title: String,
    pub items: Vec<PickerItem<T>>,
    pub filtered_indices: Vec<usize>,
    pub selected_index: usize,
    pub scroll_offset: usize,
    pub max_visible_items: usize,
    pub search_query: String,
    pub is_open: bool,
    pub empty_message: String,
    pub footer: String,
}

impl<T: Clone> PickerState<T> {
    /// Creates a new open picker state populated with the given items.
    pub fn new(title: String, items: Vec<PickerItem<T>>) -> Self {
        let count = items.len();
        Self {
            title,
            items,
            filtered_indices: (0..count).collect(),
            selected_index: 0,
            scroll_offset: 0,
            max_visible_items: 8,
            search_query: String::new(),
            is_open: true,
            empty_message: String::new(),
            footer: String::new(),
        }
    }

    /// Re-evaluates filtered items based on the search query across label, detail, and meta.
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
        } else {
            if self.selected_index >= self.filtered_indices.len() {
                self.selected_index = self.filtered_indices.len() - 1;
            }
            self.adjust_scroll();
        }
    }

    /// Pushes a character to the search query and refreshes the filter.
    pub fn type_char(&mut self, c: char) {
        self.search_query.push(c);
        self.filter();
    }

    /// Deletes the trailing character from the search query and refreshes the filter.
    pub fn backspace(&mut self) {
        self.search_query.pop();
        self.filter();
    }

    /// Moves the selection cursor up, wrapping to the bottom if at the top.
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

    /// Moves the selection cursor down, wrapping to the top if at the bottom.
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

    /// Keeps the selected item inside the visible scrolling window.
    pub fn adjust_scroll(&mut self) {
        if self.selected_index < self.scroll_offset {
            self.scroll_offset = self.selected_index;
        } else if self.selected_index >= self.scroll_offset + self.max_visible_items {
            self.scroll_offset = (self.selected_index + 1).saturating_sub(self.max_visible_items);
        }

        // Clamp scroll_offset so the visible window never scrolls past the end of the list,
        // ensuring all items are visible when the list fits within max_visible_items.
        let max_offset = self
            .filtered_indices
            .len()
            .saturating_sub(self.max_visible_items);
        if self.scroll_offset > max_offset {
            self.scroll_offset = max_offset;
        }
    }

    /// Returns a reference to the currently selected item, if any.
    pub fn selected_item(&self) -> Option<&PickerItem<T>> {
        let item_idx = *self.filtered_indices.get(self.selected_index)?;
        self.items.get(item_idx)
    }

    /// Commits the active selection: closes the modal and returns the selected value.
    pub fn commit_selection(&mut self) -> Option<T> {
        let item = self.selected_item()?.value.clone();
        self.is_open = false;
        Some(item)
    }

    /// Cancels and closes the modal without committing.
    pub fn cancel(&mut self) {
        self.is_open = false;
    }
}

// ============================================================================
// 2. Concrete Domain Types: ModelItem & EffortLevel
// ============================================================================

/// Metadata model descriptor for AI models available in the model picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelItem {
    pub provider_id: String,
    pub id: String,
    pub provider: String,
    pub status: String,
    pub latency: String,
}

impl ModelItem {
    pub fn new(
        id: impl Into<String>,
        provider: impl Into<String>,
        status: impl Into<String>,
        latency: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            provider_id: String::new(),
            provider: provider.into(),
            status: status.into(),
            latency: latency.into(),
        }
    }

    pub fn to_picker_item(&self) -> PickerItem<ModelItem> {
        PickerItem {
            label: self.id.clone(),
            detail: format!("{} \u{2022} {}", self.provider, self.status),
            meta: self.latency.clone(),
            value: self.clone(),
        }
    }
}

/// Reasoning effort level settings matching top-tier harness specs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EffortLevel {
    Low,
    Medium,
    #[default]
    High,
    Max,
}

impl EffortLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Max => "max",
        }
    }

    pub fn detail(self) -> &'static str {
        match self {
            Self::Low => "Fastest response, minimal reasoning",
            Self::Medium => "Balanced reasoning for standard coding",
            Self::High => "Deep reasoning with multi-step planning",
            Self::Max => "Full thinking budget, architectural breakdown",
        }
    }

    pub fn meta(self) -> &'static str {
        match self {
            Self::Low => "~1k tokens",
            Self::Medium => "~4k tokens",
            Self::High => "~16k tokens",
            Self::Max => "~32k tokens",
        }
    }

    pub fn to_picker_item(self) -> PickerItem<EffortLevel> {
        PickerItem {
            label: self.as_str().to_string(),
            detail: self.detail().to_string(),
            meta: self.meta().to_string(),
            value: self,
        }
    }
}

// ============================================================================
// 3. Helper Item Constructors
// ============================================================================

/// Constructs the 5 theme items matching Spec 6.
pub fn theme_picker_items() -> Vec<PickerItem<Palette>> {
    vec![
        PickerItem {
            label: DASHBOARD.name.into(),
            detail: "Black · cyan · green".into(),
            meta: String::new(),
            value: DASHBOARD,
        },
        PickerItem {
            label: "Classic".into(),
            detail: "Win95 retro grey".into(),
            meta: "light".into(),
            value: CLASSIC,
        },
        PickerItem {
            label: "Phosphor".into(),
            detail: "VT220 amber/green CRT".into(),
            meta: "retro".into(),
            value: PHOSPHOR,
        },
        PickerItem {
            label: "Dark Slate".into(),
            detail: "Modern dark slate palette".into(),
            meta: "dark".into(),
            value: DARK_SLATE,
        },
        PickerItem {
            label: "Dracula".into(),
            detail: "Vampire purple & pink".into(),
            meta: "dark".into(),
            value: DRACULA,
        },
        PickerItem {
            label: "Tokyo Night".into(),
            detail: "Neon night downtown Tokyo".into(),
            meta: "dark".into(),
            value: TOKYO_NIGHT,
        },
    ]
}

/// Constructs the default models available for switching.
/// No endpoint roster is available to this compatibility helper.
/// Interactive sessions use WorkstationState::available_models.
pub fn model_picker_items() -> Vec<PickerItem<ModelItem>> {
    Vec::new()
}
/// Constructs the 4 reasoning effort levels.
pub fn effort_picker_items() -> Vec<PickerItem<EffortLevel>> {
    vec![
        EffortLevel::Low.to_picker_item(),
        EffortLevel::Medium.to_picker_item(),
        EffortLevel::High.to_picker_item(),
        EffortLevel::Max.to_picker_item(),
    ]
}

/// Default theme picker state initialized with the current palette selected.
pub fn default_theme_picker(current_theme: &Palette) -> PickerState<Palette> {
    let items = theme_picker_items();
    let mut state = PickerState::new("Switch Theme".into(), items);
    if let Some(pos) = state
        .items
        .iter()
        .position(|it| it.value.name == current_theme.name)
    {
        state.selected_index = pos;
    }
    state
}

/// Default model picker state initialized with the current model selected.
pub fn default_model_picker(current_model: &str) -> PickerState<ModelItem> {
    let items =
        vec![ModelItem::new(current_model, "endpoint", "not queried", "--").to_picker_item()];
    let mut state = PickerState::new("Switch Model".into(), items);
    if let Some(pos) = state.items.iter().position(|it| it.label == current_model) {
        state.selected_index = pos;
    }
    state
}

/// Default effort picker state initialized with the current effort selected.
pub fn default_effort_picker(current_effort: &str) -> PickerState<EffortLevel> {
    let items = effort_picker_items();
    let mut state = PickerState::new("Reasoning Effort".into(), items);
    if let Some(pos) = state.items.iter().position(|it| it.label == current_effort) {
        state.selected_index = pos;
    }
    state
}

// ============================================================================
// 4. Active Modal Enum
// ============================================================================

/// Represents any modal dialog currently overlaid on the workstation.
#[derive(Debug, Clone, Default)]
pub enum ActiveModal {
    #[default]
    None,
    ThemePicker(PickerState<Palette>),
    ModelPicker(PickerState<ModelItem>),
    EffortPicker(PickerState<EffortLevel>),
}

impl ActiveModal {
    /// Returns true if a modal is currently open.
    pub fn is_open(&self) -> bool {
        match self {
            Self::None => false,
            Self::ThemePicker(p) => p.is_open,
            Self::ModelPicker(p) => p.is_open,
            Self::EffortPicker(p) => p.is_open,
        }
    }
}

// ============================================================================
// 5. Overlay Rendering
// ============================================================================

/// Renders a centered floating modal picker over the provided area.
pub fn render_picker_modal<T: Clone>(
    frame: &mut Frame,
    area: Rect,
    picker: &PickerState<T>,
    pal: &Palette,
) {
    if !picker.is_open || area.width < 10 || area.height < 5 {
        return;
    }

    // Modal dimensions: 64 wide (clamped), 14 high (clamped), centered
    let width = 64.min(area.width.saturating_sub(4)).max(10);
    let height = 14.min(area.height.saturating_sub(2)).max(5);
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    let modal_area = Rect::new(x, y, width, height);

    // Clear background cells to prevent underlying text bleed
    frame.render_widget(Clear, modal_area);

    let modal_block = Block::default()
        .style(Style::default().bg(pal.chrome_bg))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(pal.accent))
        .title(Span::styled(
            format!(" {} ", picker.title),
            Style::default()
                .fg(pal.titlebar_fg)
                .bg(pal.titlebar_bg)
                .add_modifier(Modifier::BOLD),
        ));
    frame.render_widget(modal_block, modal_area);

    let inner = Rect::new(
        modal_area.x + 1,
        modal_area.y + 1,
        modal_area.width.saturating_sub(2),
        modal_area.height.saturating_sub(2),
    );
    if inner.is_empty() {
        return;
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // search input
            Constraint::Length(1), // divider rule
            Constraint::Min(3),    // item list
            Constraint::Length(1), // footer
        ])
        .split(inner);

    // Row 0: Search input bar
    let clean_query = crate::sanitize_ansi(&picker.search_query);
    let search_line = Line::from(vec![
        Span::styled(" Search: ", Style::default().fg(pal.chrome_fg)),
        Span::styled(
            clean_query,
            Style::default()
                .fg(pal.viewport_fg)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("\u{258c}", Style::default().fg(pal.accent)),
    ]);
    frame.render_widget(Paragraph::new(search_line), chunks[0]);

    // Row 1: Divider rule
    let sep = Line::from(Span::styled(
        "\u{2500}".repeat(chunks[1].width as usize),
        Style::default().fg(pal.panel_border),
    ));
    frame.render_widget(Paragraph::new(sep), chunks[1]);

    // Row 2: Item list with scroll window
    if picker.filtered_indices.is_empty() {
        let empty_msg = if picker.search_query.is_empty() && !picker.empty_message.is_empty() {
            picker.empty_message.clone()
        } else {
            "   No matching items found".into()
        };
        frame.render_widget(
            Paragraph::new(empty_msg).style(Style::default().fg(pal.chrome_fg)),
            chunks[2],
        );
    } else {
        let mut lines = Vec::new();
        let capacity = chunks[2].height as usize;
        let start_idx = picker
            .selected_index
            .saturating_sub(capacity.saturating_sub(1));
        let end_idx = (start_idx + capacity).min(picker.filtered_indices.len());
        for (pos, &actual_idx) in picker.filtered_indices[start_idx..end_idx]
            .iter()
            .enumerate()
        {
            let list_idx = start_idx + pos;
            let item = &picker.items[actual_idx];
            let is_selected = list_idx == picker.selected_index;

            let (prefix, style) = if is_selected {
                (
                    " > ",
                    Style::default()
                        .fg(pal.chrome_bg)
                        .bg(pal.accent)
                        .add_modifier(Modifier::BOLD),
                )
            } else {
                ("   ", Style::default().fg(pal.viewport_fg))
            };

            let detail_width = if inner.width >= 46 { 18 } else { 0 };
            let label_width = inner.width.saturating_sub(detail_width + 3) as usize;
            let label = crate::fit_text(&item.label, label_width);
            let padding = label_width.saturating_sub(Span::raw(&label).width());
            let detail = crate::fit_text(&item.detail, detail_width as usize);
            lines.push(Line::from(vec![
                Span::styled(prefix, style),
                Span::styled(format!("{label}{}{detail}", " ".repeat(padding)), style),
            ]));
        }
        frame.render_widget(Paragraph::new(lines), chunks[2]);
    }

    // Row 3: Footer navigation hint with counter
    let count_str = if picker.filtered_indices.is_empty() {
        "(0/0)".to_string()
    } else {
        format!(
            "({}/{})",
            picker.selected_index + 1,
            picker.filtered_indices.len()
        )
    };
    let footer_line = Line::from(vec![Span::styled(
        if !picker.footer.is_empty() {
            crate::fit_text(&picker.footer, chunks[3].width as usize)
        } else {
            format!(
                " \u{2191}/\u{2193} Navigate \u{2022} Enter Select \u{2022} Esc Cancel   {}",
                count_str
            )
        },
        Style::default().fg(pal.chrome_fg),
    )]);
    frame.render_widget(Paragraph::new(footer_line), chunks[3]);
}

/// Renders the currently active modal overlay, if any.
pub fn render_active_modal(frame: &mut Frame, area: Rect, modal: &ActiveModal, pal: &Palette) {
    match modal {
        ActiveModal::None => {}
        ActiveModal::ThemePicker(p) => render_picker_modal(frame, area, p, pal),
        ActiveModal::ModelPicker(p) => render_picker_modal(frame, area, p, pal),
        ActiveModal::EffortPicker(p) => render_picker_modal(frame, area, p, pal),
    }
}

// ============================================================================
// 6. Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{backend::TestBackend, Terminal};

    fn sample_picker() -> PickerState<String> {
        let items = vec![
            PickerItem {
                label: "apple".into(),
                detail: "red fruit".into(),
                meta: "fruit".into(),
                value: "A".into(),
            },
            PickerItem {
                label: "banana".into(),
                detail: "yellow fruit".into(),
                meta: "fruit".into(),
                value: "B".into(),
            },
            PickerItem {
                label: "carrot".into(),
                detail: "orange root".into(),
                meta: "vegetable".into(),
                value: "C".into(),
            },
        ];
        PickerState::new("Test Picker".into(), items)
    }

    #[test]
    fn test_picker_navigation_and_wraparound() {
        let mut picker = sample_picker();
        assert_eq!(picker.selected_index, 0);
        assert_eq!(picker.selected_item().unwrap().label, "apple");

        picker.move_down();
        assert_eq!(picker.selected_index, 1);
        assert_eq!(picker.selected_item().unwrap().label, "banana");

        picker.move_down();
        assert_eq!(picker.selected_index, 2);
        assert_eq!(picker.selected_item().unwrap().label, "carrot");

        // Wraparound down -> 0
        picker.move_down();
        assert_eq!(picker.selected_index, 0);

        // Wraparound up -> 2
        picker.move_up();
        assert_eq!(picker.selected_index, 2);
    }

    #[test]
    fn test_picker_search_and_backspace() {
        let mut picker = sample_picker();

        picker.type_char('r');
        picker.type_char('o');
        picker.type_char('o');
        picker.type_char('t');
        assert_eq!(picker.search_query, "root");
        assert_eq!(picker.filtered_indices.len(), 1);
        assert_eq!(picker.selected_item().unwrap().label, "carrot");

        picker.backspace();
        picker.backspace();
        picker.backspace();
        picker.backspace();
        assert_eq!(picker.search_query, "");
        assert_eq!(picker.filtered_indices.len(), 3);
    }

    #[test]
    fn test_picker_empty_search_and_commit() {
        let mut picker = sample_picker();
        for c in "xyz123".chars() {
            picker.type_char(c);
        }
        assert!(picker.filtered_indices.is_empty());
        assert_eq!(picker.selected_item(), None);
        assert_eq!(picker.commit_selection(), None);
        assert!(picker.is_open);

        picker.cancel();
        assert!(!picker.is_open);
    }

    #[test]
    fn test_picker_scrolling_window() {
        let items: Vec<PickerItem<usize>> = (0..20)
            .map(|i| PickerItem {
                label: format!("item-{i:02}"),
                detail: "desc".into(),
                meta: "tag".into(),
                value: i,
            })
            .collect();
        let mut picker = PickerState::new("Scroll Test".into(), items);
        picker.max_visible_items = 5;

        for _ in 0..12 {
            picker.move_down();
        }
        assert_eq!(picker.selected_index, 12);
        assert!(picker.scroll_offset <= 12);
        assert!(picker.scroll_offset + picker.max_visible_items > 12);
        assert_eq!(picker.selected_item().unwrap().value, 12);
    }

    #[test]
    fn test_effort_and_model_helpers() {
        let model = ModelItem::new("qwen2.5-coder:7b", "local", "ready", "45ms");
        let item = model.to_picker_item();
        assert_eq!(item.label, "qwen2.5-coder:7b");
        assert!(item.detail.contains("local"));

        let effort = EffortLevel::High;
        assert_eq!(effort.as_str(), "high");
        let eff_item = effort.to_picker_item();
        assert_eq!(eff_item.label, "high");

        let default_tp = default_theme_picker(&DARK_SLATE);
        assert_eq!(default_tp.selected_item().unwrap().label, "Dark Slate");

        let default_mp = default_model_picker("gpt-4o");
        assert_eq!(default_mp.selected_item().unwrap().label, "gpt-4o");

        let default_ep = default_effort_picker("max");
        assert_eq!(default_ep.selected_item().unwrap().label, "max");
    }

    #[test]
    fn test_render_picker_modal() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let picker = sample_picker();
        let pal = DARK_SLATE;

        terminal
            .draw(|f| {
                render_picker_modal(f, Rect::new(0, 0, 80, 24), &picker, &pal);
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        let mut found_title = false;
        let mut found_apple = false;
        let mut found_search = false;
        for y in 0..24 {
            let mut line = String::new();
            for x in 0..80 {
                line.push_str(buffer[(x, y)].symbol());
            }
            if line.contains("Test Picker") {
                found_title = true;
            }
            if line.contains("apple") {
                found_apple = true;
            }
            if line.contains("Search:") {
                found_search = true;
            }
        }
        assert!(found_title, "Title must be rendered");
        assert!(found_apple, "Item must be rendered");
        assert!(found_search, "Search prompt must be rendered");
    }
}
