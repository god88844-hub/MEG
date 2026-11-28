//! OS-level Act-mode confinement, secret environment stripping, and granular
//! `.meg/permissions.json` (`allow` / `ask` / `deny`) policy enforcement.

use anyhow::{bail, Result};
use serde_json::Value;
use std::collections::HashSet;
use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SandboxMode {
    Off,
    Auto,
    Strict,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionDecision {
    Allow,
    Ask { rule: String },
    Deny { rule: String },
}

#[derive(Debug, Clone)]
pub struct PermissionPolicy {
    pub deny: Vec<String>,
    pub ask: Vec<String>,
    pub allow: Vec<String>,
    pub env_allow: HashSet<String>,
    pub sandbox: SandboxMode,
    pub allow_network: bool,
}

impl Default for PermissionPolicy {
    fn default() -> Self {
        Self {
            deny: Vec::new(),
            ask: Vec::new(),
            allow: Vec::new(),
            env_allow: HashSet::new(),
            sandbox: SandboxMode::Auto,
            allow_network: true,
        }
    }
}

fn parse_mode(raw: &str) -> Option<SandboxMode> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "0" | "false" | "off" | "none" => Some(SandboxMode::Off),
        "auto" => Some(SandboxMode::Auto),
        "1" | "true" | "strict" | "bwrap" | "seatbelt" | "restricted" => Some(SandboxMode::Strict),
        _ => None,
    }
}

fn push_string_list(dst: &mut Vec<String>, val: Option<&Value>) {
    if let Some(arr) = val.and_then(Value::as_array) {
        for item in arr {
            if let Some(s) = item.as_str().map(str::trim).filter(|s| !s.is_empty()) {
                dst.push(s.to_string());
            }
        }
    }
}

impl PermissionPolicy {
    /// Load workspace `.meg/permissions.json` (plus env overrides `MEG_SANDBOX`, `MEG_SANDBOX_NET`).
    pub fn load(workspace: &Path) -> Self {
        let mut policy = Self::default();
        let cfg = workspace.join(".meg").join("permissions.json");
        if let Ok(raw) = std::fs::read_to_string(&cfg) {
            if let Ok(val) = serde_json::from_str::<Value>(&raw) {
                push_string_list(&mut policy.deny, val.get("deny"));
                push_string_list(&mut policy.ask, val.get("ask"));
                push_string_list(&mut policy.allow, val.get("allow"));
                if let Some(arr) = val.get("env_allow").and_then(Value::as_array) {
                    for item in arr {
                        if let Some(s) = item.as_str().map(str::trim).filter(|s| !s.is_empty()) {
                            policy.env_allow.insert(s.to_ascii_uppercase());
                        }
                    }
                }
                if let Some(mode_str) = val.get("sandbox").and_then(Value::as_str) {
                    if let Some(m) = parse_mode(mode_str) {
                        policy.sandbox = m;
                    }
                }
                if let Some(net) = val.get("network").and_then(Value::as_bool) {
                    policy.allow_network = net;
                }
            }
        }
        if let Ok(env_mode) = std::env::var("MEG_SANDBOX") {
            if let Some(m) = parse_mode(&env_mode) {
                policy.sandbox = m;
            }
        }
        if matches!(
            std::env::var("MEG_SANDBOX_NET").as_deref(),
            Ok("0") | Ok("false") | Ok("off")
        ) {
            policy.allow_network = false;
        }
        policy
    }

    /// Evaluate a shell command against `deny` > `ask` > `allow`.
    pub fn evaluate_command(&self, cmd_line: &str) -> PermissionDecision {
        let normalized = cmd_line.trim();
        for rule in &self.deny {
            if glob_match_ci(rule, normalized) {
                return PermissionDecision::Deny { rule: rule.clone() };
            }
        }
        for rule in &self.allow {
            if glob_match_ci(rule, normalized) {
                return PermissionDecision::Allow;
            }
        }
        for rule in &self.ask {
            if glob_match_ci(rule, normalized) {
                return PermissionDecision::Ask { rule: rule.clone() };
            }
        }
        PermissionDecision::Allow
    }

    /// Enforce policy for a bash command, failing closed on `Deny` and unapproved `Ask`.
    pub fn enforce_command(&self, cmd_line: &str) -> Result<()> {
        match self.evaluate_command(cmd_line) {
            PermissionDecision::Allow => Ok(()),
            PermissionDecision::Deny { rule } => bail!(
                "PERMISSION_DENIED: command blocked by .meg/permissions.json deny rule '{rule}'"
            ),
            PermissionDecision::Ask { rule } => {
                if std::env::var("MEG_APPROVE_ASK").as_deref() == Ok("1") {
                    Ok(())
                } else {
                    bail!(
                        "PERMISSION_ASK_REQUIRED: command matches ask rule '{rule}' (approve interactively or set MEG_APPROVE_ASK=1)"
                    )
                }
            }
        }
    }
}

