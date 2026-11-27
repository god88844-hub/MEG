//! Tool registry -- name -> executor dispatch with strict JSON Schemas,
//! core 6-tool menu (`bash`, `read_file`, `write_file`, `edit_file`, `glob_grep`, `finish`),
//! and Copy-Constrained argument span validation.

use anyhow::{bail, Result};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use crate::bash;
use crate::edit;
use crate::fs_tools::{self, ReadCache};
use crate::grep;

pub const CORE_SIX_TOOLS: &[&str] = &[
    "bash",
    "read_file",
    "write_file",
    "edit_file",
    "glob_grep",
    "finish",
];

pub const DEFAULT_MAX_OUTPUT_BYTES: usize = 32_768;

pub struct ToolResult {
    pub ok: bool,
    pub output: String,
}

#[derive(Debug, Clone)]
pub struct ToolSchema {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

pub type ToolFn = dyn Fn(&Value) -> Result<String> + Send + Sync;

pub struct ToolRegistry {
    tools: BTreeMap<String, Box<ToolFn>>,
    pub stubs: BTreeMap<String, String>,
    pub schemas: BTreeMap<String, Value>,
    pub max_output_bytes: usize,
}

/// Copy-Constrained argument span validation: ensures file paths (`path`) and
/// 7-40 hex commit SHAs (`commit`, `sha`, `ref`) appear in the verified context
/// spans (or exist on disk for read/edit paths) before a tool executes.
pub fn validate_copy_constrained_args(
    tool: &str,
    args: &Value,
    context_spans: &[&str],
    workspace: Option<&Path>,
) -> Result<()> {
    if let Some(sha_key) = ["commit", "sha", "commit_sha"]
        .iter()
        .find(|&&k| args.get(k).and_then(Value::as_str).is_some())
    {
        let sha = args[*sha_key].as_str().unwrap_or("");
        if sha.len() >= 7
            && sha.len() <= 40
            && sha.chars().all(|c| c.is_ascii_hexdigit())
            && !context_spans.iter().any(|s| s.contains(sha))
        {
            bail!(
                "copy-constrained validation failed for '{tool}': commit SHA '{sha}' not found in context spans"
            );
        }
    }
    if matches!(tool, "read_file" | "edit_file") {
        if let Some(p) = args.get("path").and_then(Value::as_str) {
            let norm = p.replace('\\', "/");
            let in_context = context_spans
                .iter()
                .any(|s| s.contains(p) || s.contains(&norm));
            let on_disk = workspace
                .and_then(|ws| fs_tools::resolve(ws, p).ok())
                .is_some_and(|full| full.exists());
            if !in_context && !on_disk {
                bail!(
                    "copy-constrained validation failed for '{tool}': path '{p}' not found in context spans or workspace"
                );
            }
        }
    }
    Ok(())
}


fn json_matches_type(val: &Value, expected: &str) -> bool {
    match expected {
        "string" => val.is_string(),
        "boolean" => val.is_boolean(),
        "integer" => val.is_i64() || val.is_u64(),
        "number" => val.is_number(),
        "array" => val.is_array(),
        "object" => val.is_object(),
        "null" => val.is_null(),
        _ => true,
    }
}

pub fn validate_args_against_schema(tool: &str, args: &Value, schema: &Value) -> Result<()> {
    if schema.get("type").and_then(Value::as_str) == Some("object") && !args.is_object() {
        bail!("schema validation failed for '{tool}': arguments must be a JSON object");
    }
    let Some(obj) = args.as_object() else {
        return Ok(());
    };
    if let Some(required) = schema.get("required").and_then(Value::as_array) {
        let has_edits_mode = tool == "edit_file" && obj.contains_key("edits");
        let has_query_alias = matches!(tool, "grep_files" | "glob_grep") && obj.contains_key("query");
        for req in required {
            if let Some(key) = req.as_str() {
                if has_edits_mode && matches!(key, "old_str" | "new_str") {
                    continue;
                }
                if has_query_alias && key == "pattern" {
                    continue;
                }
                if !obj.contains_key(key) || obj[key].is_null() {
                    bail!("schema validation failed for '{tool}': missing required argument '{key}'");
                }
            }
        }
    }
    if let Some(props) = schema.get("properties").and_then(Value::as_object) {
        for (k, v) in obj {
            if v.is_null() {
                continue;
            }
            if let Some(prop_schema) = props.get(k) {
                if let Some(expected_type) = prop_schema.get("type").and_then(Value::as_str) {
                    if !json_matches_type(v, expected_type) {
                        bail!(
                            "schema validation failed for '{tool}': argument '{k}' expected type '{expected_type}'"
                        );
                    }
                }
            }
        }
    }
    Ok(())
}

fn read_file_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "path": { "type": "string", "description": "Workspace-relative path to the file." },
            "offset": { "type": "integer", "minimum": 1, "description": "Optional 1-based starting line number." },
            "limit": { "type": "integer", "minimum": 1, "maximum": 2000, "description": "Optional max lines to return (default 100)." },
            "line_numbers": { "type": "boolean", "description": "Format output with 1-based line numbers." },
            "hashlines": { "type": "boolean", "description": "Tag each line with 3-hex content hash (42#f9a)." },
            "force": { "type": "boolean", "description": "Bypass repeated unchanged-read suppression." }
        },
        "required": ["path"]
    })
}

