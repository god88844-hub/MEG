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
            .stderr(Stdio::null());
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
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(serde_json::to_string(&Value::Object(req_obj))?.as_bytes());
            let _ = stdin.flush();
        }

        // Read bounded stdout (max 2 MiB) on a helper thread.
        let stdout = child.stdout.take().context("Worker stdout unavailable")?;
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
        if !status.success() {
            emit("failed", "done", None);
            bail!("Delegate failed or timed out");
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
        merged.insert("message".into(), Value::String(format!("{role}: {answer}")));
        Ok(Value::Object(merged))
    }
}
