//! Pure-Rust workspace extensions: compile-time embedded builtin skills,
//! disk skills with DDIPE scanning, local plugin registry, and trusted MCP
//! server lifecycle.

use super::mcp::{McpConfig, McpHost};
use super::security::{scan_skill_supply_chain, sha256_hex};
use crate::CancellationToken;
use anyhow::{anyhow, bail, Context, Result};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

const EMBEDDED_BUILTIN_SKILLS: &[(&str, &str)] = &[
    ("compiler-debugging", include_str!("../../../../../meg-plane/skills/compiler-debugging/SKILL.md")),
    ("diagnosing-bugs", include_str!("../../../../../meg-plane/skills/diagnosing-bugs/SKILL.md")),
    ("git-recovery", include_str!("../../../../../meg-plane/skills/git-recovery/SKILL.md")),
    ("pack-appdev", include_str!("../../../../../meg-plane/skills/pack-appdev/SKILL.md")),
    ("pack-daily", include_str!("../../../../../meg-plane/skills/pack-daily/SKILL.md")),
    ("pack-games", include_str!("../../../../../meg-plane/skills/pack-games/SKILL.md")),
    ("pack-systems", include_str!("../../../../../meg-plane/skills/pack-systems/SKILL.md")),
    ("pack-vibe", include_str!("../../../../../meg-plane/skills/pack-vibe/SKILL.md")),
    ("pack-visual-motion", include_str!("../../../../../meg-plane/skills/pack-visual-motion/SKILL.md")),
    ("tdd-discipline", include_str!("../../../../../meg-plane/skills/tdd-discipline/SKILL.md")),
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptResource {
    pub name: String,
    pub description: String,
    pub body: String,
    pub source: String,
    #[serde(rename = "ddipeSafe", skip_serializing_if = "Option::is_none")]
    pub ddipe_safe: Option<bool>,
    #[serde(rename = "ddipeFindings", skip_serializing_if = "Option::is_none")]
    pub ddipe_findings: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginEntry {
    pub name: String,
    pub path: String,
    pub enabled: bool,
}

pub fn config_root() -> PathBuf {
    if let Ok(dir) = std::env::var("MEG_CONFIG_DIR") {
        if !dir.trim().is_empty() {
            return PathBuf::from(dir);
        }
    }
    if let Ok(appdata) = std::env::var("APPDATA") {
        if !appdata.trim().is_empty() {
            return PathBuf::from(appdata).join("MEG");
        }
    }
    home_dir().join(".config").join("meg")
}

fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn read_json_bounded(file: &Path, fallback: Value) -> Result<Value> {
    if !file.exists() {
        return Ok(fallback);
    }
    let meta = fs::metadata(file)?;
    if meta.len() > 1024 * 1024 {
        let name = file.file_name().and_then(|n| n.to_str()).unwrap_or("file");
        bail!("Configuration exceeds 1 MiB: {name}");
    }
    let raw = fs::read_to_string(file)?;
    Ok(serde_json::from_str(&raw)?)
}

fn atomic_write_json(file: &Path, value: &Value) -> Result<()> {
    if let Some(parent) = file.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = file.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    let payload = serde_json::to_string_pretty(value)?;
    let write_res = (|| -> Result<()> {
        let mut f = OpenOptions::new().write(true).create_new(true).open(&tmp)?;
        f.write_all(payload.as_bytes())?;
        f.flush()?;
        drop(f);
        fs::rename(&tmp, file)?;
        Ok(())
    })();
    if write_res.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    write_res
}

fn inside(root: &Path, target: &Path) -> Result<PathBuf> {
    let real_root = fs::canonicalize(root)?;
    let real_target = fs::canonicalize(target)?;
    if !real_target.starts_with(&real_root) {
        bail!("Plugin resource escapes its directory");
    }
    Ok(real_target)
}

fn resource_name_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^[a-zA-Z0-9][a-zA-Z0-9._-]{0,79}$").unwrap())
}

fn plugin_name_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^[a-zA-Z0-9][a-zA-Z0-9_-]{0,79}$").unwrap())
}

fn mcp_server_name_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^[a-zA-Z0-9_-]{1,80}$").unwrap())
}

