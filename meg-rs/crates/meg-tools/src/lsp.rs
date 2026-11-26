//! Native stdio Language Server Protocol (LSP) client for write-through diagnostics.
//!
//! Automatically attaches workspace language servers (`rust-analyzer`,
//! `typescript-language-server`, `pyright-langserver`, `gopls`, or overrides in
//! `.meg/lsp.json`) and returns compiler/type diagnostics right inside
//! `edit_file` / `write_file` tool responses. Fails open (returns `None`) if no
//! language server is installed or if `MEG_LSP=0` is set.

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

const DEFAULT_DIAG_TIMEOUT_MS: u64 = 1500;
const MAX_DIAG_TIMEOUT_MS: u64 = 5000;
const MAX_DIAGNOSTICS_SHOWN: usize = 15;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ServerSpec {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Diagnostic {
    pub line: u64,
    pub col: u64,
    pub severity: u64,
    pub message: String,
}

enum Incoming {
    Response { id: u64, result: Result<Value, String> },
    Diagnostics { uri: String, items: Vec<Diagnostic> },
}

struct LspSession {
    name: String,
    stdin: ChildStdin,
    _child: Child,
    rx: Receiver<Incoming>,
    next_id: u64,
    doc_versions: HashMap<String, i64>,
}

type SessionKey = (PathBuf, String, Vec<String>);

fn sessions() -> &'static Mutex<HashMap<SessionKey, Arc<Mutex<LspSession>>>> {
    static MAP: OnceLock<Mutex<HashMap<SessionKey, Arc<Mutex<LspSession>>>>> = OnceLock::new();
    MAP.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn path_to_uri(path: &Path) -> String {
    let abs = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let mut s = abs.to_string_lossy().replace('\\', "/");
    if let Some(stripped) = s.strip_prefix("//?/") {
        s = stripped.to_string();
    }
    let encoded = s.replace('%', "%25").replace(' ', "%20");
    if encoded.starts_with('/') {
        format!("file://{encoded}")
    } else {
        format!("file:///{encoded}")
    }
}

fn normalize_uri(uri: &str) -> String {
    if cfg!(windows) {
        uri.to_ascii_lowercase()
    } else {
        uri.to_string()
    }
}

fn language_id_for_ext(ext: &str) -> &'static str {
    match ext {
        "rs" => "rust",
        "ts" => "typescript",
        "tsx" => "typescriptreact",
        "js" | "mjs" | "cjs" => "javascript",
        "jsx" => "javascriptreact",
        "py" => "python",
        "go" => "go",
        "c" | "h" => "c",
        "cc" | "cpp" | "cxx" | "hpp" => "cpp",
        _ => "plaintext",
    }
}

fn load_workspace_override(workspace: &Path, ext: &str) -> Option<ServerSpec> {
    let cfg_path = workspace.join(".meg").join("lsp.json");
    let raw = std::fs::read_to_string(&cfg_path).ok()?;
    let val: Value = serde_json::from_str(&raw).ok()?;
    let entry = val.get(ext).or_else(|| val.get(format!(".{ext}")))?;
    let arr = entry.as_array()?;
    let cmd = arr.first()?.as_str()?.to_string();
    let args = arr
        .iter()
        .skip(1)
        .filter_map(|v| v.as_str().map(str::to_string))
        .collect();
    let name = Path::new(&cmd)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| cmd.clone());
    Some(ServerSpec {
        name,
        command: cmd,
        args,
    })
}

fn command_on_path(bin: &str) -> bool {
    let p = Path::new(bin);
    if p.components().count() > 1 {
        return p.is_file();
    }
    let Some(path_var) = std::env::var_os("PATH") else {
        return false;
    };
    let exts: Vec<String> = if cfg!(windows) {
        vec!["".into(), ".exe".into(), ".cmd".into(), ".bat".into()]
    } else {
        vec!["".into()]
    };
    for dir in std::env::split_paths(&path_var) {
        for ext in &exts {
            if dir.join(format!("{bin}{ext}")).is_file() {
                return true;
            }
        }
    }
    false
}