/// Simple case-insensitive wildcard matcher supporting `*` and `?`.
pub fn glob_match_ci(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.to_ascii_lowercase().chars().collect();
    let t: Vec<char> = text.to_ascii_lowercase().chars().collect();
    let (mut pi, mut ti) = (0usize, 0usize);
    let (mut star_idx, mut match_idx) = (None, 0usize);
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star_idx = Some(pi);
            match_idx = ti;
            pi += 1;
        } else if let Some(si) = star_idx {
            pi = si + 1;
            match_idx += 1;
            ti = match_idx;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

/// Returns true if an environment variable name likely holds a credential, API key, or token.
pub fn is_secret_env_name(name: &str) -> bool {
    let up = name.to_ascii_uppercase();
    // Never strip standard runtime/toolchain variables.
    if matches!(
        up.as_str(),
        "PATH"
            | "PATHEXT"
            | "HOME"
            | "USERPROFILE"
            | "HOMEDRIVE"
            | "HOMEPATH"
            | "TEMP"
            | "TMP"
            | "TMPDIR"
            | "SYSTEMROOT"
            | "WINDIR"
            | "COMSPEC"
            | "APPDATA"
            | "LOCALAPPDATA"
            | "PROGRAMFILES"
            | "PROGRAMFILES(X86)"
            | "CARGO_HOME"
            | "RUSTUP_HOME"
            | "GOPATH"
            | "GOROOT"
            | "VIRTUAL_ENV"
            | "CONDA_PREFIX"
            | "TERM"
            | "LANG"
            | "LC_ALL"
            | "COLORTERM"
            | "NO_COLOR"
            | "CI"
    ) {
        return false;
    }
    if matches!(
        up.as_str(),
        "OPENAI_API_KEY"
            | "ANTHROPIC_API_KEY"
            | "GEMINI_API_KEY"
            | "GOOGLE_API_KEY"
            | "DEEPSEEK_API_KEY"
            | "OPENROUTER_API_KEY"
            | "NVIDIA_API_KEY"
            | "GROQ_API_KEY"
            | "MISTRAL_API_KEY"
            | "XAI_API_KEY"
            | "MEG_API_KEY"
            | "MEG_TUI_API_KEY"
            | "MEG_RECEIPT_KEY"
            | "GH_TOKEN"
            | "GITHUB_TOKEN"
            | "GITLAB_TOKEN"
            | "NPM_TOKEN"
            | "PYPI_TOKEN"
            | "CARGO_REGISTRY_TOKEN"
            | "HF_TOKEN"
            | "HUGGING_FACE_HUB_TOKEN"
            | "AWS_ACCESS_KEY_ID"
            | "AWS_SECRET_ACCESS_KEY"
            | "AWS_SESSION_TOKEN"
            | "AZURE_CLIENT_SECRET"
            | "DOCKER_PASSWORD"
    ) {
        return true;
    }
    up.ends_with("_API_KEY")
        || up.ends_with("_SECRET")
        || up.ends_with("_SECRET_KEY")
        || up.ends_with("_ACCESS_KEY")
        || up.ends_with("_PRIVATE_KEY")
        || up.ends_with("_AUTH_TOKEN")
        || up.ends_with("_ACCESS_TOKEN")
        || up.ends_with("_PASSWORD")
        || up.ends_with("_PASSWD")
        || up.contains("_SECRET_")
}

/// Remove sensitive environment variables from a child `Command` unless
/// `MEG_PASSTHROUGH_SECRETS=1` or explicitly allowlisted in `policy.env_allow`.
pub fn scrub_command_env(cmd: &mut Command, policy: &PermissionPolicy) -> usize {
    if std::env::var("MEG_PASSTHROUGH_SECRETS").as_deref() == Ok("1") {
        return 0;
    }
    let mut removed = 0usize;
    for (k, _) in std::env::vars_os() {
        let name = k.to_string_lossy();
        if is_secret_env_name(&name) && !policy.env_allow.contains(&name.to_ascii_uppercase()) {
            cmd.env_remove(&k);
            removed += 1;
        }
    }
    removed
}

/// Defense-in-depth proxy sinkhole when `.meg/permissions.json` or `MEG_SANDBOX_NET=0`
/// disables outbound network access. Complements OS-level `--unshare-net` / Seatbelt denial
/// and protects Windows where non-admin firewall rules are unavailable.
pub fn apply_network_policy_env(cmd: &mut Command, policy: &PermissionPolicy) {
    if !policy.allow_network {
        for key in [
            "HTTP_PROXY",
            "HTTPS_PROXY",
            "ALL_PROXY",
            "http_proxy",
            "https_proxy",
            "all_proxy",
        ] {
            cmd.env(key, "http://127.0.0.1:9");
        }
        cmd.env("NO_PROXY", "");
        cmd.env("no_proxy", "");
    }
}

/// Resource & fork-bomb bounds enforced by Windows Job Objects per sandbox mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowsJobLimits {
    pub max_active_processes: u32,
    pub max_job_memory_bytes: usize,
    pub apply_ui_limits: bool,
}

/// Resolve Windows Job Object resource limits for the active `SandboxMode`.
pub fn windows_job_limits(mode: SandboxMode) -> Option<WindowsJobLimits> {
    let env_mb = std::env::var("MEG_SANDBOX_MAX_MB")
        .ok()
        .and_then(|s| s.trim().parse::<usize>().ok())
        .filter(|&mb| mb >= 256);
    match mode {
        SandboxMode::Off => None,
        SandboxMode::Auto => Some(WindowsJobLimits {
            max_active_processes: 128,
            max_job_memory_bytes: env_mb.unwrap_or(4096).saturating_mul(1024 * 1024),
            apply_ui_limits: true,
        }),
        SandboxMode::Strict => Some(WindowsJobLimits {
            max_active_processes: 64,
            max_job_memory_bytes: env_mb.unwrap_or(2048).saturating_mul(1024 * 1024),
            apply_ui_limits: true,
        }),
    }
}

fn command_on_path(bin: &str) -> bool {
    let p = Path::new(bin);
    if p.components().count() > 1 {
        return p.is_file();
    }
    let Some(path_var) = std::env::var_os("PATH") else {
        return false;
    };
    for dir in std::env::split_paths(&path_var) {
        if dir.join(bin).is_file() {
            return true;
        }
    }
    false
}

/// Build an OS-confined command preview for Linux (`bwrap`) or macOS (`sandbox-exec`).
pub fn unix_sandbox_argv(
    platform: &str,
    workspace: &Path,
    cmd_line: &str,
    allow_network: bool,
) -> Vec<String> {
    let ws = workspace.display().to_string();
    match platform {
        "linux" => {
            // Mount /tmp BEFORE binding workspace so workspaces under /tmp are not shadowed.
            let mut argv = vec![
                "bwrap".into(),
                "--ro-bind".into(),
                "/".into(),
                "/".into(),
                "--dev".into(),
                "/dev".into(),
                "--proc".into(),
                "/proc".into(),
                "--tmpfs".into(),
                "/tmp".into(),
                "--bind".into(),
                ws.clone(),
                ws.clone(),
                "--chdir".into(),
                ws,
                "--unshare-pid".into(),
                "--new-session".into(),
                "--die-with-parent".into(),
            ];
            if !allow_network {
                argv.push("--unshare-net".into());
            }
            argv.extend(["sh".into(), "-c".into(), cmd_line.into()]);
            argv
        }
        "macos" => {
            let escaped_ws = ws.replace('\\', "\\\\").replace('"', "\\\"");
            let canon_extra = std::fs::canonicalize(workspace)
                .ok()
                .map(|p| p.display().to_string())
                .filter(|c| c != &ws)
                .map(|c| {
                    let esc = c.replace('\\', "\\\\").replace('"', "\\\"");
                    format!(" (subpath \"{esc}\")")
                })
                .unwrap_or_default();
            let net_rule = if allow_network {
                "(allow network*)"
            } else {
                "(deny network*)"
            };
            let profile = format!(
                "(version 1)(deny default)(allow process*)(allow sysctl-read)(allow file-read*){net_rule}(allow file-write* (subpath \"{escaped_ws}\"){canon_extra} (subpath \"/tmp\") (subpath \"/private/tmp\") (subpath \"/dev/null\"))"
            );
            vec![
                "/usr/bin/sandbox-exec".into(),
                "-p".into(),
                profile,
                "sh".into(),
                "-c".into(),
                cmd_line.into(),
            ]
        }
        _ => vec!["sh".into(), "-c".into(), cmd_line.into()],
    }
}

