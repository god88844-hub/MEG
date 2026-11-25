//! The freebuff-style workspace selector — meg-tui's startup screen.
//!
//! `meg` opens here first: the recent-workspaces list (freebuff's
//! `recent-projects.json` model: `[{path, lastOpened}]`), a type-to-navigate
//! path line, and directory browsing. Enter opens the dashboard in the chosen
//! workspace; Esc exits the startup picker and closes an in-session picker.

use std::path::{Path, PathBuf};

use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::Line,
    widgets::Paragraph,
    Frame,
};

use crate::theme::Palette;

const RECENT_JSON: &str = "recent_workspaces.json";
const LEGACY_TXT: &str = "recent_workspaces.txt";
const MAX_RECENT: usize = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickPhase {
    Projects,
    Recent,
    Browse,
}

#[derive(Debug, Clone)]
pub struct WorkspacePicker {
    pub projects: Vec<PathBuf>,
    pub fullscreen: bool,
    pub directory: PathBuf,
    pub phase: PickPhase,
    /// Recent entries, newest first.
    pub recent: Vec<(String, i64)>,
    /// The path currently typed in Browse mode.
    pub query: String,
    pub selected: usize,
    /// Directories under the browsed path.
    pub dirs: Vec<PathBuf>,
    /// The starting directory (Esc fallback).
    pub fallback: PathBuf,
    pub create_mode: bool,
    pub error: Option<String>,
}

impl WorkspacePicker {
    /// Loads recent workspaces (freebuff JSON, then our legacy txt) and opens.
    pub fn open(fallback: PathBuf) -> Self {
        let recent = load_recent();
        let dirs = browser_dirs(&fallback);
        Self {
            projects: project_roots(&fallback, &recent),
            fullscreen: false,
            directory: fallback.clone(),
            phase: PickPhase::Recent,
            recent,
            query: String::new(),
            selected: 0,
            dirs,
            fallback,
            create_mode: false,
            error: None,
        }
    }
    pub fn open_startup(fallback: PathBuf) -> Self {
        let mut picker = Self::open(fallback.clone());
        picker.fullscreen = true;
        // A workspace can be any folder; project markers are suggestions only.
        picker.phase = PickPhase::Browse;
        if project_root(&fallback).is_some() {
            if let Some(parent) = fallback.parent() {
                picker.directory = parent.to_path_buf();
            }
        }
        picker.refresh_dirs();
        picker.selected = picker.dirs.iter().position(|p| p == &fallback).unwrap_or(0);
        picker
    }
    pub fn paste(&mut self, text: &str) {
        self.error = None;
        self.phase = PickPhase::Browse;
        // ConPTY may deliver one clipboard operation in multiple text bursts.
        // Append each burst instead of replacing the path's previous prefix.
        self.query.extend(
            text.chars()
                .filter(|c| !c.is_control())
                .take(16_384usize.saturating_sub(self.query.len())),
        );
        self.selected = 0;
        if !self.create_mode {
            self.refresh_dirs();
        }
    }
    pub fn parent(&mut self) {
        if self.phase == PickPhase::Projects {
            self.directory = self
                .selected_workspace()
                .and_then(|p| p.parent().map(Path::to_path_buf))
                .unwrap_or_else(|| self.fallback.clone());
            self.phase = PickPhase::Browse;
            self.query.clear();
            self.selected = 0;
            self.refresh_dirs();
            return;
        }
        if let Some(parent) = self.current_directory().parent() {
            self.directory = parent.to_path_buf();
            self.query.clear();
            self.phase = PickPhase::Browse;
            self.selected = 0;
            self.refresh_dirs();
        }
    }

    pub fn current_directory(&self) -> PathBuf {
        if self.phase == PickPhase::Browse && self.fullscreen {
            return self
                .dirs
                .first()
                .cloned()
                .unwrap_or_else(|| self.directory.clone());
        }
        if self.phase == PickPhase::Browse && !self.query.is_empty() {
            let candidate = PathBuf::from(self.query.trim_matches('"'));
            let candidate = if candidate.is_absolute() {
                candidate
            } else {
                self.directory.join(candidate)
            };
            if candidate.is_dir() {
                return candidate;
            }
        }
        self.directory.clone()
    }

