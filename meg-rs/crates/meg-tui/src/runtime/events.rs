use std::sync::mpsc;
use super::Controller;
use crate::*;
use crate::session::Usage;
use anyhow::Result;
use std::time::{Duration, Instant};

impl Controller {
    pub fn poll(&mut self, delta: f32) -> Result<bool> {
        let mut changed = false;
        if self
            .notice_deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            self.state.notice.clear();
            self.notice_deadline = None;
            changed = true;
        }
        for _ in 0..100 {
            match self.service_rx.try_recv() {
                Ok(msg) => {
                    self.apply(msg);
                    changed = true;
                }
                Err(_) => break,
            }
        }
        self.service_threads
            .retain_mut(|thread| !thread.is_finished());
        if self.service_threads.is_empty() {
            self.service_tokens.clear();
        }
        let mut ended = false;
        let mut completed_ok = false;
        for _ in 0..500 {
            let msg = match self.active.as_ref().map(|a| a.rx.try_recv()) {
                Some(Ok(m)) => m,
                Some(Err(mpsc::TryRecvError::Disconnected)) => {
                    self.state.finish_response(None);
                    self.system("Runner disconnected.");
                    ended = true;
                    changed = true;
                    break;
                }
                _ => break,
            };
            changed = true;
            if let RunnerMsg::Done(answer) = msg {
                let cancelled = self
                    .active
                    .as_ref()
                    .is_some_and(|a| a.cancel.is_cancelled());
                completed_ok = answer.is_some() && !cancelled;
                if !cancelled {
                    if let Some(answer) = &answer {
                        self.state.conversation.push(meg_core::ChatMessage {
                            role: "assistant".into(),
                            content: answer.clone(),
                        });
                    }
                    self.state.finish_response(answer);
                } else {
                    self.state.finish_response(None);
                    self.system("Run cancelled.");
                    self.state.notice = "Stopped.".into();
                }
                ended = true;
                break;
            }
            self.apply(msg);
        }
        if ended {
            self.state.busy = false;
            self.state.state_label = "READY".into();
            self.state.active_job = None;
            for task in &mut self.state.background_tasks {
                if task.active && !task.id.starts_with("service-") {
                    task.active = false;
                    task.failed = true;
                }
            }
            if let Some(active) = self.active.take() {
                let _ = active.thread.join();
            }
            self.save()?;
            if !self.state.should_quit && completed_ok {
                if let Some((body, display)) = self.queue.pop_front() {
                    self.state.queued_count = self.queue.len();
                    self.start_named(body, display)?;
                }
            } else if !self.queue.is_empty() {
                self.state.notice="Queued follow-ups paused after the run stopped. /queue lists them; /queue run continues.".into();
            }
        }
        if self.state.background_tasks.iter().any(|t| t.active) {
            for task in &mut self.state.background_tasks {
                task.tick(delta);
            }
            changed = true;
        }
        if self.state.busy {
            if let Some(stream) = &mut self.state.stream {
                stream.elapsed_secs += delta.max(0.0);
            }
            if let Some(job) = &mut self.state.active_job {
                job.tick(delta);
            }
            changed = true;
        }
        self.pump_model_catalog()?;
        Ok(changed)
    }


    pub fn apply(&mut self, msg: RunnerMsg) {
        match msg {
            RunnerMsg::DelegatedUsage(value) => {
                if let Ok(usage) = serde_json::from_value::<Usage>(value) {
                    self.state.usage.input_tokens += usage.input_tokens;
                    self.state.usage.output_tokens += usage.output_tokens;
                    self.state.usage.cached_tokens += usage.cached_tokens;
                    self.state.usage.requests += usage.requests;
                    self.state.usage.reported_requests += usage.reported_requests;
                }
            }
            RunnerMsg::ServiceResult { method, result } => {
                if result["_workspace"]
                    .as_str()
                    .is_some_and(|s| s != self.state.cwd)
                {
                    return;
                }
                if method == "provider.models" {
                    use crate::model_catalog::{DiscoveryStatus, ProviderModels};
                    let Some(id) = result["provider_id"].as_str() else {
                        return;
                    };
                    if !self.state.provider_models.contains_key(id) {
                        let profile = self
                            .state
                            .providers
                            .iter()
                            .find(|p| p.id == id)
                            .cloned()
                            .or_else(|| {
                                self.state.provider.as_ref().filter(|p| p.id == id).cloned()
                            });
                        let Some(profile) = profile else {
                            return;
                        };
                        self.state.provider_models.insert(
                            id.into(),
                            ProviderModels {
                                profile,
                                models: Vec::new(),
                                status: DiscoveryStatus::Checking,
                                message: String::new(),
                                request_id: String::new(),
                            },
                        );
                    }
                    let entry = self.state.provider_models.get_mut(id).unwrap();
                    if result["base_url"]
                        .as_str()
                        .is_some_and(|url| url != entry.profile.base_url)
                        || result["_request_id"].as_str().is_some_and(|request| {
                            !entry.request_id.is_empty() && request != entry.request_id
                        })
                    {
                        return;
                    }
                    let failed = result["failed"].as_bool() == Some(true);
                    entry.message = result["message"].as_str().unwrap_or("").into();
                    if failed {
                        entry.status = DiscoveryStatus::Unavailable;
                    } else {
                        entry.models = result["models"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(|v| v.as_str())
                            .filter(|m| {
                                !m.is_empty() && m.len() <= 256 && !m.chars().any(char::is_control)
                            })
                            .take(10000)
                            .map(str::to_owned)
                            .collect();
                        entry.models.sort();
                        entry.models.dedup();
                        entry.status = if entry.models.is_empty() {
                            DiscoveryStatus::Empty
                        } else {
                            DiscoveryStatus::Ready
                        };
                    }
                    let models = entry.models.clone();
                    let label = entry.label();
                    let name = entry.profile.name.clone();
                    if self.state.provider.as_ref().is_some_and(|p| p.id == id) {
                        self.state.online = !failed;
                        self.state.available_models = models;
                        self.state.notice = if failed {
                            format!("{name}: connection unavailable. /provider to inspect; /model shows other services.")
                        } else {
                            format!("{name}: {label}")
                        };
                        if self.state.model.is_empty() && !failed {
                            if self.state.provider.as_ref().is_some_and(|p| {
                                p.kind == meg_core::providers::ProviderKind::Ollama
                            }) {
                                if let Some(model) = self.state.available_models.first().cloned() {
                                    self.state.select_model(model);
                                }
                            } else if !self.state.available_models.is_empty()
                                && !self.overlay_open()
                                && !self.state.busy
                            {
                                self.state.open_model_picker();
                            }
                        }
                    }
                    if let Some(dialog) = &mut self.state.provider_dialog {
                        dialog.statuses.insert(id.into(), label);
                        dialog
                            .messages
                            .insert(id.into(), result["message"].as_str().unwrap_or("").into());
                    }
                    if matches!(self.state.active_modal, ActiveModal::ModelPicker(_)) {
                        self.state.open_model_picker();
                    }
                    return;
                }
                if result["failed"].as_bool() == Some(true) {
                    self.error(result["message"].as_str().unwrap_or("Operation failed"));
                    return;
                }
                if let Some(catalog) = result.get("catalog") {
                    self.state.extensions = catalog.clone();
                    self.state.palette.extra.clear();
                    for command in catalog["commands"].as_array().into_iter().flatten() {
                        if let Some(name) = command["name"].as_str() {
                            self.state.palette.extra.push((
                                format!("/{name}"),
                                command["description"]
                                    .as_str()
                                    .unwrap_or("Plugin command")
                                    .into(),
                            ));
                        }
                    }
                    for skill in catalog["skills"].as_array().into_iter().flatten() {
                        if let Some(name) = skill["name"].as_str() {
                            self.state.palette.extra.push((
                                format!("/skill:{name}"),
                                skill["description"]
                                    .as_str()
                                    .unwrap_or("Installed skill")
                                    .into(),
                            ));
                        }
                    }
                }
                if method == "agents.run" {
                    if let Some(answer) = result["answer"].as_str() {
                        self.state.transcript.push(format!(
                            "[MEG] {}: {answer}",
                            result["role"].as_str().unwrap_or("delegate")
                        ));
                    }
                    self.apply(RunnerMsg::DelegatedUsage(result["usage"].clone()));
                }
                if let Some(message) = result["message"].as_str() {
                    self.state.notice = message.into();
                    self.notice_deadline = Some(Instant::now() + Duration::from_secs(6));
                }
            }
            RunnerMsg::StreamStart => {
                self.state.archive_stream();
                self.state.usage.requests += 1;
                self.state.stream = Some(StreamState {
                    is_thinking_collapsed: true,
                    ..Default::default()
                });
            }
            RunnerMsg::ThinkingChunk(text) => self
                .state
                .stream
                .get_or_insert_with(Default::default)
                .push_thinking(&text),
            RunnerMsg::ContentChunk(text) => self
                .state
                .stream
                .get_or_insert_with(Default::default)
                .push_content(&text),
            RunnerMsg::Line(text) => self.state.transcript.push(text),
            RunnerMsg::StreamMetrics {
                tok_per_sec,
                elapsed_secs,
                token_count,
            } => {
                let s = self.state.stream.get_or_insert_with(Default::default);
                s.tok_per_sec = tok_per_sec;
                s.elapsed_secs = elapsed_secs;
                s.token_count = token_count;
            }
            RunnerMsg::Usage {
                input,
                output,
                cached,
            } => {
                self.state.usage.input_tokens += input;
                self.state.usage.output_tokens += output;
                self.state.usage.cached_tokens += cached;
                self.state.usage.reported_requests += 1;
                let ctx_window = std::env::var("MEG_CONTEXT_WINDOW")
                    .ok()
                    .and_then(|v| v.parse::<u64>().ok())
                    .filter(|&n| n > 0)
                    .unwrap_or_else(|| {
                        let m = self.state.model.to_ascii_lowercase();
                        if m.contains("gemini") {
                            1_000_000
                        } else if m.contains("claude") {
                            200_000
                        } else if m.contains(":7b") || m.contains(":8b") || m.contains(":14b") || m.contains("qwen2.5-coder:7b") {
                            32_768
                        } else {
                            128_000
                        }
                    });
                let turn_tokens = input.saturating_add(output);
                if turn_tokens > 0 {
                    let pct = (turn_tokens.saturating_mul(100) / ctx_window).min(100) as u8;
                    self.state.context_percent = pct.max(1);
                }
                if let Some(s) = &mut self.state.stream {
                    s.token_count = output as usize;
                    s.tok_per_sec = StreamState::calculate_rate(output as usize, s.elapsed_secs);
                }
            }
            RunnerMsg::ToolStarted { id, name, args } => {
                let mut task = BackgroundTask::start(format!("{name} {args}"));
                task.id = id;
                self.state.background_tasks.push(task);
            }
            RunnerMsg::ToolFinished { id, ok, output } => {
                if let Some(task) = self.state.background_tasks.iter_mut().find(|t| t.id == id) {
                    task.finish();
                    task.failed = !ok;
                }
                if !ok && output.to_lowercase().contains("cancelled") {
                    for (_, status) in &mut self.state.subagents {
                        if matches!(status.as_str(), "running" | "cancelling") {
                            *status = "cancelled".into();
                        }
                    }
                }
                self.state.transcript.push(format!(
                    "[tool {}] {output}",
                    if ok { "ok" } else { "failed" }
                ));
                self.state
                    .background_tasks
                    .retain(|t| t.active || t.id == id);
            }
            RunnerMsg::BackgroundStart(name) => self
                .state
                .background_tasks
                .push(BackgroundTask::start(name)),
            RunnerMsg::BackgroundDone(name) => {
                if let Some(t) = self
                    .state
                    .background_tasks
                    .iter_mut()
                    .find(|t| t.name == name && t.active)
                {
                    t.finish();
                }
            }
            RunnerMsg::TodoList(steps) => self.state.plan_steps = steps,
            RunnerMsg::PlanStep(done, text) => self.state.plan_steps.push((done, text)),
            RunnerMsg::TodoStep(index, done) => {
                if let Some(step) = self.state.plan_steps.get_mut(index) {
                    step.0 = done;
                }
            }
            RunnerMsg::Online(online) => self.state.online = online,
            RunnerMsg::ModelList(models) => self.state.available_models = models,
            RunnerMsg::SubagentState(name, status) => {
                if let Some(row) = self.state.subagents.iter_mut().find(|(n, _)| n == &name) {
                    row.1 = status;
                } else {
                    if self.state.subagents.len() >= 32 {
                        if let Some(index) = self
                            .state
                            .subagents
                            .iter()
                            .position(|(_, s)| !matches!(s.as_str(), "running" | "cancelling"))
                        {
                            self.state.subagents.remove(index);
                        }
                    }
                    self.state.subagents.push((name, status));
                }
            }
            RunnerMsg::JobStart { id, description } => {
                let mut job = JobState::default();
                job.start(id, description);
                self.state.active_job = Some(job);
            }
            RunnerMsg::JobProgress { id, status } => {
                if let Some(job) = &mut self.state.active_job {
                    if job.id == id {
                        job.push_output(status);
                    }
                }
            }
            RunnerMsg::JobDone { id } => {
                if self.state.active_job.as_ref().is_some_and(|j| j.id == id) {
                    self.state.active_job = None;
                }
            }
            RunnerMsg::Done(answer) => self.state.finish_response(answer),
        }
    }

}
