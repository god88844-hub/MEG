//! Tool registry -- name -> executor dispatch with JSON Schemas and prompt stubs.

use anyhow::Result;
use serde_json::{json, Value};
use std::collections::BTreeMap;

use crate::bash;
use crate::edit;
use crate::fs_tools;
use crate::grep;

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
}

fn read_file_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "path": { "type": "string", "description": "Workspace-relative path to the file." },
            "offset": { "type": "integer", "minimum": 1, "description": "Optional 1-based starting line number." },
            "limit": { "type": "integer", "minimum": 1, "maximum": 2000, "description": "Optional max lines to return." },
            "line_numbers": { "type": "boolean", "description": "Format output with 1-based line numbers." }
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
            "replace_all": { "type": "boolean", "description": "Replace all occurrences instead of requiring uniqueness." }
        },
        "required": ["path", "old_string", "new_string"]
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
            "pattern": { "type": "string", "description": "Glob pattern relative to workspace (supports **, *, ?)." }
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

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: BTreeMap::new(),
            stubs: BTreeMap::new(),
            schemas: BTreeMap::new(),
        }
    }

    pub fn with_core_tools(workspace: &std::path::Path) -> Self {
        let mut reg = Self::new();
        reg.register_with_schema(
            "read_file",
            "Read a text file from the workspace: {path, offset?, limit?, line_numbers?}.",
            read_file_schema(),
            {
                let ws = workspace.to_path_buf();
                move |args| fs_tools::read_file(&ws, args)
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
            "Exact str_replace edit: {path, old_string, new_string, replace_all?}. old_string must be unique.",
            edit_file_schema(),
            {
                let ws = workspace.to_path_buf();
                move |args| edit::str_replace(&ws, args)
            },
        );
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
        reg
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
            Some(f) => match f(args) {
                Ok(output) => ToolResult { ok: true, output },
                Err(e) => ToolResult {
                    ok: false,
                    output: format!("tool error: {e}"),
                },
            },
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
    fn core_tools_registered_with_valid_json_schemas() {
        let reg = ToolRegistry::with_core_tools(&std::env::temp_dir());
        for name in [
            "read_file",
            "write_file",
            "edit_file",
            "bash",
            "search_files",
            "grep_files",
            "list_dir",
        ] {
            assert!(reg.stubs.contains_key(name), "missing stub {name}");
            let schema = reg.schemas.get(name).expect("missing schema");
            assert_eq!(schema["type"], "object", "schema for {name} must be object");
            assert!(schema["properties"].is_object(), "schema for {name} needs properties");
        }
        assert_eq!(reg.tool_schemas().len(), 7);
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
}