    pub fn toggle_projects(&mut self) {
        self.phase = if self.phase == PickPhase::Projects {
            PickPhase::Browse
        } else {
            PickPhase::Projects
        };
        self.query.clear();
        self.error = None;
        self.selected = 0;
        self.refresh_dirs();
    }

    pub fn move_up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    /// Selection pool depends on the phase (recent entries vs. browsed dirs).
    fn pool_len(&self) -> usize {
        match self.phase {
            PickPhase::Projects => self.projects.len(),
            PickPhase::Recent => self.recent.len(),
            PickPhase::Browse => self.dirs.len(),
        }
    }

    pub fn move_down(&mut self) {
        let n = self.pool_len();
        if n > 0 {
            self.selected = (self.selected + 1).min(n - 1);
        }
    }

    /// Feed a typed character — any typing switches into Browse mode.
    pub fn push_char(&mut self, c: char) {
        self.error = None;
        if self.phase != PickPhase::Browse {
            self.phase = PickPhase::Browse;
            self.query.clear();
            self.selected = 0;
        }
        self.query.push(c);
        if !self.create_mode {
            self.refresh_dirs();
        }
    }

    pub fn backspace(&mut self) {
        self.error = None;
        if self.create_mode {
            self.query.pop();
            return;
        }
        if self.phase == PickPhase::Browse && !self.query.is_empty() {
            self.query.pop();
            self.refresh_dirs();
        } else if self.phase == PickPhase::Browse && self.fullscreen {
            self.parent();
        } else if self.phase == PickPhase::Browse {
            self.phase = PickPhase::Recent;
            self.selected = 0;
        }
    }

    /// Path '/' or '\\' in Browse mode descends into the highlighted dir.
    pub fn descend(&mut self) {
        if matches!(self.phase, PickPhase::Projects | PickPhase::Recent) {
            self.directory = self
                .selected_workspace()
                .unwrap_or_else(|| self.fallback.clone());
            self.phase = PickPhase::Browse;
            self.query.clear();
            self.selected = 0;
            self.refresh_dirs();
            return;
        }
        if let Some(dir) = self.dirs.get(self.selected) {
            let joined = dir.clone();
            if self.fullscreen {
                self.directory = joined;
                self.query.clear();
            } else {
                self.directory = joined;
                self.query.clear();
                self.phase = PickPhase::Browse;
            }
            self.refresh_dirs();
            self.selected = 0;
        }
    }

    fn refresh_dirs(&mut self) {
        if self.fullscreen {
            let query = self.query.trim_matches('"');
            let expanded = if query.starts_with('~') {
                std::env::var("USERPROFILE")
                    .or_else(|_| std::env::var("HOME"))
                    .map(|home| format!("{home}{}", &query[1..]))
                    .unwrap_or_else(|_| query.into())
            } else {
                query.into()
            };
            let candidate = if expanded.is_empty() {
                self.directory.clone()
            } else {
                let path = PathBuf::from(expanded);
                if path.is_absolute() {
                    path
                } else {
                    self.directory.join(path)
                }
            };
            let (base, prefix) = if candidate.is_dir() {
                (candidate, String::new())
            } else {
                (
                    candidate.parent().unwrap_or(&self.directory).to_path_buf(),
                    candidate
                        .file_name()
                        .map(|n| n.to_string_lossy().to_lowercase())
                        .unwrap_or_default(),
                )
            };
            self.dirs = vec![base.clone()];
            if let Some(parent) = base.parent() {
                self.dirs.push(parent.to_path_buf());
            }
            self.dirs
                .extend(browser_dirs(&base).into_iter().filter(|p| {
                    p.file_name()
                        .is_some_and(|n| n.to_string_lossy().to_lowercase().starts_with(&prefix))
                }));
            self.selected = self.selected.min(self.dirs.len().saturating_sub(1));
            return;
        }
        let base = if self.query.is_empty() {
            self.directory.clone()
        } else {
            let path = PathBuf::from(self.query.trim_matches('"'));
            if path.is_absolute() {
                path
            } else {
                self.directory.join(path)
            }
        };
        self.dirs = browser_dirs(&base);
        self.selected = self.selected.min(self.dirs.len().saturating_sub(1));
    }

