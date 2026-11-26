//! Native pure-Rust service layer replacing the external Bun control plane
//! for standalone single-binary execution.

pub mod agents;
pub mod extensions;
pub mod mcp;
pub mod security;
#[cfg(test)]
mod tests;

use crate::CancellationToken;
use agents::AgentManager;
use anyhow::{anyhow, bail, Result};
use extensions::WorkspaceExtensions;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

pub struct NativeServices {
    workspaces: Mutex<HashMap<PathBuf, Arc<WorkspaceExtensions>>>,
    delegates: AgentManager,
}

impl Default for NativeServices {
    fn default() -> Self {
        Self::new()
    }
}

impl NativeServices {
    pub fn new() -> Self {
        Self {
            workspaces: Mutex::new(HashMap::new()),
            delegates: AgentManager::default(),
        }
    }

    fn workspace_runtime(&self, workspace: &str) -> Result<Arc<WorkspaceExtensions>> {
        let path = Path::new(workspace);
        if workspace.is_empty() || !path.is_dir() {
            bail!("Workspace directory is unavailable");
        }
        let key = std::fs::canonicalize(path)?;
        {
            let map = self.workspaces.lock().unwrap();
            if let Some(rt) = map.get(&key) {
                return Ok(rt.clone());
            }
        }
        let created = Arc::new(WorkspaceExtensions::new(&key)?);
        created.reconnect_trusted();
        let mut map = self.workspaces.lock().unwrap();
        Ok(map.entry(key).or_insert_with(|| created.clone()).clone())
    }

    pub fn call(
        &self,
        method: &str,
        params: Value,
        cancel: &CancellationToken,
    ) -> Result<Value> {
        self.call_with_events(method, params, cancel, |_| {})
    }

    pub fn call_with_events<F>(
        &self,
        method: &str,
        params: Value,
        cancel: &CancellationToken,
        on_event: F,
    ) -> Result<Value>
    where
        F: FnMut(Value),
    {
        match method {
            "runtime.ping" => Ok(json!({ "ready": true, "version": "0.2.0", "native": true })),
            "agents.status" => Ok(json!({ "agents": self.delegates.status() })),
            "agents.run" => self.delegates.run(&params, cancel, on_event),
            "runtime.catalog" | "skills.read" | "plugins.command" | "mcp.command"
            | "runtime.tool" => {
                let ws = params
                    .get("workspace")
                    .and_then(Value::as_str)
                    .ok_or_else(|| anyhow!("Workspace directory is unavailable"))?;
                let rt = self.workspace_runtime(ws)?;
                match method {
                    "runtime.catalog" => Ok(json!({
                        "catalog": rt.catalog()?,
                        "message": "Skills, plugins and MCP catalog refreshed"
                    })),
                    "skills.read" => {
                        let name = params.get("name").and_then(Value::as_str).unwrap_or("");
                        rt.read_skill(name)
                    }
                    "plugins.command" => {
                        let cmd = params.get("command").and_then(Value::as_str).unwrap_or("");
                        rt.plugin_command(cmd)
                    }
                    "mcp.command" => {
                        let cmd = params.get("command").and_then(Value::as_str).unwrap_or("");
                        rt.mcp_command(cmd, cancel)
                    }
                    "runtime.tool" => {
                        let tool = params.get("tool").and_then(Value::as_str).unwrap_or("");
                        let args = params
                            .get("args")
                            .cloned()
                            .unwrap_or_else(|| json!({}));
                        let read_only = params
                            .get("readOnly")
                            .and_then(Value::as_bool)
                            .unwrap_or(false);
                        rt.call_tool(tool, args, read_only, cancel)
                    }
                    _ => unreachable!(),
                }
            }
            _ => bail!("unknown native service method: {method}"),
        }
    }

    pub fn shutdown(&self) {
        let map = self.workspaces.lock().unwrap();
        for rt in map.values() {
            rt.host.stop_all();
        }
    }
}

impl Drop for NativeServices {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Unified service backend: pure-Rust `NativeServices` by default, or an opt-in
/// external Bun `PlaneClient` when `MEG_LEGACY_BUN_PLANE=1` / `MEG_PLANE_ENTRY` is set.
pub enum ServicePlane {
    Native(Arc<NativeServices>),
    External(Arc<crate::plane_client::PlaneClient>),
}

impl ServicePlane {
    pub fn native() -> Arc<Self> {
        Arc::new(Self::Native(Arc::new(NativeServices::new())))
    }

    pub fn external(client: Arc<crate::plane_client::PlaneClient>) -> Arc<Self> {
        Arc::new(Self::External(client))
    }

    pub fn is_native(&self) -> bool {
        matches!(self, Self::Native(_))
    }

    pub fn call(
        &self,
        method: &str,
        params: Value,
        cancel: &CancellationToken,
    ) -> Result<Value> {
        match self {
            Self::Native(svc) => svc.call(method, params, cancel),
            Self::External(plane) => plane.call(method, params, cancel),
        }
    }

    pub fn call_with_events<F>(
        &self,
        method: &str,
        params: Value,
        cancel: &CancellationToken,
        on_event: F,
    ) -> Result<Value>
    where
        F: FnMut(Value),
    {
        match self {
            Self::Native(svc) => svc.call_with_events(method, params, cancel, on_event),
            Self::External(plane) => plane.call_with_events(method, params, cancel, on_event),
        }
    }
}
