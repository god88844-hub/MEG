//! Production event controller shared by the terminal loop and integration tests.

mod events;
mod keys;
mod media;

use crate::session::{SessionStore, Usage};
use crate::*;
use anyhow::{Context, Result};
use std::{
    sync::{
        mpsc::{self, Receiver},
        Arc,
    },
    time::{Duration, Instant},
};

struct ActiveRun {
    rx: Receiver<RunnerMsg>,
    cancel: meg_core::CancellationToken,
    thread: std::thread::JoinHandle<()>,
}
pub struct Controller {
    notice_deadline: Option<Instant>,
    ctrl_c_deadline: Option<Instant>,
    service_runner: Option<Arc<ServiceRunner>>,
    service_tx: mpsc::Sender<RunnerMsg>,
    service_rx: Receiver<RunnerMsg>,
    service_threads: Vec<std::thread::JoinHandle<()>>,
    service_tokens: Vec<meg_core::CancellationToken>,
    pub state: WorkstationState,
    pub(crate) store: Option<SessionStore>,
    runner: Option<Arc<GoalRunner>>,
    active: Option<ActiveRun>,
    pub(crate) queue: std::collections::VecDeque<(String, String)>,
    palette_from_composer: bool,
    editor_width: u16,
    model_queue: std::collections::VecDeque<meg_core::providers::ProviderProfile>,
}
impl Controller {
    pub fn in_memory(state: WorkstationState, runner: Option<Arc<GoalRunner>>) -> Self {
        let (service_tx, service_rx) = mpsc::channel();
        Self {
            notice_deadline: None,
            ctrl_c_deadline: None,
            service_runner: None,
            service_tx,
            service_rx,
            service_threads: Vec::new(),
            service_tokens: Vec::new(),
            state,
            store: None,
            runner,
            active: None,
            queue: Default::default(),
            palette_from_composer: false,
            editor_width: 80,
            model_queue: Default::default(),
        }
    }


    pub fn with_services(mut self, services: Option<Arc<ServiceRunner>>) -> Self {
        self.service_runner = services;
        self
    }


    pub(crate) fn invoke_service(&mut self, method: &str, params: serde_json::Value) -> Result<()> {
        self.service_threads.retain_mut(|t| !t.is_finished());
        if self.service_threads.is_empty() {
            self.service_tokens.clear();
        }
        anyhow::ensure!(
            self.service_threads.len() < 16,
            "Too many operations are active; wait or use /stop"
        );
        let runner = self
            .service_runner
            .as_ref()
            .context("This command needs the installed MEG control plane")?
            .clone();
        let method = method.to_string();
        let workspace = self.state.cwd.clone();
        let profile_id = params["id"].clone();
        let base_url = params["base_url"].clone();
        let discovery_profile = (method == "provider.models")
            .then(|| {
                serde_json::from_value::<meg_core::providers::ProviderProfile>(params.clone()).ok()
            })
            .flatten();
        let request = ServiceRequest {
            method: method.clone(),
            workspace: workspace.clone().into(),
            params,
        };
        let cancel = meg_core::CancellationToken::default();
        self.service_tokens.push(cancel.clone());
        let tx = self.service_tx.clone();
        let id = format!("service-{}", uuid::Uuid::new_v4());
        if let Some(profile) = discovery_profile {
            let entry = self
                .state
                .provider_models
                .entry(profile.id.clone())
                .or_insert_with(|| crate::model_catalog::ProviderModels {
                    profile: profile.clone(),
                    models: Vec::new(),
                    status: crate::model_catalog::DiscoveryStatus::Checking,
                    message: String::new(),
                    request_id: String::new(),
                });
            if entry.profile.base_url != profile.base_url {
                entry.models.clear();
            }
            entry.profile = profile;
            entry.status = crate::model_catalog::DiscoveryStatus::Checking;
            entry.request_id = id.clone();
        }
        let quiet = matches!(method.as_str(), "provider.models" | "runtime.catalog");
        if !quiet {
            self.apply(RunnerMsg::ToolStarted {
                id: id.clone(),
                name: method.clone(),
                args: String::new(),
            });
        }
        self.service_threads.push(std::thread::spawn(move||{
            let outcome=std::panic::catch_unwind(std::panic::AssertUnwindSafe(||runner(request,cancel,tx.clone())));
            let result=match outcome{Ok(value)=>value,Err(_)=>Err(anyhow::anyhow!("Control-plane operation stopped unexpectedly"))};
            match result{Ok(mut result)=>{result["_workspace"]=workspace.into();result["_request_id"]=id.clone().into();let message=if method=="agents.run"{"Delegate completed".to_string()}else{result["message"].as_str().unwrap_or("Completed").to_string()};let _=tx.send(RunnerMsg::ServiceResult{method,result});if !quiet{let _=tx.send(RunnerMsg::ToolFinished{id,ok:true,output:message});}},Err(error)=>{if quiet{let _=tx.send(RunnerMsg::ServiceResult{method,result:serde_json::json!({"_workspace":workspace,"provider_id":profile_id,"base_url":base_url,"_request_id":id,"failed":true,"message":error.to_string()})});}else{let _=tx.send(RunnerMsg::ToolFinished{id,ok:false,output:error.to_string()});}}}
        }));
        Ok(())
    }