    /// Resolve the current selection to a workspace directory.
    pub fn selected_workspace(&self) -> Option<PathBuf> {
        if self.fullscreen && self.phase == PickPhase::Browse {
            return self.dirs.get(self.selected).cloned();
        }
        match self.phase {
            PickPhase::Projects => self.projects.get(self.selected).cloned(),
            PickPhase::Recent => self
                .recent
                .get(self.selected)
                .map(|(p, _)| PathBuf::from(p)),
            PickPhase::Browse => {
                if self.query.is_empty() {
                    self.dirs.get(self.selected).cloned()
                } else {
                    let candidate = PathBuf::from(&self.query);
                    if candidate.is_dir() {
                        Some(candidate)
                    } else {
                        self.dirs.get(self.selected).cloned()
                    }
                }
            }
        }
    }

    /// Commit the selection: save to recents and return the workspace.
    pub fn commit(&self) -> Option<PathBuf> {
        let ws = self.selected_workspace()?;
        if ws.is_dir() {
            save_recent(&ws);
            Some(ws)
        } else {
            None
        }
    }

    pub fn begin_create(&mut self) {
        self.directory = self.current_directory();
        self.create_mode = true;
        self.error = None;
        self.query.clear();
    }
    pub fn cancel_create(&mut self) {
        self.create_mode = false;
        self.error = None;
        self.query.clear();
        self.refresh_dirs();
    }
    pub fn open_selected(&mut self) -> Option<PathBuf> {
        self.error = None;
        if self.create_mode {
            let name = self.query.trim();
            if name.is_empty()
                || name == "."
                || name == ".."
                || name.len() > 255
                || name.contains(['/', '\\'])
                || name.chars().any(char::is_control)
            {
                self.error = Some("Enter one new folder name; browse to its parent first.".into());
                return None;
            }
            let target = self.directory.join(name);
            match std::fs::create_dir(&target) {
                Ok(()) => {
                    save_recent(&target);
                    return Some(target);
                }
                Err(error) => {
                    self.error = Some(format!("Could not create folder: {error}"));
                    return None;
                }
            }
        }
        if self.phase == PickPhase::Browse && !self.query.is_empty() {
            let expanded = if let Some(rest) = self.query.trim_matches('"').strip_prefix('~') {
                PathBuf::from(
                    std::env::var("USERPROFILE")
                        .or_else(|_| std::env::var("HOME"))
                        .unwrap_or_default(),
                )
                .join(rest.trim_start_matches(['/', '\\']))
            } else {
                PathBuf::from(self.query.trim_matches('"'))
            };
            let candidate = if expanded.is_absolute() {
                expanded
            } else {
                self.directory.join(expanded)
            };
            if candidate.is_dir() {
                save_recent(&candidate);
                return Some(candidate);
            }
            if self.selected < 2 {
                self.error =
                    Some("Folder not found. Browse to a folder, or Ctrl+N to create one.".into());
                return None;
            }
        }
        let chosen = self.commit();
        if chosen.is_none() {
            self.error = Some("Choose an existing folder or press Ctrl+N to create one.".into());
        }
        chosen
    }
}

fn meg_dir() -> Option<PathBuf> {
    Some(meg_core::providers::config_dir())
}

