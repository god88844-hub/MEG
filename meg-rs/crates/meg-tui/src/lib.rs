//! Native MEG dashboard: command registry, durable sessions, cancellable
//! run controller, responsive transcript, live tool events and shared input layout.
//! The terminal adapter and tests exercise the same Controller and draw functions.

pub mod chips;
pub mod commands;
pub mod composer;
pub mod dashboard;
pub mod hero;
pub mod input;
pub mod job;
pub mod model_catalog;
pub mod palette;
pub mod picker;
pub mod provider;
pub mod runtime;
pub mod session;
pub mod stream;
pub mod theme;
pub mod viewport;
pub mod pro_ui;
pub mod ui_config;
pub mod workspace;

pub use chips::*;
pub use composer::*;
pub use dashboard::*;
pub use hero::*;
pub use job::*;
pub use palette::*;
pub use picker::*;
pub use runtime::{run_app, run_app_with_services, tui_restore, Controller};
pub use stream::*;
pub use theme::*;
pub use viewport::*;
pub use pro_ui::*;
pub use ui_config::*;
pub use workspace::*;

pub type ThemePalette = Palette;

use anyhow::Result;

/// One row of the "Background Tasks" sidebar panel (real tool executions).
#[derive(Debug, Clone)]
pub struct BackgroundTask {
    pub id: String,
    pub failed: bool,
    pub name: String,
    /// Measured completion, 0-100. Zero while active means indeterminate.
    pub progress: u8,
    pub secs: f32,
    pub active: bool,
}

impl BackgroundTask {
    pub fn start(name: impl Into<String>) -> Self {
        Self {
            id: String::new(),
            failed: false,
            name: name.into(),
            progress: 0,
            secs: 0.0,
            active: true,
        }
    }

    /// Advance elapsed time. Time alone cannot tell us how much work is done.
    pub fn tick(&mut self, delta: f32) {
        if self.active {
            self.secs += delta.max(0.0);
        }
    }

    pub fn finish(&mut self) {
        self.active = false;
        self.progress = 100;
    }
}

/// Live workstation state the loop feeds and user manipulates.
#[derive(Debug, Clone, Default)]
pub struct StartupOptions {
    pub resume: Option<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
}

#[derive(Debug, Clone)]
pub struct WorkstationState {
    pub extensions: serde_json::Value,
    pub provider: Option<meg_core::providers::ProviderProfile>,
    pub providers: Vec<meg_core::providers::ProviderProfile>,
    pub provider_models: std::collections::BTreeMap<String, model_catalog::ProviderModels>,
    pub provider_root: Option<std::path::PathBuf>,
    pub provider_dialog: Option<provider::ProviderDialog>,
    pub startup: StartupOptions,
    pub session_id: String,
    pub conversation: Vec<meg_core::ChatMessage>,
    pub usage: session::Usage,
    pub busy: bool,
    pub notice: String,
    pub session_picker: Option<PickerState<session::SessionRecord>>,
    pub settings_picker: Option<PickerState<String>>,
    pub ui_picker: Option<PickerState<String>>,
    pub ui_mode_snapshot: Option<ui_config::UiMode>,
    pub pasted_chunks: Vec<(String, String)>,
    pub attached_media: Vec<(String, String)>,
    pub queued_count: usize,
    pub project: String,
    pub model: String,
    pub state_label: String,
    pub context_percent: u8,
    pub clock: String,
    pub transcript: Vec<String>,
    /// Visual rows above the live tail; zero follows new output.
    pub transcript_scroll: u16,
    pub plan_steps: Vec<(bool, String)>,
    pub status: Vec<(String, String)>,
    pub input: String,
    pub cursor_position: usize,
    pub history: Vec<String>,
    pub history_index: Option<usize>,
    pub input_draft: String,
    pub mode: Mode,
    pub effort: String,
    pub active_modal: ActiveModal,
    pub theme: Palette,
    pub theme_snapshot: Option<Palette>,
    pub stream: Option<StreamState>,
    pub active_job: Option<JobState>,
    /// Floating `/` command palette (Cline-style popup).
    pub palette: CommandPalette,
    /// Selected endpoint's last known reachability.
    pub online: bool,
    /// Absolute workspace path shown in the header + statusbar.
    pub cwd: String,
    /// Git branch of the workspace ("" when not a repo).
    pub git_branch: String,
    /// Real tool executions in flight, for the Background Tasks panel.
    pub background_tasks: Vec<BackgroundTask>,
    /// Agent rows for the Subagents panel: (name, state).
    pub subagents: Vec<(String, String)>,
    /// The freebuff-style startup workspace selector (Some = showing).
    pub workspace_picker: Option<WorkspacePicker>,
    /// The live brain roster (fetched from the shim's /v1/models).
    pub available_models: Vec<String>,
    pub ui_mode: ui_config::UiMode,
    pub ui_config: ui_config::UserUiConfig,
    pub pair_mode: Option<String>,
    pub executor_model: Option<String>,
    pub reviewer_model: Option<String>,
    pub collapse_tool_cards: bool,
    pub should_quit: bool,
}

