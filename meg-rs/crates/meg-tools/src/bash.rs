//! bash tool — Prompt-1 Task 1.2 (F3 fix): explicit `timeout_seconds`
//! (default 60, max 600), detached process group, forced non-interactive
//! env (DEBIAN_FRONTEND/GIT_TERMINAL_PROMPT/GIT_EDITOR/PAGER/PYTHONUNBUFFERED),
//! full-tree kill on timeout, and the 8,000-char output cap (head 3,500 +
//! tail 3,500).

use anyhow::{bail, Result};
use serde_json::Value;
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const HEAD: usize = 3_500;
const TAIL: usize = 3_500;
const MAX_OUTPUT: usize = 8_000;

fn headless_env(cmd: &mut Command) {
    // Forced non-interactive environment — no apt prompts, no git editors,
    // no pagers, unbuffered python. These are the F3 killers.
    cmd.env("DEBIAN_FRONTEND", "noninteractive");
    cmd.env("GIT_TERMINAL_PROMPT", "0");
    cmd.env("GIT_EDITOR", "true");
    cmd.env("PAGER", "cat");
    cmd.env("PYTHONUNBUFFERED", "1");
    cmd.env("TERM", "dumb");
}

#[cfg(windows)]
fn spawn_detached(cmd: &mut Command) -> Result<std::process::Child> {
    use std::os::windows::process::CommandExt;
    // CREATE_NEW_PROCESS_GROUP (0x200) + CREATE_NO_WINDOW (0x08000000):
    // the child gets its own group so taskkill /T can take the whole tree.
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    cmd.creation_flags(CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW);
    Ok(cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?)
}

#[cfg(unix)]
fn spawn_detached(cmd: &mut Command) -> Result<std::process::Child> {
    use std::os::unix::process::CommandExt;
    unsafe {
        cmd.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    Ok(cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?)
}

#[cfg(windows)]
fn kill_tree(pid: u32) {
    let _ = Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

#[cfg(unix)]
fn kill_tree(pid: u32) {
    let _ = Command::new("kill")
        .args(["-9", &format!("-{pid}")]) // negative pid = whole process group
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

fn bounded_output<R: Read>(mut reader: R) -> String {
    let mut head = Vec::new();
    let mut tail = std::collections::VecDeque::new();
    let mut total = 0usize;
    let mut chunk = [0u8; 8192];
    while let Ok(n) = reader.read(&mut chunk) {
        if n == 0 {
            break;
        }
        for byte in &chunk[..n] {
            total += 1;
            if head.len() < HEAD {
                head.push(*byte);
            } else {
                tail.push_back(*byte);
                if tail.len() > TAIL {
                    tail.pop_front();
                }
            }
        }
    }
    let mut out = String::from_utf8_lossy(&head).into_owned();
    let hidden = total.saturating_sub(head.len() + tail.len());
    if hidden > 0 {
        out.push_str(&format!("\n...[truncated {hidden} bytes]...\n"));
    }
    out.push_str(&String::from_utf8_lossy(
        &tail.into_iter().collect::<Vec<_>>(),
    ));
    out
}

#[cfg(windows)]
struct ProcessJob(windows_sys::Win32::Foundation::HANDLE);
#[cfg(windows)]
impl ProcessJob {
    fn attach(child: &std::process::Child, mode: crate::sandbox::SandboxMode) -> Result<Self> {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::System::JobObjects::*;
        unsafe {
            let handle = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if handle.is_null() {
                return Err(std::io::Error::last_os_error().into());
            }
            let job = Self(handle);
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            let mut flags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
                | JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION;
            let job_cfg = crate::sandbox::windows_job_limits(mode);
            if let Some(cfg) = job_cfg {
                flags |= JOB_OBJECT_LIMIT_ACTIVE_PROCESS | JOB_OBJECT_LIMIT_JOB_MEMORY;
                limits.BasicLimitInformation.ActiveProcessLimit = cfg.max_active_processes;
                limits.JobMemoryLimit = cfg.max_job_memory_bytes;
            }
            limits.BasicLimitInformation.LimitFlags = flags;
            if SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                &limits as *const _ as *const _,
                std::mem::size_of_val(&limits) as u32,
            ) == 0
            {
                return Err(std::io::Error::last_os_error().into());
            }
            if let Some(cfg) = job_cfg {
                if cfg.apply_ui_limits {
                    crate::sandbox::apply_windows_job_ui_restrictions(handle)?;
                }
            }
            if AssignProcessToJobObject(handle, child.as_raw_handle() as _) == 0 {
                return Err(std::io::Error::last_os_error().into());
            }
            Ok(job)
        }
    }
}
#[cfg(windows)]
impl Drop for ProcessJob {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.0);
        }
    }
}


