use super::Controller;
use crate::*;
use anyhow::{Context, Result};
use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEventKind};
use std::time::{Duration, Instant};

impl Controller {
    pub fn handle_event(&mut self, event: Event) -> Result<()> {
        match event {
            Event::Key(key) if key.kind != KeyEventKind::Release => self.key(key),
            Event::Paste(text) => {
                let text = crate::sanitize_ansi(&text.replace("\r\n", "\n").replace('\r', "\n"))
                    .replace('\t', "    ");
                anyhow::ensure!(
                    self.state.input.len() + text.len() <= 1024 * 1024,
                    "Input exceeds 1 MiB; attach the file instead"
                );
                if let Some(dialog) = &mut self.state.provider_dialog {
                    dialog.paste(&text);
                } else if self.state.palette.open && !self.palette_from_composer {
                    for c in text.chars().filter(|c| !c.is_control()) {
                        self.state.palette.push_char(c);
                    }
                } else if let Some(picker) = &mut self.state.workspace_picker {
                    picker.paste(&text);
                } else if let Some(picker) = &mut self.state.ui_picker {
                    for c in text.chars().filter(|c| !c.is_control()) {
                        picker.type_char(c);
                    }
                } else if let Some(picker) = &mut self.state.settings_picker {
                    for c in text.chars().filter(|c| !c.is_control()) {
                        picker.type_char(c);
                    }
                } else if let Some(picker) = &mut self.state.session_picker {
                    for c in text.chars().filter(|c| !c.is_control()) {
                        picker.type_char(c);
                    }
                } else if self.state.has_active_modal() {
                    for c in text.chars().filter(|c| !c.is_control()) {
                        self.state.active_modal_type_char(c);
                    }
                } else if self.state.workspace_picker.is_none()
                    && !self.state.has_active_modal()
                    && self.state.session_picker.is_none()
                {
                    self.handle_composer_paste(&text);
                    self.sync_palette();
                }
                Ok(())
            }
            Event::Mouse(mouse) if !self.overlay_open() => {
                match mouse.kind {
                    MouseEventKind::ScrollUp => {
                        self.state.transcript_scroll =
                            self.state.transcript_scroll.saturating_add(3)
                    }
                    MouseEventKind::ScrollDown => {
                        self.state.transcript_scroll =
                            self.state.transcript_scroll.saturating_sub(3)
                    }
                    _ => {}
                };
                Ok(())
            }
            Event::Resize(width, height) => {
                self.editor_width = DashboardLayout::new(
                    ratatui::layout::Rect::new(0, 0, width, height),
                    &self.state.input,
                )
                .composer
                .width;
                Ok(())
            }
            _ => Ok(()),
        }
    }


    pub(super) fn overlay_open(&self) -> bool {
        self.state.provider_dialog.is_some()
            || self.state.palette.open
            || self.state.has_active_modal()
            || self.state.workspace_picker.is_some()
            || self.state.session_picker.is_some()
            || self.state.settings_picker.is_some()
            || self.state.ui_picker.is_some()
    }


    pub(super) fn close_overlay(&mut self) {
        self.state.palette.close();
        self.state.cancel_active_modal();
        self.state.session_picker = None;
        self.state.settings_picker = None;
        let ui_snap = if self.state.ui_picker.take().is_some() { self.state.ui_mode_snapshot.take() } else { None };
        if let Some(mode) = ui_snap { self.state.ui_mode = mode; }
        self.state.workspace_picker = None;
        self.state.provider_dialog = None;
    }


    pub(super) fn preview_ui_picker(&mut self, picker: &PickerState<String>) {
        if let Some(val) = picker.clone().commit_selection() {
            match val.as_str() {
                "pro" => self.state.ui_mode = crate::ui_config::UiMode::Pro,
                "classic" => self.state.ui_mode = crate::ui_config::UiMode::Classic,
                _ => {}
            }
        }
    }


    pub(super) fn sync_palette(&mut self) {
        if self.palette_from_composer && self.state.palette.open {
            self.state.palette.query = self.state.input.clone();
            self.state.palette.selected = 0;
            if !self.state.input.starts_with('/') {
                self.state.palette.close();
            }
        }
    }