    pub(crate) fn open_provider(&mut self) {
        self.state.provider_dialog = Some(crate::provider::ProviderDialog::new(
            self.state.providers.clone(),
            self.state.provider.as_ref().map(|p| p.id.as_str()),
        ));
        if let Some(dialog) = &mut self.state.provider_dialog {
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
        }
    }


    pub(crate) fn refresh_model_catalog(&mut self) -> Result<()> {
        if self.service_runner.is_none() {
            return Ok(());
        }
        let mut profiles = self.state.providers.clone();
        if let Some(active) = &self.state.provider {
            if !profiles.iter().any(|p| p.id == active.id) {
                profiles.push(active.clone());
            }
        }
        for preset in meg_core::provider_catalog::PROVIDER_PRESETS {
            if (preset.local || meg_core::providers::has_key(&preset.profile()))
                && !profiles
                    .iter()
                    .any(|p| p.id == preset.id || p.base_url.trim_end_matches('/') == preset.url)
            {
                profiles.push(preset.profile());
            }
        }
        for profile in profiles {
            let cloud =
                meg_core::provider_catalog::provider_preset(&profile.id).is_some_and(|p| !p.local);
            if cloud && !meg_core::providers::has_key(&profile) {
                continue;
            }
            if self
                .state
                .provider_models
                .get(&profile.id)
                .is_some_and(|e| e.status == crate::model_catalog::DiscoveryStatus::Checking)
            {
                continue;
            }
            if !self.model_queue.iter().any(|p| p.id == profile.id) {
                let entry = self
                    .state
                    .provider_models
                    .entry(profile.id.clone())
                    .or_insert_with(|| crate::model_catalog::ProviderModels {
                        profile: profile.clone(),
                        models: Vec::new(),
                        status: crate::model_catalog::DiscoveryStatus::Checking,
                        message: String::new(),
                        request_id: String::new(),
                    });
                entry.status = crate::model_catalog::DiscoveryStatus::Checking;
                entry.request_id.clear();
                self.model_queue.push_back(profile);
            }
        }
        self.pump_model_catalog()?;
        if matches!(self.state.active_modal, ActiveModal::ModelPicker(_)) {
            self.state.open_model_picker();
        }
        Ok(())
    }


    pub(super) fn pump_model_catalog(&mut self) -> Result<()> {
        while !self.model_queue.is_empty()
            && self
                .state
                .provider_models
                .values()
                .filter(|e| {
                    e.status == crate::model_catalog::DiscoveryStatus::Checking
                        && !e.request_id.is_empty()
                })
                .count()
                < 4
            && self
                .service_threads
                .iter()
                .filter(|t| !t.is_finished())
                .count()
                < 14
        {
            let profile = self.model_queue.pop_front().unwrap();
            self.invoke_service("provider.models", serde_json::to_value(profile)?)?;
        }
        Ok(())
    }


    pub(crate) fn use_provider(
        &mut self,
        profile: meg_core::providers::ProviderProfile,
    ) -> Result<()> {
        if meg_core::provider_catalog::provider_preset(&profile.id).is_some_and(|p| !p.local)
            && !meg_core::providers::has_key(&profile)
        {
            self.open_provider();
            self.state
                .provider_dialog
                .as_mut()
                .unwrap()
                .edit_connection(profile);
            return Ok(());
        }
        if profile.id != "environment" {
            if let Some(root) = &self.state.provider_root {
                let store = meg_core::providers::ProviderStore { root: root.clone() };
                if self.state.providers.iter().any(|p| p.id == profile.id) {
                    store.activate(&profile.id)?;
                } else {
                    self.state.providers = store.upsert(profile.clone(), None)?.profiles;
                }
            }
        }
        self.state.provider = Some(profile.clone());
        self.state.model = profile.default_model.clone();
        self.state.available_models = if self.state.model.is_empty() {
            Vec::new()
        } else {
            vec![self.state.model.clone()]
        };
        self.state.online = false;
        self.state.provider_dialog = None;
        self.state.notice = format!("Connecting to {}…", profile.name);
        self.invoke_service("provider.models", serde_json::to_value(&profile)?)?;
        self.save()
    }