fn build_sandboxed_command(
    workspace: &Path,
    cmd_line: &str,
    use_powershell: bool,
    policy: &crate::sandbox::PermissionPolicy,
) -> Result<Command> {
    if cfg!(windows) {
        if use_powershell {
            let translated = translate_posix_for_powershell51(cmd_line);
            let mut c = Command::new("powershell.exe");
            c.args(["-NoProfile", "-NonInteractive", "-Command", &translated])
                .current_dir(workspace);
            return Ok(c);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            let mut c = Command::new("cmd");
            c.arg("/S")
                .arg("/C")
                .raw_arg(format!("\"{cmd_line}\""))
                .current_dir(workspace);
            return Ok(c);
        }
        #[cfg(not(windows))]
        {
            let mut c = Command::new("cmd");
            c.arg("/C").arg(cmd_line).current_dir(workspace);
            return Ok(c);
        }
    }

    if let Some(wrapped) = crate::sandbox::maybe_wrap_unix_command(workspace, cmd_line, policy)? {
        return Ok(wrapped);
    }

    let mut c = Command::new("sh");
    c.arg("-c").arg(cmd_line).current_dir(workspace);
    Ok(c)
}
pub fn run(workspace: &Path, args: &Value) -> Result<String> {
    run_cancellable(workspace, args, &std::sync::atomic::AtomicBool::new(false))
}

