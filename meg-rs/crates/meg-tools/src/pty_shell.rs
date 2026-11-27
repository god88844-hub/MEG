//! Persistent stateful shell sessions (`shell_session`):
//! Preserves working directory, exported environment variables, and virtualenv state
//! across tool calls while enforcing `PermissionPolicy`, compound-bash guards,
//! secret env scrubbing, cancellation, and timeouts.

use anyhow::{bail, Result};
use serde_json::Value;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const MAX_SESSION_OUTPUT: usize = 8_000;
const HEAD: usize = 3_500;
const TAIL: usize = 3_500;

struct PersistentShell {
    child: Child,
    stdin: ChildStdin,
    lines_rx: Receiver<String>,
}

impl PersistentShell {
    fn spawn(workspace: &Path, policy: &crate::sandbox::PermissionPolicy) -> Result<Self> {
        let mut cmd = if cfg!(windows) {
            let mut c = Command::new("cmd");
            c.args(["/Q", "/K"]).current_dir(workspace);
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
                const CREATE_NO_WINDOW: u32 = 0x0800_0000;
                c.creation_flags(CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW);
            }
            c
        } else {
            let mut c = Command::new("sh");
            c.current_dir(workspace);
            c
        };

        cmd.env("PYTHONDONTWRITEBYTECODE", "1");
        cmd.env("DEBIAN_FRONTEND", "noninteractive");
        cmd.env("GIT_TERMINAL_PROMPT", "0");
        cmd.env("GIT_EDITOR", "true");
        cmd.env("PAGER", "cat");
        cmd.env("PYTHONUNBUFFERED", "1");
        cmd.env("TERM", "dumb");
        crate::sandbox::scrub_command_env(&mut cmd, policy);
        crate::sandbox::apply_network_policy_env(&mut cmd, policy);