impl Default for WorkstationState {
    fn default() -> Self {
        Self {
            provider: None,
            providers: Vec::new(),
            provider_models: Default::default(),
            provider_root: None,
            provider_dialog: None,
            extensions: serde_json::json!({}),
            startup: StartupOptions::default(),
            session_id: uuid::Uuid::new_v4().to_string(),
            conversation: Vec::new(),
            usage: session::Usage::default(),
            busy: false,
            notice: String::new(),
            session_picker: None,
            settings_picker: None,
            ui_picker: None,
            ui_mode_snapshot: None,
            pasted_chunks: Vec::new(),
            attached_media: Vec::new(),
            queued_count: 0,
            project: String::new(),
            model: meg_core::shim_client::default_model(),
            state_label: "\u{25cf} READY".into(),
            context_percent: 0,
            clock: String::new(),
            transcript: Vec::new(),
            transcript_scroll: 0,
            plan_steps: Vec::new(),
            status: Vec::new(),
            input: String::new(),
            cursor_position: 0,
            history: Vec::new(),
            history_index: None,
            input_draft: String::new(),
            mode: Mode::Build,
            effort: "high".into(),
            active_modal: ActiveModal::None,
            theme: DARK_SLATE,
            theme_snapshot: None,
            stream: None,
            active_job: None,
            palette: CommandPalette::default(),
            online: false,
            cwd: String::new(),
            git_branch: String::new(),
            background_tasks: Vec::new(),
            subagents: Vec::new(),
            workspace_picker: None,
            available_models: Vec::new(),
            ui_mode: ui_config::UiMode::Classic,
            ui_config: ui_config::UserUiConfig::default(),
            pair_mode: None,
            executor_model: None,
            reviewer_model: None,
            collapse_tool_cards: true,
            should_quit: false,
        }
    }
}