/// Resolve whether Linux/macOS OS sandboxing should wrap this command.
/// Fails closed if `SandboxMode::Strict` is active and the OS sandbox binary is missing.
pub fn maybe_wrap_unix_command(
    workspace: &Path,
    cmd_line: &str,
    policy: &PermissionPolicy,
) -> Result<Option<Command>> {
    if policy.sandbox == SandboxMode::Off {
        return Ok(None);
    }
    if cfg!(target_os = "linux") {
        static BWRAP_USABLE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        let bwrap_ok = *BWRAP_USABLE.get_or_init(|| {
            if !command_on_path("bwrap") {
                return false;
            }
            Command::new("bwrap")
                .args(["--ro-bind", "/", "/", "--unshare-pid", "--", "true"])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .map(|s| s.success())
                .unwrap_or(false)
        });
        if bwrap_ok {
            let argv = unix_sandbox_argv("linux", workspace, cmd_line, policy.allow_network);
            let mut c = Command::new(&argv[0]);
            c.args(&argv[1..]).current_dir(workspace);
            return Ok(Some(c));
        }
        if policy.sandbox == SandboxMode::Strict {
            bail!("SANDBOX_UNAVAILABLE: MEG_SANDBOX=strict requires usable 'bwrap' (bubblewrap) on Linux");
        }
    } else if cfg!(target_os = "macos") {
        let has_seatbelt = Path::new("/usr/bin/sandbox-exec").is_file();
        if has_seatbelt {
            let argv = unix_sandbox_argv("macos", workspace, cmd_line, policy.allow_network);
            let mut c = Command::new(&argv[0]);
            c.args(&argv[1..]).current_dir(workspace);
            return Ok(Some(c));
        }
        if policy.sandbox == SandboxMode::Strict {
            bail!("SANDBOX_UNAVAILABLE: MEG_SANDBOX=strict requires '/usr/bin/sandbox-exec' on macOS");
        }
    }
    Ok(None)
}

/// Apply Windows Job Object UI & handle isolation (`JOBOBJECT_BASIC_UI_RESTRICTIONS`)
/// so Act-mode child processes cannot read/write clipboard, access user desktop windows,
/// modify display/system parameters, or trigger ExitWindows.
#[cfg(windows)]
pub(crate) fn apply_windows_job_ui_restrictions(job: windows_sys::Win32::Foundation::HANDLE) -> Result<()> {
    use windows_sys::Win32::System::JobObjects::*;
    unsafe {
        let ui = JOBOBJECT_BASIC_UI_RESTRICTIONS {
            UIRestrictionsClass: JOB_OBJECT_UILIMIT_DESKTOP
                | JOB_OBJECT_UILIMIT_DISPLAYSETTINGS
                | JOB_OBJECT_UILIMIT_EXITWINDOWS
                | JOB_OBJECT_UILIMIT_GLOBALATOMS
                | JOB_OBJECT_UILIMIT_HANDLES
                | JOB_OBJECT_UILIMIT_READCLIPBOARD
                | JOB_OBJECT_UILIMIT_SYSTEMPARAMETERS
                | JOB_OBJECT_UILIMIT_WRITECLIPBOARD,
        };
        let ok = SetInformationJobObject(
            job,
            JobObjectBasicUIRestrictions,
            &ui as *const _ as *const std::ffi::c_void,
            std::mem::size_of::<JOBOBJECT_BASIC_UI_RESTRICTIONS>() as u32,
        );
        if ok == 0 {
            bail!("SetInformationJobObject(JobObjectBasicUIRestrictions) failed");
        }
    }
    Ok(())
}

/// Verify Windows `CreateRestrictedToken(DISABLE_MAX_PRIVILEGE)` works on this host.
#[cfg(windows)]
pub fn probe_windows_restricted_token() -> Result<()> {
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::Security::{
        CreateRestrictedToken, DISABLE_MAX_PRIVILEGE, TOKEN_DUPLICATE, TOKEN_QUERY,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
    unsafe {
        let mut proc_token: HANDLE = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_DUPLICATE | TOKEN_QUERY, &mut proc_token) == 0 {
            bail!("OpenProcessToken failed");
        }
        let mut restricted: HANDLE = std::ptr::null_mut();
        let ok = CreateRestrictedToken(
            proc_token,
            DISABLE_MAX_PRIVILEGE,
            0,
            std::ptr::null(),
            0,
            std::ptr::null(),
            0,
            std::ptr::null(),
            &mut restricted,
        );
        CloseHandle(proc_token);
        if ok == 0 || restricted.is_null() {
            bail!("CreateRestrictedToken(DISABLE_MAX_PRIVILEGE) failed");
        }
        CloseHandle(restricted);
    }
    Ok(())
}


/// Result of running a Windows command under a real `CreateRestrictedToken` primary token
/// attached to a resource + UI-restricted Job Object before the main thread resumes.
#[cfg(windows)]
struct BoundedPipeDrain {
    buf: std::sync::Arc<std::sync::Mutex<Vec<u8>>>,
    done: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

#[cfg(windows)]
impl BoundedPipeDrain {
    fn spawn(raw_handle: windows_sys::Win32::Foundation::HANDLE) -> Self {
        use std::io::Read;
        use std::os::windows::io::FromRawHandle;
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::{Arc, Mutex};
        let buf = Arc::new(Mutex::new(Vec::new()));
        let done = Arc::new(AtomicBool::new(false));
        let b = buf.clone();
        let d = done.clone();
        let mut file = unsafe { std::fs::File::from_raw_handle(raw_handle as _) };
        std::thread::spawn(move || {
            let mut tmp = [0u8; 4096];
            loop {
                match file.read(&mut tmp) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        let mut guard = b.lock().unwrap_or_else(|e| e.into_inner());
                        let rem = (64 * 1024usize).saturating_sub(guard.len());
                        if rem > 0 {
                            guard.extend_from_slice(&tmp[..n.min(rem)]);
                        }
                    }
                }
            }
            d.store(true, Ordering::Release);
        });
        Self { buf, done }
    }

    fn finish(self, max_wait: std::time::Duration) -> String {
        use std::sync::atomic::Ordering;
        let deadline = std::time::Instant::now() + max_wait;
        while !self.done.load(Ordering::Acquire) && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let guard = self.buf.lock().unwrap_or_else(|e| e.into_inner());
        String::from_utf8_lossy(&guard).into_owned()
    }
}

#[cfg(windows)]
pub(crate) struct RestrictedWindowsOutput {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub interrupted: Option<String>,
}

#[cfg(windows)]
fn to_wide_null(s: &str) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    std::ffi::OsStr::new(s)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

#[cfg(windows)]
fn build_wide_env_block(policy: &PermissionPolicy, temp_override: Option<&Path>) -> Vec<u16> {
    use std::collections::BTreeMap;
    let mut env_map: BTreeMap<String, String> = BTreeMap::new();
    for (k, v) in std::env::vars() {
        if is_secret_env_name(&k) && !policy.env_allow.contains(&k) {
            continue;
        }
        env_map.insert(k, v);
    }
    if let Some(tmp_dir) = temp_override {
        let tmp_s = tmp_dir.display().to_string();
        env_map.insert("TEMP".into(), tmp_s.clone());
        env_map.insert("TMP".into(), tmp_s);
    }
    env_map.insert("PYTHONDONTWRITEBYTECODE".into(), "1".into());
    env_map.insert("DEBIAN_FRONTEND".into(), "noninteractive".into());
    env_map.insert("GIT_TERMINAL_PROMPT".into(), "0".into());
    env_map.insert("GIT_EDITOR".into(), "true".into());
    env_map.insert("PAGER".into(), "cat".into());
    env_map.insert("PYTHONUNBUFFERED".into(), "1".into());
    env_map.insert("TERM".into(), "dumb".into());
    if !policy.allow_network {
        for key in [
            "HTTP_PROXY",
            "HTTPS_PROXY",
            "ALL_PROXY",
            "http_proxy",
            "https_proxy",
            "all_proxy",
        ] {
            env_map.insert(key.into(), "http://127.0.0.1:9".into());
        }
        env_map.insert("NO_PROXY".into(), "".into());
        env_map.insert("no_proxy".into(), "".into());
        env_map.insert("MEG_NETWORK_DISABLED".into(), "1".into());
    }
    let mut block: Vec<u16> = Vec::new();
    for (k, v) in env_map {
        let entry = format!("{k}={v}");
        block.extend(entry.encode_utf16());
        block.push(0);
    }
    block.push(0);
    block
}