        let mut child = cmd
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow::anyhow!("persistent shell stdin unavailable"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("persistent shell stdout unavailable"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| anyhow::anyhow!("persistent shell stderr unavailable"))?;

        let (tx, rx) = mpsc::channel::<String>();
        let tx_err = tx.clone();

        std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            let mut line = String::new();
            while let Ok(n) = reader.read_line(&mut line) {
                if n == 0 {
                    break;
                }
                let _ = tx.send(line.trim_end_matches(&['\r', '\n'][..]).to_string());
                line.clear();
            }
        });
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stderr);
            let mut line = String::new();
            while let Ok(n) = reader.read_line(&mut line) {
                if n == 0 {
                    break;
                }
                let trimmed = line.trim_end_matches(&['\r', '\n'][..]);
                let _ = tx_err.send(format!("[stderr] {trimmed}"));
                line.clear();
            }
        });

        let mut shell = Self {
            child,
            stdin,
            lines_rx: rx,
        };
        // Synchronize initial prompt/banner output before first command
        if cfg!(windows) {
            let _ = writeln!(shell.stdin, "@echo off");
            let _ = shell.stdin.flush();
        }
        Ok(shell)
    }

    fn is_alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    fn exec_command(
        &mut self,
        cmd_line: &str,
        timeout_secs: u64,
        cancel: &AtomicBool,
    ) -> Result<(i32, String)> {
        // Drain any stale lines from previous commands
        while self.lines_rx.try_recv().is_ok() {}

        let nonce = format!(
            "__MEG_DONE_{:x}__",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        );
        if cfg!(windows) {
            writeln!(self.stdin, "{cmd_line}")?;
            writeln!(self.stdin, "echo {nonce} %ERRORLEVEL%")?;
        } else {
            writeln!(self.stdin, "{cmd_line}")?;
            writeln!(self.stdin, "printf '\\n{nonce} %d\\n' $?")?;
        }
        self.stdin.flush()?;

        let deadline = Instant::now() + Duration::from_secs(timeout_secs);
        let mut collected: Vec<String> = Vec::new();
        let mut total_bytes = 0usize;

        loop {
            if cancel.load(Ordering::SeqCst) {
                self.kill();
                bail!("Run cancelled");
            }
            if Instant::now() >= deadline {
                self.kill();
                bail!("shell_session command timed out after {timeout_secs}s");
            }
            match self.lines_rx.recv_timeout(Duration::from_millis(25)) {
                Ok(line) => {
                    if let Some(pos) = line.find(&nonce) {
                        let before = line[..pos].trim_end();
                        if !before.is_empty() && total_bytes < MAX_SESSION_OUTPUT * 2 {
                            collected.push(before.to_string());
                        }
                        let code_str = line[pos + nonce.len()..].trim();
                        let exit_code = code_str.parse::<i32>().unwrap_or(0);
                        let mut body = collected.join("\n");
                        if body.len() > MAX_SESSION_OUTPUT {
                            let start = body.floor_char_boundary(HEAD);
                            let end = body.ceil_char_boundary(body.len().saturating_sub(TAIL));
                            body = format!(
                                "{}\n...[truncated {} bytes]...\n{}",
                                &body[..start],
                                end.saturating_sub(start),
                                &body[end..]
                            );
                        }
                        body = crate::security_guards::scrub_sal_observation(&body);
                        return Ok((exit_code, body));
                    }
                    if total_bytes < MAX_SESSION_OUTPUT * 2 {
                        total_bytes += line.len() + 1;
                        collected.push(line);
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if !self.is_alive() {
                        bail!("persistent shell exited unexpectedly");
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    bail!("persistent shell output channel closed");
                }
            }
        }
    }

    fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for PersistentShell {
    fn drop(&mut self) {
        self.kill();
    }
}

#[derive(Clone)]
pub struct ShellSessionManager {
    workspace: PathBuf,
    sessions: Arc<Mutex<HashMap<String, PersistentShell>>>,
}

impl ShellSessionManager {
    pub fn new(workspace: &Path) -> Self {
        Self {
            workspace: workspace.to_path_buf(),
            sessions: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn execute(&self, args: &Value, cancel: &AtomicBool) -> Result<String> {
        if cancel.load(Ordering::SeqCst) {
            bail!("Run cancelled");
        }
        let action = args
            .get("action")
            .and_then(Value::as_str)
            .unwrap_or("exec");
        let session_id = args
            .get("session_id")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .unwrap_or("default")
            .to_string();

        match action {
            "status" => {
                let mut map = self.sessions.lock().unwrap();
                let alive = map
                    .get_mut(&session_id)
                    .map(|s| s.is_alive())
                    .unwrap_or(false);
                Ok(format!(
                    "shell_session '{session_id}': {}",
                    if alive { "running" } else { "closed" }
                ))
            }
            "close" | "reset" => {
                let mut map = self.sessions.lock().unwrap();
                let existed = map.remove(&session_id).is_some();
                Ok(format!(
                    "shell_session '{session_id}' {}",
                    if existed { "closed" } else { "was not running" }
                ))
            }
            "exec" => {
                let cmd_line = args
                    .get("command")
                    .or_else(|| args.get("cmd"))
                    .and_then(Value::as_str)
                    .ok_or_else(|| anyhow::anyhow!("shell_session exec requires 'command'"))?;
                crate::security_guards::validate_compound_bash(cmd_line)?;
                let policy = crate::sandbox::PermissionPolicy::load(&self.workspace);
                policy.enforce_command(cmd_line)?;
                let allowed_repo = std::env::var("MEG_ALLOWED_GH_REPO").ok();
                if let Err(msg) = crate::deliverable_guard::check_gh_privacy_and_publish(
                    cmd_line,
                    allowed_repo.as_deref(),
                ) {
                    bail!("{msg}");
                }
                if policy.sandbox == crate::sandbox::SandboxMode::Strict {
                    bail!(
                        "SANDBOX_STRICT_MODE: persistent shell_session is disabled in MEG_SANDBOX=strict; use isolated one-shot 'bash'"
                    );
                }
                let timeout_secs = args
                    .get("timeout_seconds")
                    .and_then(Value::as_u64)
                    .unwrap_or(60)
                    .clamp(1, 600);

                let mut map = self.sessions.lock().unwrap();
                let need_spawn = match map.get_mut(&session_id) {
                    Some(s) => !s.is_alive(),
                    None => true,
                };
                if need_spawn {
                    let shell = PersistentShell::spawn(&self.workspace, &policy)?;
                    map.insert(session_id.clone(), shell);
                }
                let shell = map.get_mut(&session_id).unwrap();
                match shell.exec_command(cmd_line, timeout_secs, cancel) {
                    Ok((code, body)) => {
                        let formatted = format!("[exit {code}]\n{body}");
                        if code != 0 {
                            bail!("{formatted}");
                        }
                        Ok(formatted)
                    }
                    Err(e) => {
                        map.remove(&session_id);
                        Err(e)
                    }
                }
            }
            other => bail!("unknown shell_session action '{other}'; expected exec, status, reset, or close"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn persistent_shell_preserves_cwd_and_env_across_calls_and_enforces_guards() {
        let ws = std::env::temp_dir().join(format!("meg-pty-shell-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&ws);
        let sub = ws.join("subdir");
        std::fs::create_dir_all(&sub).unwrap();
        let mgr = ShellSessionManager::new(&ws);
        let cancel = AtomicBool::new(false);

        // Change directory + set session env var
        let set_cmd = if cfg!(windows) {
            "cd subdir && set MEG_SESSION_VAR=hello_stateful"
        } else {
            "cd subdir && export MEG_SESSION_VAR=hello_stateful"
        };
        let r1 = mgr.execute(&json!({"action": "exec", "command": set_cmd}), &cancel).unwrap();
        assert!(r1.starts_with("[exit 0]"), "r1={r1}");

        // Next call sees persisted cwd + env var
        let check_cmd = if cfg!(windows) {
            "echo %CD% :: %MEG_SESSION_VAR%"
        } else {
            "printf '%s :: %s\\n' \"$PWD\" \"$MEG_SESSION_VAR\""
        };
        let r2 = mgr.execute(&json!({"action": "exec", "command": check_cmd}), &cancel).unwrap();
        assert!(r2.contains("subdir"), "cwd must persist: {r2}");
        assert!(r2.contains("hello_stateful"), "env must persist: {r2}");

        // Compound guard still blocks .env access inside persistent session
        let blocked = mgr.execute(&json!({"action": "exec", "command": "cat .env"}), &cancel);
        assert!(blocked.is_err(), "compound bash guard must block .env in shell_session");

        // Close cleans up session cleanly
        let closed = mgr.execute(&json!({"action": "close"}), &cancel).unwrap();
        assert!(closed.contains("closed"));
        let _ = std::fs::remove_dir_all(&ws);
    }
}
