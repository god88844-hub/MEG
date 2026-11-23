//! Durable native sessions. Atomic writes and validated identifiers keep a bad file
//! or interrupted save from destroying a different conversation.
use crate::{get_palette, Mode, WorkstationState};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cached_tokens: u64,
    pub requests: u64,
    pub reported_requests: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRecord {
    #[serde(default)]
    pub provider_id: Option<String>,
    pub version: u32,
    pub id: String,
    pub title: String,
    pub workspace: String,
    pub updated: i64,
    pub model: String,
    pub effort: String,
    pub theme: String,
    pub mode: String,
    pub transcript: Vec<String>,
    pub messages: Vec<meg_core::ChatMessage>,
    pub todos: Vec<(bool, String)>,
    pub history: Vec<String>,
    pub usage: Usage,
}

#[derive(Debug, Clone)]
pub struct SessionStore {
    pub root: PathBuf,
    pub workspace: String,
}

impl SessionStore {
    pub fn open(workspace: &Path) -> Result<Self> {
        let workspace = workspace
            .canonicalize()
            .context("Workspace does not exist")?;
        let root = workspace.join(".meg").join("tui");
        for candidate in [workspace.join(".meg"), root.clone(), root.join("sessions")] {
            if std::fs::symlink_metadata(&candidate).is_ok()
                && !candidate.canonicalize()?.starts_with(&workspace)
            {
                bail!("Session storage must remain inside its workspace");
            }
        }
        std::fs::create_dir_all(root.join("sessions"))?;
        use std::io::Write;
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(root.join(".gitignore"))
        {
            Ok(mut file) => file.write_all(b"*\n")?,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.into()),
        }
        Ok(Self {
            root,
            workspace: workspace.to_string_lossy().into_owned(),
        })
    }
    fn path(&self, id: &str) -> Result<PathBuf> {
        uuid::Uuid::parse_str(id).context("Invalid session identifier")?;
        Ok(self.root.join("sessions").join(format!("{id}.json")))
    }
    pub fn save(&self, st: &WorkstationState) -> Result<()> {
        let record = SessionRecord::capture(st, &self.workspace);
        atomic_json(&self.path(&record.id)?, &record)
    }
    pub fn load(&self, id: &str) -> Result<SessionRecord> {
        let path = self.path(id)?;
        if path.metadata()?.len() > 16 * 1024 * 1024 {
            bail!("Session file exceeds 16 MiB");
        }
        let record: SessionRecord = serde_json::from_slice(&std::fs::read(path)?)?;
        if record.version != 1 || record.id != id {
            bail!("Unsupported or mismatched session file");
        }
        if Path::new(&record.workspace).canonicalize()?
            != Path::new(&self.workspace).canonicalize()?
        {
            bail!("Session belongs to a different workspace");
        }
        Ok(record)
    }
    pub fn list(&self) -> Result<Vec<SessionRecord>> {
        let mut records = Vec::new();
        for entry in std::fs::read_dir(self.root.join("sessions"))? {
            let entry = entry?;
            if entry.path().extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            if let Some(id) = entry.path().file_stem().and_then(|s| s.to_str()) {
                if let Ok(record) = self.load(id) {
                    records.push(record);
                }
            }
        }
        records.sort_by_key(|record| std::cmp::Reverse(record.updated));
        Ok(records)
    }
    pub fn save_preferences(&self, st: &WorkstationState) -> Result<()> {
        atomic_json(
            &self.root.join("preferences.json"),
            &serde_json::json!({
                "model": st.model, "effort": st.effort, "theme": st.theme.name,
                "mode": st.mode.label(), "provider_id": st.provider.as_ref().map(|p|p.id.clone()),
            }),
        )
    }
    pub fn restore_preferences(&self, st: &mut WorkstationState) -> Result<()> {
        let path = self.root.join("preferences.json");
        if !path.exists() {
            return Ok(());
        }
        let value: serde_json::Value = serde_json::from_slice(&std::fs::read(path)?)?;
        let mut restore_model = true;
        if let Some(id) = value["provider_id"].as_str() {
            if let Some(profile) = st.providers.iter().find(|p| p.id == id) {
                st.provider = Some(profile.clone());
            } else {
                restore_model = false;
                if !st
                    .provider
                    .as_ref()
                    .is_some_and(|p| st.providers.iter().any(|known| known.id == p.id))
                {
                    st.provider = None;
                    st.model.clear();
                }
                st.notice =
                    "Saved provider is unavailable; using the current connection settings.".into();
            }
        }
        if let Some(model) = value["model"]
            .as_str()
            .filter(|s| restore_model && !s.is_empty())
        {
            st.model = model.into();
        }
        if let Some(theme) = value["theme"].as_str().and_then(get_palette) {
            st.theme = theme;
        }
        if let Some(effort) = value["effort"]
            .as_str()
            .filter(|s| matches!(*s, "low" | "medium" | "high" | "max"))
        {
            st.effort = effort.into();
        }
        st.mode = parse_mode(value["mode"].as_str().unwrap_or("BUILD"));
        Ok(())
    }
}

pub fn atomic_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let tmp = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    let bytes = serde_json::to_vec_pretty(value)?;
    anyhow::ensure!(
        bytes.len() <= 16 * 1024 * 1024,
        "Session data exceeds 16 MiB; export or start a new session"
    );
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    use std::io::Write;
    let result = (|| -> Result<()> {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&tmp, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(tmp);
    }
    result
}

fn parse_mode(mode: &str) -> Mode {
    match mode.to_lowercase().as_str() {
        "plan" => Mode::Plan,
        _ => Mode::Build,
    }
}

impl SessionRecord {
    pub fn capture(st: &WorkstationState, workspace: &str) -> Self {
        let title = st
            .conversation
            .iter()
            .find(|m| m.role == "user")
            .map(|m| {
                m.content
                    .lines()
                    .next()
                    .unwrap_or("New session")
                    .chars()
                    .take(80)
                    .collect()
            })
            .unwrap_or_else(|| "New session".into());
        Self {
            provider_id: st.provider.as_ref().map(|p| p.id.clone()),
            version: 1,
            id: st.session_id.clone(),
            title,
            workspace: workspace.into(),
            updated: chrono::Utc::now().timestamp_millis(),
            model: st.model.clone(),
            effort: st.effort.clone(),
            theme: st.theme.name.into(),
            mode: st.mode.label().into(),
            transcript: st.transcript.clone(),
            messages: st.conversation.clone(),
            todos: st.plan_steps.clone(),
            history: st.history.clone(),
            usage: st.usage.clone(),
        }
    }
    pub fn restore(self, st: &mut WorkstationState) {
        let provider_id = self.provider_id;
        st.session_id = self.id;
        st.transcript = self.transcript;
        st.conversation = self.messages;
        st.plan_steps = self.todos;
        st.model = self.model;
        if let Some(id) = provider_id {
            st.provider = st.providers.iter().find(|p| p.id == id).cloned();
            if st.provider.is_none() {
                st.model.clear();
                st.notice =
                    "The saved provider is unavailable. Select /provider before sending a message."
                        .into();
            }
        }
        st.effort = self.effort;
        st.theme = get_palette(&self.theme).unwrap_or(crate::DASHBOARD);
        st.mode = parse_mode(&self.mode);
        st.history = self.history;
        st.usage = self.usage;
        st.transcript_scroll = 0;
        st.stream = None;
        st.background_tasks.clear();
        st.subagents.clear();
        st.input.clear();
        st.cursor_position = 0;
    }
}