fn write_file_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "path": { "type": "string", "description": "Workspace-relative file path to create or overwrite." },
            "content": { "type": "string", "description": "Full text content to write." }
        },
        "required": ["path", "content"]
    })
}

fn edit_file_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "path": { "type": "string", "description": "Workspace-relative path to edit." },
            "old_string": { "type": "string", "description": "Exact existing text to replace (must be unique unless replace_all is true)." },
            "new_string": { "type": "string", "description": "Replacement text." },
            "replace_all": { "type": "boolean", "description": "Replace all occurrences instead of requiring uniqueness." },
            "edits": {
                "type": "array",
                "description": "Optional hashline patch list: [{start_anchor: '42#f9a', end_anchor?, new_content}].",
                "items": {
                    "type": "object",
                    "properties": {
                        "start_anchor": { "type": "string" },
                        "end_anchor": { "type": "string" },
                        "new_content": { "type": "string" }
                    },
                    "required": ["start_anchor", "new_content"]
                }
            }
        },
        "required": ["path"]
    })
}

fn bash_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "command": { "type": "string", "description": "Shell command to execute in the workspace." },
            "timeout_seconds": { "type": "integer", "minimum": 1, "maximum": 600, "description": "Command timeout in seconds (default 60)." }
        },
        "required": ["command"]
    })
}

fn search_files_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "pattern": { "type": "string", "description": "Glob pattern relative to workspace (supports **, *, ?)." },
            "max_results": { "type": "integer", "minimum": 1, "maximum": 200 }
        },
        "required": ["pattern"]
    })
}

fn grep_files_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "pattern": { "type": "string", "description": "Regular expression pattern to search for." },
            "path": { "type": "string", "description": "Optional workspace-relative directory or file to search." },
            "glob": { "type": "string", "description": "Optional file glob filter (e.g. **/*.rs)." },
            "case_insensitive": { "type": "boolean", "description": "Perform case-insensitive matching." },
            "context_lines": { "type": "integer", "minimum": 0, "maximum": 5, "description": "Context lines before and after each match." },
            "max_results": { "type": "integer", "minimum": 1, "maximum": 200, "description": "Maximum matches to return (default 50)." }
        },
        "required": ["pattern"]
    })
}

fn glob_grep_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "mode": { "type": "string", "enum": ["glob", "grep"], "description": "Search filenames ('glob') or file contents ('grep')." },
            "pattern": { "type": "string", "description": "Glob or regex pattern." },
            "path": { "type": "string", "description": "Optional workspace-relative search root." },
            "glob": { "type": "string", "description": "Optional file glob filter when mode='grep'." },
            "case_insensitive": { "type": "boolean" },
            "max_results": { "type": "integer", "minimum": 1, "maximum": 200 }
        },
        "required": ["pattern"]
    })
}

fn finish_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "summary": { "type": "string", "description": "Concise verified summary of completed work." },
            "deliverables": {
                "type": "array",
                "items": { "type": "string" },
                "description": "Workspace-relative paths of created/updated deliverables."
            }
        },
        "required": ["summary"]
    })
}

fn shell_session_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "action": { "type": "string", "enum": ["exec", "status", "reset", "close"], "description": "Session action (default 'exec')." },
            "session_id": { "type": "string", "description": "Optional session name (default 'default')." },
            "command": { "type": "string", "description": "Shell command to run in the persistent session when action='exec'." },
            "timeout_seconds": { "type": "integer", "minimum": 1, "maximum": 600 }
        }
    })
}