pub fn resolve_server(workspace: &Path, file_path: &Path) -> Option<ServerSpec> {
    let ext = file_path.extension()?.to_str()?.to_ascii_lowercase();
    if let Some(ov) = load_workspace_override(workspace, &ext) {
        return Some(ov);
    }
    let candidates: &[(&str, &str, &[&str])] = match ext.as_str() {
        "rs" => {
            if !workspace.join("Cargo.toml").is_file()
                && !file_path.ancestors().any(|a| a.join("Cargo.toml").is_file())
            {
                return None;
            }
            &[("rust-analyzer", "rust-analyzer", &[])]
        }
        "ts" | "tsx" | "js" | "jsx" => {
            if !workspace.join("tsconfig.json").is_file() && !workspace.join("package.json").is_file() {
                return None;
            }
            &[(
                "typescript-language-server",
                "typescript-language-server",
                &["--stdio"],
            )]
        }
        "py" => {
            if !workspace.join("pyproject.toml").is_file()
                && !workspace.join("pyrightconfig.json").is_file()
                && !workspace.join("requirements.txt").is_file()
                && !workspace.join("setup.py").is_file()
            {
                return None;
            }
            &[
                ("basedpyright", "basedpyright-langserver", &["--stdio"]),
                ("pyright", "pyright-langserver", &["--stdio"]),
            ]
        }
        "go" => {
            if !workspace.join("go.mod").is_file() {
                return None;
            }
            &[("gopls", "gopls", &[])]
        }
        _ => &[],
    };
    for &(name, bin, args) in candidates {
        if command_on_path(bin) {
            return Some(ServerSpec {
                name: name.into(),
                command: bin.into(),
                args: args.iter().map(|s| (*s).to_string()).collect(),
            });
        }
    }
    None
}

fn write_lsp_message(stdin: &mut ChildStdin, payload: &Value) -> Result<()> {
    let body = serde_json::to_vec(payload)?;
    write!(stdin, "Content-Length: {}\r\n\r\n", body.len())?;
    stdin.write_all(&body)?;
    stdin.flush()?;
    Ok(())
}

fn read_lsp_message<R: BufRead>(reader: &mut R) -> Result<Option<Value>> {
    let mut content_length: Option<usize> = None;
    loop {
        let mut line = String::new();
        let n = reader.read_line(&mut line)?;
        if n == 0 {
            return Ok(None);
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            break;
        }
        if let Some((k, v)) = trimmed.split_once(':') {
            if k.trim().eq_ignore_ascii_case("Content-Length") {
                content_length = v.trim().parse::<usize>().ok();
            }
        }
    }
    let Some(len) = content_length else {
        return Ok(None);
    };
    if len > 16 * 1024 * 1024 {
        bail!("LSP frame exceeds 16 MiB cap");
    }
    let mut buf = vec![0u8; len];
    reader.read_exact(&mut buf)?;
    let val: Value = serde_json::from_slice(&buf)?;
    Ok(Some(val))
}

fn parse_diagnostics(params: &Value) -> Option<(String, Vec<Diagnostic>)> {
    let uri = params.get("uri")?.as_str()?.to_string();
    let arr = params.get("diagnostics")?.as_array()?;
    let mut out = Vec::with_capacity(arr.len());
    for item in arr {
        let line = item["range"]["start"]["line"].as_u64().unwrap_or(0) + 1;
        let col = item["range"]["start"]["character"].as_u64().unwrap_or(0) + 1;
        let severity = item["severity"].as_u64().unwrap_or(1);
        let msg = item["message"].as_str().unwrap_or("").trim().to_string();
        if !msg.is_empty() {
            out.push(Diagnostic {
                line,
                col,
                severity,
                message: msg,
            });
        }
    }
    Some((uri, out))
}