/// Spawn and run a Windows command under `CreateRestrictedToken(DISABLE_MAX_PRIVILEGE)`
/// via `CreateProcessAsUserW` (`CREATE_SUSPENDED`), attaching it to a Job Object
/// with memory/process/UI limits BEFORE resuming the thread.
#[cfg(windows)]
pub(crate) fn run_windows_restricted_process(
    workspace: &Path,
    cmd_line: &str,
    use_powershell: bool,
    policy: &PermissionPolicy,
    timeout_secs: u64,
    cancel: &std::sync::atomic::AtomicBool,
    job_handle: windows_sys::Win32::Foundation::HANDLE,
) -> Result<RestrictedWindowsOutput> {
    use std::sync::atomic::Ordering;
    use std::time::{Duration, Instant};
    use windows_sys::Win32::Foundation::{
        CloseHandle, SetHandleInformation, HANDLE, HANDLE_FLAG_INHERIT, INVALID_HANDLE_VALUE,
    };
    use windows_sys::Win32::Security::{
        CreateRestrictedToken, DISABLE_MAX_PRIVILEGE, SECURITY_ATTRIBUTES, TOKEN_ASSIGN_PRIMARY,
        TOKEN_DUPLICATE, TOKEN_QUERY,
    };
    use windows_sys::Win32::System::JobObjects::AssignProcessToJobObject;
    use windows_sys::Win32::System::Pipes::CreatePipe;
    use windows_sys::Win32::System::Threading::{
        CreateProcessAsUserW, GetCurrentProcess, GetExitCodeProcess, OpenProcessToken,
        ResumeThread, TerminateProcess, WaitForSingleObject, CREATE_NEW_PROCESS_GROUP,
        CREATE_NO_WINDOW, CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT, PROCESS_INFORMATION,
        STARTF_USESTDHANDLES, STARTUPINFOW,
    };

    let full_cmdline = if use_powershell {
        let escaped = cmd_line.replace('"', "\\\"");
        format!("powershell.exe -NoProfile -NonInteractive -Command \"{escaped}\"")
    } else {
        format!("cmd.exe /S /C \"{cmd_line}\"")
    };
    let mut cmd_w = to_wide_null(&full_cmdline);
    let cwd_w = to_wide_null(&workspace.display().to_string());
    let mut env_block = build_wide_env_block(policy, None);

    unsafe {
        let mut proc_token: HANDLE = std::ptr::null_mut();
        if OpenProcessToken(
            GetCurrentProcess(),
            TOKEN_DUPLICATE | TOKEN_QUERY | TOKEN_ASSIGN_PRIMARY,
            &mut proc_token,
        ) == 0
        {
            bail!(
                "OpenProcessToken failed: {}",
                std::io::Error::last_os_error()
            );
        }
        let mut restricted: HANDLE = std::ptr::null_mut();
        let ok = CreateRestrictedToken(
            proc_token,
            DISABLE_MAX_PRIVILEGE,
            0,
            std::ptr::null(),
            0,
            std::ptr::null(),
            0,
            std::ptr::null(),
            &mut restricted,
        );
        CloseHandle(proc_token);
        if ok == 0 || restricted.is_null() {
            bail!(
                "CreateRestrictedToken(DISABLE_MAX_PRIVILEGE) failed: {}",
                std::io::Error::last_os_error()
            );
        }

        let sa = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: std::ptr::null_mut(),
            bInheritHandle: 1,
        };
        let mut out_rd: HANDLE = std::ptr::null_mut();
        let mut out_wr: HANDLE = std::ptr::null_mut();
        let mut err_rd: HANDLE = std::ptr::null_mut();
        let mut err_wr: HANDLE = std::ptr::null_mut();
        if CreatePipe(&mut out_rd, &mut out_wr, &sa, 0) == 0 {
            CloseHandle(restricted);
            bail!("CreatePipe(stdout) failed: {}", std::io::Error::last_os_error());
        }
        if CreatePipe(&mut err_rd, &mut err_wr, &sa, 0) == 0 {
            CloseHandle(out_rd);
            CloseHandle(out_wr);
            CloseHandle(restricted);
            bail!("CreatePipe(stderr) failed: {}", std::io::Error::last_os_error());
        }
        let _ = SetHandleInformation(out_rd, HANDLE_FLAG_INHERIT, 0);
        let _ = SetHandleInformation(err_rd, HANDLE_FLAG_INHERIT, 0);

        let mut si: STARTUPINFOW = std::mem::zeroed();
        si.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
        si.dwFlags = STARTF_USESTDHANDLES;
        si.hStdInput = INVALID_HANDLE_VALUE;
        si.hStdOutput = out_wr;
        si.hStdError = err_wr;

        let mut pi: PROCESS_INFORMATION = std::mem::zeroed();
        let flags = CREATE_SUSPENDED
            | CREATE_NEW_PROCESS_GROUP
            | CREATE_NO_WINDOW
            | CREATE_UNICODE_ENVIRONMENT;
        let created = CreateProcessAsUserW(
            restricted,
            std::ptr::null(),
            cmd_w.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            1, // inherit pipe write handles
            flags,
            env_block.as_mut_ptr() as *mut std::ffi::c_void,
            cwd_w.as_ptr(),
            &si,
            &mut pi,
        );
        CloseHandle(restricted);
        CloseHandle(out_wr);
        CloseHandle(err_wr);

        if created == 0 {
            let err = std::io::Error::last_os_error();
            CloseHandle(out_rd);
            CloseHandle(err_rd);
            bail!("CreateProcessAsUserW(restricted token) failed: {err}");
        }

        if AssignProcessToJobObject(job_handle, pi.hProcess) == 0 {
            let err = std::io::Error::last_os_error();
            TerminateProcess(pi.hProcess, 1);
            CloseHandle(pi.hThread);
            CloseHandle(pi.hProcess);
            CloseHandle(out_rd);
            CloseHandle(err_rd);
            bail!("AssignProcessToJobObject(restricted) failed: {err}");
        }

        ResumeThread(pi.hThread);
        CloseHandle(pi.hThread);

        let out_drain = BoundedPipeDrain::spawn(out_rd);
        let err_drain = BoundedPipeDrain::spawn(err_rd);

        let deadline = Instant::now() + Duration::from_secs(timeout_secs);
        let mut interrupted = None;
        loop {
            let wait = WaitForSingleObject(pi.hProcess, 25);
            if wait == 0 {
                break;
            }
            if cancel.load(Ordering::SeqCst) || Instant::now() >= deadline {
                interrupted = Some(if cancel.load(Ordering::SeqCst) {
                    "Run cancelled".to_string()
                } else {
                    format!("command timed out after {timeout_secs}s")
                });
                TerminateProcess(pi.hProcess, 1);
                let _ = WaitForSingleObject(pi.hProcess, 2_000);
                break;
            }
        }
        let mut exit_code: u32 = 1;
        let _ = GetExitCodeProcess(pi.hProcess, &mut exit_code);
        CloseHandle(pi.hProcess);

        let stdout = out_drain.finish(Duration::from_millis(400));
        let stderr = err_drain.finish(Duration::from_millis(400));
        Ok(RestrictedWindowsOutput {
            exit_code: exit_code as i32,
            stdout,
            stderr,
            interrupted,
        })
    }
}


