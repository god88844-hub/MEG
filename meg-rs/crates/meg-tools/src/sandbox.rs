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
            let mut argv = vec![
                "bwrap".into(),
                "--ro-bind".into(),
                "/".into(),
                "/".into(),
                "--bind".into(),
                ws.clone(),
                ws,
                "--dev".into(),
                "/dev".into(),
                "--proc".into(),
                "/proc".into(),
                "--tmpfs".into(),
                "/tmp".into(),
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
            let net_rule = if allow_network {
                "(allow network*)"
            } else {
                "(deny network*)"
            };
            let profile = format!(
                "(version 1)(deny default)(allow process*)(allow sysctl-read)(allow file-read*){net_rule}(allow file-write* (subpath \"{escaped_ws}\") (subpath \"/tmp\") (subpath \"/private/tmp\") (subpath \"/dev/null\"))"
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
        let has_bwrap = command_on_path("bwrap");
        if has_bwrap {
            let argv = unix_sandbox_argv("linux", workspace, cmd_line, policy.allow_network);
            let mut c = Command::new(&argv[0]);
            c.args(&argv[1..]).current_dir(workspace);
            return Ok(Some(c));
        }
        if policy.sandbox == SandboxMode::Strict {
            bail!("SANDBOX_UNAVAILABLE: MEG_SANDBOX=strict requires 'bwrap' (bubblewrap) on Linux");
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
