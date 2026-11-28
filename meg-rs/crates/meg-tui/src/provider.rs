//! Connection management is a dedicated masked editor, never a chat prompt.
use crate::{fit_text, Palette};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use meg_core::providers::{ProviderKind, ProviderProfile, Secret};
use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::Line,
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
    Frame,
};

#[derive(Debug, Clone)]
pub struct ProviderDialog {
    pub profiles: Vec<ProviderProfile>,
    pub selected: usize,
    pub editor: Option<ProviderEditor>,
    pub query: String,
    pub pending_key: Secret,
    pub statuses: std::collections::BTreeMap<String, String>,
    pub messages: std::collections::BTreeMap<String, String>,
    pub connected: std::collections::HashSet<String>,
}
#[derive(Debug, Clone)]
pub struct ProviderEditor {
    pub auto_detect: bool,
    pub advanced: bool,
    pub profile: ProviderProfile,
    pub key: Secret,
    pub key_env: String,
    pub focus: usize,
    pub cursor: usize,
    pub replace_next: bool,
    pub stored_key: bool,
}
pub enum ProviderAction {
    None,
    Use(String),
    Save(ProviderProfile, Option<Secret>),
    Remove(String),
    Close,
}
impl ProviderDialog {
    pub fn new(mut profiles: Vec<ProviderProfile>, active: Option<&str>) -> Self {
        for preset in meg_core::provider_catalog::PROVIDER_PRESETS {
            if !profiles.iter().any(|p| p.id == preset.id) {
                profiles.push(preset.profile());
            }
        }
        let connected: std::collections::HashSet<_> = profiles
            .iter()
            .filter(|p| meg_core::providers::has_key(p))
            .map(|p| p.id.clone())
            .collect();
        profiles.sort_by_key(|p| {
            (
                if connected.contains(&p.id) {
                    0
                } else if meg_core::provider_catalog::is_local(p) {
                    1
                } else {
                    2
                },
                p.name.to_lowercase(),
            )
        });
        let selected = active
            .and_then(|id| profiles.iter().position(|p| p.id == id))
            .unwrap_or(0);
        Self {
            profiles,
            selected,
            editor: None,
            query: String::new(),
            pending_key: Secret::default(),
            statuses: Default::default(),
            messages: Default::default(),
            connected,
        }
    }
    pub fn filtered(&self) -> Vec<usize> {
        let q = self.query.to_lowercase();
        self.profiles
            .iter()
            .enumerate()
            .filter(|(_, p)| {
                format!("{} {} {}", p.name, p.id, p.base_url)
                    .to_lowercase()
                    .contains(&q)
            })
            .map(|(i, _)| i)
            .collect()
    }
    pub fn new_connection(&mut self, kind: ProviderKind) {
        let (name, url) = match kind {
            ProviderKind::OpenAi => ("Custom API", ""),
            ProviderKind::OpenAiResponses => ("OpenAI Responses", "https://api.openai.com/v1"),
            ProviderKind::Anthropic => ("Anthropic", "https://api.anthropic.com/v1"),
            ProviderKind::Gemini => ("Google Gemini", "https://generativelanguage.googleapis.com/v1beta"),
            ProviderKind::Ollama => ("Local Ollama", "http://127.0.0.1:11434/v1"),
            ProviderKind::Shim => ("Local API", "http://127.0.0.1:1234/v1"),
        };
        let mut editor = ProviderEditor::new(
            ProviderProfile {
                id: uuid::Uuid::new_v4().to_string(),
                name: name.into(),
                kind: if kind == ProviderKind::Shim {
                    ProviderKind::OpenAi
                } else {
                    kind
                },
                base_url: url.into(),
                default_model: String::new(),
                key_env: None,
                credential_scope: meg_core::providers::credential_scope(
                    &meg_core::providers::config_dir(),
                ),
                send_effort: false,
            },
            false,
        );
        editor.advanced = true;
        if !self.pending_key.is_empty() {
            editor.key = std::mem::take(&mut self.pending_key);
        }
        self.editor = Some(editor);
    }
    pub fn new_preset(&mut self, id: &str) -> bool {
        let profile = self
            .profiles
            .iter()
            .find(|p| p.id == id)
            .cloned()
            .or_else(|| meg_core::provider_catalog::provider_preset(id).map(|p| p.profile()));
        if let Some(profile) = profile {
            self.edit_connection(profile);
            true
        } else {
            false
        }
    }
    pub fn edit_connection(&mut self, profile: ProviderProfile) {
        let stored = meg_core::providers::has_key(&profile);
        let mut editor = ProviderEditor::new(profile, stored);
        editor.auto_detect = false;
        editor.advanced = meg_core::provider_catalog::provider_preset(&editor.profile.id).is_none()
            || meg_core::provider_catalog::is_local(&editor.profile);
        editor.focus = 4;
        if !self.pending_key.is_empty() {
            editor.key = std::mem::take(&mut self.pending_key);
        }
        editor.end();
        self.editor = Some(editor);
    }
    pub fn paste(&mut self, text: &str) {
        let text = text.trim_matches(['\r', '\n']);
        if let Some(editor) = &mut self.editor {
            editor.insert(text);
            return;
        }
        if let Some(preset) = meg_core::provider_catalog::key_preset(text) {
            self.new_preset(preset.id);
            let e = self.editor.as_mut().unwrap();
            e.focus = 4;
            e.auto_detect = false;
            e.insert(text);
        } else if text.len() >= 20
            && !text.chars().any(char::is_whitespace)
            && !text.starts_with("http")
        {
            // Keep an opaque pasted credential out of search text and render buffers.
            self.pending_key.replace(text.into());
            self.query.clear();
            self.selected = 0;
        } else {
            self.query
                .extend(text.chars().filter(|c| !c.is_control()).take(256));
            self.selected = 0;
        }
    }
    pub fn key(&mut self, key: KeyEvent) -> ProviderAction {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if let Some(editor) = &mut self.editor {
            if (ctrl && key.code == KeyCode::Char('s')) || key.code == KeyCode::Enter {
                let mut profile = editor.profile.clone();
                profile.key_env = if !editor.key.is_empty() || editor.key_env.trim().is_empty() {
                    None
                } else {
                    Some(editor.key_env.trim().into())
                };
                return ProviderAction::Save(
                    profile,
                    (!editor.key.is_empty()).then(|| editor.key.clone()),
                );
            }
            if ctrl && key.code == KeyCode::Char('e') {
                editor.advanced = !editor.advanced;
                editor.focus = 4;
                editor.end();
                return ProviderAction::None;
            }
            if ctrl && key.code == KeyCode::Char('a') {
                editor.replace_next = true;
                editor.cursor = 0;
                return ProviderAction::None;
            }
            if !editor.advanced && matches!(key.code, KeyCode::Tab | KeyCode::BackTab) {
                editor.advanced = true;
                editor.focus = 5;
                editor.end();
                return ProviderAction::None;
            }
            match key.code {
                KeyCode::Tab | KeyCode::Down if editor.advanced => {
                    editor.focus = (editor.focus + 1) % 6;
                    editor.end();
                }
                KeyCode::BackTab | KeyCode::Up if editor.advanced => {
                    editor.focus = (editor.focus + 5) % 6;
                    editor.end();
                }
                KeyCode::Left | KeyCode::Right if editor.focus == 1 => editor.cycle(),
                KeyCode::Left => editor.cursor = editor.cursor.saturating_sub(1),
                KeyCode::Right => {
                    editor.cursor = (editor.cursor + 1).min(editor.value().chars().count())
                }
                KeyCode::Home => editor.cursor = 0,
                KeyCode::End => editor.end(),
                KeyCode::Backspace => editor.backspace(),
                KeyCode::Delete => editor.delete(),
                KeyCode::Char(' ') if editor.focus == 1 => editor.cycle(),
                KeyCode::Char(ch) if !ctrl => editor.insert(&ch.to_string()),
                _ => {}
            }
            return ProviderAction::None;
        }
        let indices = self.filtered();
        match key.code {
            KeyCode::Up => self.selected = self.selected.saturating_sub(1),
            KeyCode::Down => {
                self.selected = (self.selected + 1).min(indices.len().saturating_sub(1))
            }
            KeyCode::Backspace => {
                self.query.pop();
                self.selected = 0;
            }
            KeyCode::Char('a' | 'u') if ctrl => {
                self.query.clear();
                self.selected = 0;
            }
            KeyCode::Char('n') if ctrl => self.new_connection(ProviderKind::OpenAi),
            KeyCode::Char('e') if ctrl => {
                if let Some(p) = indices
                    .get(self.selected)
                    .and_then(|i| self.profiles.get(*i))
                {
                    self.edit_connection(p.clone());
                }
            }
            KeyCode::Delete => {
                if let Some(p) = indices
                    .get(self.selected)
                    .and_then(|i| self.profiles.get(*i))
                {
                    return ProviderAction::Remove(p.id.clone());
                }
            }
            KeyCode::Enter => {
                if let Some(p) = indices
                    .get(self.selected)
                    .and_then(|i| self.profiles.get(*i))
                    .cloned()
                {
                    let cloud = meg_core::provider_catalog::provider_preset(&p.id)
                        .is_some_and(|p| !p.local);
                    if !self.pending_key.is_empty() || (cloud && !meg_core::providers::has_key(&p))
                    {
                        self.edit_connection(p);
                    } else {
                        return ProviderAction::Use(p.id);
                    }
                }
            }
            KeyCode::Char('1'..='5') if self.query.is_empty() => {
                let id = match key.code {
                    KeyCode::Char('1') => "openrouter",
                    KeyCode::Char('2') => "openai",
                    KeyCode::Char('3') => "anthropic",
                    KeyCode::Char('4') => "ollama",
                    _ => "lmstudio",
                };
                self.new_preset(id);
            }
            KeyCode::Char(ch) if !ctrl && self.query.len() < 256 => {
                self.query.push(ch);
                self.selected = 0;
            }
            _ => {}
        }
        ProviderAction::None
    }
}
impl ProviderEditor {
    fn new(profile: ProviderProfile, stored_key: bool) -> Self {
        let key_env = profile.key_env.clone().unwrap_or_default();
        let cursor = profile.name.chars().count();
        Self {
            auto_detect: !stored_key,
            advanced: true,
            profile,
            key: Secret::default(),
            key_env,
            focus: 0,
            cursor,
            replace_next: false,
            stored_key,
        }
    }
    fn value(&self) -> &str {
        match self.focus {
            0 => &self.profile.name,
            2 => &self.profile.base_url,
            3 => &self.profile.default_model,
            4 => self.key.expose(),
            5 => &self.key_env,
            _ => "",
        }
    }
    fn end(&mut self) {
        self.cursor = self.value().chars().count();
        self.replace_next = false;
    }
    fn byte(&self, pos: usize) -> usize {
        self.value()
            .char_indices()
            .nth(pos)
            .map(|(i, _)| i)
            .unwrap_or(self.value().len())
    }
    fn plain(&mut self) -> Option<&mut String> {
        match self.focus {
            0 => Some(&mut self.profile.name),
            2 => Some(&mut self.profile.base_url),
            3 => Some(&mut self.profile.default_model),
            5 => Some(&mut self.key_env),
            _ => None,
        }
    }
    pub fn insert(&mut self, text: &str) {
        if self.focus == 1 {
            return;
        }
        if self.focus == 2 {
            self.auto_detect = false;
        }
        let text: String = text.chars().filter(|c| !c.is_control()).collect();
        if self.value().len() + text.len() > 2048 {
            return;
        }
        if self.replace_next {
            if self.focus == 4 {
                self.key.replace(String::new());
            } else if let Some(s) = self.plain() {
                s.clear();
            }
            self.cursor = 0;
            self.replace_next = false;
        }
        let byte = self.byte(self.cursor);
        if self.focus == 4 {
            self.key.insert_str(byte, &text);
        } else if let Some(s) = self.plain() {
            s.insert_str(byte, &text);
        }
        self.cursor += text.chars().count();
        if self.focus == 4 && self.auto_detect {
            if let Some((name, kind, url)) =
                meg_core::providers::detect_key_provider(self.key.expose())
            {
                if self.profile.base_url != url {
                    self.profile.default_model.clear();
                }
                self.profile.name = name.into();
                self.profile.id = meg_core::provider_catalog::key_preset(self.key.expose())
                    .unwrap()
                    .id
                    .into();
                self.profile.kind = kind;
                self.profile.base_url = url.into();
                self.profile.send_effort = false;
            }
        }
    }
    fn backspace(&mut self) {
        if self.replace_next {
            self.insert("");
            return;
        }
        if self.cursor > 0 {
            let end = self.byte(self.cursor);
            let start = self.byte(self.cursor - 1);
            if self.focus == 4 {
                self.key.erase(start, end);
            } else if let Some(s) = self.plain() {
                s.replace_range(start..end, "");
            }
            self.cursor -= 1;
        }
    }
    fn delete(&mut self) {
        let start = self.byte(self.cursor);
        let end = self.byte(self.cursor + 1);
        if self.focus == 4 {
            self.key.erase(start, end);
        } else if let Some(s) = self.plain() {
            s.replace_range(start..end, "");
        }
    }
    fn cycle(&mut self) {
        self.profile.kind = match self.profile.kind {
            ProviderKind::OpenAi => ProviderKind::Anthropic,
            ProviderKind::Anthropic => ProviderKind::Ollama,
            ProviderKind::Ollama => ProviderKind::OpenAiResponses,
            ProviderKind::OpenAiResponses => ProviderKind::Gemini,
            ProviderKind::Gemini => ProviderKind::OpenAi,
            ProviderKind::Shim => ProviderKind::OpenAi,
        };
        self.profile.send_effort = false;
    }
}