#[cfg(windows)]
fn appcontainer_profile_name(workspace: &Path) -> String {
    let canon = workspace
        .canonicalize()
        .unwrap_or_else(|_| workspace.to_path_buf());
    let raw = canon.to_string_lossy().to_ascii_lowercase();
    let mut h: u64 = 0xcbf29ce484222325;
    for b in raw.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("MEG.Sandbox.{h:016x}")
}

#[cfg(windows)]
mod win_appcontainer {
    use super::*;
    use std::ffi::c_void;
    use std::sync::atomic::Ordering;
    use std::time::{Duration, Instant};
    use windows_sys::Win32::Foundation::{
        CloseHandle, SetHandleInformation, HANDLE, HANDLE_FLAG_INHERIT,
    };
    use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
    use windows_sys::Win32::System::JobObjects::AssignProcessToJobObject;
    use windows_sys::Win32::System::Pipes::CreatePipe;
    use windows_sys::Win32::System::Threading::{
        CreateProcessW, GetExitCodeProcess, ResumeThread, TerminateProcess, WaitForSingleObject,
        CREATE_NEW_PROCESS_GROUP, CREATE_NO_WINDOW, CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT,
        PROCESS_INFORMATION, STARTF_USESTDHANDLES, STARTUPINFOW,
    };

    const EXTENDED_STARTUPINFO_PRESENT: u32 = 0x0008_0000;
    const PROC_THREAD_ATTRIBUTE_HANDLE_LIST: usize = 0x0002_0002;
    const PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES: usize = 0x0002_0009;
    const SE_GROUP_ENABLED: u32 = 0x0000_0004;
    const WIN_CAPABILITY_INTERNET_CLIENT_SID: u32 = 94;
    const SE_FILE_OBJECT: u32 = 1;
    const DACL_SECURITY_INFORMATION: u32 = 0x0000_0004;
    const GRANT_ACCESS: u32 = 1;
    const SUB_CONTAINERS_AND_OBJECTS_INHERIT: u32 = 0x3;
    const FILE_ALL_ACCESS: u32 = 0x001F_01FF;
    const HRESULT_ALREADY_EXISTS: i32 = 0x8007_00B7u32 as i32;

    #[repr(C)]
    struct SidAndAttributes {
        sid: *mut c_void,
        attributes: u32,
    }

    #[repr(C)]
    struct SecurityCapabilities {
        app_container_sid: *mut c_void,
        capabilities: *mut SidAndAttributes,
        capability_count: u32,
        reserved: u32,
    }

    #[repr(C)]
    struct StartupInfoExW {
        startup_info: STARTUPINFOW,
        lp_attribute_list: *mut c_void,
    }

    #[repr(C)]
    struct TrusteeW {
        p_multiple_trustee: *mut c_void,
        multiple_trustee_operation: u32,
        trustee_form: u32,
        trustee_type: u32,
        ptstr_name: *mut c_void,
    }

    #[repr(C)]
    struct ExplicitAccessW {
        grf_access_permissions: u32,
        grf_access_mode: u32,
        grf_inheritance: u32,
        trustee: TrusteeW,
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn LoadLibraryW(lp_lib_file_name: *const u16) -> *mut c_void;
        fn GetProcAddress(h_module: *mut c_void, lp_proc_name: *const u8) -> *const c_void;
        fn FreeLibrary(h_lib_module: *mut c_void) -> i32;
        fn LocalFree(h_mem: *mut c_void) -> *mut c_void;
        fn InitializeProcThreadAttributeList(
            lp_attribute_list: *mut c_void,
            dw_attribute_count: u32,
            dw_flags: u32,
            lp_size: *mut usize,
        ) -> i32;
        fn UpdateProcThreadAttribute(
            lp_attribute_list: *mut c_void,
            dw_flags: u32,
            attribute: usize,
            lp_value: *mut c_void,
            cb_size: usize,
            lp_previous_value: *mut c_void,
            lp_return_size: *mut usize,
        ) -> i32;
        fn DeleteProcThreadAttributeList(lp_attribute_list: *mut c_void);
    }

    #[link(name = "advapi32")]
    extern "system" {
        fn FreeSid(p_sid: *mut c_void) -> *mut c_void;
        fn CreateWellKnownSid(
            well_known_sid_type: u32,
            domain_sid: *mut c_void,
            p_sid: *mut c_void,
            cb_sid: *mut u32,
        ) -> i32;
        fn GetNamedSecurityInfoW(
            p_object_name: *const u16,
            object_type: u32,
            security_info: u32,
            ppsid_owner: *mut *mut c_void,
            ppsid_group: *mut *mut c_void,
            pp_dacl: *mut *mut c_void,
            pp_sacl: *mut *mut c_void,
            pp_security_descriptor: *mut *mut c_void,
        ) -> u32;
        fn SetEntriesInAclW(
            c_count_of_explicit_entries: u32,
            p_list_of_explicit_entries: *mut ExplicitAccessW,
            old_acl: *mut c_void,
            new_acl: *mut *mut c_void,
        ) -> u32;
        fn SetNamedSecurityInfoW(
            p_object_name: *mut u16,
            object_type: u32,
            security_info: u32,
            psid_owner: *mut c_void,
            psid_group: *mut c_void,
            p_dacl: *mut c_void,
            p_sacl: *mut c_void,
        ) -> u32;
    }

    type CreateAppContainerProfileFn = unsafe extern "system" fn(
        *const u16,
        *const u16,
        *const u16,
        *const SidAndAttributes,
        u32,
        *mut *mut c_void,
    ) -> i32;
    type DeriveAppContainerSidFn =
        unsafe extern "system" fn(*const u16, *mut *mut c_void) -> i32;
    type DeleteAppContainerProfileFn = unsafe extern "system" fn(*const u16) -> i32;

    struct UserenvApi {
        lib: *mut c_void,
        create_profile: CreateAppContainerProfileFn,
        derive_sid: DeriveAppContainerSidFn,
        delete_profile: DeleteAppContainerProfileFn,
    }

