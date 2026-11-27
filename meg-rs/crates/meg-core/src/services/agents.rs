//! Pure-Rust read-only subagent delegator spawning `meg worker` with security-router
//! enforcement, 5-handoff cap, concurrency cap, heartbeats, and Windows process-tree kill.

use super::mcp::kill_child_tree;
use super::security::{route_task, RouteVerdict};
use crate::CancellationToken;
use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

struct ActiveAgent {
    role: String,
    workspace: String,
    started: Instant,
}

pub struct AgentManager {
    limit: usize,
    heartbeat_ms: u64,
    active: Mutex<HashMap<String, ActiveAgent>>,
    handoffs: Mutex<HashMap<String, usize>>,
}

impl Default for AgentManager {
    fn default() -> Self {
        Self::new(4, 1500)
    }
}

impl AgentManager {
    pub fn new(limit: usize, heartbeat_ms: u64) -> Self {
        Self {
            limit,
            heartbeat_ms,
            active: Mutex::new(HashMap::new()),
            handoffs: Mutex::new(HashMap::new()),
        }
    }

    pub fn status(&self) -> Vec<Value> {
        let map = self.active.lock().unwrap();
        map.iter()
            .map(|(id, a)| {
                json!({
                    "id": id,
                    "role": a.role,
                    "workspace": a.workspace,
                    "elapsed": a.started.elapsed().as_secs_f64(),
                })
            })
            .collect()
    }