/// Use ordinary drive/UNC paths in the UI and as the shell working directory.
pub fn friendly_path(path: &std::path::Path) -> String {
    let path = path.to_string_lossy();
    if let Some(rest) = path.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else {
        path.strip_prefix(r"\\?\").unwrap_or(&path).to_string()
    }
}

impl WorkstationState {
    /// Preserve reasoning and partial output at their actual place in the timeline.
    pub fn archive_stream(&mut self) {
        if let Some(stream) = &mut self.stream {
            if !stream.thinking_buffer.is_empty() {
                self.transcript.push(format!(
                    "[thinking] {}",
                    std::mem::take(&mut stream.thinking_buffer)
                ));
            }
            if !stream.content_buffer.is_empty() {
                self.transcript.push(format!(
                    "[MEG] {}",
                    std::mem::take(&mut stream.content_buffer)
                ));
            }
        }
    }

    pub fn finish_response(&mut self, answer: Option<String>) {
        self.state_label = "READY".into();
        if let Some(stream) = &mut self.stream {
            stream.is_complete = true;
            if !stream.user_pinned_thinking {
                stream.is_thinking_collapsed = true;
            }
            if answer.is_some() {
                stream.content_buffer.clear();
            }
        }
        self.archive_stream();
        self.active_job = None;
        if let Some(answer) = answer {
            self.transcript.push(format!("✓ {answer}"));
        }
    }

    pub fn select_model(&mut self, model: String) {
        self.model = model;
        for effort in ["low", "medium", "high", "max", "xhigh"] {
            if self.model.ends_with(&format!("-{effort}")) {
                self.effort = if effort == "xhigh" { "max" } else { effort }.into();
                break;
            }
        }
    }

    /// Some shim rosters encode effort in the model ID. Only use advertised IDs.
    pub fn select_effort(&mut self, effort: String) {
        let base = ["-low", "-medium", "-high", "-max", "-xhigh"]
            .iter()
            .find_map(|suffix| self.model.strip_suffix(suffix))
            .unwrap_or(&self.model);
        let variants = if effort == "max" {
            vec![format!("{base}-max"), format!("{base}-xhigh")]
        } else {
            vec![format!("{base}-{effort}")]
        };
        if let Some(model) = variants
            .into_iter()
            .find(|model| self.available_models.contains(model))
        {
            self.model = model;
        }
        self.effort = effort;
    }

    /// Toggle thinking block collapse in the active stream if present.
    pub fn toggle_thinking_collapse(&mut self) {
        self.stream
            .get_or_insert_with(|| StreamState {
                is_complete: true,
                is_thinking_collapsed: true,
                ..Default::default()
            })
            .toggle_thinking_collapse();
    }

    /// Alias for toggle_thinking_collapse.
    pub fn toggle_thinking_collapsed(&mut self) {
        self.toggle_thinking_collapse();
    }

    /// Cancel the active background job if running.
    pub fn cancel_active_job(&mut self) {
        if let Some(ref mut j) = self.active_job {
            if j.is_active {
                j.cancel();
            }
        }
    }
    // ------------------------------------------------------------------------
    // 1. Character Insertion & Deletion (UTF-8 Safe Char-Indexed)
    // ------------------------------------------------------------------------

    /// Insert a character at the current char-based cursor position.
    pub fn insert_char(&mut self, c: char) {
        let char_count = self.input.chars().count();
        let pos = self.cursor_position.min(char_count);
        let byte_idx = self
            .input
            .char_indices()
            .nth(pos)
            .map(|(i, _)| i)
            .unwrap_or(self.input.len());
        self.input.insert(byte_idx, c);
        self.cursor_position = pos + 1;
    }

    /// Insert a string slice at the cursor position.
    pub fn insert_str(&mut self, s: &str) {
        let char_count = self.input.chars().count();
        let pos = self.cursor_position.min(char_count);
        let byte_idx = self
            .input
            .char_indices()
            .nth(pos)
            .map(|(i, _)| i)
            .unwrap_or(self.input.len());
        self.input.insert_str(byte_idx, s);
        self.cursor_position = pos + s.chars().count();
    }

    /// Backspace: delete character before cursor. Returns false if already at start.
    pub fn delete_backspace(&mut self) -> bool {
        let end = self
            .input
            .char_indices()
            .nth(self.cursor_position)
            .map(|(i, _)| i)
            .unwrap_or(self.input.len());
        if end == 0 {
            return false;
        }
        self.move_cursor_left();
        let start = self
            .input
            .char_indices()
            .nth(self.cursor_position)
            .map(|(i, _)| i)
            .unwrap_or(0);
        self.input.replace_range(start..end, "");
        true
    }
    pub fn delete_char(&mut self) -> bool {
        let start = self
            .input
            .char_indices()
            .nth(self.cursor_position)
            .map(|(i, _)| i)
            .unwrap_or(self.input.len());
        if start == self.input.len() {
            return false;
        }
        let old = self.cursor_position;
        self.move_cursor_right();
        let end = self
            .input
            .char_indices()
            .nth(self.cursor_position)
            .map(|(i, _)| i)
            .unwrap_or(self.input.len());
        self.input.replace_range(start..end, "");
        self.cursor_position = old;
        true
    }
    pub fn move_cursor_left(&mut self) {
        use unicode_segmentation::UnicodeSegmentation;
        let byte = self
            .input
            .char_indices()
            .nth(self.cursor_position)
            .map(|(i, _)| i)
            .unwrap_or(self.input.len());
        let previous = self
            .input
            .grapheme_indices(true)
            .map(|(i, _)| i)
            .take_while(|i| *i < byte)
            .last()
            .unwrap_or(0);
        self.cursor_position = self.input[..previous].chars().count();
    }
    pub fn move_cursor_right(&mut self) {
        use unicode_segmentation::UnicodeSegmentation;
        let byte = self
            .input
            .char_indices()
            .nth(self.cursor_position)
            .map(|(i, _)| i)
            .unwrap_or(self.input.len());
        let next = self
            .input
            .grapheme_indices(true)
            .map(|(i, g)| i + g.len())
            .find(|i| *i > byte)
            .unwrap_or(self.input.len());
        self.cursor_position = self.input[..next].chars().count();
    }
    pub fn move_cursor_home(&mut self) {
        self.cursor_position = 0;
    }

    pub fn move_cursor_end(&mut self) {
        self.cursor_position = self.input.chars().count();
    }

    pub fn move_cursor_word_left(&mut self) {
        let chars: Vec<char> = self.input.chars().collect();
        let mut pos = self.cursor_position.min(chars.len());
        while pos > 0 && chars[pos - 1].is_whitespace() {
            pos -= 1;
        }
        while pos > 0 && !chars[pos - 1].is_whitespace() {
            pos -= 1;
        }
        self.cursor_position = pos;
    }

    pub fn move_cursor_word_right(&mut self) {
        let chars: Vec<char> = self.input.chars().collect();
        let len = chars.len();
        let mut pos = self.cursor_position.min(len);
        while pos < len && !chars[pos].is_whitespace() {
            pos += 1;
        }
        while pos < len && chars[pos].is_whitespace() {
            pos += 1;
        }
        self.cursor_position = pos;
    }

    // ------------------------------------------------------------------------
    // 3. History Buffer Navigation (Preserving In-Progress Draft)
    // ------------------------------------------------------------------------

    pub fn history_up(&mut self) {
        if self.history.is_empty() {
            return;
        }
        if self.history_index.is_none() {
            // Save in-progress draft before entering history browsing
            self.input_draft = self.input.clone();
            let last_idx = self.history.len() - 1;
            self.history_index = Some(last_idx);
            self.input = self.history[last_idx].clone();
            self.cursor_position = self.input.chars().count();
        } else if let Some(idx) = self.history_index {
            if idx > 0 {
                let new_idx = idx - 1;
                self.history_index = Some(new_idx);
                self.input = self.history[new_idx].clone();
                self.cursor_position = self.input.chars().count();
            }
        }
    }

    pub fn history_down(&mut self) {
        if let Some(idx) = self.history_index {
            if idx + 1 < self.history.len() {
                let new_idx = idx + 1;
                self.history_index = Some(new_idx);
                self.input = self.history[new_idx].clone();
                self.cursor_position = self.input.chars().count();
            } else {
                // Reached the end: restore the in-progress draft
                self.history_index = None;
                self.input = std::mem::take(&mut self.input_draft);
                self.cursor_position = self.input.chars().count();
            }
        }
    }

    pub fn cancel_history(&mut self) {
        if self.history_index.is_some() {
            self.history_index = None;
            self.input = std::mem::take(&mut self.input_draft);
            self.cursor_position = self.input.chars().count();
        }
    }

    // ------------------------------------------------------------------------
    // 4. Mode Cycling
    // ------------------------------------------------------------------------

    pub fn cycle_mode(&mut self) {
        self.mode = self.mode.next();
    }

    pub fn cycle_mode_reverse(&mut self) {
        self.mode = self.mode.prev();
    }

    // ------------------------------------------------------------------------
    // 5. Input Submission
    // ------------------------------------------------------------------------

    pub fn commit_input(&mut self) -> Option<String> {
        let line = self.input.trim().to_string();
        if line.is_empty() {
            return None;
        }
        // Deduplicate consecutive identical history entries
        if self.history.last().map(|s| s.as_str()) != Some(&line) {
            self.history.push(line.clone());
        }
        self.cursor_position = 0;
        self.history_index = None;
        self.input_draft.clear();
        let submitted = std::mem::take(&mut self.input);
        Some(submitted)
    }

    // Compatibility aliases
    pub fn cursor_pos(&self) -> usize {
        self.cursor_position
    }

    pub fn type_char(&mut self, c: char) {
        self.insert_char(c);
    }

    pub fn type_str(&mut self, s: &str) {
        self.insert_str(s);
    }

    pub fn backspace(&mut self) {
        self.delete_backspace();
    }

    pub fn delete(&mut self) {
        self.delete_char();
    }

    pub fn move_left(&mut self) {
        self.move_cursor_left();
    }

    pub fn move_right(&mut self) {
        self.move_cursor_right();
    }

    pub fn move_home(&mut self) {
        self.move_cursor_home();
    }

    pub fn move_end(&mut self) {
        self.move_cursor_end();
    }

    pub fn submit(&mut self) -> Option<String> {
        self.commit_input()
    }

    // ------------------------------------------------------------------------
    // 6. Modal Picker State & Lifecycle Delegation
    // ------------------------------------------------------------------------

    /// Returns true if an interactive modal overlay is currently active.
    pub fn has_active_modal(&self) -> bool {
        self.active_modal.is_open()
    }

    /// Opens the Theme Picker modal, taking a snapshot of the active theme for rollback.
    pub fn open_theme_picker(&mut self) {
        self.theme_snapshot = Some(self.theme);
        self.active_modal = ActiveModal::ThemePicker(default_theme_picker(&self.theme));
    }

    /// Opens the Model Picker modal initialized to the current model.
    /// Items come from the selected endpoint's model discovery.
    pub fn open_model_picker(&mut self) {
        let mut next = crate::model_catalog::picker(self);
        if let ActiveModal::ModelPicker(previous) = &self.active_modal {
            let selected = previous.selected_item().map(|item| item.value.clone());
            next.search_query = previous.search_query.clone();
            next.filter();
            if let Some(selected) = selected {
                if let Some(index) = next.filtered_indices.iter().position(|&i| {
                    let value = &next.items[i].value;
                    value.id == selected.id && value.provider_id == selected.provider_id
                }) {
                    next.selected_index = index;
                    next.adjust_scroll();
                }
            }
        }
        self.active_modal = ActiveModal::ModelPicker(next);
    }

    /// Opens the Reasoning Effort Picker modal initialized to the current effort level.
    pub fn open_effort_picker(&mut self) {
        self.active_modal = ActiveModal::EffortPicker(default_effort_picker(&self.effort));
    }

    /// Cancels the active modal overlay, rolling back theme preview if applicable.
    pub fn cancel_active_modal(&mut self) {
        if let Some(snapshot) = self.theme_snapshot.take() {
            self.theme = snapshot;
        }
        match &mut self.active_modal {
            ActiveModal::ThemePicker(p) => p.cancel(),
            ActiveModal::ModelPicker(p) => p.cancel(),
            ActiveModal::EffortPicker(p) => p.cancel(),
            ActiveModal::None => {}
        }
        self.active_modal = ActiveModal::None;
    }

    /// Commits the active selection in the modal, updating state and closing the modal.
    pub fn commit_active_modal(&mut self) {
        match &mut self.active_modal {
            ActiveModal::ThemePicker(p) => {
                if let Some(pal) = p.commit_selection() {
                    self.theme = pal;
                    self.ui_config.theme_name = Some(pal.name.to_string());
                    self.transcript
                        .push(format!("[system] Theme switched to {}", pal.name));
                }
                self.theme_snapshot = None;
            }
            ActiveModal::ModelPicker(p) => {
                if let Some(m) = p.commit_selection() {
                    self.select_model(m.id);
                    self.transcript
                        .push(format!("[system] Model switched to {}", self.model));
                }
            }
            ActiveModal::EffortPicker(p) => {
                if let Some(eff) = p.commit_selection() {
                    self.select_effort(eff.as_str().to_string());
                    self.transcript
                        .push(format!("[system] Reasoning effort set to {}", self.effort));
                }
            }
            ActiveModal::None => {}
        }
        self.active_modal = ActiveModal::None;
    }

    /// Moves the active modal selection cursor up (with live preview for theme picker).
    pub fn active_modal_move_up(&mut self) {
        match &mut self.active_modal {
            ActiveModal::ThemePicker(p) => {
                p.move_up();
                if let Some(it) = p.selected_item() {
                    self.theme = it.value;
                }
            }
            ActiveModal::ModelPicker(p) => p.move_up(),
            ActiveModal::EffortPicker(p) => p.move_up(),
            ActiveModal::None => {}
        }
    }

    /// Moves the active modal selection cursor down (with live preview for theme picker).
    pub fn active_modal_move_down(&mut self) {
        match &mut self.active_modal {
            ActiveModal::ThemePicker(p) => {
                p.move_down();
                if let Some(it) = p.selected_item() {
                    self.theme = it.value;
                }
            }
            ActiveModal::ModelPicker(p) => p.move_down(),
            ActiveModal::EffortPicker(p) => p.move_down(),
            ActiveModal::None => {}
        }
    }

    /// Types a character into the modal search filter (updating live preview for theme picker).
    pub fn active_modal_type_char(&mut self, c: char) {
        match &mut self.active_modal {
            ActiveModal::ThemePicker(p) => {
                p.type_char(c);
                if let Some(it) = p.selected_item() {
                    self.theme = it.value;
                }
            }
            ActiveModal::ModelPicker(p) => p.type_char(c),
            ActiveModal::EffortPicker(p) => p.type_char(c),
            ActiveModal::None => {}
        }
    }

    /// Backspaces in the modal search filter (updating live preview for theme picker).
    pub fn active_modal_backspace(&mut self) {
        match &mut self.active_modal {
            ActiveModal::ThemePicker(p) => {
                p.backspace();
                if let Some(it) = p.selected_item() {
                    self.theme = it.value;
                }
            }
            ActiveModal::ModelPicker(p) => p.backspace(),
            ActiveModal::EffortPicker(p) => p.backspace(),
            ActiveModal::None => {}
        }
    }
}

/// Actual native package version.
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Clone)]
pub struct RunRequest {
    pub provider: Option<meg_core::providers::ProviderProfile>,
    pub goal: String,
    pub workspace: std::path::PathBuf,
    pub model: String,
    pub base_url: String,
    pub history: Vec<meg_core::ChatMessage>,
    pub mode: Mode,
    pub effort: String,
    pub cancel: meg_core::CancellationToken,
    pub pair_mode: Option<String>,
    pub executor_model: Option<String>,
    pub reviewer_model: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ServiceRequest {
    pub method: String,
    pub workspace: std::path::PathBuf,
    pub params: serde_json::Value,
}
pub type ServiceRunner = dyn Fn(
        ServiceRequest,
        meg_core::CancellationToken,
        std::sync::mpsc::Sender<RunnerMsg>,
    ) -> Result<serde_json::Value>
    + Send
    + Sync;

impl WorkstationState {
    pub fn run_request(&self, goal: String) -> RunRequest {
        RunRequest {
            provider: self.provider.clone(),
            goal,
            workspace: std::path::PathBuf::from(&self.cwd),
            model: self.model.clone(),
            base_url: self
                .provider
                .as_ref()
                .map(|p| p.base_url.clone())
                .unwrap_or_else(meg_core::shim_client::default_base_url),
            history: self.conversation.clone(),
            mode: self.mode,
            effort: self.effort.clone(),
            cancel: meg_core::CancellationToken::default(),
            pair_mode: self.pair_mode.clone(),
            executor_model: self.executor_model.clone(),
            reviewer_model: self.reviewer_model.clone(),
        }
    }
}

pub type GoalRunner =
    dyn Fn(RunRequest, std::sync::mpsc::Sender<RunnerMsg>) -> Result<Option<String>> + Send + Sync;

#[derive(Debug, Clone)]
pub enum RunnerMsg {
    DelegatedUsage(serde_json::Value),
    ServiceResult {
        method: String,
        result: serde_json::Value,
    },
    Usage {
        input: u64,
        output: u64,
        cached: u64,
    },
    ToolStarted {
        id: String,
        name: String,
        args: String,
    },
    ToolFinished {
        id: String,
        ok: bool,
        output: String,
    },
    StreamStart,
    Line(String),
    ThinkingChunk(String),
    ContentChunk(String),
    StreamMetrics {
        tok_per_sec: f32,
        elapsed_secs: f32,
        token_count: usize,
    },
    JobStart {
        id: String,
        description: String,
    },
    JobProgress {
        id: String,
        status: String,
    },
    JobDone {
        id: String,
    },
    PlanStep(bool, String),
    /// Replace the Todo checklist with this list.
    TodoList(Vec<(bool, String)>),
    /// Mark todo item `idx` done/undone.
    TodoStep(usize, bool),
    /// A background task (tool execution) started.
    BackgroundStart(String),
    /// The background task finished.
    BackgroundDone(String),
    /// A subagent row changed state (idle/running/done).
    SubagentState(String, String),
    /// Brain liveness probe result.
    Online(bool),
    /// The brain's model roster.
    ModelList(Vec<String>),
    Done(Option<String>),
}

pub const RESET_SEQUENCE: &str = "\x1b[?1049l\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1006l\x1b[?1004l\x1b[?2004l\x1b[<u\x1b[>4;0m\x1b[?25h";

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{
        backend::TestBackend,
        layout::{Position, Rect},
        Terminal,
    };