fn parse_prompt_resource(text: &str, source: &str, explicit_name: Option<&str>, fallback_name: &str) -> Result<PromptResource> {
    if text.len() > 64 * 1024 {
        bail!("Prompt resource exceeds 64 KiB: {source}");
    }
    let normalized = text.replace("\r\n", "\n");
    let (front, body) = if let Some(rest) = normalized.strip_prefix("---\n") {
        if let Some(end_idx) = rest.find("\n---") {
            let fm = &rest[..end_idx];
            let after = &rest[end_idx + 4..];
            let after = after.strip_prefix('\n').unwrap_or(after);
            (Some(fm), after.to_string())
        } else {
            (None, normalized)
        }
    } else {
        (None, normalized)
    };

    let field = |key: &str| -> Option<String> {
        let fm = front?;
        let prefix = format!("{key}:");
        for line in fm.lines() {
            if let Some(val) = line.strip_prefix(&prefix) {
                let v = val.trim().trim_matches(|c| c == '\'' || c == '"');
                if !v.is_empty() {
                    return Some(v.to_string());
                }
            }
        }
        None
    };

    let id = explicit_name
        .map(ToString::to_string)
        .or_else(|| field("name"))
        .unwrap_or_else(|| fallback_name.to_string());
    if !resource_name_re().is_match(&id) {
        bail!("Invalid prompt resource name: {id}");
    }
    let description = field("description").unwrap_or_else(|| id.clone());
    Ok(PromptResource {
        name: id,
        description,
        body,
        source: source.to_string(),
        ddipe_safe: None,
        ddipe_findings: None,
    })
}

fn read_resource_file(file: &Path, explicit_name: Option<&str>) -> Result<PromptResource> {
    let meta = fs::metadata(file)?;
    if meta.len() > 64 * 1024 {
        let fname = file.file_name().and_then(|n| n.to_str()).unwrap_or("file");
        bail!("Prompt resource exceeds 64 KiB: {fname}");
    }
    let text = fs::read_to_string(file)?;
    let fallback = file
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .unwrap_or("skill");
    parse_prompt_resource(&text, &file.display().to_string(), explicit_name, fallback)
}

fn load_skills_from_dir(dir: &Path) -> Result<Vec<PromptResource>> {
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut entries: Vec<_> = fs::read_dir(dir)?.filter_map(|e| e.ok()).collect();
    entries.sort_by_key(|e| e.file_name());
    let mut out = Vec::new();
    for entry in entries.into_iter().take(128) {
        let path = entry.path();
        let skill_md = path.join("SKILL.md");
        if path.is_dir() && skill_md.is_file() {
            let safe_path = inside(dir, &skill_md)?;
            out.push(read_resource_file(&safe_path, None)?);
        }
    }
    Ok(out)
}

pub struct WorkspaceExtensions {
    pub workspace: PathBuf,
    pub host: McpHost,
    registry_file: PathBuf,
    plugin_servers: Mutex<HashMap<String, HashSet<String>>>,
}

impl WorkspaceExtensions {
    pub fn new(workspace: &Path) -> Result<Self> {
        let real = fs::canonicalize(workspace).context("Workspace directory is unavailable")?;
        if !real.is_dir() {
            bail!("Workspace directory is unavailable");
        }
        let registry_file = real.join(".meg").join("plugins.json");
        Ok(Self {
            workspace: real,
            host: McpHost::new(),
            registry_file,
            plugin_servers: Mutex::new(HashMap::new()),
        })
    }

    fn plugins(&self) -> Result<Vec<PluginEntry>> {
        let val = read_json_bounded(&self.registry_file, json!([]))?;
        let list: Vec<PluginEntry> =
            serde_json::from_value(val).map_err(|_| anyhow!("Invalid plugin registry"))?;
        if list.len() > 64 {
            bail!("Invalid plugin registry");
        }
        Ok(list)
    }

    fn manifest(&self, directory: &Path) -> Result<Value> {
        let candidates = [
            directory.join("meg-plugin.json"),
            directory.join(".claude-plugin").join("plugin.json"),
            directory.join("plugin.json"),
        ];
        let found = candidates
            .into_iter()
            .find(|p| p.is_file())
            .ok_or_else(|| anyhow!("Plugin needs meg-plugin.json or .claude-plugin/plugin.json"))?;
        let safe = inside(directory, &found)?;
        read_json_bounded(&safe, json!({}))
    }