pub fn run_cancellable(
    workspace: &Path,
    args: &Value,
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<String> {
    use std::sync::atomic::Ordering;
    if cancel.load(Ordering::SeqCst) {
        bail!("Run cancelled");
    }
    let cmd_line = args
        .get("command")
        .or_else(|| args.get("cmd"))
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("missing 'command'"))?;
    crate::security_guards::validate_compound_bash(cmd_line)?;
    let policy = crate::sandbox::PermissionPolicy::load(workspace);
    policy.enforce_command(cmd_line)?;
    let allowed_repo = std::env::var("MEG_ALLOWED_GH_REPO").ok();
    if let Err(msg) =
        crate::deliverable_guard::check_gh_privacy_and_publish(cmd_line, allowed_repo.as_deref())
    {
        bail!("{msg}");
    }
    let timeout_secs = args
        .get("timeout_seconds")
        .and_then(Value::as_u64)
        .unwrap_or(60)
        .clamp(1, 600);
    let use_ps = args.get("shell").and_then(Value::as_str) == Some("powershell")
        || (cfg!(windows) && cmd_line.trim_start().starts_with("export "));
    #[cfg(windows)]
    if policy.sandbox == crate::sandbox::SandboxMode::Strict {
        crate::sandbox::probe_windows_restricted_token()?;
    }
    let mut cmd = build_sandboxed_command(workspace, cmd_line, use_ps, &policy)?;
    cmd.env("PYTHONDONTWRITEBYTECODE", "1");
    headless_env(&mut cmd);
    crate::sandbox::scrub_command_env(&mut cmd, &policy);
    crate::sandbox::apply_network_policy_env(&mut cmd, &policy);
    let mut child = spawn_detached(&mut cmd)?;
    let pid = child.id();
    #[cfg(windows)]
    let job = match ProcessJob::attach(&child, policy.sandbox) {
        Ok(job) => Some(job),
        Err(error) => {
            if child.try_wait()?.is_none() {
                kill_tree(pid);
                let _ = child.wait();
                return Err(error);
            }
            None
        }
    };
    // Drain pipes while the process is running; waiting first deadlocks when
    // output fills a Windows pipe buffer. Readers retain bounded head/tail data.
    let stdout = child
        .stdout
        .take()
        .map(|pipe| std::thread::spawn(move || bounded_output(pipe)));
    let stderr = child
        .stderr
        .take()
        .map(|pipe| std::thread::spawn(move || bounded_output(pipe)));
    let deadline = Instant::now() + Duration::from_secs(timeout_secs);
    let mut interrupted = None;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if cancel.load(Ordering::SeqCst) || Instant::now() >= deadline {
            interrupted = Some(if cancel.load(Ordering::SeqCst) {
                "Run cancelled".to_string()
            } else {
                format!("command timed out after {timeout_secs}s")
            });
            kill_tree(pid);
            break child.wait()?;
        }
        std::thread::sleep(Duration::from_millis(25));
    };
    #[cfg(windows)]
    drop(job); // Also closes any surviving descendant's inherited pipe handles.
    #[cfg(unix)]
    kill_tree(pid);
    let stdout = stdout.and_then(|t| t.join().ok()).unwrap_or_default();
    let stderr = stderr.and_then(|t| t.join().ok()).unwrap_or_default();
    let mut body = if stderr.is_empty() {
        stdout
    } else {
        format!("{stdout}\n[stderr]\n{stderr}")
    };
    if body.len() > MAX_OUTPUT {
        let start = body.floor_char_boundary(HEAD);
        let end = body.ceil_char_boundary(body.len().saturating_sub(TAIL));
        body = format!(
            "{}\n...[truncated {} bytes]...\n{}",
            &body[..start],
            end - start,
            &body[end..]
        );
    }
    body = crate::security_guards::scrub_sal_observation(&body);
    if let Some(reason) = interrupted {
        bail!("{reason}; process tree killed.\n{body}");
    }
    if !status.success() {
        bail!("exit code: {}\n{body}", status.code().unwrap_or(-1));
    }
    Ok(format!("exit code: 0\n{body}"))
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::path::PathBuf;

    fn tempdir() -> PathBuf {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let path = std::env::temp_dir().join(format!("meg-bash-{}-{}", std::process::id(), n));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn simple_command_captures_output() {
        let ws = tempdir();
        let script = if cfg!(windows) {
            "echo hello-meg"
        } else {
            "echo hello-meg"
        };
        let out = run(&ws, &json!({"command": script, "timeout_seconds": 10})).unwrap();
        assert!(out.contains("hello-meg"), "{out}");
        assert!(out.contains("exit code: 0"));
    }

    #[test]
    fn timeout_kills_and_reports() {
        let ws = tempdir();
        let script = if cfg!(windows) {
            "ping -n 30 127.0.0.1"
        } else {
            "sleep 30"
        };
        let started = std::time::Instant::now();
        let err = run(&ws, &json!({"command": script, "timeout_seconds": 2}));
        assert!(err.is_err(), "expected timeout error");
        assert!(
            started.elapsed().as_secs() < 10,
            "kill must be prompt, not 30s"
        );
        assert!(err.unwrap_err().to_string().contains("timed out"));
    }

    #[test]
    fn output_capped_at_8000() {
        let ws = tempdir();
        let script = if cfg!(windows) {
            "for /L %i in (1,1,2000) do @echo line %i abcdefghijklmnop"
        } else {
            "seq 1 2000 | awk '{printf \"line %d abcdefghijklmnop\\n\", $1}'"
        };
        let out = run(&ws, &json!({"command": script, "timeout_seconds": 60})).unwrap();
        assert!(out.len() <= 8_500, "output was {} chars", out.len());
        assert!(out.contains("truncated"));
    }

    #[test]
    fn explicit_cancellation_kills_owned_shell_promptly() {
        let ws = tempdir();
        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let started = Instant::now();
        let worker = std::thread::spawn(move || {
            run_cancellable(
                &ws,
                &json!({"command":if cfg!(windows){"ping -n 30 127.0.0.1"}else{"sleep 30"},"timeout_seconds":30}),
                &worker_cancel,
            )
        });
        std::thread::sleep(Duration::from_millis(150));
        cancel.store(true, std::sync::atomic::Ordering::SeqCst);
        assert!(worker
            .join()
            .unwrap()
            .unwrap_err()
            .to_string()
            .contains("cancelled"));
        assert!(started.elapsed() < Duration::from_secs(3));
    }
}