    #[test]
    fn palettes_differ_only_in_color() {
        let a = format!("{:?}", CLASSIC);
        let b = format!("{:?}", PHOSPHOR);
        assert_ne!(a, b);
    }

    #[test]
    fn state_defaults() {
        let st = WorkstationState::default();
        assert!(st.transcript.is_empty());
        assert_eq!(st.context_percent, 0);
        assert_eq!(st.mode, Mode::Build);
        assert_eq!(st.cursor_position, 0);
        assert!(st.history.is_empty());
    }

    #[test]
    fn test_cursor_navigation_and_editing() {
        let mut st = WorkstationState::default();
        st.insert_str("hello world");
        assert_eq!(st.cursor_position, 11);

        st.move_cursor_home();
        assert_eq!(st.cursor_position, 0);
        st.insert_char('>');
        assert_eq!(st.input, ">hello world");
        assert_eq!(st.cursor_position, 1);

        st.delete_char(); // deletes 'h'
        assert_eq!(st.input, ">ello world");

        st.delete_backspace(); // deletes '>'
        assert_eq!(st.input, "ello world");
        assert_eq!(st.cursor_position, 0);

        st.move_cursor_end();
        assert_eq!(st.cursor_position, 10);
        st.move_cursor_word_left();
        assert_eq!(st.cursor_position, 5); // start of "world"
        st.move_cursor_word_right();
        assert_eq!(st.cursor_position, 10);
    }