    pub fn run<F>(
        &self,
        params: &Value,
        cancel: &CancellationToken,
        mut notify: F,
    ) -> Result<Value>
    where
        F: FnMut(Value),
    {
        let role = params
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or("researcher");
        if !matches!(role, "planner" | "researcher" | "reviewer") {
            bail!("Delegate role must be planner, researcher or reviewer; delegated tools are read-only");
        }
        let task = params
            .get("task")
            .and_then(Value::as_str)
            .unwrap_or("");
        if task.trim().is_empty() || task.len() > 32_000 {
            bail!("Delegate needs a task (up to 32,000 characters)");
        }
        let workspace = params
            .get("workspace")
            .and_then(Value::as_str)
            .unwrap_or(".");
        let handoff_id = params
            .get("worker")
            .and_then(|w| w.get("handoff_id"))
            .and_then(Value::as_str)
            .map(ToString::to_string)
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

        {
            let mut handoffs = self.handoffs.lock().unwrap();
            let count = handoffs.get(&handoff_id).copied().unwrap_or(0);
            if let RouteVerdict::RootThread(reason) = route_task(None, task, count) {
                bail!("{reason}");
            }
            handoffs.insert(handoff_id.clone(), count + 1);
            if handoffs.len() > 1024 {
                if let Some(first_key) = handoffs.keys().next().cloned() {
                    handoffs.remove(&first_key);
                }
            }
        }

        let id = format!("{role}-{}", &uuid::Uuid::new_v4().to_string()[..8]);
        let started = Instant::now();
        {
            let mut active = self.active.lock().unwrap();
            if active.len() >= self.limit {
                bail!("Delegate limit reached ({}); wait for an active worker", self.limit);
            }
            active.insert(
                id.clone(),
                ActiveAgent {
                    role: role.to_string(),
                    workspace: workspace.to_string(),
                    started,
                },
            );
        }

        struct Guard<'a> {
            mgr: &'a AgentManager,
            id: String,
        }
        impl Drop for Guard<'_> {
            fn drop(&mut self) {
                self.mgr.active.lock().unwrap().remove(&self.id);
            }
        }
        let _guard = Guard {
            mgr: self,
            id: id.clone(),
        };

        if cancel.is_cancelled() {
            bail!("Operation cancelled");
        }

        let exe: PathBuf = std::env::var_os("MEG_NATIVE_EXE")
            .map(PathBuf::from)
            .filter(|p| p.is_file())
            .or_else(|| std::env::current_exe().ok())
            .context("Native MEG worker executable is unavailable")?;

        let spawn_args: Vec<String> = params
            .get("_testArgs")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(ToString::to_string))
                    .collect()
            })
            .unwrap_or_else(|| vec!["worker".to_string()]);

        let mut cmd = Command::new(&exe);
        cmd.args(&spawn_args)
            .env("MEG_PLANE_DISABLED", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        let mut child = cmd
            .spawn()
            .context("Could not start native MEG worker")?;

        let mut emit = |status: &str, phase: &str, usage: Option<&Value>| {
            let active_count = self.active.lock().unwrap().len();
            let mut ev = json!({
                "id": id,
                "role": role,
                "status": status,
                "phase": phase,
                "elapsed": started.elapsed().as_secs_f64(),
                "activeCount": active_count,
            });
            if let Some(u) = usage {
                ev["usage"] = u.clone();
            }
            notify(ev);
        };
        emit("running", "starting", None);

        // Feed worker JSON request via stdin and close it.
        let mut req_obj = params
            .get("worker")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        req_obj.insert("workspace".into(), Value::String(workspace.to_string()));
        req_obj.insert(
            "goal".into(),
            Value::String(format!(
                "Act as a {role}. This delegated run is read-only. Return useful findings and evidence.\nTask: {task}"
            )),
        );
        req_obj.insert("mode".into(), Value::String("plan".into()));
        let default_prof: Option<crate::providers::ProviderProfile> = req_obj
            .get("provider")
            .cloned()
            .and_then(|v| serde_json::from_value(v).ok());
        let settings = crate::providers::ProviderStore::global()
            .load()
            .unwrap_or_default();
        let disabled = crate::providers::disabled_providers_from_env();
        let routed = resolve_subagent_route(
            role,
            params,
            default_prof.as_ref(),
            &settings,
            &disabled,
        )?;
        if let Some(ref target) = routed {
            req_obj.insert("model".into(), Value::String(target.model.clone()));
            if let Some(ref prof) = target.provider {
                req_obj.insert("provider".into(), serde_json::to_value(prof)?);
                req_obj.insert("base_url".into(), Value::String(prof.base_url.clone()));
            }
        }
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(serde_json::to_string(&Value::Object(req_obj))?.as_bytes());
            let _ = stdin.flush();
        }

        // Read bounded stdout (max 2 MiB) on a helper thread.
        let stdout = child.stdout.take().context("Worker stdout unavailable")?;
        let stderr_pipe = child.stderr.take();
        let stderr_reader = std::thread::spawn(move || -> String {
            let mut buf = Vec::new();
            if let Some(s) = stderr_pipe {
                let _ = s.take(2048).read_to_end(&mut buf);
            }
            String::from_utf8_lossy(&buf).trim().to_string()
        });
        let overflowed = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let ov_flag = overflowed.clone();
        let reader = std::thread::spawn(move || -> Vec<u8> {
            let mut buf = Vec::new();
            let mut limited = stdout.take(2 * 1024 * 1024 + 1);
            let _ = limited.read_to_end(&mut buf);
            if buf.len() > 2 * 1024 * 1024 {
                ov_flag.store(true, std::sync::atomic::Ordering::Relaxed);
                buf.truncate(2 * 1024 * 1024);
            }
            buf
        });

        let timeout_ms = params
            .get("timeoutMs")
            .and_then(Value::as_u64)
            .filter(|&v| v > 0)
            .map(|v| v.min(300_000))
            .unwrap_or(300_000);
        let deadline = Instant::now() + Duration::from_millis(timeout_ms);
        let mut last_hb = Instant::now();

        let status = loop {
            if cancel.is_cancelled() {
                kill_child_tree(&mut child);
                let _ = child.wait();
                emit("cancelled", "done", None);
                bail!("Delegate cancelled");
            }
            if overflowed.load(std::sync::atomic::Ordering::Relaxed)
                || Instant::now() >= deadline
            {
                kill_child_tree(&mut child);
                let _ = child.wait();
                emit("failed", "done", None);
                bail!("Delegate failed or timed out");
            }
            if let Some(st) = child.try_wait()? {
                break st;
            }
            if self.heartbeat_ms > 0
                && last_hb.elapsed() >= Duration::from_millis(self.heartbeat_ms)
            {
                emit("running", "working", None);
                last_hb = Instant::now();
            }
            std::thread::sleep(Duration::from_millis(40));
        };

        let bytes = reader.join().unwrap_or_default();
        let stderr_tail = stderr_reader.join().unwrap_or_default();
        if !status.success() {
            emit("failed", "done", None);
            if stderr_tail.is_empty() {
                bail!("Delegate failed or timed out");
            }
            bail!("Delegate failed or timed out (stderr: {stderr_tail})");
        }
        let payload: Value = serde_json::from_slice(&bytes)
            .map_err(|_| anyhow!("Native worker returned invalid JSON"))?;
        let Some(answer) = payload.get("answer").and_then(Value::as_str) else {
            let err_msg = payload
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("Native worker returned no answer");
            emit("failed", "done", None);
            bail!("{err_msg}");
        };
        emit("completed", "done", payload.get("usage"));

        let mut merged = payload
            .as_object()
            .cloned()
            .unwrap_or_default();
        merged.insert("id".into(), Value::String(id));
        merged.insert("role".into(), Value::String(role.to_string()));
        if let Some(ref target) = routed {
            merged.insert("routed_model".into(), Value::String(target.model.clone()));
            if let Some(ref prof) = target.provider {
                merged.insert("routed_provider".into(), Value::String(prof.id.clone()));
            }
        }
        merged.insert("message".into(), Value::String(format!("{role}: {answer}")));
        Ok(Value::Object(merged))
    }
}