fn normalized_directory(path: &Path) -> Option<PathBuf> {
    path.canonicalize()
        .ok()
        .filter(|p| p.is_dir())
        .map(|p| PathBuf::from(crate::friendly_path(&p)))
}
fn fixture_path(path: &Path) -> bool {
    path.components().any(|part| {
        let name = part.as_os_str().to_string_lossy();
        [
            "meg-production-test-",
            "meg-extensions-test-",
            "meg-provider-test-",
            "hybrid-fixture-",
            "meg-tui-test-",
            "meg_tui_probe_ws",
        ]
        .iter()
        .any(|prefix| name.starts_with(prefix))
    })
}
fn generic_container(path: &Path) -> bool {
    if path.parent().is_none() {
        return true;
    }
    let key = crate::friendly_path(path).to_lowercase();
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .and_then(|h| normalized_directory(Path::new(&h)))
        .is_some_and(|home| {
            home.ancestors()
                .any(|p| crate::friendly_path(p).to_lowercase() == key)
        })
}
fn project_marker(path: &Path) -> bool {
    [
        ".git",
        ".hg",
        "Cargo.toml",
        "package.json",
        "pyproject.toml",
        "go.mod",
        "CMakeLists.txt",
        "pom.xml",
        "build.gradle",
        "build.sbt",
    ]
    .iter()
    .any(|name| path.join(name).exists())
}
fn project_root(path: &Path) -> Option<PathBuf> {
    let path = normalized_directory(path)?;
    path.ancestors()
        .find(|p| p.join(".git").exists())
        .or_else(|| path.ancestors().find(|p| project_marker(p)))
        .map(Path::to_path_buf)
}
fn project_roots(current: &Path, recent: &[(String, i64)]) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut add = |path: PathBuf| {
        if let Some(path) = normalized_directory(&path) {
            if !fixture_path(&path)
                && !generic_container(&path)
                && seen.insert(path.to_string_lossy().to_lowercase())
            {
                roots.push(path);
            }
        }
    };
    if let Some(root) = project_root(current) {
        add(root);
    }
    for (path, _) in recent {
        add(project_root(Path::new(path)).unwrap_or_else(|| path.into()));
    }
    let mut containers = Vec::new();
    if let Some(root) = project_root(current) {
        if let Some(parent) = root.parent() {
            containers.push(parent.to_path_buf());
        }
    } else {
        containers.push(current.to_path_buf());
    }
    if let Some(home) = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")) {
        let home = PathBuf::from(home);
        for folder in [
            "Projects",
            "projects",
            "source/repos",
            "dev",
            "develop",
            "Documents",
        ] {
            containers.push(home.join(folder));
        }
    }
    if let Some(paths) = std::env::var_os("MEG_PROJECTS_DIR") {
        containers.extend(std::env::split_paths(&paths));
    }
    for container in containers {
        if project_marker(&container) {
            add(container.clone());
        }
        for child in list_dirs(&container).into_iter().take(128) {
            if project_marker(&child) {
                add(child);
            }
        }
    }
    roots.truncate(64);
    roots
}

/// Freebuff's model: `[{"path": "...", "lastOpened": 1794498356261}]`.
/// Sources, newest first: our ~/.meg store, then freebuff's own
/// ~/.config/manicode/recent-projects.json (so the picker opens with the
/// same projects the freebuff app shows).
fn load_recent() -> Vec<(String, i64)> {
    let mut out: Vec<(String, i64)> = load_recent_meg();
    if let Some(manicode) = std::env::var("USERPROFILE")
        .ok()
        .map(|home| {
            PathBuf::from(home)
                .join(".config")
                .join("manicode")
                .join("recent-projects.json")
        })
        .filter(|p| p.is_file())
    {
        if let Ok(text) = std::fs::read_to_string(&manicode) {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
                if let Some(arr) = v.as_array() {
                    for e in arr {
                        if let Some(path) = e.get("path").and_then(|p| p.as_str()) {
                            let ts = e.get("lastOpened").and_then(|t| t.as_i64()).unwrap_or(0);
                            if !out.iter().any(|(p, _)| p.eq_ignore_ascii_case(path)) {
                                out.push((path.to_string(), ts));
                            }
                        }
                    }
                }
            }
        }
    }
    out.sort_by_key(|a| std::cmp::Reverse(a.1));
    let mut seen = std::collections::HashSet::new();
    out = out
        .into_iter()
        .filter_map(|(path, time)| {
            let path = normalized_directory(Path::new(&path))?;
            if fixture_path(&path) || generic_container(&path) {
                return None;
            }
            let root = project_root(&path).unwrap_or(path);
            let label = crate::friendly_path(&root);
            seen.insert(label.to_lowercase()).then_some((label, time))
        })
        .collect();
    out.truncate(MAX_RECENT);
    out
}