pub fn render_provider(frame: &mut Frame, screen: Rect, dialog: &ProviderDialog, pal: &Palette) {
    if screen.width < 20 || screen.height < 8 {
        return;
    }
    let width = screen.width.saturating_sub(4).min(92);
    let requested = match &dialog.editor {
        Some(e) if e.advanced => 23,
        Some(_) => 12,
        None => 22,
    };
    let height = requested.min(screen.height.saturating_sub(2));
    let area = Rect::new(
        screen.x + (screen.width - width) / 2,
        screen.y + (screen.height - height) / 2,
        width,
        height,
    );
    frame.render_widget(Clear, area);
    let title = dialog
        .editor
        .as_ref()
        .map(|e| format!(" Connect {} ", e.profile.name))
        .unwrap_or_else(|| " Connect a provider ".into());
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title(title)
        .style(Style::default().bg(pal.chrome_bg).fg(pal.viewport_fg))
        .border_style(Style::default().fg(pal.accent));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if let Some(editor) = &dialog.editor {
        let labels = [
            "Name",
            "Protocol (Space to change)",
            "API base URL",
            "Default model (optional)",
            "API key — paste here (masked)",
            "API-key environment variable",
        ];
        let mut top = inner.y;
        if !editor.advanced {
            frame.render_widget(
                Paragraph::new(fit_text(&editor.profile.base_url, inner.width as usize))
                    .style(Style::default().fg(pal.chrome_fg)),
                Rect::new(inner.x + 1, top, inner.width.saturating_sub(2), 1),
            );
            top += 2;
        }
        let capacity = (inner.bottom().saturating_sub(top + 2) / 3).max(1) as usize;
        let first = if editor.advanced {
            editor.focus.saturating_sub(capacity.saturating_sub(1))
        } else {
            4
        };
        let count = if editor.advanced {
            capacity.min(6 - first)
        } else {
            1
        };
        for (row, index) in (first..first + count).enumerate() {
            let y = top + row as u16 * 3;
            if y + 1 >= inner.bottom().saturating_sub(1) {
                break;
            }
            let selected = index == editor.focus;
            let value = match index {
                0 => editor.profile.name.clone(),
                1 => editor.profile.kind.label().into(),
                2 => editor.profile.base_url.clone(),
                3 => editor.profile.default_model.clone(),
                4 => {
                    if !editor.key.is_empty() {
                        "••••••••".into()
                    } else if editor.stored_key {
                        "Stored key retained; paste to replace".into()
                    } else {
                        "Paste your API key".into()
                    }
                }
                _ => editor.key_env.clone(),
            };
            frame.render_widget(
                Paragraph::new(fit_text(
                    &format!(" {} {}", if selected { "›" } else { " " }, labels[index]),
                    inner.width as usize,
                ))
                .style(Style::default().fg(if selected {
                    pal.accent
                } else {
                    pal.chrome_fg
                })),
                Rect::new(inner.x, y, inner.width, 1),
            );
            let columns = inner.width.saturating_sub(5) as usize;
            use unicode_width::UnicodeWidthStr;
            let caret = if index == 4 {
                if editor.key.is_empty() {
                    0
                } else {
                    8
                }
            } else {
                editor
                    .value()
                    .chars()
                    .take(editor.cursor)
                    .collect::<String>()
                    .width()
            };
            let mut offset = 0;
            let mut skipped = 0;
            if selected && index != 4 && index != 1 {
                for (byte, ch) in value.char_indices() {
                    if caret.saturating_sub(skipped) < columns {
                        break;
                    }
                    skipped += unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
                    offset = byte + ch.len_utf8();
                }
            }
            frame.render_widget(
                Paragraph::new(format!("   {}", fit_text(&value[offset..], columns)))
                    .style(Style::default().fg(pal.viewport_fg)),
                Rect::new(inner.x, y + 1, inner.width, 1),
            );
            if selected && index != 1 {
                frame.set_cursor_position((
                    inner.x + 3 + caret.saturating_sub(skipped).min(columns) as u16,
                    y + 1,
                ));
            }
        }
        if !editor.advanced && inner.height >= 8 {
            let hint = if editor.profile.key_env.is_some() {
                format!(" Environment: {}", editor.key_env)
            } else {
                " Stored in Windows Credential Manager; never in chat or settings.".into()
            };
            frame.render_widget(
                Paragraph::new(fit_text(&hint, inner.width as usize))
                    .style(Style::default().fg(pal.chrome_fg)),
                Rect::new(inner.x, inner.bottom() - 3, inner.width, 1),
            );
        }
        frame.render_widget(
            Paragraph::new(fit_text(
                if editor.advanced {
                    " Enter connect · Tab fields · Ctrl+A replace · Ctrl+E simple · Esc cancel"
                } else {
                    " Enter connect · Ctrl+E advanced · Esc cancel"
                },
                inner.width as usize,
            ))
            .style(Style::default().fg(pal.chrome_fg)),
            Rect::new(inner.x, inner.bottom() - 1, inner.width, 1),
        );
    } else {
        let indices = dialog.filtered();
        let subtitle = if dialog.pending_key.is_empty() {
            "Select a service, then paste its API key. Local servers need no key."
        } else {
            "Key received (masked). Select its provider to continue."
        };
        frame.render_widget(
            Paragraph::new(fit_text(subtitle, inner.width as usize))
                .style(Style::default().fg(pal.chrome_fg)),
            Rect::new(inner.x + 1, inner.y, inner.width.saturating_sub(2), 1),
        );
        frame.render_widget(
            Paragraph::new(fit_text(
                &format!(" Search: {}▌", dialog.query),
                inner.width as usize,
            ))
            .style(Style::default().fg(pal.accent)),
            Rect::new(inner.x, inner.y + 2, inner.width, 1),
        );
        let capacity = inner.height.saturating_sub(7) as usize;
        let start = dialog.selected.saturating_sub(capacity.saturating_sub(1));
        let lines: Vec<_> = indices
            .iter()
            .skip(start)
            .take(capacity)
            .enumerate()
            .map(|(offset, index)| {
                let p = &dialog.profiles[*index];
                let selected = start + offset == dialog.selected;
                let status = dialog.statuses.get(&p.id).cloned().unwrap_or_else(|| {
                    if dialog.connected.contains(&p.id) {
                        "key configured".into()
                    } else if meg_core::provider_catalog::is_local(p) {
                        "local runtime".into()
                    } else {
                        "Connect".into()
                    }
                });
                let label = format!(" {} {}", if selected { "›" } else { " " }, p.name);
                let row = format!("{:<30} {}", fit_text(&label, 30), status);
                Line::styled(
                    fit_text(&row, inner.width as usize),
                    Style::default()
                        .fg(if selected {
                            pal.chrome_bg
                        } else {
                            pal.viewport_fg
                        })
                        .bg(if selected { pal.accent } else { pal.chrome_bg })
                        .add_modifier(if selected {
                            Modifier::BOLD
                        } else {
                            Modifier::empty()
                        }),
                )
            })
            .collect();
        frame.render_widget(
            Paragraph::new(lines),
            Rect::new(inner.x, inner.y + 4, inner.width, capacity as u16),
        );
        if indices.is_empty() {
            frame.render_widget(
                Paragraph::new(" No matching service. Ctrl+N adds a custom endpoint.")
                    .style(Style::default().fg(pal.chrome_fg)),
                Rect::new(inner.x, inner.y + 4, inner.width, 1),
            );
        }
        if let Some(profile) = indices
            .get(dialog.selected)
            .and_then(|i| dialog.profiles.get(*i))
        {
            let detail = dialog
                .messages
                .get(&profile.id)
                .filter(|s| !s.is_empty())
                .unwrap_or(&profile.base_url);
            frame.render_widget(
                Paragraph::new(fit_text(&format!(" {detail}"), inner.width as usize))
                    .style(Style::default().fg(pal.chrome_fg)),
                Rect::new(inner.x, inner.bottom() - 3, inner.width, 1),
            );
        }
        let hints = [
            " Enter connect/use · Ctrl+E edit · Del disconnect · Ctrl+N custom",
            " Paste a key here for recognition, or search for its provider.",
        ];
        for (i, hint) in hints.iter().enumerate() {
            frame.render_widget(
                Paragraph::new(fit_text(hint, inner.width as usize))
                    .style(Style::default().fg(pal.chrome_fg)),
                Rect::new(inner.x, inner.bottom() - 2 + i as u16, inner.width, 1),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn distinctive_key_selects_provider_but_an_explicit_endpoint_is_preserved() {
        let mut dialog = ProviderDialog::new(Vec::new(), None);
        dialog.new_connection(ProviderKind::OpenAi);
        dialog.editor.as_mut().unwrap().focus = 4;
        dialog.paste("sk-or-v1-fixture");
        let editor = dialog.editor.as_ref().unwrap();
        assert_eq!(editor.profile.name, "OpenRouter");
        assert_eq!(editor.profile.base_url, "https://openrouter.ai/api/v1");
        let mut explicit = ProviderDialog::new(Vec::new(), None);
        explicit.new_connection(ProviderKind::OpenAi);
        let editor = explicit.editor.as_mut().unwrap();
        editor.focus = 2;
        editor.replace_next = true;
        editor.insert("http://127.0.0.1:8000/v1");
        editor.focus = 4;
        explicit.paste("sk-or-v1-fixture");
        assert_eq!(
            explicit.editor.as_ref().unwrap().profile.base_url,
            "http://127.0.0.1:8000/v1"
        );
    }
    #[test]
    fn ambiguous_keys_require_an_explicit_endpoint_before_they_can_be_saved() {
        let mut dialog = ProviderDialog::new(Vec::new(), None);
        dialog.new_connection(ProviderKind::OpenAi);
        dialog.editor.as_mut().unwrap().focus = 4;
        dialog.paste("sk-ambiguous-fixture-key");
        assert!(dialog.editor.as_ref().unwrap().profile.base_url.is_empty());
        let action = dialog.key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
        let ProviderAction::Save(profile, _) = action else {
            panic!("save action expected")
        };
        assert!(profile.validate().is_err());
    }
    #[test]
    fn pasted_key_is_masked_and_not_in_render_buffer() {
        use ratatui::{backend::TestBackend, Terminal};
        let mut d = ProviderDialog::new(Vec::new(), None);
        d.new_connection(ProviderKind::OpenAi);
        d.editor.as_mut().unwrap().focus = 4;
        d.paste("fixture-secret-key");
        assert!(!format!("{d:?}").contains("fixture-secret-key"));
        let mut terminal = Terminal::new(TestBackend::new(110, 30)).unwrap();
        terminal
            .draw(|f| render_provider(f, f.area(), &d, &crate::DASHBOARD))
            .unwrap();
        let text = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<String>();
        assert!(!text.contains("fixture-secret-key"));
        assert!(text.contains("••••••••"));
    }
}