    impl UserenvApi {
        unsafe fn load() -> Result<Self> {
            let name = to_wide_null("userenv.dll");
            let lib = LoadLibraryW(name.as_ptr());
            if lib.is_null() {
                bail!("LoadLibraryW(userenv.dll) failed");
            }
            let p_create = GetProcAddress(lib, c"CreateAppContainerProfile".as_ptr().cast());
            let p_derive =
                GetProcAddress(lib, c"DeriveAppContainerSidFromAppContainerName".as_ptr().cast());
            let p_delete = GetProcAddress(lib, c"DeleteAppContainerProfile".as_ptr().cast());
            if p_create.is_null() || p_derive.is_null() || p_delete.is_null() {
                FreeLibrary(lib);
                bail!("AppContainer APIs unavailable in userenv.dll");
            }
            Ok(Self {
                lib,
                create_profile: std::mem::transmute::<*const c_void, CreateAppContainerProfileFn>(
                    p_create,
                ),
                derive_sid: std::mem::transmute::<*const c_void, DeriveAppContainerSidFn>(p_derive),
                delete_profile: std::mem::transmute::<*const c_void, DeleteAppContainerProfileFn>(
                    p_delete,
                ),
            })
        }
    }

    impl Drop for UserenvApi {
        fn drop(&mut self) {
            unsafe {
                FreeLibrary(self.lib);
            }
        }
    }

    unsafe fn grant_workspace_acl(workspace: &Path, app_sid: *mut c_void) -> Result<()> {
        let mut ws_w = to_wide_null(&workspace.display().to_string());
        let mut old_dacl: *mut c_void = std::ptr::null_mut();
        let mut sd: *mut c_void = std::ptr::null_mut();
        let err = GetNamedSecurityInfoW(
            ws_w.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut old_dacl,
            std::ptr::null_mut(),
            &mut sd,
        );
        if err != 0 {
            bail!("GetNamedSecurityInfoW(workspace) failed with error {err}");
        }
        let mut ea = ExplicitAccessW {
            grf_access_permissions: FILE_ALL_ACCESS,
            grf_access_mode: GRANT_ACCESS,
            grf_inheritance: SUB_CONTAINERS_AND_OBJECTS_INHERIT,
            trustee: TrusteeW {
                p_multiple_trustee: std::ptr::null_mut(),
                multiple_trustee_operation: 0,
                trustee_form: 0, // TRUSTEE_IS_SID
                trustee_type: 0,
                ptstr_name: app_sid,
            },
        };
        let mut new_dacl: *mut c_void = std::ptr::null_mut();
        let acl_err = SetEntriesInAclW(1, &mut ea, old_dacl, &mut new_dacl);
        if acl_err != 0 {
            if !sd.is_null() {
                LocalFree(sd);
            }
            bail!("SetEntriesInAclW(workspace) failed with error {acl_err}");
        }
        let set_err = SetNamedSecurityInfoW(
            ws_w.as_mut_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            new_dacl,
            std::ptr::null_mut(),
        );
        if !new_dacl.is_null() {
            LocalFree(new_dacl);
        }
        if !sd.is_null() {
            LocalFree(sd);
        }
        if set_err != 0 {
            bail!("SetNamedSecurityInfoW(workspace) failed with error {set_err}");
        }
        Ok(())
    }

    pub(crate) fn delete_profile(workspace: &Path) {
        let name_w = to_wide_null(&appcontainer_profile_name(workspace));
        unsafe {
            if let Ok(api) = UserenvApi::load() {
                let _ = (api.delete_profile)(name_w.as_ptr());
            }
        }
    }