fn load_recent_meg() -> Vec<(String, i64)> {
    let Some(dir) = meg_dir() else {
        return Vec::new();
    };
    // New: JSON store (freebuff-compatible).
    let json_path = dir.join(RECENT_JSON);
    if let Ok(text) = std::fs::read_to_string(&json_path) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
            let mut out: Vec<(String, i64)> = v
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|e| {
                            let path = e.get("path")?.as_str()?.to_string();
                            let ts = e.get("lastOpened").and_then(|t| t.as_i64()).unwrap_or(0);
                            Some((path, ts))
                        })
                        .collect()
                })
                .unwrap_or_default();
            out.sort_by_key(|a| std::cmp::Reverse(a.1));
            out.truncate(MAX_RECENT);
            if !out.is_empty() {
                return out;
            }
        }
    }
    // Legacy: our plain-text list.
    let txt_path = dir.join(LEGACY_TXT);
    if let Ok(text) = std::fs::read_to_string(&txt_path) {
        return text
            .lines()
            .filter(|l| !l.trim().is_empty())
            .take(MAX_RECENT)
            .map(|l| (l.to_string(), 0))
            .collect();
    }
    Vec::new()
}

fn save_recent(ws: &Path) {
    if fixture_path(ws) {
        return;
    }
    let Some(dir) = meg_dir() else { return };
    remember(&dir, ws, load_recent());
}
fn remember(dir: &Path, ws: &Path, mut recent: Vec<(String, i64)>) {
    let _ = std::fs::create_dir_all(dir);
    let path_str = normalized_directory(ws)
        .map(|p| crate::friendly_path(&p))
        .unwrap_or_else(|| crate::friendly_path(ws));
    recent.retain(|(p, _)| *p != path_str);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    recent.insert(0, (path_str, now));
    recent.truncate(MAX_RECENT);
    let arr: Vec<serde_json::Value> = recent
        .iter()
        .map(|(p, ts)| serde_json::json!({"path": p, "lastOpened": ts}))
        .collect();
    let _ = crate::session::atomic_json(&dir.join(RECENT_JSON), &arr);
}

/// Browsing accepts ordinary directories regardless of project markers or names.
fn browser_dirs(base: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(base) else {
        return Vec::new();
    };
    let mut dirs: Vec<_> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.is_dir()
                && p.file_name().is_some_and(|n| {
                    !n.to_string_lossy().starts_with('.')
                        && !matches!(
                            n.to_string_lossy().to_lowercase().as_str(),
                            "$recycle.bin" | "system volume information"
                        )
                })
        })
        .collect();
    dirs.sort_by_key(|p| p.file_name().map(|n| n.to_string_lossy().to_lowercase()));
    dirs
}
/// Project suggestions skip dependencies and system aliases; browsing does not.
fn list_dirs(base: &Path) -> Vec<PathBuf> {
    let skip = [
        "appdata",
        "application data",
        "local settings",
        "my documents",
        "cookies",
        "nethood",
        "printhood",
        "recent",
        "sendto",
        "start menu",
        "templates",
        "node_modules",
        "__pycache__",
        ".git",
        "target",
        ".venv",
        ".meg",
        "chroma_db",
        "knowledge_db",
        "$recycle.bin",
        "system volume information",
    ];
    let Ok(entries) = std::fs::read_dir(base) else {
        return Vec::new();
    };
    let mut dirs: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .map(|e| e.path())
        .filter(|p| {
            let name = p
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            !name.starts_with('.') && !skip.contains(&name.to_lowercase().as_str())
        })
        .collect();
    dirs.sort_by_key(|p| {
        p.file_name()
            .map(|n| n.to_string_lossy().to_lowercase())
            .unwrap_or_default()
    });
    dirs
}