    #[test]
    fn test_history_draft_preservation() {
        let mut st = WorkstationState::default();
        st.history.push("cargo test".into());
        st.history.push("git status".into());

        st.insert_str("my uncommitted draft");

        // Up recalls "git status"
        st.history_up();
        assert_eq!(st.input, "git status");
        assert_eq!(st.history_index, Some(1));

        // Up recalls "cargo test"
        st.history_up();
        assert_eq!(st.input, "cargo test");
        assert_eq!(st.history_index, Some(0));

        // Down recalls "git status"
        st.history_down();
        assert_eq!(st.input, "git status");
        assert_eq!(st.history_index, Some(1));

        // Down restores original uncommitted draft!
        st.history_down();
        assert_eq!(st.input, "my uncommitted draft");
        assert_eq!(st.history_index, None);
    }

    #[test]
    fn test_mode_cycling() {
        // 2026-11-20: Plan and Build are the only modes (Act removed).
        let mut st = WorkstationState::default();
        assert_eq!(st.mode, Mode::Build);
        st.cycle_mode();
        assert_eq!(st.mode, Mode::Plan);
        st.cycle_mode();
        assert_eq!(st.mode, Mode::Build);
        st.cycle_mode_reverse();
        assert_eq!(st.mode, Mode::Plan);
    }