fn list_dir_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "path": { "type": "string", "description": "Workspace-relative directory path (default '.')." },
            "include_ignored": { "type": "boolean", "description": "Include ignored directories like .git or target." },
            "max_entries": { "type": "integer", "minimum": 1, "maximum": 500, "description": "Maximum entries to list (default 200)." }
        }
    })
}

fn run_glob_grep(
    workspace: &Path,
    args: &Value,
    cancel: Option<&std::sync::atomic::AtomicBool>,
) -> Result<String> {
    let mode = args.get("mode").and_then(Value::as_str).unwrap_or_else(|| {
        if args.get("glob").is_some() || args.get("case_insensitive").is_some() {
            "grep"
        } else {
            "glob"
        }
    });
    match (mode, cancel) {
        ("grep", Some(c)) => grep::grep_files_cancellable(workspace, args, c),
        ("grep", None) => grep::grep_files(workspace, args),
        (_, Some(c)) => fs_tools::search_cancellable(workspace, args, c),
        (_, None) => fs_tools::search(workspace, args),
    }
}

fn run_finish(args: &Value) -> Result<String> {
    let summary = args
        .get("summary")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| anyhow::anyhow!("finish requires non-empty 'summary'"))?;
    Ok(summary.to_string())
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: BTreeMap::new(),
            stubs: BTreeMap::new(),
            schemas: BTreeMap::new(),
            max_output_bytes: DEFAULT_MAX_OUTPUT_BYTES,
        }
    }

    /// Locked default 6-primitive menu (`bash`, `read_file`, `write_file`, `edit_file`, `glob_grep`, `finish`).
    pub fn with_core_six(workspace: &std::path::Path) -> Self {
        let mut reg = Self::new();
        let cache = Arc::new(ReadCache::new());
        reg.register_with_schema(
            "bash",
            "Run a shell command with timeout_seconds (default 60, max 600).",
            bash_schema(),
            {
                let ws = workspace.to_path_buf();
                move |args| bash::run(&ws, args)
            },
        );
        reg.register_with_schema(
            "read_file",
            "Read a text file from the workspace: {path, offset?, limit?, line_numbers?, hashlines?, force?}.",
            read_file_schema(),
            {
                let ws = workspace.to_path_buf();
                let c = cache.clone();
                move |args| fs_tools::read_file_cached(&ws, args, Some(&c))
            },
        );
        reg.register_with_schema(
            "write_file",
            "Write (create/overwrite) a text file in the workspace.",
            write_file_schema(),
            {
                let ws = workspace.to_path_buf();
                move |args| fs_tools::write_file(&ws, args)
            },
        );
        reg.register_with_schema(
            "edit_file",
            "Hashline patch ({path, edits}) or exact str_replace ({path, old_string, new_string, replace_all?}).",
            edit_file_schema(),
            {
                let ws = workspace.to_path_buf();
                move |args| edit::str_replace(&ws, args)
            },
        );
        reg.register_with_schema(
            "glob_grep",
            "Unified glob/regex search: {mode?: 'glob'|'grep', pattern, path?, glob?, case_insensitive?, max_results?}.",
            glob_grep_schema(),
            {
                let ws = workspace.to_path_buf();
                move |args| run_glob_grep(&ws, args, None)
            },
        );
        reg.register_with_schema(
            "finish",
            "Declare task completion with verified summary and optional deliverables list.",
            finish_schema(),
            run_finish,
        );
        reg
    }

    pub fn with_core_tools(workspace: &std::path::Path) -> Self {
        let mut reg = Self::with_core_six(workspace);
        // Legacy aliases preserved for meg-cli, meg-ffi, and frozen meg-tui.
        reg.register_with_schema(
            "search_files",
            "Glob search: {pattern} relative to workspace, returns matching paths.",
            search_files_schema(),
            {
                let ws = workspace.to_path_buf();
                move |args| fs_tools::search(&ws, args)
            },
        );
        reg.register_with_schema(
            "grep_files",
            "Regex content search: {pattern, path?, glob?, case_insensitive?, context_lines?, max_results?}.",
            grep_files_schema(),
            {
                let ws = workspace.to_path_buf();
                move |args| grep::grep_files(&ws, args)
            },
        );
        reg.register_with_schema(
            "list_dir",
            "List directory entries in workspace: {path?, include_ignored?, max_entries?}.",
            list_dir_schema(),
            {
                let ws = workspace.to_path_buf();
                move |args| fs_tools::list_dir(&ws, args)
            },
        );
        let shell_mgr = crate::pty_shell::ShellSessionManager::new(workspace);
        let no_cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
        reg.register_with_schema(
            "shell_session",
            "Persistent stateful shell session preserving cwd & env across calls: {action?: 'exec'|'status'|'reset'|'close', session_id?, command?, timeout_seconds?}.",
            shell_session_schema(),
            move |args| shell_mgr.execute(args, &no_cancel),
        );
        reg
    }

    /// Strict locked 6-tool menu with cancellation support (`--core-six` / `MEG_CORE_SIX=1`).
    pub fn with_core_six_cancellable(
        workspace: &std::path::Path,
        cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) -> Self {
        let mut registry = Self::with_core_six(workspace);
        let gg_ws = workspace.to_path_buf();
        let gg_cancel = cancel.clone();
        registry.register_with_schema(
            "glob_grep",
            "Unified glob/regex search with cancellation.",
            glob_grep_schema(),
            move |args| run_glob_grep(&gg_ws, args, Some(&gg_cancel)),
        );
        let ws = workspace.to_path_buf();
        registry.register_with_schema(
            "bash",
            "Run a command with cancellation and timeout_seconds.",
            bash_schema(),
            move |args| bash::run_cancellable(&ws, args, &cancel),
        );
        registry
    }

    pub fn with_cancellation(
        workspace: &std::path::Path,
        cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) -> Self {
        let mut registry = Self::with_core_tools(workspace);
        let search_ws = workspace.to_path_buf();
        let search_cancel = cancel.clone();
        registry.register_with_schema(
            "search_files",
            "Glob search: {pattern} with cancellation.",
            search_files_schema(),
            move |args| fs_tools::search_cancellable(&search_ws, args, &search_cancel),
        );
        let grep_ws = workspace.to_path_buf();
        let grep_cancel = cancel.clone();
        registry.register_with_schema(
            "grep_files",
            "Regex content search: {pattern, path?, glob?, case_insensitive?, context_lines?, max_results?}.",
            grep_files_schema(),
            move |args| grep::grep_files_cancellable(&grep_ws, args, &grep_cancel),
        );
        let gg_ws = workspace.to_path_buf();
        let gg_cancel = cancel.clone();
        registry.register_with_schema(
            "glob_grep",
            "Unified glob/regex search with cancellation.",
            glob_grep_schema(),
            move |args| run_glob_grep(&gg_ws, args, Some(&gg_cancel)),
        );
        let shell_mgr = crate::pty_shell::ShellSessionManager::new(workspace);
        let shell_cancel = cancel.clone();
        registry.register_with_schema(
            "shell_session",
            "Persistent stateful shell session preserving cwd & env with cancellation.",
            shell_session_schema(),
            move |args| shell_mgr.execute(args, &shell_cancel),
        );
        let ws = workspace.to_path_buf();
        registry.register_with_schema(
            "bash",
            "Run a command with cancellation and timeout_seconds.",
            bash_schema(),
            move |args| bash::run_cancellable(&ws, args, &cancel),
        );
        registry
    }

    pub fn register(
        &mut self,
        name: &str,
        stub: &str,
        f: impl Fn(&Value) -> Result<String> + Send + Sync + 'static,
    ) {
        self.register_with_schema(
            name,
            stub,
            json!({ "type": "object", "properties": {} }),
            f,
        );
    }

    pub fn register_with_schema(
        &mut self,
        name: &str,
        stub: &str,
        schema: Value,
        f: impl Fn(&Value) -> Result<String> + Send + Sync + 'static,
    ) {
        self.tools.insert(name.to_string(), Box::new(f));
        self.stubs.insert(name.to_string(), stub.to_string());
        self.schemas.insert(name.to_string(), schema);
    }

    pub fn tool_schemas(&self) -> Vec<ToolSchema> {
        self.stubs
            .iter()
            .map(|(name, description)| ToolSchema {
                name: name.clone(),
                description: description.clone(),
                parameters: self
                    .schemas
                    .get(name)
                    .cloned()
                    .unwrap_or_else(|| json!({ "type": "object", "properties": {} })),
            })
            .collect()
    }

    pub fn execute(&self, name: &str, args: &Value) -> ToolResult {
        match self.tools.get(name) {
            Some(f) => {
                if let Some(schema) = self.schemas.get(name) {
                    if let Err(err) = validate_args_against_schema(name, args, schema) {
                        return ToolResult {
                            ok: false,
                            output: format!("tool error: {err}"),
                        };
                    }
                }
                match f(args) {
                Ok(mut output) => {
                    if output.len() > self.max_output_bytes {
                        let end = output.floor_char_boundary(self.max_output_bytes);
                        output = format!(
                            "{}\n...[capped at {} bytes]",
                            &output[..end],
                            self.max_output_bytes
                        );
                    }
                    ToolResult { ok: true, output }
                }
                Err(e) => ToolResult {
                    ok: false,
                    output: format!("tool error: {e}"),
                },
            }},
            None => ToolResult {
                ok: false,
                output: format!(
                    "unknown tool '{name}'. available: {}",
                    self.tools.keys().cloned().collect::<Vec<_>>().join(", ")
                ),
            },
        }
    }

    pub fn names(&self) -> Vec<String> {
        self.tools.keys().cloned().collect()
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn core_six_locks_default_menu_to_six_primitives() {
        let reg = ToolRegistry::with_core_six(&std::env::temp_dir());
        assert_eq!(reg.names().len(), 6);
        for name in CORE_SIX_TOOLS {
            assert!(reg.stubs.contains_key(*name), "missing core-6 tool {name}");
            let schema = reg.schemas.get(*name).expect("missing schema");
            assert_eq!(schema["type"], "object");
        }
        let fin = reg.execute("finish", &json!({"summary": "all verified"}));
        assert!(fin.ok && fin.output == "all verified");
    }

    #[test]
    fn copy_constrained_span_validation_blocks_hallucinated_paths_and_shas() {
        let spans = &["Changed src/main.rs in commit 7012f39"];
        assert!(validate_copy_constrained_args(
            "read_file",
            &json!({"path": "src/main.rs"}),
            spans,
            None
        )
        .is_ok());
        assert!(validate_copy_constrained_args(
            "read_file",
            &json!({"path": "src/hallucinated.rs"}),
            spans,
            None
        )
        .is_err());
        assert!(validate_copy_constrained_args(
            "bash",
            &json!({"commit": "7012f39"}),
            spans,
            None
        )
        .is_ok());
        assert!(validate_copy_constrained_args(
            "bash",
            &json!({"commit": "deadbeef"}),
            spans,
            None
        )
        .is_err());
    }

    #[test]
    fn core_tools_registered_with_valid_json_schemas() {
        let reg = ToolRegistry::with_core_tools(&std::env::temp_dir());
        for name in [
            "read_file",
            "write_file",
            "edit_file",
            "bash",
            "glob_grep",
            "finish",
            "search_files",
            "grep_files",
            "list_dir",
        ] {
            assert!(reg.stubs.contains_key(name), "missing stub {name}");
            let schema = reg.schemas.get(name).expect("missing schema");
            assert_eq!(schema["type"], "object", "schema for {name} must be object");
            assert!(schema["properties"].is_object(), "schema for {name} needs properties");
        }
    }

    #[test]
    fn unknown_tool_reports_available() {
        let reg = ToolRegistry::with_core_tools(&std::env::temp_dir());
        let r = reg.execute("nope", &serde_json::json!({}));
        assert!(!r.ok);
        assert!(r.output.contains("available:"));
    }
}

