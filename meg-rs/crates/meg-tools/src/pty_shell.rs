//! Persistent stateful shell sessions (`shell_session`):
//! Supports both deterministic pipe-backed persistent shells (default) and
//! real OS pseudo-terminal (`pty: true`) sessions backed by Windows ConPTY
//! (`CreatePseudoConsole`) or Unix POSIX PTY (`posix_openpt` / `TIOCSCTTY`),
//! preserving `cwd`, environment, and real `isatty()` behavior while enforcing
//! `PermissionPolicy`, compound-bash guards, secret scrubbing, and timeouts.

use anyhow::{bail, Result};
use serde_json::Value;
use std::collections::{HashMap, VecDeque};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const MAX_SESSION_OUTPUT: usize = 8_000;
const MAX_PTY_BUFFER_BYTES: usize = 64 * 1024;
const HEAD: usize = 3_500;
const TAIL: usize = 3_500;

fn cap_and_scrub(mut body: String) -> String {
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
    crate::security_guards::scrub_sal_observation(&body)
}

/// Strip ANSI/VT CSI and OSC escape sequences for clean sentinel & text extraction.
fn strip_ansi(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == 0x1b {
            if i + 1 < bytes.len() && bytes[i + 1] == b'[' {
                i += 2;
                while i < bytes.len() && !(0x40..=0x7e).contains(&bytes[i]) {
                    i += 1;
                }
                if i < bytes.len() {
                    i += 1;
                }
                continue;
            } else if i + 1 < bytes.len() && bytes[i + 1] == b']' {
                i += 2;
                while i < bytes.len() {
                    if bytes[i] == 0x07 {
                        i += 1;
                        break;
                    }
                    if bytes[i] == 0x1b && i + 1 < bytes.len() && bytes[i + 1] == b'\\' {
                        i += 2;
                        break;
                    }
                    i += 1;
                }
                continue;
            } else if i + 1 < bytes.len() {
                i += 2;
                continue;
            }
        }
        if bytes[i] != b'\r' {
            out.push(bytes[i]);
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn validate_interactive_input(workspace: &Path, input: &str) -> Result<()> {
    let policy = crate::sandbox::PermissionPolicy::load(workspace);
    for raw_line in input.lines() {
        let trimmed = raw_line.trim();
        if trimmed.is_empty() || trimmed.chars().all(char::is_control) {
            continue;
        }
        crate::security_guards::validate_compound_bash(trimmed)?;
        policy.enforce_command(trimmed)?;
        let allowed_repo = std::env::var("MEG_ALLOWED_GH_REPO").ok();
        if let Err(msg) =
            crate::deliverable_guard::check_gh_privacy_and_publish(trimmed, allowed_repo.as_deref())
        {
            bail!("{msg}");
        }
    }
    Ok(())
}

// =============================================================================
// 1. Deterministic Pipe-Backed Persistent Shell
// =============================================================================

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
                        return Ok((exit_code, cap_and_scrub(collected.join("\n"))));
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

// =============================================================================
// 2. True OS Pseudo-Terminal Backend (Windows ConPTY / Unix POSIX PTY)
// =============================================================================

#[cfg(windows)]
mod os_pty {
    use super::*;
    use std::ffi::c_void;
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::FromRawHandle;
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
    use windows_sys::Win32::System::Pipes::CreatePipe;
    use windows_sys::Win32::System::Threading::{
        CreateProcessW, TerminateProcess, WaitForSingleObject, CREATE_UNICODE_ENVIRONMENT,
        PROCESS_INFORMATION, STARTF_USESTDHANDLES, STARTUPINFOW,
    };

    const EXTENDED_STARTUPINFO_PRESENT: u32 = 0x0008_0000;
    const PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE: usize = 0x0002_0016;

    #[repr(C)]
    struct StartupInfoExW {
        startup_info: STARTUPINFOW,
        lp_attribute_list: *mut c_void,
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn LoadLibraryW(lp_lib_file_name: *const u16) -> *mut c_void;
        fn GetProcAddress(h_module: *mut c_void, lp_proc_name: *const u8) -> *const c_void;
        fn FreeLibrary(h_lib_module: *mut c_void) -> i32;
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

    type CreatePseudoConsoleFn =
        unsafe extern "system" fn(u32, HANDLE, HANDLE, u32, *mut *mut c_void) -> i32;
    type ResizePseudoConsoleFn = unsafe extern "system" fn(*mut c_void, u32) -> i32;
    type ClosePseudoConsoleFn = unsafe extern "system" fn(*mut c_void);

    fn pack_coord(cols: u16, rows: u16) -> u32 {
        (cols as u32) | ((rows as u32) << 16)
    }

    fn to_wide_null(s: &str) -> Vec<u16> {
        std::ffi::OsStr::new(s)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }

    fn build_pty_env_block(policy: &crate::sandbox::PermissionPolicy) -> Vec<u16> {
        use std::collections::BTreeMap;
        let mut env_map: BTreeMap<String, String> = BTreeMap::new();
        for (k, v) in std::env::vars() {
            if crate::sandbox::is_secret_env_name(&k) && !policy.env_allow.contains(&k) {
                continue;
            }
            env_map.insert(k, v);
        }
        env_map.insert("PYTHONDONTWRITEBYTECODE".into(), "1".into());
        env_map.insert("PYTHONUNBUFFERED".into(), "1".into());
        env_map.insert("TERM".into(), "xterm-256color".into());
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

    pub(super) struct NativePty {
        kernel32: *mut c_void,
        resize_fn: ResizePseudoConsoleFn,
        close_fn: ClosePseudoConsoleFn,
        hpc: *mut c_void,
        process: HANDLE,
        writer: std::fs::File,
        output_buf: Arc<Mutex<VecDeque<u8>>>,
        pub(super) cols: u16,
        pub(super) rows: u16,
    }

    // SAFETY: NativePty handles are owned exclusively behind ShellSessionManager's Mutex.
    unsafe impl Send for NativePty {}

    impl NativePty {
        pub(super) fn spawn(
            workspace: &Path,
            policy: &crate::sandbox::PermissionPolicy,
            initial_cmd: Option<&str>,
            cols: u16,
            rows: u16,
        ) -> Result<Self> {
            unsafe {
                let k32_name = to_wide_null("kernel32.dll");
                let kernel32 = LoadLibraryW(k32_name.as_ptr());
                if kernel32.is_null() {
                    bail!("LoadLibraryW(kernel32.dll) failed");
                }
                let p_create = GetProcAddress(kernel32, c"CreatePseudoConsole".as_ptr().cast());
                let p_resize = GetProcAddress(kernel32, c"ResizePseudoConsole".as_ptr().cast());
                let p_close = GetProcAddress(kernel32, c"ClosePseudoConsole".as_ptr().cast());
                if p_create.is_null() || p_resize.is_null() || p_close.is_null() {
                    FreeLibrary(kernel32);
                    bail!("ConPTY APIs unavailable on this Windows version");
                }
                let create_fn: CreatePseudoConsoleFn = std::mem::transmute(p_create);
                let resize_fn: ResizePseudoConsoleFn = std::mem::transmute(p_resize);
                let close_fn: ClosePseudoConsoleFn = std::mem::transmute(p_close);

                let sa = SECURITY_ATTRIBUTES {
                    nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
                    lpSecurityDescriptor: std::ptr::null_mut(),
                    bInheritHandle: 0,
                };
                let mut in_rd: HANDLE = std::ptr::null_mut();
                let mut in_wr: HANDLE = std::ptr::null_mut();
                let mut out_rd: HANDLE = std::ptr::null_mut();
                let mut out_wr: HANDLE = std::ptr::null_mut();
                if CreatePipe(&mut in_rd, &mut in_wr, &sa, 0) == 0 {
                    FreeLibrary(kernel32);
                    bail!("CreatePipe(conpty in) failed");
                }
                if CreatePipe(&mut out_rd, &mut out_wr, &sa, 0) == 0 {
                    CloseHandle(in_rd);
                    CloseHandle(in_wr);
                    FreeLibrary(kernel32);
                    bail!("CreatePipe(conpty out) failed");
                }

                let mut hpc: *mut c_void = std::ptr::null_mut();
                let hr = create_fn(pack_coord(cols, rows), in_rd, out_wr, 0, &mut hpc);
                CloseHandle(in_rd);
                CloseHandle(out_wr);
                if hr < 0 || hpc.is_null() {
                    CloseHandle(in_wr);
                    CloseHandle(out_rd);
                    FreeLibrary(kernel32);
                    bail!("CreatePseudoConsole failed: HRESULT 0x{:08x}", hr as u32);
                }

                let mut attr_size: usize = 0;
                InitializeProcThreadAttributeList(std::ptr::null_mut(), 1, 0, &mut attr_size);
                let mut attr_buf: Vec<u64> = vec![0u64; attr_size.div_ceil(8)];
                let attr_ptr = attr_buf.as_mut_ptr() as *mut c_void;
                if InitializeProcThreadAttributeList(attr_ptr, 1, 0, &mut attr_size) == 0 {
                    close_fn(hpc);
                    CloseHandle(in_wr);
                    CloseHandle(out_rd);
                    FreeLibrary(kernel32);
                    bail!("InitializeProcThreadAttributeList(ConPTY) failed");
                }
                if UpdateProcThreadAttribute(
                    attr_ptr,
                    0,
                    PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE,
                    hpc,
                    std::mem::size_of::<*mut c_void>(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                ) == 0
                {
                    DeleteProcThreadAttributeList(attr_ptr);
                    close_fn(hpc);
                    CloseHandle(in_wr);
                    CloseHandle(out_rd);
                    FreeLibrary(kernel32);
                    bail!("UpdateProcThreadAttribute(PSEUDOCONSOLE) failed");
                }

                let cmdline = match initial_cmd.filter(|s| !s.trim().is_empty()) {
                    Some(c) => format!("cmd.exe /S /K \"{c}\""),
                    None => "cmd.exe /Q /K".to_string(),
                };
                let mut cmd_w = to_wide_null(&cmdline);
                let cwd_w = to_wide_null(&workspace.display().to_string());
                let mut env_block = build_pty_env_block(policy);

                let mut siex: StartupInfoExW = std::mem::zeroed();
                siex.startup_info.cb = std::mem::size_of::<StartupInfoExW>() as u32;
                siex.startup_info.dwFlags = STARTF_USESTDHANDLES;
                siex.startup_info.hStdInput = std::ptr::null_mut();
                siex.startup_info.hStdOutput = std::ptr::null_mut();
                siex.startup_info.hStdError = std::ptr::null_mut();
                siex.lp_attribute_list = attr_ptr;

                let mut pi: PROCESS_INFORMATION = std::mem::zeroed();
                let created = CreateProcessW(
                    std::ptr::null(),
                    cmd_w.as_mut_ptr(),
                    std::ptr::null(),
                    std::ptr::null(),
                    0,
                    EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT,
                    env_block.as_mut_ptr() as *mut c_void,
                    cwd_w.as_ptr(),
                    &siex.startup_info,
                    &mut pi,
                );
                DeleteProcThreadAttributeList(attr_ptr);
                if created == 0 {
                    let err = std::io::Error::last_os_error();
                    close_fn(hpc);
                    CloseHandle(in_wr);
                    CloseHandle(out_rd);
                    FreeLibrary(kernel32);
                    bail!("CreateProcessW(ConPTY) failed: {err}");
                }
                CloseHandle(pi.hThread);

                let output_buf = Arc::new(Mutex::new(VecDeque::new()));
                let buf_clone = output_buf.clone();
                let mut reader_file = std::fs::File::from_raw_handle(out_rd as _);
                std::thread::spawn(move || {
                    let mut tmp = [0u8; 4096];
                    while let Ok(n) = reader_file.read(&mut tmp) {
                        if n == 0 {
                            break;
                        }
                        let mut q = buf_clone.lock().unwrap_or_else(|e| e.into_inner());
                        for &b in &tmp[..n] {
                            if q.len() >= MAX_PTY_BUFFER_BYTES {
                                q.pop_front();
                            }
                            q.push_back(b);
                        }
                    }
                });

                let mut writer = std::fs::File::from_raw_handle(in_wr as _);
                if initial_cmd.is_none() {
                    let _ = writer.write_all(b"@echo off\r\n");
                    let _ = writer.flush();
                }

                Ok(Self {
                    kernel32,
                    resize_fn,
                    close_fn,
                    hpc,
                    process: pi.hProcess,
                    writer,
                    output_buf,
                    cols,
                    rows,
                })
            }
        }

        pub(super) fn is_alive(&mut self) -> bool {
            unsafe { WaitForSingleObject(self.process, 0) != 0 }
        }

        pub(super) fn write_bytes(&mut self, data: &[u8]) -> Result<()> {
            self.writer.write_all(data)?;
            self.writer.flush()?;
            Ok(())
        }

        pub(super) fn drain_output(&mut self) -> String {
            let mut q = self.output_buf.lock().unwrap_or_else(|e| e.into_inner());
            let bytes: Vec<u8> = q.drain(..).collect();
            String::from_utf8_lossy(&bytes).into_owned()
        }

        pub(super) fn resize(&mut self, cols: u16, rows: u16) -> Result<()> {
            let hr = unsafe { (self.resize_fn)(self.hpc, pack_coord(cols, rows)) };
            if hr < 0 {
                bail!("ResizePseudoConsole failed: HRESULT 0x{:08x}", hr as u32);
            }
            self.cols = cols;
            self.rows = rows;
            Ok(())
        }
    }

    impl Drop for NativePty {
        fn drop(&mut self) {
            unsafe {
                if !self.process.is_null() {
                    let _ = TerminateProcess(self.process, 1);
                    let _ = WaitForSingleObject(self.process, 500);
                    CloseHandle(self.process);
                    self.process = std::ptr::null_mut();
                }
                if !self.hpc.is_null() {
                    (self.close_fn)(self.hpc);
                    self.hpc = std::ptr::null_mut();
                }
                if !self.kernel32.is_null() {
                    FreeLibrary(self.kernel32);
                    self.kernel32 = std::ptr::null_mut();
                }
            }
        }
    }
}

#[cfg(unix)]
mod os_pty {
    use super::*;
    use std::ffi::{c_char, c_int, c_ulong, CStr, CString};
    use std::os::unix::io::{AsRawFd, FromRawFd};
    use std::os::unix::process::CommandExt;
    use std::sync::OnceLock;

    const O_RDWR: c_int = 2;
    #[cfg(target_os = "macos")]
    const O_NOCTTY: c_int = 0x20000;
    #[cfg(not(target_os = "macos"))]
    const O_NOCTTY: c_int = 0x100;

    #[cfg(target_os = "macos")]
    const TIOCSCTTY: c_ulong = 0x2000_7461;
    #[cfg(not(target_os = "macos"))]
    const TIOCSCTTY: c_ulong = 0x540E;

    #[cfg(target_os = "macos")]
    const TIOCSWINSZ: c_ulong = 0x8008_7467;
    #[cfg(not(target_os = "macos"))]
    const TIOCSWINSZ: c_ulong = 0x5414;

    #[repr(C)]
    struct Winsize {
        ws_row: u16,
        ws_col: u16,
        ws_xpixel: u16,
        ws_ypixel: u16,
    }

    extern "C" {
        fn posix_openpt(flags: c_int) -> c_int;
        fn grantpt(fd: c_int) -> c_int;
        fn unlockpt(fd: c_int) -> c_int;
        fn ptsname(fd: c_int) -> *const c_char;
        fn open(path: *const c_char, oflag: c_int, ...) -> c_int;
        fn close(fd: c_int) -> c_int;
        fn dup2(oldfd: c_int, newfd: c_int) -> c_int;
        fn setsid() -> c_int;
        fn ioctl(fd: c_int, request: c_ulong, ...) -> c_int;
    }

    fn ptsname_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    pub(super) struct NativePty {
        child: Child,
        writer: std::fs::File,
        output_buf: Arc<Mutex<VecDeque<u8>>>,
        pub(super) cols: u16,
        pub(super) rows: u16,
    }

    impl NativePty {
        pub(super) fn spawn(
            workspace: &Path,
            policy: &crate::sandbox::PermissionPolicy,
            initial_cmd: Option<&str>,
            cols: u16,
            rows: u16,
        ) -> Result<Self> {
            let (master_fd, slave_path) = {
                let _guard = ptsname_lock().lock().unwrap_or_else(|e| e.into_inner());
                unsafe {
                    let fd = posix_openpt(O_RDWR | O_NOCTTY);
                    if fd < 0 {
                        bail!("posix_openpt failed: {}", std::io::Error::last_os_error());
                    }
                    if grantpt(fd) != 0 || unlockpt(fd) != 0 {
                        let err = std::io::Error::last_os_error();
                        close(fd);
                        bail!("grantpt/unlockpt failed: {err}");
                    }
                    let name_ptr = ptsname(fd);
                    if name_ptr.is_null() {
                        let err = std::io::Error::last_os_error();
                        close(fd);
                        bail!("ptsname failed: {err}");
                    }
                    let slave_cstr = CStr::from_ptr(name_ptr).to_owned();
                    let ws = Winsize {
                        ws_row: rows,
                        ws_col: cols,
                        ws_xpixel: 0,
                        ws_ypixel: 0,
                    };
                    let _ = ioctl(fd, TIOCSWINSZ, &ws as *const Winsize);
                    (fd, slave_cstr)
                }
            };

            let master_file = unsafe { std::fs::File::from_raw_fd(master_fd) };
            let mut reader_file = master_file.try_clone()?;

            let mut cmd = Command::new("sh");
            if let Some(c) = initial_cmd.filter(|s| !s.trim().is_empty()) {
                cmd.args(["-c", c]);
            }
            cmd.current_dir(workspace);
            cmd.env("PYTHONDONTWRITEBYTECODE", "1");
            cmd.env("PYTHONUNBUFFERED", "1");
            cmd.env("TERM", "xterm-256color");
            cmd.env("PS1", "");
            crate::sandbox::scrub_command_env(&mut cmd, policy);
            crate::sandbox::apply_network_policy_env(&mut cmd, policy);

            let slave_for_child: CString = slave_path;
            let master_fd_num = master_file.as_raw_fd();
            unsafe {
                cmd.pre_exec(move || {
                    close(master_fd_num);
                    if setsid() < 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                    let sfd = open(slave_for_child.as_ptr(), O_RDWR);
                    if sfd < 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                    let _ = ioctl(sfd, TIOCSCTTY, 0 as c_int);
                    if dup2(sfd, 0) < 0 || dup2(sfd, 1) < 0 || dup2(sfd, 2) < 0 {
                        let err = std::io::Error::last_os_error();
                        close(sfd);
                        return Err(err);
                    }
                    if sfd > 2 {
                        close(sfd);
                    }
                    Ok(())
                });
            }

            let child = cmd
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()?;

            let output_buf = Arc::new(Mutex::new(VecDeque::new()));
            let buf_clone = output_buf.clone();
            std::thread::spawn(move || {
                let mut tmp = [0u8; 4096];
                while let Ok(n) = reader_file.read(&mut tmp) {
                    if n == 0 {
                        break;
                    }
                    let mut q = buf_clone.lock().unwrap_or_else(|e| e.into_inner());
                    for &b in &tmp[..n] {
                        if q.len() >= MAX_PTY_BUFFER_BYTES {
                            q.pop_front();
                        }
                        q.push_back(b);
                    }
                }
            });

            Ok(Self {
                child,
                writer: master_file,
                output_buf,
                cols,
                rows,
            })
        }

        pub(super) fn is_alive(&mut self) -> bool {
            matches!(self.child.try_wait(), Ok(None))
        }

        pub(super) fn write_bytes(&mut self, data: &[u8]) -> Result<()> {
            self.writer.write_all(data)?;
            self.writer.flush()?;
            Ok(())
        }

        pub(super) fn drain_output(&mut self) -> String {
            let mut q = self.output_buf.lock().unwrap_or_else(|e| e.into_inner());
            let bytes: Vec<u8> = q.drain(..).collect();
            String::from_utf8_lossy(&bytes).into_owned()
        }

        pub(super) fn resize(&mut self, cols: u16, rows: u16) -> Result<()> {
            let ws = Winsize {
                ws_row: rows,
                ws_col: cols,
                ws_xpixel: 0,
                ws_ypixel: 0,
            };
            let rc = unsafe { ioctl(self.writer.as_raw_fd(), TIOCSWINSZ, &ws as *const Winsize) };
            if rc != 0 {
                bail!("ioctl(TIOCSWINSZ) failed: {}", std::io::Error::last_os_error());
            }
            self.cols = cols;
            self.rows = rows;
            Ok(())
        }
    }

    impl Drop for NativePty {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

impl os_pty::NativePty {
    fn exec_command(
        &mut self,
        cmd_line: &str,
        timeout_secs: u64,
        cancel: &AtomicBool,
    ) -> Result<(i32, String)> {
        let _ = self.drain_output();
        let tag = format!(
            "{:x}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        );
        let sentinel = format!("__MEG_PTY_DONE_{tag}__");
        // Split sentinel in command text so terminal local-echo never matches the output sentinel.
        let payload = if cfg!(windows) {
            format!("{cmd_line}\r\necho __MEG_PTY_^DONE_{tag}__ %ERRORLEVEL%\r\n")
        } else {
            format!("{cmd_line}\nprintf '\\n__MEG_PTY_''DONE_{tag}__ %d\\n' $?\n")
        };
        self.write_bytes(payload.as_bytes())?;

        let deadline = Instant::now() + Duration::from_secs(timeout_secs);
        let mut raw = String::new();
        loop {
            if cancel.load(Ordering::SeqCst) {
                bail!("Run cancelled");
            }
            if Instant::now() >= deadline {
                bail!("pty shell_session command timed out after {timeout_secs}s");
            }
            let chunk = self.drain_output();
            if !chunk.is_empty() {
                raw.push_str(&chunk);
                let clean = strip_ansi(&raw);
                if let Some(pos) = clean.find(&sentinel) {
                    let after = &clean[pos + sentinel.len()..];
                    if after.contains('\n') {
                        let code_line = after.lines().next().unwrap_or("").trim();
                        let exit_code = code_line
                            .split_whitespace()
                            .next()
                            .and_then(|s| s.parse::<i32>().ok())
                            .unwrap_or(0);
                        let body = clean[..pos].trim().to_string();
                        return Ok((exit_code, cap_and_scrub(body)));
                    }
                }
            }
            if !self.is_alive() {
                let tail = strip_ansi(&raw);
                bail!("PTY shell exited unexpectedly:\n{}", cap_and_scrub(tail));
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    fn wait_and_read(&mut self, wait_ms: u64) -> String {
        if wait_ms > 0 {
            let deadline = Instant::now() + Duration::from_millis(wait_ms.min(5_000));
            let mut acc = String::new();
            let mut last_data = Instant::now();
            while Instant::now() < deadline {
                let chunk = self.drain_output();
                if !chunk.is_empty() {
                    acc.push_str(&chunk);
                    last_data = Instant::now();
                } else if !acc.is_empty() && last_data.elapsed() >= Duration::from_millis(60) {
                    break;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            return cap_and_scrub(strip_ansi(&acc));
        }
        cap_and_scrub(strip_ansi(&self.drain_output()))
    }
}

enum SessionBackend {
    Pipe(PersistentShell),
    Pty(os_pty::NativePty),
}

impl SessionBackend {
    fn is_alive(&mut self) -> bool {
        match self {
            Self::Pipe(s) => s.is_alive(),
            Self::Pty(p) => p.is_alive(),
        }
    }

    fn status_label(&mut self) -> String {
        if !self.is_alive() {
            return "closed".into();
        }
        match self {
            Self::Pipe(_) => "running (pipe)".into(),
            Self::Pty(p) => format!("running (pty {}x{})", p.cols, p.rows),
        }
    }
}

#[derive(Clone)]
pub struct ShellSessionManager {
    workspace: PathBuf,
    sessions: Arc<Mutex<HashMap<String, SessionBackend>>>,
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
        let want_pty = args.get("pty").and_then(Value::as_bool).unwrap_or(false);
        let cols = args
            .get("cols")
            .and_then(Value::as_u64)
            .unwrap_or(120)
            .clamp(20, 400) as u16;
        let rows = args
            .get("rows")
            .and_then(Value::as_u64)
            .unwrap_or(30)
            .clamp(5, 200) as u16;
        let wait_ms = args
            .get("wait_ms")
            .and_then(Value::as_u64)
            .unwrap_or(250)
            .min(5_000);

        match action {
            "status" => {
                let mut map = self.sessions.lock().unwrap();
                let label = map
                    .get_mut(&session_id)
                    .map(|s| s.status_label())
                    .unwrap_or_else(|| "closed".into());
                Ok(format!("shell_session '{session_id}': {label}"))
            }
            "close" | "reset" => {
                let mut map = self.sessions.lock().unwrap();
                let existed = map.remove(&session_id).is_some();
                Ok(format!(
                    "shell_session '{session_id}' {}",
                    if existed { "closed" } else { "was not running" }
                ))
            }
            "open" => {
                let policy = crate::sandbox::PermissionPolicy::load(&self.workspace);
                if policy.sandbox == crate::sandbox::SandboxMode::Strict {
                    bail!(
                        "SANDBOX_STRICT_MODE: persistent shell_session is disabled in MEG_SANDBOX=strict; use isolated one-shot 'bash'"
                    );
                }
                let initial_cmd = args
                    .get("command")
                    .or_else(|| args.get("cmd"))
                    .and_then(Value::as_str)
                    .filter(|s| !s.trim().is_empty());
                if let Some(cmd_line) = initial_cmd {
                    crate::security_guards::validate_compound_bash(cmd_line)?;
                    policy.enforce_command(cmd_line)?;
                    let allowed_repo = std::env::var("MEG_ALLOWED_GH_REPO").ok();
                    if let Err(msg) = crate::deliverable_guard::check_gh_privacy_and_publish(
                        cmd_line,
                        allowed_repo.as_deref(),
                    ) {
                        bail!("{msg}");
                    }
                }
                let mut map = self.sessions.lock().unwrap();
                let use_pty = args.get("pty").and_then(Value::as_bool).unwrap_or(true);
                if use_pty {
                    let mut pty =
                        os_pty::NativePty::spawn(&self.workspace, &policy, initial_cmd, cols, rows)?;
                    let initial = pty.wait_and_read(wait_ms);
                    map.insert(session_id.clone(), SessionBackend::Pty(pty));
                    if initial.trim().is_empty() {
                        Ok(format!(
                            "shell_session '{session_id}' opened (pty {cols}x{rows})"
                        ))
                    } else {
                        Ok(format!(
                            "shell_session '{session_id}' opened (pty {cols}x{rows})\n{initial}"
                        ))
                    }
                } else {
                    let shell = PersistentShell::spawn(&self.workspace, &policy)?;
                    map.insert(session_id.clone(), SessionBackend::Pipe(shell));
                    Ok(format!("shell_session '{session_id}' opened (pipe)"))
                }
            }
            "write" => {
                let input = args
                    .get("input")
                    .and_then(Value::as_str)
                    .ok_or_else(|| anyhow::anyhow!("shell_session write requires 'input'"))?;
                validate_interactive_input(&self.workspace, input)?;
                let mut map = self.sessions.lock().unwrap();
                let Some(backend) = map.get_mut(&session_id) else {
                    bail!("shell_session '{session_id}' is not open; call action='open' first");
                };
                match backend {
                    SessionBackend::Pty(pty) => {
                        if !pty.is_alive() {
                            map.remove(&session_id);
                            bail!("PTY shell_session '{session_id}' has exited");
                        }
                        pty.write_bytes(input.as_bytes())?;
                        Ok(pty.wait_and_read(wait_ms))
                    }
                    SessionBackend::Pipe(_) => {
                        bail!("shell_session '{session_id}' is pipe-backed; open with pty: true for raw interactive write/read")
                    }
                }
            }
            "read" => {
                let mut map = self.sessions.lock().unwrap();
                let Some(backend) = map.get_mut(&session_id) else {
                    bail!("shell_session '{session_id}' is not open");
                };
                match backend {
                    SessionBackend::Pty(pty) => Ok(pty.wait_and_read(wait_ms)),
                    SessionBackend::Pipe(_) => {
                        bail!("shell_session '{session_id}' is pipe-backed; open with pty: true for raw interactive write/read")
                    }
                }
            }
            "resize" => {
                let mut map = self.sessions.lock().unwrap();
                let Some(backend) = map.get_mut(&session_id) else {
                    bail!("shell_session '{session_id}' is not open");
                };
                match backend {
                    SessionBackend::Pty(pty) => {
                        pty.resize(cols, rows)?;
                        Ok(format!(
                            "shell_session '{session_id}' resized to {cols}x{rows}"
                        ))
                    }
                    SessionBackend::Pipe(_) => {
                        bail!("shell_session '{session_id}' is pipe-backed; resize requires a PTY session")
                    }
                }
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
                    let backend = if want_pty {
                        let mut pty =
                            os_pty::NativePty::spawn(&self.workspace, &policy, None, cols, rows)?;
                        let _ = pty.wait_and_read(150);
                        SessionBackend::Pty(pty)
                    } else {
                        SessionBackend::Pipe(PersistentShell::spawn(&self.workspace, &policy)?)
                    };
                    map.insert(session_id.clone(), backend);
                }
                let res = match map.get_mut(&session_id).unwrap() {
                    SessionBackend::Pipe(shell) => {
                        shell.exec_command(cmd_line, timeout_secs, cancel)
                    }
                    SessionBackend::Pty(pty) => pty.exec_command(cmd_line, timeout_secs, cancel),
                };
                match res {
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
            other => bail!(
                "unknown shell_session action '{other}'; expected exec, open, write, read, resize, status, reset, or close"
            ),
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

        let set_cmd = if cfg!(windows) {
            "cd subdir && set MEG_SESSION_VAR=hello_stateful"
        } else {
            "cd subdir && export MEG_SESSION_VAR=hello_stateful"
        };
        let r1 = mgr
            .execute(&json!({"action": "exec", "command": set_cmd}), &cancel)
            .unwrap();
        assert!(r1.starts_with("[exit 0]"), "r1={r1}");

        let check_cmd = if cfg!(windows) {
            "echo %CD% :: %MEG_SESSION_VAR%"
        } else {
            "printf '%s :: %s\\n' \"$PWD\" \"$MEG_SESSION_VAR\""
        };
        let r2 = mgr
            .execute(&json!({"action": "exec", "command": check_cmd}), &cancel)
            .unwrap();
        assert!(r2.contains("subdir"), "cwd must persist: {r2}");
        assert!(r2.contains("hello_stateful"), "env must persist: {r2}");

        let blocked = mgr.execute(&json!({"action": "exec", "command": "cat .env"}), &cancel);
        assert!(
            blocked.is_err(),
            "compound bash guard must block .env in shell_session"
        );

        let closed = mgr.execute(&json!({"action": "close"}), &cancel).unwrap();
        assert!(closed.contains("closed"));
        let _ = std::fs::remove_dir_all(&ws);
    }

    #[test]
    fn true_pty_session_reports_real_tty_supports_interactive_write_read_and_resize() {
        let ws = std::env::temp_dir().join(format!("meg-true-pty-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&ws);
        std::fs::create_dir_all(&ws).unwrap();
        let mgr = ShellSessionManager::new(&ws);
        let cancel = AtomicBool::new(false);

        let opened = mgr
            .execute(
                &json!({
                    "action": "open",
                    "session_id": "tty1",
                    "pty": true,
                    "cols": 100,
                    "rows": 30,
                    "wait_ms": 200
                }),
                &cancel,
            )
            .unwrap();
        assert!(opened.contains("pty 100x30"), "opened={opened}");

        let status1 = mgr
            .execute(&json!({"action": "status", "session_id": "tty1"}), &cancel)
            .unwrap();
        assert!(status1.contains("running (pty 100x30)"), "status1={status1}");

        // Prove the process inside the PTY actually has a real TTY attached
        let tty_cmd = if cfg!(windows) {
            "python -c \"import sys; print('IS_TTY=' + str(sys.stdout.isatty()))\""
        } else {
            "[ -t 1 ] && echo IS_TTY=True || echo IS_TTY=False"
        };
        let tty_out = mgr
            .execute(
                &json!({"action": "exec", "session_id": "tty1", "command": tty_cmd}),
                &cancel,
            )
            .unwrap();
        assert!(
            tty_out.contains("IS_TTY=True"),
            "expected real OS PTY to report isatty()==True, got: {tty_out}"
        );

        // Resize live PTY
        let resized = mgr
            .execute(
                &json!({"action": "resize", "session_id": "tty1", "cols": 120, "rows": 40}),
                &cancel,
            )
            .unwrap();
        assert!(resized.contains("120x40"), "resized={resized}");
        let status2 = mgr
            .execute(&json!({"action": "status", "session_id": "tty1"}), &cancel)
            .unwrap();
        assert!(status2.contains("running (pty 120x40)"), "status2={status2}");

        // Interactive write + read
        let write_payload = if cfg!(windows) {
            "echo MEG_INTERACTIVE_OK\r\n"
        } else {
            "echo MEG_INTERACTIVE_OK\n"
        };
        let mut combined = mgr
            .execute(
                &json!({"action": "write", "session_id": "tty1", "input": write_payload, "wait_ms": 400}),
                &cancel,
            )
            .unwrap();
        if !combined.contains("MEG_INTERACTIVE_OK") {
            let extra = mgr
                .execute(
                    &json!({"action": "read", "session_id": "tty1", "wait_ms": 400}),
                    &cancel,
                )
                .unwrap();
            combined.push_str(&extra);
        }
        assert!(
            combined.contains("MEG_INTERACTIVE_OK"),
            "interactive PTY write/read failed: {combined}"
        );

        // Security guard must still block .env access via raw PTY write
        let blocked = mgr.execute(
            &json!({"action": "write", "session_id": "tty1", "input": "cat .env\n"}),
            &cancel,
        );
        assert!(blocked.is_err(), "interactive PTY write must enforce compound-bash guard");

        let closed = mgr
            .execute(&json!({"action": "close", "session_id": "tty1"}), &cancel)
            .unwrap();
        assert!(closed.contains("closed"));
        let _ = std::fs::remove_dir_all(&ws);
    }
}