    #[test]
    fn test_multibyte_utf8_cursor_safety() {
        let mut st = WorkstationState::default();
        st.insert_str("café 🚀 中文");
        assert_eq!(st.cursor_position, 9); // 9 characters
        st.move_cursor_left(); // before '文'
        assert_eq!(st.cursor_position, 8);
        st.insert_char('!');
        assert_eq!(st.input, "café 🚀 中!文");
        assert_eq!(st.cursor_position, 9);
        st.delete_backspace(); // deletes '!'
        assert_eq!(st.input, "café 🚀 中文");
        assert_eq!(st.cursor_position, 8);
    }

    #[test]
    fn test_composer_height_1_does_not_crush_borders() {
        let backend = TestBackend::new(80, 1);
        let mut terminal = Terminal::new(backend).unwrap();
        let st = WorkstationState {
            input: "test input".into(),
            ..Default::default()
        };
        terminal
            .draw(|f| {
                render_composer(f, f.area(), &st, &CLASSIC);
            })
            .unwrap();
        let buf = terminal.backend().buffer();
        let row_str: String = (0..80).map(|x| buf[(x, 0)].symbol()).collect();
        assert!(
            row_str.contains("test input"),
            "Input must be visible in height-1 area, got: {row_str}"
        );
        assert!(
            !row_str.contains('╰'),
            "Must not render crushed bottom border in height 1"
        );
    }