#[cfg(test)]
mod production_regressions {
    #[test]
    fn failing_shell_command_is_not_reported_as_success() {
        let registry = super::ToolRegistry::with_core_tools(&std::env::temp_dir());
        let result = registry.execute("bash", &serde_json::json!({"command":"exit 7"}));
        assert!(!result.ok, "nonzero process exit was painted as success");
    }

    #[test]
    fn adversarial_registry_enforces_schema_types_and_required_fields() {
        let ws = std::env::temp_dir().join(format!("meg-reg-c10-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&ws);
        let reg = super::ToolRegistry::with_core_tools(&ws);

        // Wrong type on optional integer field must be rejected rather than silently defaulted.
        let bad_type = reg.execute("grep_files", &serde_json::json!({"pattern": "foo", "max_results": "fifty"}));
        assert!(!bad_type.ok);
        assert!(bad_type.output.contains("expected type 'integer'"), "{}", bad_type.output);

        // Wrong type on optional boolean field must be rejected.
        let bad_bool = reg.execute("grep_files", &serde_json::json!({"pattern": "foo", "case_insensitive": "yes"}));
        assert!(!bad_bool.ok);
        assert!(bad_bool.output.contains("expected type 'boolean'"), "{}", bad_bool.output);

        // Missing required argument must be rejected at the registry boundary.
        let missing = reg.execute("read_file", &serde_json::json!({}));
        assert!(!missing.ok);
        assert!(missing.output.contains("missing required argument 'path'"), "{}", missing.output);

        let _ = std::fs::remove_dir_all(&ws);
    }
}