    fn configs(&self) -> Result<BTreeMap<String, McpConfig>> {
        let mut result: BTreeMap<String, McpConfig> = BTreeMap::new();
        let files = [
            config_root().join("mcp.json"),
            self.workspace.join(".mcp.json"),
            self.workspace.join("mcp_config.json"),
            self.workspace.join(".meg").join("mcp.json"),
        ];
        for file in files {
            let val = read_json_bounded(&file, json!({}))?;
            if let Some(obj) = val.get("mcpServers").and_then(Value::as_object) {
                for (name, cfg_val) in obj {
                    let cfg: McpConfig = serde_json::from_value(cfg_val.clone())?;
                    result.insert(name.clone(), cfg);
                }
            }
        }
        for plugin in self.plugins()?.into_iter().filter(|p| p.enabled) {
            let Ok(manifest) = self.manifest(Path::new(&plugin.path)) else {
                continue;
            };
            let mut owned = HashSet::new();
            if let Some(servers) = manifest.get("mcpServers").and_then(Value::as_object) {
                for (name, cfg_val) in servers {
                    let server = format!("{}_{name}", plugin.name);
                    if result.contains_key(&server) {
                        bail!("Duplicate MCP server name: {server}");
                    }
                    let cfg: McpConfig = serde_json::from_value(cfg_val.clone())?;
                    result.insert(server.clone(), cfg);
                    owned.insert(server);
                }
            }
            self.plugin_servers.lock().unwrap().insert(plugin.name, owned);
        }
        for name in result.keys() {
            if !mcp_server_name_re().is_match(name) {
                bail!("MCP server names must use letters, numbers, '-' or '_'");
            }
        }
        Ok(result)
    }

    fn resolve_mcp_artifact_path(&self, raw: &str, search_path: bool) -> Option<PathBuf> {
        let trimmed = raw.trim();
        if trimmed.is_empty()
            || trimmed.starts_with("http://")
            || trimmed.starts_with("https://")
        {
            return None;
        }
        let p = Path::new(trimmed);
        if p.is_absolute() {
            if p.is_file() {
                return Some(p.to_path_buf());
            }
            if search_path {
                return resolve_windows_pathext(p);
            }
            return None;
        }
        let has_sep = trimmed.contains('/') || trimmed.contains('\\');
        if has_sep || !search_path {
            let ws_candidate = self.workspace.join(p);
            if ws_candidate.is_file() {
                return Some(ws_candidate);
            }
            if search_path {
                if let Some(found) = resolve_windows_pathext(&ws_candidate) {
                    return Some(found);
                }
            }
            if !search_path {
                return None;
            }
        }
        if search_path {
            if let Some(path_var) = std::env::var_os("PATH") {
                for dir in std::env::split_paths(&path_var) {
                    let candidate = dir.join(trimmed);
                    if candidate.is_file() {
                        return Some(candidate);
                    }
                    if let Some(found) = resolve_windows_pathext(&candidate) {
                        return Some(found);
                    }
                }
            }
        }
        None
    }

    pub(crate) fn mcp_artifact_hashes(&self, config: &McpConfig) -> Vec<String> {
        let mut candidates: Vec<(&str, bool)> = Vec::new();
        if let Some(ref cmd) = config.command {
            candidates.push((cmd.as_str(), true));
        }
        if let Some(ref args) = config.args {
            for arg in args.iter().take(16) {
                if !arg.starts_with('-') {
                    candidates.push((arg.as_str(), false));
                } else if let Some((_, val)) = arg.split_once('=') {
                    if !val.is_empty() {
                        candidates.push((val, false));
                    }
                }
            }
        }
        let mut pinned = std::collections::BTreeSet::new();
        for (raw, search_path) in candidates {
            if let Some(resolved) = self.resolve_mcp_artifact_path(raw, search_path) {
                let canon = fs::canonicalize(&resolved).unwrap_or(resolved);
                if let Some(digest) = hash_mcp_artifact_file(&canon) {
                    pinned.insert(format!("{}:{}", canon.display(), digest));
                }
            }
        }
        pinned.into_iter().collect()
    }