    #[test]
    fn test_hero_renders_gradient_banner_at_48_cols() {
        let backend = TestBackend::new(48, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        let st = WorkstationState::default();
        terminal
            .draw(|f| {
                render_hero(f, f.area(), &st, &CLASSIC);
            })
            .unwrap();
        let buf = terminal.backend().buffer();
        let mut full = String::new();
        for y in 0..10u16 {
            for x in 0..48u16 {
                full.push_str(buf[(x, y)].symbol());
            }
        }
        // The minimax banner (43 cols) fits at 48: blocks, not letters.
        // (Row 0 is the top spacer — scan the whole frame.)
        assert!(
            full.contains('\u{2588}'),
            "Banner blocks must render when the wordmark fits"
        );
        assert!(!full.contains('╭'), "Borders must not render on width < 52");
    }

    #[test]
    fn test_composer_cursor_positioning() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut st = WorkstationState::default();
        st.insert_str("test");

        terminal
            .draw(|f| {
                let area = Rect::new(0, 18, 80, 4);
                render_composer(f, area, &st, &CLASSIC);
            })
            .unwrap();

        // Hardware cursor: area.x(0) + left_border(1) + rail(2) + "test"(4) = 7, y = 18 + top_border(1) = 19
        terminal
            .backend_mut()
            .assert_cursor_position(Position::new(7, 19));
    }

    #[test]
    fn test_modal_lifecycle_theme_picker_live_preview_and_cancel() {
        let mut st = WorkstationState::default();
        assert!(!st.has_active_modal());
        assert_eq!(st.theme.name, "Dark Slate");

        st.open_theme_picker();
        assert!(st.has_active_modal());
        assert_eq!(st.theme_snapshot.unwrap().name, "Dark Slate");

        // Move cursor down -> preview changes live
        st.active_modal_move_down();
        assert_ne!(st.theme.name, "Dark Slate");

        // Cancel -> reverts to snapshot
        st.cancel_active_modal();
        assert!(!st.has_active_modal());
        assert_eq!(st.theme.name, "Dark Slate");
    }

    #[test]
    fn test_modal_lifecycle_model_commit() {
        let mut st = WorkstationState::default();
        st.available_models = vec![
            "gemini-3.8-flash-medium".into(),
            "gemini-3.8-flash-high".into(),
        ];
        st.open_model_picker();
        assert!(st.has_active_modal());

        st.active_modal_move_down();
        st.commit_active_modal();
        assert!(!st.has_active_modal());
        // 2026-11-20: the picker lists the shim roster, not Ollama models.
        assert_eq!(st.model, "gemini-3.8-flash-high");
    }

    #[test]
    fn test_modal_lifecycle_effort_commit() {
        let mut st = WorkstationState::default();
        assert_eq!(st.effort, "high");
        st.open_effort_picker();
        assert!(st.has_active_modal());

        // Move down from high (index 2) to max (index 3)
        st.active_modal_move_down();
        st.commit_active_modal();
        assert!(!st.has_active_modal());
        assert_eq!(st.effort, "max");
    }

    #[test]
    fn test_modal_isolation_from_composer_buffer() {
        let mut st = WorkstationState::default();
        st.insert_str("draft in composer");
        let initial_cursor = st.cursor_position;

        st.open_model_picker();
        // Typing characters while modal is open updates modal search, NOT composer
        st.active_modal_type_char('q');
        st.active_modal_type_char('w');
        st.active_modal_type_char('e');

        assert_eq!(st.input, "draft in composer");
        assert_eq!(st.cursor_position, initial_cursor);

        st.cancel_active_modal();
        assert_eq!(st.input, "draft in composer");
    }