    pub(crate) fn run(
        workspace: &Path,
        cmd_line: &str,
        use_powershell: bool,
        policy: &PermissionPolicy,
        timeout_secs: u64,
        cancel: &std::sync::atomic::AtomicBool,
        job_handle: HANDLE,
    ) -> Result<RestrictedWindowsOutput> {
        let full_cmdline = if use_powershell {
            let escaped = cmd_line.replace('"', "\\\"");
            format!("powershell.exe -NoProfile -NonInteractive -Command \"{escaped}\"")
        } else {
            format!("cmd.exe /S /C \"{cmd_line}\"")
        };
        let mut cmd_w = to_wide_null(&full_cmdline);
        let cwd_w = to_wide_null(&workspace.display().to_string());
        let mut env_block = build_wide_env_block(policy, Some(workspace));

        let profile_name = appcontainer_profile_name(workspace);
        let name_w = to_wide_null(&profile_name);
        let desc_w = to_wide_null("MEG workspace sandbox AppContainer");

        unsafe {
            let api = UserenvApi::load()?;
            let mut app_sid: *mut c_void = std::ptr::null_mut();
            let hr = (api.create_profile)(
                name_w.as_ptr(),
                name_w.as_ptr(),
                desc_w.as_ptr(),
                std::ptr::null(),
                0,
                &mut app_sid,
            );
            if hr == HRESULT_ALREADY_EXISTS {
                let hr2 = (api.derive_sid)(name_w.as_ptr(), &mut app_sid);
                if hr2 < 0 || app_sid.is_null() {
                    bail!("DeriveAppContainerSidFromAppContainerName failed: 0x{:08x}", hr2 as u32);
                }
            } else if hr < 0 || app_sid.is_null() {
                bail!("CreateAppContainerProfile failed: 0x{:08x}", hr as u32);
            }

            let acl_res = grant_workspace_acl(workspace, app_sid);
            if let Err(e) = acl_res {
                FreeSid(app_sid);
                return Err(e);
            }

            let mut net_sid_buf = [0u8; 68];
            let mut cap_list: Vec<SidAndAttributes> = Vec::new();
            if policy.allow_network {
                let mut cb = net_sid_buf.len() as u32;
                if CreateWellKnownSid(
                    WIN_CAPABILITY_INTERNET_CLIENT_SID,
                    std::ptr::null_mut(),
                    net_sid_buf.as_mut_ptr() as *mut c_void,
                    &mut cb,
                ) != 0
                {
                    cap_list.push(SidAndAttributes {
                        sid: net_sid_buf.as_mut_ptr() as *mut c_void,
                        attributes: SE_GROUP_ENABLED,
                    });
                }
            }

            let mut sec_caps = SecurityCapabilities {
                app_container_sid: app_sid,
                capabilities: if cap_list.is_empty() {
                    std::ptr::null_mut()
                } else {
                    cap_list.as_mut_ptr()
                },
                capability_count: cap_list.len() as u32,
                reserved: 0,
            };

            let sa = SECURITY_ATTRIBUTES {
                nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: std::ptr::null_mut(),
                bInheritHandle: 1,
            };
            let mut out_rd: HANDLE = std::ptr::null_mut();
            let mut out_wr: HANDLE = std::ptr::null_mut();
            let mut err_rd: HANDLE = std::ptr::null_mut();
            let mut err_wr: HANDLE = std::ptr::null_mut();
            if CreatePipe(&mut out_rd, &mut out_wr, &sa, 0) == 0 {
                FreeSid(app_sid);
                bail!("CreatePipe(stdout) failed");
            }
            if CreatePipe(&mut err_rd, &mut err_wr, &sa, 0) == 0 {
                CloseHandle(out_rd);
                CloseHandle(out_wr);
                FreeSid(app_sid);
                bail!("CreatePipe(stderr) failed");
            }
            let _ = SetHandleInformation(out_rd, HANDLE_FLAG_INHERIT, 0);
            let _ = SetHandleInformation(err_rd, HANDLE_FLAG_INHERIT, 0);

            let mut inherit_handles: [HANDLE; 2] = [out_wr, err_wr];
            let mut attr_size: usize = 0;
            InitializeProcThreadAttributeList(std::ptr::null_mut(), 2, 0, &mut attr_size);
            if attr_size == 0 {
                CloseHandle(out_rd);
                CloseHandle(out_wr);
                CloseHandle(err_rd);
                CloseHandle(err_wr);
                FreeSid(app_sid);
                bail!("InitializeProcThreadAttributeList returned zero size");
            }
            let mut attr_buf: Vec<u64> = vec![0u64; attr_size.div_ceil(8)];
            let attr_ptr = attr_buf.as_mut_ptr() as *mut c_void;
            if InitializeProcThreadAttributeList(attr_ptr, 2, 0, &mut attr_size) == 0 {
                let err = std::io::Error::last_os_error();
                CloseHandle(out_rd);
                CloseHandle(out_wr);
                CloseHandle(err_rd);
                CloseHandle(err_wr);
                FreeSid(app_sid);
                bail!("InitializeProcThreadAttributeList failed: {err}");
            }
            if UpdateProcThreadAttribute(
                attr_ptr,
                0,
                PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES,
                &mut sec_caps as *mut _ as *mut c_void,
                std::mem::size_of::<SecurityCapabilities>(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            ) == 0
                || UpdateProcThreadAttribute(
                    attr_ptr,
                    0,
                    PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
                    inherit_handles.as_mut_ptr() as *mut c_void,
                    std::mem::size_of_val(&inherit_handles),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                ) == 0
            {
                let err = std::io::Error::last_os_error();
                DeleteProcThreadAttributeList(attr_ptr);
                CloseHandle(out_rd);
                CloseHandle(out_wr);
                CloseHandle(err_rd);
                CloseHandle(err_wr);
                FreeSid(app_sid);
                bail!("UpdateProcThreadAttribute failed: {err}");
            }

            let mut siex: StartupInfoExW = std::mem::zeroed();
            siex.startup_info.cb = std::mem::size_of::<StartupInfoExW>() as u32;
            siex.startup_info.dwFlags = STARTF_USESTDHANDLES;
            siex.startup_info.hStdInput = std::ptr::null_mut();
            siex.startup_info.hStdOutput = out_wr;
            siex.startup_info.hStdError = err_wr;
            siex.lp_attribute_list = attr_ptr;

            let mut pi: PROCESS_INFORMATION = std::mem::zeroed();
            let flags = EXTENDED_STARTUPINFO_PRESENT
                | CREATE_SUSPENDED
                | CREATE_NEW_PROCESS_GROUP
                | CREATE_NO_WINDOW
                | CREATE_UNICODE_ENVIRONMENT;
            let created = CreateProcessW(
                std::ptr::null(),
                cmd_w.as_mut_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                1,
                flags,
                env_block.as_mut_ptr() as *mut c_void,
                cwd_w.as_ptr(),
                &siex.startup_info,
                &mut pi,
            );

            DeleteProcThreadAttributeList(attr_ptr);
            FreeSid(app_sid);
            CloseHandle(out_wr);
            CloseHandle(err_wr);

            if created == 0 {
                let err = std::io::Error::last_os_error();
                CloseHandle(out_rd);
                CloseHandle(err_rd);
                bail!("CreateProcessW(AppContainer) failed: {err}");
            }

            if AssignProcessToJobObject(job_handle, pi.hProcess) == 0 {
                let err = std::io::Error::last_os_error();
                TerminateProcess(pi.hProcess, 1);
                CloseHandle(pi.hThread);
                CloseHandle(pi.hProcess);
                CloseHandle(out_rd);
                CloseHandle(err_rd);
                bail!("AssignProcessToJobObject(AppContainer) failed: {err}");
            }

            ResumeThread(pi.hThread);
            CloseHandle(pi.hThread);

            let out_drain = BoundedPipeDrain::spawn(out_rd);
            let err_drain = BoundedPipeDrain::spawn(err_rd);

            let deadline = Instant::now() + Duration::from_secs(timeout_secs);
            let mut interrupted = None;
            loop {
                let wait = WaitForSingleObject(pi.hProcess, 25);
                if wait == 0 {
                    break;
                }
                if cancel.load(Ordering::SeqCst) || Instant::now() >= deadline {
                    interrupted = Some(if cancel.load(Ordering::SeqCst) {
                        "Run cancelled".to_string()
                    } else {
                        format!("command timed out after {timeout_secs}s")
                    });
                    TerminateProcess(pi.hProcess, 1);
                    let _ = WaitForSingleObject(pi.hProcess, 2_000);
                    break;
                }
            }
            let mut exit_code: u32 = 1;
            let _ = GetExitCodeProcess(pi.hProcess, &mut exit_code);
            CloseHandle(pi.hProcess);

            let stdout = out_drain.finish(Duration::from_millis(400));
            let stderr = err_drain.finish(Duration::from_millis(400));
            Ok(RestrictedWindowsOutput {
                exit_code: exit_code as i32,
                stdout,
                stderr,
                interrupted,
            })
        }
    }
}

/// Spawn a Windows `MEG_SANDBOX=strict` child under real AppContainer kernel FS/network
/// isolation (with workspace-scoped ACL grant + Job Object limits), falling back to
/// `CreateRestrictedToken` only if the host session disallows AppContainer profiles
/// (unless `MEG_WINDOWS_REQUIRE_APPCONTAINER=1` is set).
#[cfg(windows)]
pub(crate) fn run_windows_strict_process(
    workspace: &Path,
    cmd_line: &str,
    use_powershell: bool,
    policy: &PermissionPolicy,
    timeout_secs: u64,
    cancel: &std::sync::atomic::AtomicBool,
    job_handle: windows_sys::Win32::Foundation::HANDLE,
) -> Result<RestrictedWindowsOutput> {
    let require_ac = std::env::var("MEG_WINDOWS_REQUIRE_APPCONTAINER")
        .ok()
        .is_some_and(|v| matches!(v.as_str(), "1" | "true" | "TRUE"));
    match win_appcontainer::run(
        workspace,
        cmd_line,
        use_powershell,
        policy,
        timeout_secs,
        cancel,
        job_handle,
    ) {
        Ok(out) => Ok(out),
        Err(ac_err) if !require_ac => run_windows_restricted_process(
            workspace,
            cmd_line,
            use_powershell,
            policy,
            timeout_secs,
            cancel,
            job_handle,
        )
        .map_err(|rt_err| anyhow::anyhow!("AppContainer failed ({ac_err}); restricted-token fallback failed ({rt_err})")),
        Err(ac_err) => Err(ac_err),
    }
}

#[cfg(windows)]
pub fn delete_windows_appcontainer_profile(workspace: &Path) {
    win_appcontainer::delete_profile(workspace);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_env_classifier_strips_keys_and_preserves_runtime_vars() {
        assert!(is_secret_env_name("OPENAI_API_KEY"));
        assert!(is_secret_env_name("ANTHROPIC_API_KEY"));
        assert!(is_secret_env_name("GH_TOKEN"));
        assert!(is_secret_env_name("MY_CUSTOM_SECRET_KEY"));
        assert!(is_secret_env_name("DB_PASSWORD"));
        assert!(is_secret_env_name("AWS_SECRET_ACCESS_KEY"));

        assert!(!is_secret_env_name("PATH"));
        assert!(!is_secret_env_name("USERPROFILE"));
        assert!(!is_secret_env_name("CARGO_HOME"));
        assert!(!is_secret_env_name("RUST_LOG"));
    }

    #[test]
    fn permission_policy_enforces_deny_over_allow_and_fails_closed_on_ask() {
        let ws = std::env::temp_dir().join(format!("meg-perm-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&ws);
        std::fs::create_dir_all(ws.join(".meg")).unwrap();
        std::fs::write(
            ws.join(".meg").join("permissions.json"),
            r#"{
                "deny": ["git push --force*", "*curl*|*sh*"],
                "ask": ["git push*", "npm publish*"],
                "allow": ["git push origin dev", "cargo test*"],
                "env_allow": ["CUSTOM_TEST_TOKEN"],
                "network": false
            }"#,
        )
        .unwrap();

        let policy = PermissionPolicy::load(&ws);
        assert!(!policy.allow_network);
        assert!(policy.env_allow.contains("CUSTOM_TEST_TOKEN"));

        // Deny wins even if allow might look similar
        assert!(matches!(
            policy.evaluate_command("git push --force origin main"),
            PermissionDecision::Deny { .. }
        ));
        assert!(policy.enforce_command("git push --force origin main").is_err());

        // Explicit allow overrides ask
        assert_eq!(policy.evaluate_command("git push origin dev"), PermissionDecision::Allow);
        assert!(policy.enforce_command("git push origin dev").is_ok());

        // Ask fails closed when MEG_APPROVE_ASK is not set
        assert!(matches!(
            policy.evaluate_command("git push origin main"),
            PermissionDecision::Ask { .. }
        ));
        let ask_err = policy.enforce_command("git push origin main").unwrap_err().to_string();
        assert!(ask_err.contains("PERMISSION_ASK_REQUIRED"), "{ask_err}");

        let _ = std::fs::remove_dir_all(&ws);
    }

    #[test]
    fn unix_sandbox_argv_applies_network_denial_flags() {
        let ws = Path::new("/work/repo");
        let linux_no_net = unix_sandbox_argv("linux", ws, "pytest", false);
        assert_eq!(linux_no_net[0], "bwrap");
        assert!(linux_no_net.contains(&"--unshare-net".to_string()));

        let mac_no_net = unix_sandbox_argv("macos", ws, "pytest", false);
        assert_eq!(mac_no_net[0], "/usr/bin/sandbox-exec");
        assert!(mac_no_net[2].contains("(deny network*)"));
        assert!(mac_no_net[2].contains("(subpath \"/work/repo\")"));
    }

    #[cfg(windows)]
    #[test]
    fn windows_restricted_token_and_job_ui_limits_succeed() {
        probe_windows_restricted_token().expect("CreateRestrictedToken should succeed on Windows");
        unsafe {
            let job = windows_sys::Win32::System::JobObjects::CreateJobObjectW(
                std::ptr::null(),
                std::ptr::null(),
            );
            assert!(!job.is_null(), "CreateJobObjectW failed");
            let res = apply_windows_job_ui_restrictions(job);
            windows_sys::Win32::Foundation::CloseHandle(job);
            res.expect("JobObjectBasicUIRestrictions should succeed on Windows");
        }
    }

    #[test]
    fn phase6c_sandbox_hardening_invariants() {
        assert!(is_secret_env_name("MEG_RECEIPT_KEY"));
        assert!(is_secret_env_name("MEG_TUI_API_KEY"));

        let linux = unix_sandbox_argv("linux", Path::new("/work/repo"), "pytest", false);
        assert!(linux.contains(&"--unshare-pid".to_string()));
        assert!(linux.contains(&"--new-session".to_string()));
        assert!(linux.contains(&"--die-with-parent".to_string()));
        assert!(linux.contains(&"--unshare-net".to_string()));

        assert!(windows_job_limits(SandboxMode::Off).is_none());
        let auto = windows_job_limits(SandboxMode::Auto).unwrap();
        let strict = windows_job_limits(SandboxMode::Strict).unwrap();
        assert!(strict.max_active_processes < auto.max_active_processes);
        assert!(strict.max_job_memory_bytes <= auto.max_job_memory_bytes);
        assert!(strict.apply_ui_limits);
    }

    #[cfg(windows)]
    #[test]
    fn windows_appcontainer_enforces_workspace_fs_boundary() {
        let base = std::env::temp_dir().join(format!(
            "meg-ac-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let ws = base.join("ws");
        let outside = base.join("outside");
        std::fs::create_dir_all(&ws).unwrap();
        std::fs::create_dir_all(&outside).unwrap();

        let mut policy = PermissionPolicy::default();
        policy.sandbox = SandboxMode::Strict;
        policy.allow_network = false;
        let cancel = std::sync::atomic::AtomicBool::new(false);

        unsafe {
            let job = windows_sys::Win32::System::JobObjects::CreateJobObjectW(
                std::ptr::null(),
                std::ptr::null(),
            );
            assert!(!job.is_null(), "CreateJobObjectW failed");
            apply_windows_job_ui_restrictions(job).unwrap();

            // 1. Inside-workspace write MUST succeed inside AppContainer
            let inside_res = win_appcontainer::run(
                &ws,
                "echo ok > inside.txt",
                false,
                &policy,
                15,
                &cancel,
                job,
            );
            // 2. Outside-workspace write MUST be blocked by Windows kernel ACLs
            let escaped = outside.join("escaped.txt");
            let outside_cmd = format!("echo bad > \"{}\"", escaped.display());
            let outside_res = win_appcontainer::run(
                &ws,
                &outside_cmd,
                false,
                &policy,
                15,
                &cancel,
                job,
            );

            windows_sys::Win32::Foundation::CloseHandle(job);

            let inside_out = inside_res.expect("AppContainer spawn inside workspace must succeed");
            assert_eq!(
                inside_out.exit_code, 0,
                "in-workspace write failed: stdout={} stderr={}",
                inside_out.stdout, inside_out.stderr
            );
            assert!(
                ws.join("inside.txt").is_file(),
                "expected inside.txt to be created in workspace"
            );

            let outside_out = outside_res.expect("AppContainer outside-write probe must execute");
            assert_ne!(
                outside_out.exit_code, 0,
                "AppContainer MUST deny writing outside workspace! stdout={} stderr={}",
                outside_out.stdout, outside_out.stderr
            );
            assert!(
                !escaped.exists(),
                "AppContainer leaked write outside workspace: {}",
                escaped.display()
            );
        }

        delete_windows_appcontainer_profile(&ws);
        let _ = std::fs::remove_dir_all(&base);
    }
}