    pub(super) fn key(&mut self, key: KeyEvent) -> Result<()> {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl && key.code == KeyCode::Char('c') {
            if self.overlay_open() {
                self.ctrl_c_deadline = None;
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        if ((alt || ctrl) && matches!(key.code, KeyCode::Char('v') | KeyCode::Char('V')))
            && !self.overlay_open()
        {
            self.paste_from_system_clipboard();
            return Ok(());
        }
                self.close_overlay();
            } else if self.state.busy {
                self.ctrl_c_deadline = None;
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        if ((alt || ctrl) && matches!(key.code, KeyCode::Char('v') | KeyCode::Char('V')))
            && !self.overlay_open()
        {
            self.paste_from_system_clipboard();
            return Ok(());
        }
                self.cancel();
            } else if !self.state.input.is_empty() {
                self.ctrl_c_deadline = None;
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        if ((alt || ctrl) && matches!(key.code, KeyCode::Char('v') | KeyCode::Char('V')))
            && !self.overlay_open()
        {
            self.paste_from_system_clipboard();
            return Ok(());
        }
                self.state.input.clear();
                self.state.cursor_position = 0;
            } else {
                let now = Instant::now();
                if self.ctrl_c_deadline.is_some_and(|d| now <= d) {
                    self.ctrl_c_deadline = None;
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        if ((alt || ctrl) && matches!(key.code, KeyCode::Char('v') | KeyCode::Char('V')))
            && !self.overlay_open()
        {
            self.paste_from_system_clipboard();
            return Ok(());
        }
                    self.state.should_quit = true;
                } else {
                    let deadline = now + Duration::from_millis(2500);
                    self.ctrl_c_deadline = Some(deadline);
                    self.notice_deadline = Some(deadline);
                    self.state.notice = "Press Ctrl+C again to exit (or /exit)".into();
                }
            }
            return Ok(());
        }
        self.ctrl_c_deadline = None;
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        if ((alt || ctrl) && matches!(key.code, KeyCode::Char('v') | KeyCode::Char('V')))
            && !self.overlay_open()
        {
            self.paste_from_system_clipboard();
            return Ok(());
        }
        if key.code == KeyCode::F(10)
            || (ctrl && key.code == KeyCode::Char('d') && self.state.input.is_empty())
        {
            self.cancel();
            self.state.should_quit = true;
            return Ok(());
        }
        if key.code == KeyCode::Esc {
            if let Some(picker) = &mut self.state.workspace_picker {
                if picker.create_mode {
                    picker.cancel_create();
                    return Ok(());
                }
            }
            if self
                .state
                .workspace_picker
                .as_ref()
                .is_some_and(|p| p.fullscreen)
            {
                self.state.should_quit = true;
                self.state.workspace_picker = None;
                return Ok(());
            }
            if self.overlay_open() {
                self.close_overlay();
            } else if self.state.busy {
                self.cancel();
            } else {
                self.state.cancel_history();
            }
            return Ok(());
        }
        if self.state.workspace_picker.is_some() {
            self.state.provider_dialog = None;
            let mut picker = self.state.workspace_picker.take().unwrap();
            let mut chosen = None;
            match key.code {
                KeyCode::Char('n') if ctrl => picker.begin_create(),
                KeyCode::Char('p') if ctrl && !picker.create_mode => picker.toggle_projects(),
                KeyCode::Char('o') if ctrl => {
                    let current = picker.current_directory();
                    chosen = current.is_dir().then_some(current)
                }
                KeyCode::Char(ch)
                    if picker.fullscreen
                        && key.modifiers.contains(KeyModifiers::ALT)
                        && ch.is_ascii_digit() =>
                {
                    if let Some(index) = ch.to_digit(10).and_then(|n| n.checked_sub(1)) {
                        chosen = picker
                            .recent
                            .get(index as usize)
                            .map(|(path, _)| path.into());
                    }
                }
                KeyCode::Up => picker.move_up(),
                KeyCode::Char('a' | 'u') if ctrl => {
                    picker.query.clear();
                    picker.paste("");
                }
                KeyCode::Down => picker.move_down(),
                KeyCode::Backspace => picker.backspace(),
                KeyCode::Tab => picker.descend(),
                KeyCode::Right => picker.descend(),
                KeyCode::Left => picker.parent(),
                KeyCode::Enter => chosen = picker.open_selected(),
                KeyCode::Char(c) if !ctrl => picker.push_char(c),
                _ => {}
            }
            if let Some(path) = chosen {
                if let Err(error) = self.change_workspace(&path) {
                    picker.error = Some(format!("Could not open folder: {error}"));
                    self.state.workspace_picker = Some(picker);
                }
            } else {
                self.state.workspace_picker = Some(picker);
            }
            return Ok(());
        }
        if self.state.provider_dialog.is_some() {
            let mut dialog = self.state.provider_dialog.take().unwrap();
            let action = dialog.key(key);
            let store = meg_core::providers::ProviderStore {
                root: self
                    .state
                    .provider_root
                    .clone()
                    .context("Provider configuration directory is unavailable")?,
            };
            match action {
                crate::provider::ProviderAction::Use(id) => {
                    let profile = dialog
                        .profiles
                        .iter()
                        .find(|p| p.id == id)
                        .context("Provider no longer exists")?
                        .clone();
                    self.use_provider(profile)?;
                }
                crate::provider::ProviderAction::Save(profile, key) => {
                    let result = store.upsert(profile.clone(), key.as_ref());
                    match result {
                        Ok(settings) => {
                            let stored = settings
                                .profiles
                                .iter()
                                .find(|p| p.id == profile.id)
                                .cloned()
                                .context("Saved provider missing")?;
                            self.state.providers = settings.profiles;
                            self.use_provider(stored)?;
                        }
                        Err(error) => {
                            self.state.provider_dialog = Some(dialog);
                            return Err(error);
                        }
                    }
                }
                crate::provider::ProviderAction::Remove(id) => {
                    let settings = store.remove(&id)?;
                    self.state.provider_models.remove(&id);
                    self.model_queue.retain(|p| p.id != id);
                    if self.state.provider.as_ref().is_some_and(|p| p.id == id) {
                        self.state.provider = None;
                        self.state.model.clear();
                        self.state.available_models.clear();
                        self.state.online = false;
                    }
                    self.state.providers = settings.profiles.clone();
                    dialog = crate::provider::ProviderDialog::new(
                        settings.profiles,
                        self.state.provider.as_ref().map(|p| p.id.as_str()),
                    );
                    dialog.statuses = self
                        .state
                        .provider_models
                        .iter()
                        .map(|(id, e)| (id.clone(), e.label()))
                        .collect();
                    dialog.messages = self
                        .state
                        .provider_models
                        .iter()
                        .map(|(id, e)| (id.clone(), e.message.clone()))
                        .collect();
                    self.state.provider_dialog = Some(dialog);
                    self.state.notice = "Provider and its stored key removed.".into();
                }
                crate::provider::ProviderAction::Close => {}
                crate::provider::ProviderAction::None => self.state.provider_dialog = Some(dialog),
            }
            return Ok(());
        }
        if self.state.ui_picker.is_some() {
            let mut picker = self.state.ui_picker.take().unwrap();
            match key.code {
                KeyCode::Up => {
                    picker.move_up();
                    self.preview_ui_picker(&picker);
                }
                KeyCode::Down => {
                    picker.move_down();
                    self.preview_ui_picker(&picker);
                }
                KeyCode::Backspace => {
                    picker.backspace();
                    self.preview_ui_picker(&picker);
                }
                KeyCode::Enter => {
                    self.state.ui_mode_snapshot = None;
                    if let Some(action) = picker.commit_selection() {
                        self.command(&format!("/ui {action}"))?;
                        return Ok(());
                    }
                }
                KeyCode::Char(c) if !ctrl => {
                    picker.type_char(c);
                    self.preview_ui_picker(&picker);
                }
                _ => {}
            }
            self.state.ui_picker = Some(picker);
            return Ok(());
        }
        if self.state.settings_picker.is_some() {
            let mut picker = self.state.settings_picker.take().unwrap();
            match key.code {
                KeyCode::Up => picker.move_up(),
                KeyCode::Down => picker.move_down(),
                KeyCode::Backspace => picker.backspace(),
                KeyCode::Enter => {
                    if let Some(command) = picker.commit_selection() {
                        self.command(&command)?;
                        return Ok(());
                    }
                }
                KeyCode::Char(ch) if !ctrl => picker.type_char(ch),
                _ => {}
            }
            self.state.settings_picker = Some(picker);
            return Ok(());
        }
        if self.state.session_picker.is_some() {
            let mut picker = self.state.session_picker.take().unwrap();
            let mut selected = None;
            match key.code {
                KeyCode::Up => picker.move_up(),
                KeyCode::Down => picker.move_down(),
                KeyCode::Backspace => picker.backspace(),
                KeyCode::Enter => selected = picker.commit_selection(),
                KeyCode::Char(c) if !ctrl => picker.type_char(c),
                _ => {}
            }
            if let Some(record) = selected {
                anyhow::ensure!(
                    !self.state.busy,
                    "Stop the active run before resuming another session"
                );
                self.save()?;
                record.restore(&mut self.state);
                self.state.notice = "Session resumed.".into();
            } else {
                self.state.session_picker = Some(picker);
            }
            return Ok(());
        }
        if self.state.has_active_modal() {
            match key.code {
                KeyCode::Up => self.state.active_modal_move_up(),
                KeyCode::Down => self.state.active_modal_move_down(),
                KeyCode::Backspace => self.state.active_modal_backspace(),
                KeyCode::Char('r')
                    if ctrl && matches!(self.state.active_modal, ActiveModal::ModelPicker(_)) =>
                {
                    self.refresh_model_catalog()?;
                }
                KeyCode::Char('n')
                    if ctrl && matches!(self.state.active_modal, ActiveModal::ModelPicker(_)) =>
                {
                    self.state.active_modal = ActiveModal::None;
                    self.open_provider();
                }
                KeyCode::Enter => {
                    let chosen = if let ActiveModal::ModelPicker(p) = &self.state.active_modal {
                        p.selected_item().map(|i| i.value.clone())
                    } else {
                        None
                    };
                    if let Some(model) = chosen {
                        if let Some(profile) = self
                            .state
                            .provider_models
                            .get(&model.provider_id)
                            .map(|e| e.profile.clone())
                            .or_else(|| {
                                self.state
                                    .providers
                                    .iter()
                                    .find(|p| p.id == model.provider_id)
                                    .cloned()
                            })
                        {
                            if self
                                .state
                                .provider
                                .as_ref()
                                .is_none_or(|p| p.id != profile.id)
                            {
                                self.use_provider(profile)?;
                            }
                        }
                        self.state.select_model(model.id);
                        self.state.active_modal = ActiveModal::None;
                        self.state.notice = format!("Model: {}", self.state.model);
                    } else {
                        self.state.commit_active_modal();
                    }
                    self.save()?;
                }
                KeyCode::Char(c) if !ctrl => self.state.active_modal_type_char(c),
                _ => {}
            }
            return Ok(());
        }
        if self.state.palette.open {
            match key.code {
                KeyCode::Up => self.state.palette.move_up(),
                KeyCode::Down => self.state.palette.move_down(),
                KeyCode::Tab => {
                    if self.palette_from_composer {
                        let query = self.state.palette.query.clone();
                        if let Some(command) = self.state.palette.selected_command() {
                            let args = query
                                .split_once(char::is_whitespace)
                                .map(|(_, args)| args)
                                .unwrap_or("");
                            self.state.input = format!("{command} {args}");
                            self.state.move_cursor_end();
                        }
                    }
                    self.state.palette.close();
                }
                KeyCode::Enter => {
                    let query = self.state.palette.query.clone();
                    let typed = query.split_whitespace().next().unwrap_or("");
                    let command = if commands::lookup(typed).is_some()
                        || query.contains(char::is_whitespace)
                    {
                        query
                    } else {
                        self.state
                            .palette
                            .selected_command()
                            .map(str::to_owned)
                            .unwrap_or(query)
                    };
                    self.state.palette.close();
                    if self.palette_from_composer {
                        self.state.input = command;
                        self.state.move_cursor_end();
                        self.submit_input()?;
                    } else {
                        if command == "/code" {
                            self.state.input = format!("/code {}", self.state.input);
                            self.state.move_cursor_end();
                        } else {
                            let result = self.command(&command)?;
                            if let Some(goal) = result {
                                self.start(goal)?;
                            }
                        }
                    }
                }
                KeyCode::Char(c) if !ctrl => {
                    if self.palette_from_composer {
                        self.state.insert_char(c);
                        self.sync_palette();
                    } else {
                        self.state.palette.push_char(c);
                    }
                }
                KeyCode::Backspace => {
                    if self.palette_from_composer {
                        self.state.delete_backspace();
                        self.sync_palette();
                    } else {
                        self.state.palette.backspace();
                    }
                }
                KeyCode::Left if self.palette_from_composer => self.state.move_cursor_left(),
                KeyCode::Right if self.palette_from_composer => self.state.move_cursor_right(),
                _ => {}
            }
            return Ok(());
        }
        if ctrl {
            match key.code {
                KeyCode::Char('p') => {
                    self.state.palette.open();
                    self.palette_from_composer = false;
                    return Ok(());
                }
                KeyCode::Char('t') => {
                    self.state.toggle_thinking_collapse();
                    return Ok(());
                }
                KeyCode::Char('m') => {
                    anyhow::ensure!(
                        !self.state.busy,
                        "Stop the active run before changing models"
                    );
                    self.state.open_model_picker();
                    return Ok(());
                }
                KeyCode::Char('e') => {
                    anyhow::ensure!(
                        !self.state.busy,
                        "Stop the active run before changing effort"
                    );
                    self.state.open_effort_picker();
                    return Ok(());
                }
                _ => {}
            }
        }
        match key.code {
            KeyCode::Enter
                if key.modifiers.intersects(
                    KeyModifiers::SHIFT | KeyModifiers::ALT | KeyModifiers::CONTROL,
                ) =>
            {
                self.state.insert_char('\n')
            }
            KeyCode::Enter => self.submit_input()?,
            KeyCode::Char(c) if !ctrl || key.modifiers.contains(KeyModifiers::ALT) => {
                let slash = c == '/' && self.state.input.is_empty();
                self.state.insert_char(c);
                if slash {
                    self.state.palette.open();
                    self.palette_from_composer = true;
                    self.sync_palette();
                }
            }
            KeyCode::Backspace => {
                self.state.delete_backspace();
            }
            KeyCode::Delete => {
                self.state.delete_char();
            }
            KeyCode::Left => {
                if ctrl {
                    self.state.move_cursor_word_left()
                } else {
                    self.state.move_cursor_left()
                }
            }
            KeyCode::Right => {
                if ctrl {
                    self.state.move_cursor_word_right()
                } else {
                    self.state.move_cursor_right()
                }
            }
            KeyCode::Home => {
                if ctrl {
                    self.state.move_cursor_home();
                } else {
                    self.state.cursor_position = self
                        .state
                        .input
                        .chars()
                        .take(self.state.cursor_position)
                        .collect::<Vec<_>>()
                        .iter()
                        .rposition(|c| *c == '\n')
                        .map(|i| i + 1)
                        .unwrap_or(0);
                }
            }
            KeyCode::End => {
                if ctrl {
                    self.state.move_cursor_end();
                    self.state.transcript_scroll = 0;
                } else {
                    let remaining = self
                        .state
                        .input
                        .chars()
                        .skip(self.state.cursor_position)
                        .position(|c| c == '\n')
                        .unwrap_or(
                            self.state
                                .input
                                .chars()
                                .count()
                                .saturating_sub(self.state.cursor_position),
                        );
                    self.state.cursor_position += remaining;
                }
            }
            KeyCode::Up | KeyCode::Down => {
                let down = key.code == KeyCode::Down;
                if input_visual_rows(self.editor_width, &self.state.input) > 1 {
                    if let Some(pos) = vertical_cursor(
                        &self.state.input,
                        self.state.cursor_position,
                        self.editor_width,
                        down,
                    ) {
                        self.state.cursor_position = pos;
                    }
                } else if down {
                    self.state.history_down();
                } else {
                    self.state.history_up();
                }
            }
            KeyCode::PageUp => {
                self.state.transcript_scroll = self.state.transcript_scroll.saturating_add(8)
            }
            KeyCode::PageDown => {
                self.state.transcript_scroll = self.state.transcript_scroll.saturating_sub(8)
            }
            KeyCode::Tab => {
                if !self.state.busy {
                    self.state.cycle_mode();
                }
            }
            KeyCode::BackTab if !self.state.busy => {
                self.state.cycle_mode_reverse();
            }
            _ => {}
        }
        Ok(())
    }

}