/// Translate common POSIX idioms (`export K=V`, `rm -rf`, `&&`) into Windows
/// PowerShell 5.1-safe syntax with explicit UTF-8 output encoding.
pub fn translate_posix_for_powershell51(command: &str) -> String {
    let mut out = command.trim().to_string();
    if let Some(rest) = out.strip_prefix("export ") {
        if let Some((k, v)) = rest.split_once('=') {
            out = format!("$env:{} = {}", k.trim(), v.trim());
        }
    }
    if out.contains("rm -rf ") {
        out = out.replace("rm -rf ", "Remove-Item -Recurse -Force ");
    }
    // PowerShell 5.1 does not support `&&`; replace top-level ` && ` with `; if ($?) { ... }`
    if out.contains(" && ") {
        let parts: Vec<&str> = out.split(" && ").collect();
        let mut chained = parts[0].to_string();
        for p in &parts[1..] {
            chained.push_str(&format!("; if ($?) {{ {p} }}"));
        }
        out = chained;
    }
    format!("[Console]::OutputEncoding = [System.Text.Encoding]::UTF8; {out}")
}

/// Cap `make -j` parallelism to `max(1, RAM_GB / 2)` so builds never OOM.
pub fn cap_make_jobs(ram_gb: u64) -> u64 {
    (ram_gb / 2).max(1)
}

/// Build cross-OS sandbox argv (`bubblewrap --unshare-all --unshare-net` on Linux,
/// `sandbox-exec` Seatbelt on macOS).
pub fn sandbox_command_preview(os: &str, workspace: &Path, inner_cmd: &str) -> Vec<String> {
    let ws = workspace.to_string_lossy().to_string();
    match os {
        "linux" => vec![
            "bwrap".into(),
            "--unshare-all".into(),
            "--unshare-net".into(),
            "--bind".into(),
            ws.clone(),
            ws,
            "--".into(),
            "sh".into(),
            "-lc".into(),
            inner_cmd.into(),
        ],
        "macos" => vec![
            "sandbox-exec".into(),
            "-p".into(),
            format!("(version 1)(deny default)(allow file-read* file-write* (subpath \"{ws}\"))"),
            "sh".into(),
            "-lc".into(),
            inner_cmd.into(),
        ],
        _ => vec!["powershell".into(), "-NoProfile".into(), "-Command".into(), translate_posix_for_powershell51(inner_cmd)],
    }
}

/// Wrap a background daemon command with detached stdio FDs so pipes never hang.
pub fn detach_daemon_command(cmd: &str, log_path: &str) -> String {
    format!("{cmd} </dev/null >{log_path} 2>&1 &")
}

#[cfg(test)]
mod p1_08_tests {
    use super::*;

    #[test]
    fn powershell51_translator_and_ram_cap_and_sandbox_preview() {
        let ps = translate_posix_for_powershell51("export FOO=bar && rm -rf tmp_dir");
        assert!(ps.contains("[Console]::OutputEncoding = [System.Text.Encoding]::UTF8"));
        assert!(ps.contains("$env:FOO = bar"));
        assert!(ps.contains("Remove-Item -Recurse -Force tmp_dir"));
        assert!(ps.contains("if ($?)"));

        assert_eq!(cap_make_jobs(1), 1);
        assert_eq!(cap_make_jobs(8), 4);
        assert_eq!(cap_make_jobs(32), 16);

        let linux = sandbox_command_preview("linux", Path::new("/ws"), "echo hi");
        assert!(linux.contains(&"--unshare-all".to_string()));
        assert!(linux.contains(&"--unshare-net".to_string()));
        let mac = sandbox_command_preview("macos", Path::new("/ws"), "echo hi");
        assert_eq!(mac[0], "sandbox-exec");

        let detached = detach_daemon_command("python server.py", "/tmp/daemon.log");
        assert!(detached.contains("</dev/null >/tmp/daemon.log 2>&1 &"));
    }