    pub fn new(state: WorkstationState, runner: Option<Arc<GoalRunner>>) -> Result<Self> {
        let mut controller = Self::in_memory(state, runner);
        if controller
            .state
            .workspace_picker
            .as_ref()
            .is_some_and(|p| p.fullscreen)
        {
            return Ok(controller);
        }
        let store = SessionStore::open(std::path::Path::new(&controller.state.cwd))?;
        if let Err(error) = store.restore_preferences(&mut controller.state) {
            controller.state.notice = format!("Preferences could not be loaded: {error}");
        }
        let startup = std::mem::take(&mut controller.state.startup);
        if let Some(id) = startup.resume {
            let id = if id == "latest" {
                store
                    .list()?
                    .first()
                    .context("No saved session to resume")?
                    .id
                    .clone()
            } else {
                id
            };
            store.load(&id)?.restore(&mut controller.state);
        }
        if let Ok(model) = std::env::var("MEG_TUI_MODEL") {
            if !model.trim().is_empty() {
                controller.state.model = model;
            }
        }
        if let Some(model) = startup.model {
            controller.state.model = model;
        }
        controller
            .state
            .select_model(controller.state.model.clone());
        if let Some(effort) = startup.effort {
            controller.state.select_effort(effort);
        }
        controller.store = Some(store);
        Ok(controller)
    }


    pub(crate) fn system(&mut self, text: impl Into<String>) {
        let text = text.into();
        if !text.is_empty() {
            self.state.transcript.push(format!("[system] {text}"));
        }
    }


    pub(super) fn error(&mut self, error: impl std::fmt::Display) {
        self.notice_deadline = None;
        self.state.notice = error.to_string();
    }


    pub(crate) fn save(&self) -> Result<()> {
        if let Some(store) = &self.store {
            store.save(&self.state)?;
            store.save_preferences(&self.state)?;
        }
        Ok(())
    }


    pub(crate) fn reset_session(&mut self) {
        self.state.session_id = uuid::Uuid::new_v4().to_string();
        self.state.conversation.clear();
        self.state.transcript.clear();
        self.state.plan_steps.clear();
        self.state.background_tasks.clear();
        self.state.subagents.clear();
        self.state.stream = None;
        self.state.usage = Usage::default();
        self.state.context_percent = 0;
        self.state.pasted_chunks.clear();
        self.state.attached_media.clear();
        self.state.transcript_scroll = 0;
    }