impl LspSession {
    fn spawn(workspace: &Path, spec: &ServerSpec) -> Result<Self> {
        let mut cmd = Command::new(&spec.command);
        cmd.args(&spec.args)
            .current_dir(workspace)
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
            .with_context(|| format!("failed to spawn LSP server '{}'", spec.command))?;
        let stdin = child.stdin.take().context("LSP stdin unavailable")?;
        let stdout = child.stdout.take().context("LSP stdout unavailable")?;
        if let Some(mut stderr) = child.stderr.take() {
            std::thread::spawn(move || {
                let mut sink = [0u8; 1024];
                while let Ok(n) = stderr.read(&mut sink) {
                    if n == 0 {
                        break;
                    }
                }
            });
        }
        let (tx, rx): (Sender<Incoming>, Receiver<Incoming>) = mpsc::channel();
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            while let Ok(Some(msg)) = read_lsp_message(&mut reader) {
                if let Some(id) = msg.get("id").and_then(Value::as_u64) {
                    if msg.get("method").is_none() {
                        let res = if let Some(err) = msg.get("error") {
                            Err(err.to_string())
                        } else {
                            Ok(msg.get("result").cloned().unwrap_or(Value::Null))
                        };
                        let _ = tx.send(Incoming::Response { id, result: res });
                    }
                } else if msg.get("method").and_then(Value::as_str)
                    == Some("textDocument/publishDiagnostics")
                {
                    if let Some((uri, items)) = parse_diagnostics(&msg["params"]) {
                        let _ = tx.send(Incoming::Diagnostics { uri, items });
                    }
                }
            }
        });

        let mut sess = Self {
            name: spec.name.clone(),
            stdin,
            _child: child,
            rx,
            next_id: 1,
            doc_versions: HashMap::new(),
        };
        sess.initialize(workspace)?;
        Ok(sess)
    }

    fn initialize(&mut self, workspace: &Path) -> Result<()> {
        let id = self.next_id;
        self.next_id += 1;
        let root_uri = path_to_uri(workspace);
        write_lsp_message(
            &mut self.stdin,
            &json!({
                "jsonrpc": "2.0",
                "id": id,
                "method": "initialize",
                "params": {
                    "processId": std::process::id(),
                    "rootUri": root_uri,
                    "capabilities": {
                        "textDocument": {
                            "publishDiagnostics": {}
                        }
                    }
                }
            }),
        )?;
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            let rem = deadline.saturating_duration_since(Instant::now());
            if rem.is_zero() {
                bail!("LSP initialize timed out");
            }
            match self.rx.recv_timeout(rem) {
                Ok(Incoming::Response { id: rid, result }) if rid == id => {
                    result.map_err(|e| anyhow::anyhow!("LSP initialize error: {e}"))?;
                    break;
                }
                Ok(_) => continue,
                Err(_) => bail!("LSP server closed during initialize"),
            }
        }
        write_lsp_message(
            &mut self.stdin,
            &json!({
                "jsonrpc": "2.0",
                "method": "initialized",
                "params": {}
            }),
        )?;
        Ok(())
    }

    fn sync_and_collect(
        &mut self,
        file_path: &Path,
        content: &str,
        timeout: Duration,
    ) -> Result<Vec<Diagnostic>> {
        let uri = path_to_uri(file_path);
        let norm_target = normalize_uri(&uri);
        // Drain any stale queued notifications before pushing the latest edit.
        while self.rx.try_recv().is_ok() {}

        let ext = file_path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let lang_id = language_id_for_ext(&ext);

        if let Some(ver) = self.doc_versions.get_mut(&norm_target) {
            *ver += 1;
            let version = *ver;
            write_lsp_message(
                &mut self.stdin,
                &json!({
                    "jsonrpc": "2.0",
                    "method": "textDocument/didChange",
                    "params": {
                        "textDocument": { "uri": uri, "version": version },
                        "contentChanges": [{ "text": content }]
                    }
                }),
            )?;
        } else {
            self.doc_versions.insert(norm_target.clone(), 1);
            write_lsp_message(
                &mut self.stdin,
                &json!({
                    "jsonrpc": "2.0",
                    "method": "textDocument/didOpen",
                    "params": {
                        "textDocument": {
                            "uri": uri,
                            "languageId": lang_id,
                            "version": 1,
                            "text": content
                        }
                    }
                }),
            )?;
        }
        write_lsp_message(
            &mut self.stdin,
            &json!({
                "jsonrpc": "2.0",
                "method": "textDocument/didSave",
                "params": {
                    "textDocument": { "uri": uri },
                    "text": content
                }
            }),
        )?;

        let deadline = Instant::now() + timeout;
        let mut latest: Option<Vec<Diagnostic>> = None;
        loop {
            let rem = deadline.saturating_duration_since(Instant::now());
            if rem.is_zero() {
                break;
            }
            match self.rx.recv_timeout(rem) {
                Ok(Incoming::Diagnostics {
                    uri: diag_uri,
                    items,
                }) if normalize_uri(&diag_uri) == norm_target => {
                    latest = Some(items);
                    // Wait briefly (80ms) in case a follow-up batch arrives, then return.
                    let settle = Duration::from_millis(80);
                    let settle_deadline = Instant::now() + settle;
                    while Instant::now() < settle_deadline {
                        let left = settle_deadline.saturating_duration_since(Instant::now());
                        match self.rx.recv_timeout(left) {
                            Ok(Incoming::Diagnostics {
                                uri: u2,
                                items: it2,
                            }) if normalize_uri(&u2) == norm_target => {
                                latest = Some(it2);
                            }
                            Ok(_) => {}
                            Err(_) => break,
                        }
                    }
                    break;
                }
                Ok(_) => continue,
                Err(_) => break,
            }
        }
        Ok(latest.unwrap_or_default())
    }
}