    fn fingerprint(&self, name: &str, config: &McpConfig) -> String {
        let artifact_hashes = self.mcp_artifact_hashes(config);
        let payload = serde_json::to_string(&json!([
            self.workspace.display().to_string(),
            name,
            config,
            artifact_hashes
        ]))
        .unwrap_or_default();
        sha256_hex(payload.as_bytes())
    }

    fn trust_key(&self, name: &str) -> String {
        let mut raw = self.workspace.display().to_string().into_bytes();
        raw.push(0);
        raw.extend_from_slice(name.as_bytes());
        sha256_hex(&raw)
    }

    fn trust_store_lock() -> &'static std::sync::Mutex<()> {
        static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
        LOCK.get_or_init(|| std::sync::Mutex::new(()))
    }

    fn set_trust(&self, name: &str, config: &McpConfig, enabled: bool) -> Result<()> {
        let _guard = Self::trust_store_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let file = config_root().join("mcp-trust.json");
        let mut value = read_json_bounded(&file, json!({}))?;
        let key = self.trust_key(name);
        if let Some(obj) = value.as_object_mut() {
            if enabled {
                obj.insert(key, Value::String(self.fingerprint(name, config)));
            } else {
                obj.remove(&key);
            }
        }
        atomic_write_json(&file, &value)
    }

    pub fn reconnect_trusted(&self) {
        let trusted = {
            let _guard = Self::trust_store_lock()
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            let file = config_root().join("mcp-trust.json");
            match read_json_bounded(&file, json!({})) {
                Ok(v) => v,
                Err(_) => return,
            }
        };
        let Ok(configs) = self.configs() else {
            return;
        };
        let cancel = CancellationToken::default();
        for (name, cfg) in configs {
            if cfg.disabled == Some(true) || self.host.status(&name).connected {
                continue;
            }
            let key = self.trust_key(&name);
            let fp = self.fingerprint(&name, &cfg);
            if trusted.get(&key).and_then(Value::as_str) == Some(fp.as_str()) {
                let _ = self.host.connect(&name, &cfg, Some(&self.workspace), &cancel);
            }
        }
    }

    pub fn catalog(&self) -> Result<Value> {
        let mut by_name: BTreeMap<String, PromptResource> = BTreeMap::new();
        let mut commands: Vec<PromptResource> = Vec::new();
        let mut used: usize = 0;

        // 1. Embedded compile-time builtin skills (always available even as a lone single binary)
        for &(folder_name, content) in EMBEDDED_BUILTIN_SKILLS {
            if let Ok(skill) = parse_prompt_resource(
                content,
                &format!("builtin://{folder_name}/SKILL.md"),
                None,
                folder_name,
            ) {
                used += skill.body.len();
                by_name.insert(skill.name.clone(), skill);
            }
        }

        // 2. Disk skill layers (later layers override earlier ones by name)
        let mut directories: Vec<PathBuf> = Vec::new();
        if let Ok(builtin) = std::env::var("MEG_BUILTIN_SKILLS") {
            if !builtin.trim().is_empty() {
                directories.push(PathBuf::from(builtin));
            }
        }
        directories.push(config_root().join("skills"));
        if let Ok(user_dir) = std::env::var("MEG_USER_SKILLS_DIR") {
            if !user_dir.trim().is_empty() {
                directories.push(PathBuf::from(user_dir));
            }
        } else {
            directories.push(home_dir().join(".agents").join("skills"));
        }
        directories.push(self.workspace.join(".claude").join("skills"));
        directories.push(self.workspace.join(".agents").join("skills"));
        directories.push(self.workspace.join(".meg").join("skills"));

        for dir in directories {
            for skill in load_skills_from_dir(&dir)? {
                used += skill.body.len();
                if used > 1024 * 1024 {
                    bail!("Skill catalog exceeds 1 MiB");
                }
                by_name.insert(skill.name.clone(), skill);
            }
        }

        // 3. Enabled plugins (skills + commands)
        let mut plugin_rows = Vec::new();
        for plugin in self.plugins()? {
            let mut err_msg: Option<String> = None;
            let pdir = PathBuf::from(&plugin.path);
            match self.manifest(&pdir) {
                Ok(manifest) => {
                    if plugin.enabled {
                        let folders: Vec<String> = manifest
                            .get("skills")
                            .and_then(Value::as_array)
                            .map(|arr| {
                                arr.iter()
                                    .filter_map(|v| v.as_str().map(ToString::to_string))
                                    .collect()
                            })
                            .unwrap_or_else(|| vec!["skills".into()]);
                        for folder in folders {
                            let full = pdir.join(&folder);
                            if full.exists() {
                                if let Ok(safe_dir) = inside(&pdir, &full) {
                                    if let Ok(list) = load_skills_from_dir(&safe_dir) {
                                        for mut skill in list {
                                            let namespaced = format!("{}.{}", plugin.name, skill.name);
                                            skill.name = namespaced.clone();
                                            by_name.insert(namespaced, skill);
                                        }
                                    }
                                }
                            }
                        }
                        let cmd_dir = pdir.join("commands");
                        if cmd_dir.is_dir() {
                            if let Ok(entries) = fs::read_dir(&cmd_dir) {
                                let mut files: Vec<_> = entries
                                    .filter_map(|e| e.ok())
                                    .map(|e| e.path())
                                    .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("md"))
                                    .collect();
                                files.sort();
                                for file in files.into_iter().take(64) {
                                    if let Ok(safe_file) = inside(&pdir, &file) {
                                        let stem = file
                                            .file_stem()
                                            .and_then(|s| s.to_str())
                                            .unwrap_or("cmd");
                                        let cname = format!("{}.{stem}", plugin.name);
                                        if let Ok(res) = read_resource_file(&safe_file, Some(&cname)) {
                                            commands.push(res);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    err_msg = Some(e.to_string());
                }
            }
            let mut row = serde_json::to_value(&plugin)?;
            if let Some(e) = err_msg {
                row["error"] = Value::String(e);
            }
            plugin_rows.push(row);
        }

        let total_bytes: usize = by_name
            .values()
            .chain(commands.iter())
            .map(|r| r.body.len())
            .sum();
        if total_bytes > 1024 * 1024 {
            bail!("Combined skill and plugin prompt catalog exceeds 1 MiB");
        }

        // 4. Annotate every skill with DDIPE scan result
        let scanned_skills: Vec<PromptResource> = by_name
            .into_values()
            .map(|mut skill| {
                let scan = scan_skill_supply_chain(&skill.body, &skill.source);
                skill.ddipe_safe = Some(scan.safe);
                skill.ddipe_findings =
                    Some(scan.findings.into_iter().map(|f| f.rule).collect());
                skill
            })
            .collect();

        // 5. MCP server status + tools
        let configs = self.configs()?;
        let servers: Vec<Value> = configs
            .iter()
            .map(|(name, cfg)| {
                let transport = if cfg.url.is_some() {
                    cfg.transport_type.as_deref().unwrap_or("http")
                } else {
                    "stdio"
                };
                let st = self.host.status(name);
                let mut obj = json!({
                    "name": name,
                    "transport": transport,
                    "disabled": cfg.disabled == Some(true),
                    "connected": st.connected,
                    "connecting": st.connecting,
                });
                if let Some(err) = st.error {
                    obj["error"] = Value::String(err);
                }
                obj
            })
            .collect();

        Ok(json!({
            "skills": scanned_skills,
            "commands": commands,
            "plugins": plugin_rows,
            "servers": servers,
            "tools": self.host.all_tools(),
        }))
    }

    pub fn read_skill(&self, name: &str) -> Result<Value> {
        let cat = self.catalog()?;
        let skill = cat["skills"]
            .as_array()
            .and_then(|arr| arr.iter().find(|s| s["name"].as_str() == Some(name)))
            .cloned()
            .ok_or_else(|| anyhow!("Unknown skill"))?;
        let body = skill["body"].as_str().unwrap_or("");
        let source = skill["source"].as_str().unwrap_or(name);
        let scan = scan_skill_supply_chain(body, source);
        if !scan.safe {
            let rules = scan
                .findings
                .iter()
                .map(|f| f.rule.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            bail!("DDIPE_BLOCKED: skill '{name}' failed supply-chain scan ({rules})");
        }
        Ok(skill)
    }

    pub fn plugin_command(&self, command: &str) -> Result<Value> {
        let trimmed = command.trim();
        let (raw_action, raw_arg) = match trimmed.split_once(char::is_whitespace) {
            Some((a, b)) => (a, b.trim().trim_matches('"')),
            None => (trimmed, ""),
        };
        let action = if raw_action == "install" {
            "add"
        } else {
            raw_action
        };
        let mut list = self.plugins()?;
        if action.is_empty() || action == "list" {
            let message = if list.is_empty() {
                "No plugins installed. /plugins add <local-plugin-directory>".to_string()
            } else {
                list.iter()
                    .map(|p| {
                        format!(
                            "{}: {} ({})",
                            p.name,
                            if p.enabled { "enabled" } else { "disabled" },
                            p.path
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            };
            return Ok(json!({ "catalog": self.catalog()?, "message": message }));
        }
        if action == "add" {
            if raw_arg.is_empty() {
                bail!("Usage: /plugins add <local directory>");
            }
            let dir = fs::canonicalize(self.workspace.join(raw_arg))?;
            let manifest = self.manifest(&dir)?;
            let name = manifest
                .get("name")
                .and_then(Value::as_str)
                .map(ToString::to_string)
                .or_else(|| dir.file_name().and_then(|n| n.to_str()).map(ToString::to_string))
                .unwrap_or_default();
            if !plugin_name_re().is_match(&name) {
                bail!("Invalid plugin name");
            }
            if list.iter().any(|p| p.name == name) {
                bail!("Plugin name already exists; remove it before replacing it");
            }
            list.push(PluginEntry {
                name,
                path: dir.display().to_string(),
                enabled: true,
            });
        } else if matches!(action, "enable" | "disable" | "remove") {
            let idx = list
                .iter()
                .position(|p| p.name == raw_arg)
                .ok_or_else(|| anyhow!("Unknown plugin name"))?;
            if action == "enable" {
                self.manifest(Path::new(&list[idx].path))?;
            }
            let pname = list[idx].name.clone();
            if let Some(owned) = self.plugin_servers.lock().unwrap().remove(&pname) {
                for server in owned {
                    self.host.stop_server(&server);
                }
            }
            if action == "remove" {
                list.remove(idx);
            } else {
                list[idx].enabled = action == "enable";
            }
        } else {
            bail!("Usage: /plugins [list|add <directory>|enable <name>|disable <name>|remove <name>]");
        }
        atomic_write_json(&self.registry_file, &serde_json::to_value(&list)?)?;
        Ok(json!({
            "catalog": self.catalog()?,
            "message": format!("Plugin {action}: {raw_arg}")
        }))
    }

    pub fn mcp_command(&self, command: &str, cancel: &CancellationToken) -> Result<Value> {
        let parts: Vec<&str> = command.split_whitespace().collect();
        let action = parts.first().copied().unwrap_or("");
        let name = parts.get(1).copied().unwrap_or("");
        if action.is_empty() || action == "list" || action == "tools" {
            let cat = self.catalog()?;
            let message = if action == "tools" {
                let tools = self.host.all_tools();
                if tools.is_empty() {
                    "No MCP tools connected. /mcp start <server>".to_string()
                } else {
                    tools
                        .iter()
                        .map(|t| {
                            format!(
                                "{}: {}",
                                t.alias,
                                t.description.as_deref().unwrap_or(&t.name)
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("\n")
                }
            } else {
                let servers = cat["servers"].as_array().cloned().unwrap_or_default();
                if servers.is_empty() {
                    "No MCP servers configured. Add mcpServers to .meg/mcp.json.".to_string()
                } else {
                    servers
                        .iter()
                        .map(|s| {
                            let state = if s["disabled"].as_bool() == Some(true) {
                                "disabled"
                            } else if s["connected"].as_bool() == Some(true) {
                                "connected"
                            } else if s["connecting"].as_bool() == Some(true) {
                                "connecting"
                            } else {
                                "stopped"
                            };
                            let err = s["error"]
                                .as_str()
                                .map(|e| format!(" ? {e}"))
                                .unwrap_or_default();
                            format!("{}: {state}{err}", s["name"].as_str().unwrap_or(""))
                        })
                        .collect::<Vec<_>>()
                        .join("\n")
                }
            };
            return Ok(json!({ "catalog": cat, "message": message }));
        }
        let configs = self.configs()?;
        let names: Vec<String> = if name == "all" {
            configs.keys().cloned().collect()
        } else if !name.is_empty() {
            vec![name.to_string()]
        } else {
            Vec::new()
        };
        if names.is_empty() || names.iter().any(|n| !configs.contains_key(n)) {
            bail!("Unknown MCP server. /mcp lists configured names.");
        }
        if action == "start" || action == "restart" {
            for n in &names {
                if action == "restart" {
                    self.host.stop_server(n);
                }
                let cfg = &configs[n];
                self.host.connect(n, cfg, Some(&self.workspace), cancel)?;
                self.set_trust(n, cfg, true)?;
            }
        } else if action == "stop" {
            for n in &names {
                let cfg = &configs[n];
                self.set_trust(n, cfg, false)?;
                self.host.stop_server(n);
            }
        } else {
            bail!("Usage: /mcp [list|tools|start <server|all>|stop <server|all>|restart <server>]");
        }
        Ok(json!({
            "catalog": self.catalog()?,
            "message": format!("MCP {action}: {}", names.join(", "))
        }))
    }

    pub fn call_tool(
        &self,
        alias: &str,
        args: Value,
        read_only: bool,
        cancel: &CancellationToken,
    ) -> Result<Value> {
        let tool = self
            .host
            .all_tools()
            .into_iter()
            .find(|t| t.alias == alias || format!("mcp:{}:{}", t.server, t.name) == alias)
            .ok_or_else(|| anyhow!("MCP tool is not connected; use /mcp start <server>"))?;
        if read_only && !tool.read_only {
            bail!("MCP tool is disabled in Plan mode: no read-only annotation");
        }
        let result = self.host.call_tool(&tool.server, &tool.name, args, cancel)?;
        if result.get("isError").and_then(Value::as_bool) == Some(true) {
            let text = result
                .get("content")
                .and_then(Value::as_array)
                .map(|arr| {
                    arr.iter()
                        .filter(|p| p.get("type").and_then(Value::as_str) == Some("text"))
                        .filter_map(|p| p.get("text").and_then(Value::as_str))
                        .collect::<Vec<_>>()
                        .join("\n")
                })
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "MCP tool failed".into());
            bail!("{text}");
        }
        Ok(result)
    }
}

fn resolve_windows_pathext(base: &Path) -> Option<PathBuf> {
    if !cfg!(windows) || base.extension().is_some() {
        return None;
    }
    let pathext = std::env::var("PATHEXT").unwrap_or_else(|_| ".EXE;.CMD;.BAT;.COM".into());
    for ext in pathext.split(';').map(str::trim).filter(|e| !e.is_empty()) {
        let mut os = base.as_os_str().to_os_string();
        os.push(ext);
        let cand = PathBuf::from(os);
        if cand.is_file() {
            return Some(cand);
        }
    }
    None
}

fn hash_mcp_artifact_file(path: &Path) -> Option<String> {
    use std::io::{Read, Seek, SeekFrom};
    const FULL_HASH_CAP: u64 = 16 * 1024 * 1024;
    const WINDOW_BYTES: usize = 1024 * 1024;

    let meta = fs::metadata(path).ok()?;
    if !meta.is_file() {
        return None;
    }
    let len = meta.len();
    if len <= FULL_HASH_CAP {
        let bytes = fs::read(path).ok()?;
        return Some(sha256_hex(&bytes));
    }
    let mut file = fs::File::open(path).ok()?;
    let mut material = format!("large:{len}:").into_bytes();
    let mut head = vec![0u8; WINDOW_BYTES];
    let n_head = file.read(&mut head).ok()?;
    material.extend_from_slice(&head[..n_head]);
    if len > WINDOW_BYTES as u64 {
        file.seek(SeekFrom::Start(len - WINDOW_BYTES as u64)).ok()?;
        let mut tail = vec![0u8; WINDOW_BYTES];
        let n_tail = file.read(&mut tail).ok()?;
        material.extend_from_slice(&tail[..n_tail]);
    }
    Some(sha256_hex(&material))
}