    pub(crate) fn change_workspace(&mut self, path: &std::path::Path) -> Result<()> {
        anyhow::ensure!(
            !self.state.busy && !self.state.background_tasks.iter().any(|t| t.active),
            "Stop active work before changing workspace"
        );
        let path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::path::Path::new(&self.state.cwd).join(path)
        };
        let path = path.canonicalize().context("Workspace does not exist")?;
        anyhow::ensure!(path.is_dir(), "Workspace must be a directory");
        let store = SessionStore::open(&path)?;
        self.save()?;
        for token in &self.service_tokens {
            token.cancel();
        }
        self.model_queue.clear();
        self.state.provider_models.clear();
        self.reset_session();
        self.state.cwd = crate::friendly_path(&path);
        self.state.project = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        self.state.git_branch = git_branch(&path);
        if let Err(error) = store.restore_preferences(&mut self.state) {
            self.state.notice = format!("Preferences: {error}");
        }
        let startup = std::mem::take(&mut self.state.startup);
        if let Some(id) = startup.resume {
            let id = if id == "latest" {
                store
                    .list()?
                    .first()
                    .context("No saved session to resume")?
                    .id
                    .clone()
            } else {
                id
            };
            store.load(&id)?.restore(&mut self.state);
        }
        if let Some(model) = startup.model {
            self.state.select_model(model);
        }
        if let Some(effort) = startup.effort {
            self.state.select_effort(effort);
        }
        self.store = Some(store);
        self.state.notice = format!("Workspace: {}", path.display());
        self.initialize_services();
        Ok(())
    }


    pub(super) fn initialize_services(&mut self) {
        if self.service_runner.is_some() {
            let _ = self.refresh_model_catalog();
            let _ = self.invoke_service("runtime.catalog", serde_json::json!({}));
        }
    }


    pub(crate) fn open_sessions(&mut self) -> Result<()> {
        self.save()?;
        let records = self
            .store
            .as_ref()
            .context("No persistent session store is attached")?
            .list()?;
        let items = records
            .into_iter()
            .map(|record| PickerItem {
                label: record.title.clone(),
                detail: record.id[..8].into(),
                meta: record.model.clone(),
                value: record,
            })
            .collect();
        self.state.session_picker = Some(PickerState::new("Resume session".into(), items));
        Ok(())
    }


    pub fn submit_line(&mut self, text: &str) -> Result<()> {
        self.state.input = text.into();
        self.state.move_cursor_end();
        self.submit_input()
    }


    pub(super) fn submit_input(&mut self) -> Result<()> {
        let text = self.state.input.trim().to_string();
        if text.is_empty() {
            return Ok(());
        }
        self.state.notice.clear();
        if text.starts_with('/') {
            let result = self.command(&text);
            match result {
                Ok(goal) => {
                    if let Some(goal) = goal {
                        self.start_named(goal, text.clone())?;
                    }
                    self.state.commit_input();
                }
                Err(error) => {
                    self.error(&error);
                    return Err(error);
                }
            }
        } else if self.state.busy {
            anyhow::ensure!(
                self.queue.len() < 8,
                "Follow-up queue is full; your draft is preserved"
            );
            let (expanded, attachments) = self.expand_attachments(&text)?;
            anyhow::ensure!(
                attachments.is_empty(),
                "Wait for the active run to finish before sending image or video attachments"
            );
            self.queue.push_back((expanded, text));
            self.state.pasted_chunks.clear();
            self.state.attached_media.clear();
            self.state.commit_input();
            self.state.queued_count = self.queue.len();
            self.state.notice = format!(
                "Queued follow-up ({}) · /stop clears queued work",
                self.queue.len()
            );
        } else {
            let (expanded, attachments) = self.expand_attachments(&text)?;
            self.start_with_attachments(expanded, text, attachments)?;
            self.state.pasted_chunks.clear();
            self.state.attached_media.clear();
            self.state.commit_input();
        }
        self.save()
    }


    pub(super) fn start(&mut self, goal: String) -> Result<()> {
        self.start_named(goal.clone(), goal)
    }


    pub(crate) fn start_named(&mut self, goal: String, display: String) -> Result<()> {
        self.start_with_attachments(goal, display, Vec::new())
    }


    pub(crate) fn start_with_attachments(
        &mut self,
        goal: String,
        display: String,
        attachments: Vec<meg_core::shim_client::ImageAttachment>,
    ) -> Result<()> {
        anyhow::ensure!(!self.state.busy, "A run is already active");
        let runner = self
            .runner
            .as_ref()
            .context("No model runner is attached")?
            .clone();
        let mut request = self.state.run_request(goal.clone());
        request.attachments = attachments.clone();
        let cancel = request.cancel.clone();
        self.state.conversation.push(if attachments.is_empty() {
            meg_core::ChatMessage {
                role: "user".into(),
                content: goal.clone(),
            }
        } else {
            meg_core::shim_client::encode_user_multimodal(&goal, &attachments)
        });
        self.state.transcript.push(format!("❯ {display}"));
        self.state.stream = Some(StreamState {
            is_thinking_collapsed: true,
            ..Default::default()
        });
        self.state.busy = true;
        self.state.state_label = "WORKING".into();
        self.state.transcript_scroll = 0;
        let (tx, rx) = mpsc::channel();
        let thread = std::thread::spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                runner(request, tx.clone())
            }));
            let answer = match result {
                Ok(Ok(answer)) => answer,
                Ok(Err(error)) => {
                    let _ = tx.send(RunnerMsg::Line(format!("[error] {error}")));
                    None
                }
                Err(_) => {
                    let _ = tx.send(RunnerMsg::Line(
                        "[error] The runner stopped unexpectedly.".into(),
                    ));
                    None
                }
            };
            let _ = tx.send(RunnerMsg::Done(answer));
        });
        self.active = Some(ActiveRun { rx, cancel, thread });
        Ok(())
    }


    pub(crate) fn cancel(&mut self) {
        self.model_queue.clear();
        for entry in self.state.provider_models.values_mut() {
            if entry.status == crate::model_catalog::DiscoveryStatus::Checking
                && entry.request_id.is_empty()
            {
                entry.status = crate::model_catalog::DiscoveryStatus::Unavailable;
                entry.message = "Discovery cancelled".into();
            }
        }
        for token in &self.service_tokens {
            token.cancel();
        }
        self.queue.clear();
        self.state.queued_count = 0;
        if let Some(active) = &self.active {
            active.cancel.cancel();
            self.state.state_label = "CANCELLING".into();
            self.state.notice = "Stopping owned work…".into();
        } else {
            self.state.notice = if self.state.background_tasks.iter().any(|t| t.active) {
                "Stopping owned work…"
            } else {
                "No active run."
            }
            .into();
        }
    }

}
impl Drop for Controller {
    fn drop(&mut self) {
        for token in &self.service_tokens {
            token.cancel();
        }
        for thread in self.service_threads.drain(..) {
            let _ = thread.join();
        }
        if let Some(active) = self.active.take() {
            active.cancel.cancel();
            let _ = active.thread.join();
        }
    }
}