fn short_home(path: &str) -> String {
    if let Ok(home) = std::env::var("USERPROFILE") {
        if path.to_lowercase().starts_with(&home.to_lowercase()) {
            return format!("~{}", &path[home.len()..]);
        }
    }
    path.to_string()
}

/// Full-screen selector render — the freebuff startup replica.
pub fn render_workspace_picker(f: &mut Frame, pal: &Palette, wp: &WorkspacePicker) {
    if wp.fullscreen {
        return render_workspace_startup(f, pal, wp);
    }
    use ratatui::widgets::{Block, BorderType, Borders, Clear};
    let screen = f.area();
    if screen.width < 8 || screen.height < 5 {
        return;
    }
    let width = screen.width.saturating_sub(4).min(84);
    let height = screen.height.saturating_sub(2).min(18);
    let area = Rect::new(
        screen.x + (screen.width - width) / 2,
        screen.y + (screen.height - height) / 2,
        width,
        height,
    );
    f.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title(" Workspace ")
        .style(Style::default().bg(pal.chrome_bg))
        .border_style(Style::default().fg(pal.accent));
    let inner = block.inner(area);
    f.render_widget(block, area);
    let mut lines = vec![
        Line::styled(
            format!(
                " Current: {}",
                crate::fit_text(
                    &wp.fallback.to_string_lossy(),
                    inner.width.saturating_sub(11) as usize
                )
            ),
            Style::default().fg(pal.chrome_fg),
        ),
        Line::styled(
            format!(
                " Path: {}",
                crate::fit_text(&wp.query, inner.width.saturating_sub(8) as usize)
            ),
            Style::default().fg(pal.accent),
        ),
        Line::raw(""),
    ];
    let capacity = inner.height.saturating_sub(5) as usize;
    let start = wp.selected.saturating_sub(capacity.saturating_sub(1));
    let paths: Vec<String> = match wp.phase {
        PickPhase::Projects => wp
            .projects
            .iter()
            .map(|p| crate::friendly_path(p))
            .collect(),
        PickPhase::Recent => wp.recent.iter().map(|(p, _)| p.clone()).collect(),
        PickPhase::Browse => wp
            .dirs
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect(),
    };
    if paths.is_empty() {
        lines.push(Line::styled(
            " Type an existing directory; Enter opens it.",
            Style::default().fg(pal.chrome_fg),
        ));
    }
    for (index, path) in paths.iter().enumerate().skip(start).take(capacity) {
        let selected = index == wp.selected;
        lines.push(Line::styled(
            format!(
                " {} {}",
                if selected { "›" } else { " " },
                crate::fit_text(&short_home(path), inner.width.saturating_sub(4) as usize)
            ),
            if selected {
                Style::default()
                    .fg(pal.chrome_bg)
                    .bg(pal.accent)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(pal.viewport_fg)
            },
        ));
    }
    f.render_widget(Paragraph::new(lines), inner);
    f.render_widget(
        Paragraph::new(crate::fit_text(
            " ↑↓ select · Enter open · Tab descend · Esc cancel",
            inner.width as usize,
        ))
        .style(Style::default().fg(pal.chrome_fg)),
        Rect::new(inner.x, inner.bottom().saturating_sub(1), inner.width, 1),
    );
}

