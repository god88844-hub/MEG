//! Tool registry — name → executor dispatch with schema stubs for the prompt.

use anyhow::Result;
use serde_json::Value;
use std::collections::BTreeMap;

use crate::bash;
use crate::edit;
use crate::fs_tools;

pub struct ToolResult {
    pub ok: bool,
    pub output: String,
}

pub type ToolFn = dyn Fn(&Value) -> Result<String> + Send + Sync;

pub struct ToolRegistry {
    tools: BTreeMap<String, Box<ToolFn>>,
    pub stubs: BTreeMap<String, String>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: BTreeMap::new(),
            stubs: BTreeMap::new(),
        }
    }

    pub fn with_core_tools(workspace: &std::path::Path) -> Self {
        let mut reg = Self::new();
        reg.register("read_file", "Read a text file from the workspace.", {
            let ws = workspace.to_path_buf();
            move |args| fs_tools::read_file(&ws, args)
        });
        reg.register(
            "write_file",
            "Write (create/overwrite) a text file in the workspace.",
            {
                let ws = workspace.to_path_buf();
                move |args| fs_tools::write_file(&ws, args)
            },
        );
        reg.register(
            "edit_file",
            "Exact str_replace edit: {path, old_string, new_string}. old_string must be unique.",
            {
                let ws = workspace.to_path_buf();
                move |args| edit::str_replace(&ws, args)
            },
        );
        reg.register(
            "bash",
            "Run a shell command with timeout_seconds (default 60, max 600).",
            {
                let ws = workspace.to_path_buf();
                move |args| bash::run(&ws, args)
            },
        );
        reg.register(
            "search_files",
            "Glob search: {pattern} relative to workspace, returns matching paths.",
            {
                let ws = workspace.to_path_buf();
                move |args| fs_tools::search(&ws, args)
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
        registry.register(
            "search_files",
            "Glob search: {pattern} with cancellation.",
            move |args| fs_tools::search_cancellable(&search_ws, args, &search_cancel),
        );
        let ws = workspace.to_path_buf();
        registry.register(
            "bash",
            "Run a command with cancellation and timeout_seconds.",
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
        self.tools.insert(name.to_string(), Box::new(f));
        self.stubs.insert(name.to_string(), stub.to_string());
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
    fn core_tools_registered() {
        let reg = ToolRegistry::with_core_tools(&std::env::temp_dir());
        for name in [
            "read_file",
            "write_file",
            "edit_file",
            "bash",
            "search_files",
        ] {
            assert!(reg.stubs.contains_key(name), "missing {name}");
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
}