    #[test]
    fn test_draw_with_active_modal_renders_overlay() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut st = WorkstationState::default();
        st.open_theme_picker();

        terminal
            .draw(|f| {
                draw(f, &st.theme, &st);
            })
            .unwrap();

        let buf = terminal.backend().buffer();
        let mut found_modal_title = false;
        for y in 0..24 {
            let row: String = (0..80).map(|x| buf[(x, y)].symbol()).collect();
            if row.contains("Switch Theme") {
                found_modal_title = true;
                break;
            }
        }
        assert!(found_modal_title, "Modal overlay must be rendered on top");
    }
}

#[cfg(test)]
mod visual_dump {
    use super::*;
    use ratatui::{backend::TestBackend, Terminal};

    #[test]
    fn dump_workstation_frame() {
        let mut st = WorkstationState::default();
        st.project = "MEG".into();
        st.cwd = r"C:\projects\MEG".into();
        st.git_branch = "main".into();
        st.model = "gemini-3.8-flash-medium".into();
        st.online = true;
        st.clock = "03:20".into();
        st.transcript.push("\u{276f} hi".into());
        st.transcript
            .push("\u{2713} Hello! How can I help you today?".into());
        st.plan_steps = vec![
            (true, "Understand the goal".into()),
            (true, "Locate context in the workspace".into()),
            (false, "Execute with tools".into()),
            (false, "Verify the result".into()),
        ];
        let mut t1 = BackgroundTask::start("index_workspace");
        t1.progress = 78;
        let mut t2 = BackgroundTask::start("run_tests");
        t2.progress = 36;
        st.background_tasks = vec![t1, t2];
        st.subagents = vec![("main".into(), "running".into())];

        let backend = TestBackend::new(190, 46);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, &st.theme, &st)).unwrap();
        let buf = terminal.backend().buffer();
        let area = buf.area();
        let mut out = String::new();
        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                out.push_str(buf[(x, y)].symbol());
            }
            out.push('\n');
        }
        println!("=== WORKSTATION FRAME ===\n{out}");
    }
}

#[cfg(test)]
mod visual_dump_picker {
    use super::*;
    use ratatui::{backend::TestBackend, Terminal};

    #[test]
    fn dump_workspace_picker() {
        let mut st = WorkstationState::default();
        st.theme = DASHBOARD;
        let root = std::env::temp_dir().join(format!("meg-picker-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        st.cwd = root.to_string_lossy().into_owned();
        st.workspace_picker = Some(WorkspacePicker::open(root.clone()));
        let backend = TestBackend::new(190, 40);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, &st.theme, &st)).unwrap();
        let buf = terminal.backend().buffer();
        let area = buf.area();
        let mut out = String::new();
        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                out.push_str(buf[(x, y)].symbol());
            }
            out.push('\n');
        }
        println!("=== PICKER FRAME ===\n{out}");
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(test)]
mod logo_test {
    #[test]
    fn compare_logo_candidates() {
        let candidates: Vec<(&str, [&str; 2])> = vec![
            ("A quadrant", [
                "\u{2593}\u{2580}\u{2593} \u{2593}\u{2580}\u{2580} \u{2593}\u{2580}\u{2580}\u{2593}",
                "\u{2593} \u{2593} \u{2593}\u{2584}\u{2584} \u{2593}\u{2584}\u{2584}\u{2593}",
            ]),
            ("B quadrant-3q", [
                "\u{259b}\u{2580}\u{259c} \u{259b}\u{2580}\u{2580} \u{259b}\u{2580}\u{2580}\u{259d}",
                "\u{258c} \u{2590} \u{2599}\u{2584}\u{2584} \u{2599}\u{2584}\u{2584}\u{259f}",
            ]),
            ("C half-blocks", [
                "\u{258c}\u{2580}\u{2590} \u{258c}\u{2580}\u{2590} \u{258c}\u{2580}\u{2580}\u{2590}",
                "\u{258c}\u{2584}\u{2590} \u{258c}\u{2584}\u{2584} \u{258c}\u{2584}\u{2584}\u{2590}",
            ]),
            ("D solid-2col", [
                "\u{2593}\u{2593} \u{2593}\u{2593}\u{2593} \u{2593}\u{2593}\u{2593}\u{2593}",
                "\u{2593}\u{2580}\u{2593} \u{2593}\u{2580}\u{2593} \u{2593}\u{2580}\u{2593}\u{2593}",
            ]),
        ];
        for (name, rows) in candidates {
            println!("--- {name} ---");
            for r in rows {
                println!("  {r}");
            }
        }
    }
}