pub fn format_diagnostics(server_name: &str, diags: &[Diagnostic]) -> Option<String> {
    let mut relevant: Vec<&Diagnostic> = diags.iter().filter(|d| d.severity <= 2).collect();
    if relevant.is_empty() {
        return None;
    }
    relevant.sort_by_key(|d| (d.severity, d.line, d.col));
    let errors = relevant.iter().filter(|d| d.severity == 1).count();
    let warnings = relevant.iter().filter(|d| d.severity == 2).count();
    let mut out = format!(
        "\n[LSP diagnostics ({server_name}): {errors} error(s), {warnings} warning(s)]"
    );
    for d in relevant.iter().take(MAX_DIAGNOSTICS_SHOWN) {
        let sev = if d.severity == 1 { "error" } else { "warning" };
        let msg: String = d.message.replace('\n', " ").chars().take(240).collect();
        out.push_str(&format!("\n- line {}:{} [{sev}]: {msg}", d.line, d.col));
    }
    if relevant.len() > MAX_DIAGNOSTICS_SHOWN {
        out.push_str(&format!(
            "\n- ...and {} more diagnostic(s)",
            relevant.len() - MAX_DIAGNOSTICS_SHOWN
        ));
    }
    Some(out)
}

/// Collect write-through LSP diagnostics for `file_path` after a write/edit.
/// Returns `None` if `MEG_LSP=0`, no language server is available, or no errors/warnings exist.
pub fn collect_write_through_diagnostics(
    workspace: &Path,
    file_path: &Path,
    content: &str,
) -> Option<String> {
    if std::env::var("MEG_LSP").as_deref() == Ok("0") {
        return None;
    }
    let spec = resolve_server(workspace, file_path)?;
    let canon_ws = workspace
        .canonicalize()
        .unwrap_or_else(|_| workspace.to_path_buf());
    let key: SessionKey = (canon_ws.clone(), spec.command.clone(), spec.args.clone());

    let sess_arc = {
        let mut map = sessions().lock().unwrap();
        if let Some(existing) = map.get(&key) {
            existing.clone()
        } else {
            let spawned = LspSession::spawn(&canon_ws, &spec).ok()?;
            let arc = Arc::new(Mutex::new(spawned));
            map.insert(key.clone(), arc.clone());
            arc
        }
    };

    let timeout_ms = std::env::var("MEG_LSP_TIMEOUT_MS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(DEFAULT_DIAG_TIMEOUT_MS)
        .clamp(100, MAX_DIAG_TIMEOUT_MS);

    let mut guard = sess_arc.lock().unwrap();
    match guard.sync_and_collect(file_path, content, Duration::from_millis(timeout_ms)) {
        Ok(diags) => format_diagnostics(&guard.name, &diags),
        Err(_) => {
            drop(guard);
            sessions().lock().unwrap().remove(&key);
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_stdio_mock_lsp_returns_and_clears_write_through_diagnostics() {
        let ws = std::env::temp_dir().join(format!("meg-lsp-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&ws);
        std::fs::create_dir_all(ws.join(".meg")).unwrap();

        let mock_py = ws.join("mock_lsp.py");
        std::fs::write(
            &mock_py,
            r#"import json, sys

def read_msg():
    length = None
    while True:
        line = sys.stdin.buffer.readline()
        if not line:
            return None
        s = line.decode("utf-8", errors="ignore").strip()
        if not s:
            break
        if s.lower().startswith("content-length:"):
            length = int(s.split(":", 1)[1].strip())
    if length is None:
        return None
    return json.loads(sys.stdin.buffer.read(length).decode("utf-8"))

def send_msg(obj):
    raw = json.dumps(obj).encode("utf-8")
    sys.stdout.buffer.write(f"Content-Length: {len(raw)}\r\n\r\n".encode("ascii") + raw)
    sys.stdout.buffer.flush()

while True:
    msg = read_msg()
    if msg is None:
        break
    method = msg.get("method")
    if method == "initialize":
        send_msg({"jsonrpc": "2.0", "id": msg["id"], "result": {"capabilities": {}}})
    elif method in ("textDocument/didOpen", "textDocument/didChange"):
        params = msg["params"]
        uri = params["textDocument"]["uri"]
        text = params["textDocument"]["text"] if method == "textDocument/didOpen" else params["contentChanges"][0]["text"]
        diags = []
        for idx, line in enumerate(text.splitlines()):
            if "BAD_SYNTAX" in line:
                diags.append({
                    "range": {"start": {"line": idx, "character": 4}, "end": {"line": idx, "character": 14}},
                    "severity": 1,
                    "message": "undefined symbol BAD_SYNTAX"
                })
        send_msg({
            "jsonrpc": "2.0",
            "method": "textDocument/publishDiagnostics",
            "params": {"uri": uri, "diagnostics": diags}
        })
"#,
        )
        .unwrap();

        let py_bin = if cfg!(windows) { "python" } else { "python3" };
        std::fs::write(
            ws.join(".meg").join("lsp.json"),
            serde_json::to_string(&json!({
                "toy": [py_bin, mock_py.display().to_string()]
            }))
            .unwrap(),
        )
        .unwrap();

        let target = ws.join("sample.toy");
        let broken = "ok()\n    BAD_SYNTAX()\n";
        std::fs::write(&target, broken).unwrap();

        let diag1 = collect_write_through_diagnostics(&ws, &target, broken)
            .expect("mock LSP must return diagnostics for broken code");
        assert!(diag1.contains("1 error(s), 0 warning(s)"), "{diag1}");
        assert!(diag1.contains("line 2:5 [error]: undefined symbol BAD_SYNTAX"), "{diag1}");

        // Fix the file -> didChange must clear diagnostics to None.
        let fixed = "ok()\n    good_call()\n";
        std::fs::write(&target, fixed).unwrap();
        let diag2 = collect_write_through_diagnostics(&ws, &target, fixed);
        assert!(diag2.is_none(), "expected clean diagnostics after fix, got: {diag2:?}");

        let _ = std::fs::remove_dir_all(&ws);
    }

    #[test]
    fn write_file_and_edit_file_return_live_lsp_diagnostics() {
        let ws = std::env::temp_dir().join(format!("meg-lsp-e2e-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&ws);
        std::fs::create_dir_all(ws.join(".meg")).unwrap();

        let mock_py = ws.join("mock_lsp_e2e.py");
        std::fs::write(
            &mock_py,
            r#"import json, sys
def read_msg():
    length = None
    while True:
        line = sys.stdin.buffer.readline()
        if not line: return None
        s = line.decode("utf-8", errors="ignore").strip()
        if not s: break
        if s.lower().startswith("content-length:"): length = int(s.split(":", 1)[1].strip())
    return None if length is None else json.loads(sys.stdin.buffer.read(length).decode("utf-8"))
def send_msg(obj):
    raw = json.dumps(obj).encode("utf-8")
    sys.stdout.buffer.write(f"Content-Length: {len(raw)}\r\n\r\n".encode("ascii") + raw)
    sys.stdout.buffer.flush()
while True:
    msg = read_msg()
    if msg is None: break
    m = msg.get("method")
    if m == "initialize":
        send_msg({"jsonrpc": "2.0", "id": msg["id"], "result": {"capabilities": {}}})
    elif m in ("textDocument/didOpen", "textDocument/didChange"):
        p = msg["params"]
        uri = p["textDocument"]["uri"]
        txt = p["textDocument"]["text"] if m == "textDocument/didOpen" else p["contentChanges"][0]["text"]
        diags = [
            {"range": {"start": {"line": i, "character": 0}, "end": {"line": i, "character": 8}}, "severity": 1, "message": "type mismatch ERR_TOKEN"}
            for i, l in enumerate(txt.splitlines()) if "ERR_TOKEN" in l
        ]
        send_msg({"jsonrpc": "2.0", "method": "textDocument/publishDiagnostics", "params": {"uri": uri, "diagnostics": diags}})
"#,
        )
        .unwrap();

        let py_bin = if cfg!(windows) { "python" } else { "python3" };
        std::fs::write(
            ws.join(".meg").join("lsp.json"),
            serde_json::to_string(&json!({ "toy2": [py_bin, mock_py.display().to_string()] })).unwrap(),
        )
        .unwrap();

        let reg = crate::ToolRegistry::with_core_tools(&ws);
        let r1 = reg.execute("write_file", &json!({ "path": "app.toy2", "content": "let x = ERR_TOKEN;\n" }));
        assert!(r1.ok, "{}", r1.output);
        assert!(r1.output.contains("[LSP diagnostics ("), "{}", r1.output);
        assert!(r1.output.contains("type mismatch ERR_TOKEN"), "{}", r1.output);

        let r2 = reg.execute("edit_file", &json!({ "path": "app.toy2", "old_str": "ERR_TOKEN", "new_str": "42" }));
        assert!(r2.ok, "{}", r2.output);
        assert!(!r2.output.contains("[LSP diagnostics"), "diagnostics should clear after fix: {}", r2.output);

        let _ = std::fs::remove_dir_all(&ws);
    }
}
