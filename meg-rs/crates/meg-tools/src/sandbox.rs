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
fn build_wide_env_block(policy: &PermissionPolicy) -> Vec<u16> {
    use std::collections::BTreeMap;
    let mut env_map: BTreeMap<String, String> = BTreeMap::new();
    for (k, v) in std::env::vars() {
        if is_secret_env_name(&k) && !policy.env_allow.contains(&k) {
            continue;
        }
        env_map.insert(k, v);
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
    use std::io::Read;
    use std::os::windows::io::FromRawHandle;
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
    let mut env_block = build_wide_env_block(policy);

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

        let mut out_file = std::fs::File::from_raw_handle(out_rd as _);
        let mut err_file = std::fs::File::from_raw_handle(err_rd as _);
        let out_thread = std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = out_file.by_ref().take(64 * 1024).read_to_end(&mut buf);
            String::from_utf8_lossy(&buf).into_owned()
        });
        let err_thread = std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = err_file.by_ref().take(64 * 1024).read_to_end(&mut buf);
            String::from_utf8_lossy(&buf).into_owned()
        });

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

        let stdout = out_thread.join().unwrap_or_default();
        let stderr = err_thread.join().unwrap_or_default();
        Ok(RestrictedWindowsOutput {
            exit_code: exit_code as i32,
            stdout,
            stderr,
            interrupted,
        })
    }
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
}