fn git_branch(path: &std::path::Path) -> String {
    std::process::Command::new("git")
        .arg("-C")
        .arg(path)
        .args(["branch", "--show-current"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().into())
        .unwrap_or_default()
}

struct TerminalGuard;
impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = tui_restore();
    }
}
pub fn tui_restore() -> Result<()> {
    use std::io::Write;
    let _ = crossterm::terminal::disable_raw_mode();
    let mut out = std::io::stdout();
    out.write_all(RESET_SEQUENCE.as_bytes())?;
    out.flush()?;
    Ok(())
}
pub fn run_app(
    state: WorkstationState,
    runner: Option<Arc<GoalRunner>>,
) -> Result<WorkstationState> {
    run_app_with_services(state, runner, None)
}
pub fn run_app_with_services(
    state: WorkstationState,
    runner: Option<Arc<GoalRunner>>,
    services: Option<Arc<ServiceRunner>>,
) -> Result<WorkstationState> {
    use std::io::IsTerminal;
    anyhow::ensure!(
        std::io::stdout().is_terminal(),
        "MEG TUI requires an interactive terminal; use meg run for headless execution"
    );
    let mut controller = Controller::new(state, runner)?.with_services(services);
    if controller.state.workspace_picker.is_none() {
        controller.initialize_services();
    }
    let _guard = TerminalGuard;
    crossterm::terminal::enable_raw_mode()?;
    crossterm::execute!(
        std::io::stdout(),
        crossterm::terminal::EnterAlternateScreen,
        crossterm::event::EnableBracketedPaste
    )?;
    static HOOK: std::sync::Once = std::sync::Once::new();
    HOOK.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let _ = tui_restore();
            previous(info);
        }));
    });
    let mut terminal =
        ratatui::Terminal::new(ratatui::backend::CrosstermBackend::new(std::io::stdout()))?;
    terminal.clear()?;
    let size = terminal.size()?;
    controller.editor_width = DashboardLayout::new(
        ratatui::layout::Rect::new(0, 0, size.width, size.height),
        &controller.state.input,
    )
    .composer
    .width;
    let mut last = Instant::now();
    let mut dirty = true;
    loop {
        let now = Instant::now();
        let delta = now.duration_since(last).as_secs_f32();
        last = now;
        match controller.poll(delta) {
            Ok(changed) => dirty |= changed,
            Err(error) => {
                controller.error(error);
                dirty = true;
            }
        }
        if controller.state.should_quit && !controller.state.busy {
            break;
        }
        if dirty || controller.state.busy {
            if controller.state.ui_config.sync_csi_2026 {
                let _ = std::io::Write::write_all(&mut std::io::stdout(), b"\x1b[?2026h");
            }
            terminal.draw(|f| draw(f, &controller.state.theme, &controller.state))?;
            if controller.state.ui_config.sync_csi_2026 {
                let mut out = std::io::stdout();
                let _ = std::io::Write::write_all(&mut out, b"\x1b[?2026l");
                let _ = std::io::Write::flush(&mut out);
            }
            dirty = false;
        }
        if crossterm::event::poll(Duration::from_millis(if controller.state.busy {
            40
        } else {
            200
        }))? {
            for event in
                crate::input::read_terminal_batch(controller.state.provider_dialog.is_some())?
            {
                if let Err(error) = controller.handle_event(event) {
                    controller.error(error);
                }
            }
            dirty = true;
        }
    }
    controller.save()?;
    Ok(controller.state.clone())
}