    #[test]
    fn live_bash_strips_secret_env_and_enforces_permissions() {
        let ws = std::env::temp_dir().join(format!("meg-bash-sandbox-e2e-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&ws);
        std::fs::create_dir_all(ws.join(".meg")).unwrap();
        std::fs::write(
            ws.join(".meg").join("permissions.json"),
            r#"{
                "deny": ["*forbidden_op*"],
                "ask": ["*needs_approval*"],
                "env_allow": ["MEG_TEST_ALLOW_SECRET_KEY"]
            }"#,
        )
        .unwrap();

        std::env::set_var("MEG_TEST_STRIP_SECRET_KEY", "SHOULD_NOT_LEAK_777");
        std::env::set_var("MEG_TEST_ALLOW_SECRET_KEY", "ALLOWED_VAL_888");

        let probe_cmd = if cfg!(windows) {
            "if defined MEG_TEST_STRIP_SECRET_KEY (echo STRIP=LEAKED) else (echo STRIP=CLEAN) && if defined MEG_TEST_ALLOW_SECRET_KEY (echo ALLOW=PRESENT) else (echo ALLOW=MISSING)"
        } else {
            "[ -n \"$MEG_TEST_STRIP_SECRET_KEY\" ] && echo STRIP=LEAKED || echo STRIP=CLEAN; [ -n \"$MEG_TEST_ALLOW_SECRET_KEY\" ] && echo ALLOW=PRESENT || echo ALLOW=MISSING"
        };

        let out = run(&ws, &serde_json::json!({ "command": probe_cmd })).unwrap();
        std::env::remove_var("MEG_TEST_STRIP_SECRET_KEY");
        std::env::remove_var("MEG_TEST_ALLOW_SECRET_KEY");

        assert!(out.contains("STRIP=CLEAN"), "secret env var leaked into child shell: {out}");
        assert!(out.contains("ALLOW=PRESENT"), "env_allow var was unexpectedly stripped: {out}");

        let denied = run(&ws, &serde_json::json!({ "command": "echo forbidden_op" }))
            .expect_err("deny rule must block command");
        assert!(denied.to_string().contains("PERMISSION_DENIED"), "{denied}");

        let asked = run(&ws, &serde_json::json!({ "command": "echo needs_approval" }))
            .expect_err("ask rule must fail closed without MEG_APPROVE_ASK=1");
        assert!(asked.to_string().contains("PERMISSION_ASK_REQUIRED"), "{asked}");

        let _ = std::fs::remove_dir_all(&ws);
    }

    #[test]
    fn live_bash_strict_sandbox_enforces_job_limits_and_network_sinkhole() {
        let ws = std::env::temp_dir().join(format!("meg-bash-strict-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&ws);
        std::fs::create_dir_all(ws.join(".meg")).unwrap();
        std::fs::write(
            ws.join(".meg").join("permissions.json"),
            r#"{"sandbox": "strict", "network": false}"#,
        )
        .unwrap();

        let cmd = if cfg!(windows) {
            "echo PROXY=%HTTP_PROXY%"
        } else {
            "echo PROXY=$HTTP_PROXY"
        };
        let out = run(&ws, &serde_json::json!({"command": cmd})).unwrap();
        assert!(
            out.contains("PROXY=http://127.0.0.1:9"),
            "expected network-off sinkhole proxy under strict sandbox, got: {out}"
        );

        let _ = std::fs::remove_dir_all(&ws);
    }
}
