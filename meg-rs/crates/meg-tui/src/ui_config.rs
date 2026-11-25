//! Pluggable UI frontend trait & user-editable `~/.meg/ui.json` configuration.
//! Users can switch between `/ui classic` and `/ui pro`, override colors/layout
//! via `~/.meg/ui.json` (or `<workspace>/.meg/ui.json`) without recompiling, or
//! implement `UiFrontend` directly in custom Rust builds.

use crate::{Palette, WorkstationState};
use ratatui::{style::Color, Frame};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UiMode {
    #[default]
    Classic,
    Pro,
}

impl UiMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Classic => "classic",
            Self::Pro => "pro",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "classic" | "legacy" | "default" => Some(Self::Classic),
            "pro" | "titanium" | "modern" => Some(Self::Pro),
            _ => None,
        }
    }
}

/// Pluggable Rust TUI frontend contract.
pub trait UiFrontend {
    fn name(&self) -> &'static str;
    fn draw(&self, f: &mut Frame, pal: &Palette, st: &WorkstationState);
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PaletteOverrides {
    pub chrome_bg: Option<String>,
    pub chrome_fg: Option<String>,
    pub titlebar_bg: Option<String>,
    pub titlebar_fg: Option<String>,
    pub panel_border: Option<String>,
    pub panel_title: Option<String>,
    pub viewport_bg: Option<String>,
    pub viewport_fg: Option<String>,
    pub accent: Option<String>,
    pub success: Option<String>,
    pub error: Option<String>,
    pub warning: Option<String>,
}

/// Parse `#RRGGBB` hex color strings into Ratatui `Color::Rgb`.
pub fn parse_hex_color(raw: &str) -> Option<Color> {
    let hex = raw.trim().trim_start_matches('#');
    if hex.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
    Some(Color::Rgb(r, g, b))
}

impl PaletteOverrides {
    pub fn apply(&self, base: Palette) -> Palette {
        let mut p = base;
        if let Some(c) = self.chrome_bg.as_deref().and_then(parse_hex_color) {
            p.chrome_bg = c;
        }
        if let Some(c) = self.chrome_fg.as_deref().and_then(parse_hex_color) {
            p.chrome_fg = c;
        }
        if let Some(c) = self.titlebar_bg.as_deref().and_then(parse_hex_color) {
            p.titlebar_bg = c;
        }
        if let Some(c) = self.titlebar_fg.as_deref().and_then(parse_hex_color) {
            p.titlebar_fg = c;
        }
        if let Some(c) = self.panel_border.as_deref().and_then(parse_hex_color) {
            p.panel_border = c;
        }
        if let Some(c) = self.panel_title.as_deref().and_then(parse_hex_color) {
            p.panel_title = c;
        }
        if let Some(c) = self.viewport_bg.as_deref().and_then(parse_hex_color) {
            p.viewport_bg = c;
        }
        if let Some(c) = self.viewport_fg.as_deref().and_then(parse_hex_color) {
            p.viewport_fg = c;
        }
        if let Some(c) = self.accent.as_deref().and_then(parse_hex_color) {
            p.accent = c;
        }
        if let Some(c) = self.success.as_deref().and_then(parse_hex_color) {
            p.success = c;
        }
        if let Some(c) = self.error.as_deref().and_then(parse_hex_color) {
            p.error = c;
        }
        if let Some(c) = self.warning.as_deref().and_then(parse_hex_color) {
            p.warning = c;
        }
        p
    }
}

/// Per-role model assignment (Antigravity-style): assign any model to any job,
/// or leave all None for single-model mode.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RoleModels {
    pub planner: Option<String>,
    pub builder: Option<String>,
    pub reviewer: Option<String>,
    pub researcher: Option<String>,
}

fn default_true() -> bool {
    true
}

/// User-editable UI & orchestration preferences (`~/.meg/ui.json` or `.meg/ui.json`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct UserUiConfig {
    pub ui_mode: UiMode,
    pub theme_name: Option<String>,
    #[serde(default = "default_true")]
    pub show_sidebar: bool,
    #[serde(default = "default_true")]
    pub compact_tool_cards: bool,
    #[serde(default = "default_true")]
    pub sync_csi_2026: bool,
    pub pair_mode: Option<String>,
    pub executor_model: Option<String>,
    pub reviewer_model: Option<String>,
    pub roles: RoleModels,
    pub colors: PaletteOverrides,
}

impl Default for UserUiConfig {
    fn default() -> Self {
        Self {
            ui_mode: UiMode::Classic,
            theme_name: None,
            show_sidebar: true,
            compact_tool_cards: true,
            sync_csi_2026: true,
            pair_mode: None,
            executor_model: None,
            reviewer_model: None,
            roles: RoleModels::default(),
            colors: PaletteOverrides::default(),
        }
    }
}

impl UserUiConfig {
    pub fn global_path() -> Option<PathBuf> {
        let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"))?;
        Some(PathBuf::from(home).join(".meg").join("ui.json"))
    }

    pub fn workspace_path(workspace: &Path) -> PathBuf {
        workspace.join(".meg").join("ui.json")
    }

    /// Load workspace `.meg/ui.json` if present, otherwise `~/.meg/ui.json`, else defaults.
    pub fn load(workspace: Option<&Path>) -> (Self, Option<PathBuf>) {
        if let Some(ws) = workspace {
            let ws_file = Self::workspace_path(ws);
            if let Ok(raw) = std::fs::read_to_string(&ws_file) {
                if let Ok(cfg) = serde_json::from_str::<Self>(&raw) {
                    return (cfg, Some(ws_file));
                }
            }
        }
        if let Some(global_file) = Self::global_path() {
            if let Ok(raw) = std::fs::read_to_string(&global_file) {
                if let Ok(cfg) = serde_json::from_str::<Self>(&raw) {
                    return (cfg, Some(global_file));
                }
            }
        }
        (Self::default(), None)
    }

    /// Persist preferences to a specific path (creating parent directories if needed).
    pub fn save_to(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, format!("{json}\n"))?;
        Ok(())
    }
}