/// Resolve an optional Delegent / local-SLM route for read-only subagents.
///
/// Priority:
/// 1. Role-specific env override (`MEG_SUBAGENT_MODEL_RESEARCHER` / `_PLANNER` / `_REVIEWER`)
/// 2. Explicit `params["subagentModel"]` or `MEG_SUBAGENT_MODEL` (`provider_id:model_id` or `model_id`)
/// 3. Delegent local-SLM preset (`MEG_DELEGENT=1` or `params["delegent"] == true`), selecting
///    an enabled local `ollama` or `lmstudio` profile using `MEG_LOCAL_SLM_MODEL` or its `default_model`.
pub(crate) fn resolve_subagent_route(
    role: &str,
    params: &Value,
    default_provider: Option<&crate::providers::ProviderProfile>,
    settings: &crate::providers::ProviderSettings,
    disabled_providers: &[String],
) -> Result<Option<crate::providers::ResolvedRoleTarget>> {
    let role_env_key = format!("MEG_SUBAGENT_MODEL_{}", role.to_ascii_uppercase());
    let explicit_spec = std::env::var(&role_env_key)
        .ok()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| {
            params
                .get("subagentModel")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(ToString::to_string)
        })
        .or_else(|| {
            std::env::var("MEG_SUBAGENT_MODEL")
                .ok()
                .filter(|s| !s.trim().is_empty())
        });

    if let Some(spec) = explicit_spec {
        let target = crate::providers::resolve_role_target(
            &spec,
            default_provider,
            settings,
            disabled_providers,
        )?;
        return Ok(Some(target));
    }

    let delegent_enabled = params
        .get("delegent")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        || matches!(
            std::env::var("MEG_DELEGENT").as_deref(),
            Ok("1") | Ok("true") | Ok("on")
        );
    if !delegent_enabled {
        return Ok(None);
    }

    let local_model_override = std::env::var("MEG_LOCAL_SLM_MODEL")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    let is_disabled = |id: &str| {
        disabled_providers
            .iter()
            .any(|d| d.eq_ignore_ascii_case(id))
    };

    for preferred_id in ["ollama", "lmstudio"] {
        if is_disabled(preferred_id) {
            continue;
        }
        if let Some(prof) = settings
            .profiles
            .iter()
            .find(|p| p.id.eq_ignore_ascii_case(preferred_id))
        {
            let model = local_model_override.clone().or_else(|| {
                let m = prof.default_model.trim();
                if m.is_empty() {
                    None
                } else {
                    Some(m.to_string())
                }
            });
            if let Some(m) = model {
                let spec = format!("{}:{m}", prof.id);
                let target = crate::providers::resolve_role_target(
                    &spec,
                    default_provider,
                    settings,
                    disabled_providers,
                )?;
                return Ok(Some(target));
            }
        }
    }

    Ok(None)
}