fn render_workspace_startup(f: &mut Frame, pal: &Palette, wp: &WorkspacePicker) {
    use ratatui::{
        text::Span,
        widgets::{Block, BorderType, Borders},
    };
    let screen = f.area();
    f.render_widget(
        Block::default().style(Style::default().bg(pal.chrome_bg)),
        screen,
    );
    if screen.width < 24 || screen.height < 10 {
        return;
    }
    let width = screen.width.saturating_sub(6).min(72);
    let x = screen.x + (screen.width - width) / 2;
    let logo = [
        "███▄ ▄███  ███████  ███████",
        "██ ▀█▀ ██  ██       ██",
        "██     ██  █████    ██  ███",
        "██     ██  ██       ██   ██",
        "██     ██  ███████  ███████",
    ];
    let top = screen.y + (screen.height / 8).min(5);
    if screen.height >= 26 {
        for (i, line) in logo.iter().enumerate() {
            f.render_widget(
                Paragraph::new(*line).style(
                    Style::default()
                        .fg(pal.titlebar_fg)
                        .add_modifier(Modifier::BOLD),
                ),
                Rect::new(
                    screen.x + screen.width.saturating_sub(27) / 2,
                    top + i as u16,
                    screen.width.min(27),
                    1,
                ),
            );
        }
    } else {
        f.render_widget(
            Paragraph::new("MEG").style(
                Style::default()
                    .fg(pal.titlebar_fg)
                    .add_modifier(Modifier::BOLD),
            ),
            Rect::new(x, top, width, 1),
        );
    }
    let heading = top + if screen.height >= 26 { 7 } else { 2 };
    f.render_widget(
        Paragraph::new("Open any folder to work in, or create a new one.")
            .style(Style::default().fg(pal.chrome_fg)),
        Rect::new(x, heading, width, 1),
    );
    let y = heading + 2;
    let path_hint = crate::friendly_path(wp.dirs.first().unwrap_or(&wp.directory));
    f.render_widget(
        Paragraph::new(format!(
            "{} {}",
            if wp.create_mode {
                "New folder:"
            } else {
                "Path:"
            },
            if wp.query.is_empty() && !wp.create_mode {
                if wp.phase == PickPhase::Projects {
                    "Project roots · type a path or Tab to browse"
                } else {
                    &path_hint
                }
            } else {
                &wp.query
            }
        ))
        .style(Style::default().fg(pal.accent)),
        Rect::new(x, y, width, 1),
    );
    let available = screen.bottom().saturating_sub(y + 8);
    let height = available
        .clamp(3, 12)
        .min(screen.bottom().saturating_sub(y + 3));
    let area = Rect::new(x, y + 2, width, height);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title(format!(
            " {} ",
            crate::fit_text(
                &if wp.phase == PickPhase::Projects {
                    "Projects".into()
                } else {
                    crate::friendly_path(wp.dirs.first().unwrap_or(&wp.directory))
                },
                width.saturating_sub(5) as usize
            )
        ))
        .style(Style::default().fg(pal.panel_border).bg(pal.chrome_bg));
    let inner = block.inner(area);
    f.render_widget(block, area);
    let start = wp
        .selected
        .saturating_sub(inner.height.saturating_sub(1) as usize);
    let pool = if wp.phase == PickPhase::Projects {
        &wp.projects
    } else {
        &wp.dirs
    };
    let lines = pool
        .iter()
        .enumerate()
        .skip(start)
        .take(inner.height as usize)
        .map(|(i, path)| {
            let selected = i == wp.selected;
            let label = if wp.phase == PickPhase::Projects {
                short_home(&crate::friendly_path(path))
            } else if i == 0 {
                "Open this folder".into()
            } else if i == 1 && path == wp.dirs[0].parent().unwrap_or(path) {
                "..".into()
            } else {
                path.file_name()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| crate::friendly_path(path))
            };
            Line::from(Span::styled(
                format!(
                    " {} {}",
                    if selected { "›" } else { "▸" },
                    crate::fit_text(&label, inner.width.saturating_sub(4) as usize)
                ),
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
            ))
        })
        .collect::<Vec<_>>();
    f.render_widget(Paragraph::new(lines), inner);
    if pool.is_empty() {
        f.render_widget(
            Paragraph::new(" No projects found. Paste a project path or press Tab to browse.")
                .style(Style::default().fg(pal.chrome_fg)),
            inner,
        );
    }
    let recent_y = area.bottom() + 1;
    if recent_y + 2 < screen.bottom() {
        let recent = wp
            .recent
            .iter()
            .take(2)
            .enumerate()
            .map(|(i, (p, _))| format!("Alt+{} {}", i + 1, short_home(p)))
            .collect::<Vec<_>>()
            .join("   ");
        f.render_widget(
            Paragraph::new(crate::fit_text(
                &format!("Recent: {recent}"),
                width as usize,
            ))
            .style(Style::default().fg(pal.chrome_fg)),
            Rect::new(x, recent_y, width, 1),
        );
    }
    f.render_widget(
        Paragraph::new(crate::fit_text(
            if wp.create_mode {
                "Enter create and open · Esc cancel creation"
            } else {
                "↑↓ · Tab browse · ← up · Enter open · Ctrl+N new · Ctrl+P projects"
            },
            width as usize,
        ))
        .style(Style::default().fg(pal.chrome_fg)),
        Rect::new(x, screen.bottom() - 2, width, 1),
    );
    if let Some(error) = &wp.error {
        f.render_widget(
            Paragraph::new(crate::fit_text(error, width as usize))
                .style(Style::default().fg(ratatui::style::Color::Red)),
            Rect::new(x, screen.bottom() - 4, width, 1),
        );
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typing_switches_to_browse_and_lists_dirs() {
        let root = std::env::temp_dir().join(format!(
            "meg-production-test-browser-{}",
            uuid::Uuid::new_v4()
        ));
        let child = root.join("plain folder");
        std::fs::create_dir_all(&child).unwrap();
        let mut wp = WorkspacePicker::open(root.clone());
        assert_eq!(wp.phase, PickPhase::Recent);
        wp.paste(&root.to_string_lossy());
        assert_eq!(wp.phase, PickPhase::Browse);
        assert_eq!(wp.dirs, vec![child.clone()]);
        wp.descend();
        assert_eq!(wp.current_directory(), child);
        assert!(wp.dirs.is_empty()); // An empty folder is still a valid workspace.
        assert_eq!(root.parent().unwrap(), std::env::temp_dir());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn commit_saves_recent_json() {
        let dir = std::env::temp_dir().join(format!(
            "meg-production-test-recents-{}",
            uuid::Uuid::new_v4()
        ));
        let json_path = dir.join(RECENT_JSON);
        let probe = dir.join("project");
        let _ = std::fs::create_dir_all(&probe);
        let mut wp = WorkspacePicker::open(probe.clone());
        wp.phase = PickPhase::Browse;
        wp.query = probe.to_string_lossy().to_string();
        wp.dirs = vec![probe.clone()];
        wp.selected = 0;
        let picked = wp
            .selected_workspace()
            .expect("selection returns the workspace");
        remember(&dir, &picked, Vec::new());
        assert_eq!(probe.is_dir(), true);
        let after = std::fs::read_to_string(&json_path).unwrap_or_default();
        assert!(
            after.contains("project"),
            "committed workspace must land in the freebuff-format store"
        );
    }

    #[test]
    fn startup_browses_folders_and_keeps_project_suggestions_separate() {
        let container =
            std::env::temp_dir().join(format!("meg-workspace-roots-{}", uuid::Uuid::new_v4()));
        let root = container.join("main-project");
        let nested = root.join("packages/library/src");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::create_dir(root.join(".git")).unwrap();
        std::fs::write(root.join("packages/library/Cargo.toml"), "[package]").unwrap();
        let sibling = container.join("other-project");
        std::fs::create_dir_all(sibling.join(".git")).unwrap();
        let picker = WorkspacePicker::open_startup(nested.clone());
        assert_eq!(picker.phase, PickPhase::Browse);
        assert_eq!(picker.selected_workspace().unwrap(), nested);
        assert!(picker
            .projects
            .contains(&normalized_directory(&sibling).unwrap()));
        assert!(!picker
            .projects
            .contains(&normalized_directory(&nested).unwrap()));
        let mut browser = picker;
        browser.descend();
        assert_eq!(browser.phase, PickPhase::Browse);
        assert_eq!(browser.dirs.first().unwrap(), &nested);
    }
    #[test]
    fn fixture_recents_and_extended_path_prefixes_are_normalized() {
        let dir = std::env::temp_dir().join("meg-production-test-old");
        assert!(fixture_path(&dir));
        let normalized = normalized_directory(&std::env::temp_dir()).unwrap();
        assert!(!normalized.to_string_lossy().starts_with(r"\\?\"));
    }
}
