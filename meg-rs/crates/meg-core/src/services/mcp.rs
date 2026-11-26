//! Pure-Rust MCP Host supporting stdio JSON-RPC 2.0 and Streamable HTTP/SSE
//! transports, CLI > MCP policy enforcement, bounded frames, cancellation, and
//! Windows process-tree cleanup.

use super::security::{route_cli_or_mcp, sanitize_mcp_alias, CliOrMcpRoute};
use crate::CancellationToken;
use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpTool {
    pub server: String,
    pub name: String,
    pub alias: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(rename = "inputSchema")]
    pub input_schema: Value,
    #[serde(rename = "readOnly")]
    pub read_only: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct McpConfig {
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub args: Option<Vec<String>>,
    #[serde(default)]
    pub env: Option<HashMap<String, String>>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(rename = "type", default)]
    pub transport_type: Option<String>,
    #[serde(default)]
    pub headers: Option<HashMap<String, String>>,
    #[serde(default)]
    pub disabled: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
pub struct McpStatus {
    pub connected: bool,
    pub connecting: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

type PendingMap = Arc<Mutex<HashMap<u64, mpsc::Sender<Result<Value, String>>>>>;

struct StdioTransport {
    stdin: Mutex<ChildStdin>,
    child: Mutex<Child>,
    pending: PendingMap,
    stderr_tail: Arc<Mutex<String>>,
    seq: AtomicU64,
}

impl StdioTransport {
    fn spawn(
        command: &str,
        args: &[String],
        cwd: Option<&Path>,
        env_map: &HashMap<String, String>,
    ) -> Result<Arc<Self>> {
        let mut cmd = Command::new(command);
        cmd.args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(dir) = cwd {
            cmd.current_dir(dir);
        }
        for (k, v) in env_map {
            cmd.env(k, v);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        let mut child = cmd
            .spawn()
            .with_context(|| format!("Failed to launch MCP stdio command '{command}'"))?;
        let stdin = child.stdin.take().context("MCP stdio stdin unavailable")?;
        let stdout = child.stdout.take().context("MCP stdio stdout unavailable")?;
        let stderr_tail: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));
        if let Some(mut stderr) = child.stderr.take() {
            let tail_ref = stderr_tail.clone();
            std::thread::spawn(move || {
                let mut buf = [0u8; 1024];
                while let Ok(n) = stderr.read(&mut buf) {
                    if n == 0 {
                        break;
                    }
                    let chunk = String::from_utf8_lossy(&buf[..n]);
                    let mut guard = tail_ref.lock().unwrap();
                    guard.push_str(&chunk);
                    if guard.len() > 2048 {
                        let cut = guard.floor_char_boundary(guard.len() - 2048);
                        guard.drain(..cut);
                    }
                }
            });
        }
        let pending: PendingMap =
            Default::default();
        let responses = pending.clone();
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                let mut line = Vec::new();
                match reader
                    .by_ref()
                    .take(4 * 1024 * 1024 + 1)
                    .read_until(b'\n', &mut line)
                {
                    Ok(0) | Err(_) => break,
                    _ => {}
                }
                if line.len() > 4 * 1024 * 1024 {
                    if !line.ends_with(b"\n") {
                        let mut sink = [0u8; 4096];
                        loop {
                            match reader.read(&mut sink) {
                                Ok(0) | Err(_) => break,
                                Ok(n) if sink[..n].contains(&b'\n') => break,
                                Ok(_) => {}
                            }
                        }
                    }
                    continue;
                }
                let Ok(val) = serde_json::from_slice::<Value>(&line) else {
                    continue;
                };
                if let Some(id) = val.get("id").and_then(Value::as_u64) {
                    if let Some(tx) = responses.lock().unwrap().remove(&id) {
                        let res = if val.get("error").is_some_and(Value::is_object) {
                            Err(val["error"]["message"]
                                .as_str()
                                .unwrap_or("MCP server error")
                                .to_string())
                        } else {
                            Ok(val.get("result").cloned().unwrap_or(Value::Null))
                        };
                        let _ = tx.send(res);
                    }
                }
            }
            for (_, tx) in responses.lock().unwrap().drain() {
                let _ = tx.send(Err("MCP stdio server exited".into()));
            }
        });
        Ok(Arc::new(Self {
            stdin: Mutex::new(stdin),
            child: Mutex::new(child),
            pending,
            stderr_tail,
            seq: AtomicU64::new(1),
        }))
    }

    fn send_notification(&self, method: &str, params: Value) -> Result<()> {
        let payload = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        });
        let mut line = serde_json::to_string(&payload)?;
        line.push('\n');
        let mut stdin = self.stdin.lock().unwrap();
        stdin.write_all(line.as_bytes())?;
        stdin.flush()?;
        Ok(())
    }

    fn request(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
        cancel: &CancellationToken,
    ) -> Result<Value> {
        let id = self.seq.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = mpsc::channel();
        self.pending.lock().unwrap().insert(id, tx);
        let payload = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        let mut line = serde_json::to_string(&payload)?;
        line.push('\n');
        if let Err(e) = {
            let mut stdin = self.stdin.lock().unwrap();
            stdin.write_all(line.as_bytes()).and_then(|_| stdin.flush())
        } {
            self.pending.lock().unwrap().remove(&id);
            return Err(e.into());
        }

        let deadline = Instant::now() + timeout;
        loop {
            if cancel.is_cancelled() {
                self.pending.lock().unwrap().remove(&id);
                bail!("Operation cancelled");
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                self.pending.lock().unwrap().remove(&id);
                bail!("MCP request '{method}' timed out");
            }
            let step = remaining.min(Duration::from_millis(25));
            match rx.recv_timeout(step) {
                Ok(Ok(val)) => return Ok(val),
                Ok(Err(msg)) => bail!("{msg}"),
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    let tail = self.stderr_tail.lock().unwrap().trim().to_string();
                    if tail.is_empty() {
                        bail!("MCP server disconnected before answering '{method}'");
                    }
                    bail!("MCP server disconnected before answering '{method}' (stderr: {tail})");
                }
            }
        }
    }

    fn close(&self) {
        let mut child = self.child.lock().unwrap();
        kill_child_tree(&mut child);
        let _ = child.wait();
    }
}

struct HttpTransport {
    url: String,
    headers: HashMap<String, String>,
    session_id: Mutex<Option<String>>,
    seq: AtomicU64,
}

impl HttpTransport {
    fn new(url: &str, headers: HashMap<String, String>) -> Result<Arc<Self>> {
        let parsed = reqwest::Url::parse(url).context("Invalid MCP URL")?;
        if !matches!(parsed.scheme(), "http" | "https")
            || !parsed.username().is_empty()
            || parsed.password().is_some()
        {
            bail!("MCP URL must be an HTTP(S) URL without embedded credentials");
        }
        Ok(Arc::new(Self {
            url: parsed.to_string(),
            headers,
            session_id: Mutex::new(None),
            seq: AtomicU64::new(1),
        }))
    }

    fn request(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
        cancel: &CancellationToken,
    ) -> Result<Value> {
        if cancel.is_cancelled() {
            bail!("Operation cancelled");
        }
        let id = self.seq.fetch_add(1, Ordering::Relaxed);
        let body = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        let url = self.url.clone();
        let headers = self.headers.clone();
        let existing_sid = self.session_id.lock().unwrap().clone();
        let cancel_clone = cancel.clone();

        let (resp_sid, text) = run_async_isolated(async move {
            let client = reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(timeout)
                .build()?;
            let mut req = client
                .post(&url)
                .header("Content-Type", "application/json")
                .header("Accept", "application/json, text/event-stream");
            if let Some(sid) = existing_sid {
                req = req.header("Mcp-Session-Id", sid);
            }
            for (k, v) in headers {
                req = req.header(&k, &v);
            }
            let resp = tokio::select! {
                r = req.json(&body).send() => r?,
                _ = cancel_clone.cancelled() => bail!("Operation cancelled"),
            };
            let status = resp.status();
            let sid = resp
                .headers()
                .get("mcp-session-id")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string());
            let bytes = tokio::select! {
                b = resp.bytes() => b?,
                _ = cancel_clone.cancelled() => bail!("Operation cancelled"),
            };
            if bytes.len() > 4 * 1024 * 1024 {
                bail!("MCP HTTP response exceeded 4 MiB");
            }
            let text = String::from_utf8_lossy(&bytes).to_string();
            if !status.is_success() {
                bail!("MCP HTTP endpoint returned {status}");
            }
            Ok::<(Option<String>, String), anyhow::Error>((sid, text))
        })?;

        if let Some(sid) = resp_sid {
            *self.session_id.lock().unwrap() = Some(sid);
        }

        // Parse either plain JSON or SSE `data: {...}` payload.
        let parsed: Value = if let Ok(v) = serde_json::from_str(&text) {
            v
        } else {
            let mut found = None;
            for line in text.lines() {
                if let Some(rest) = line.trim().strip_prefix("data:") {
                    let payload = rest.trim();
                    if let Ok(v) = serde_json::from_str::<Value>(payload) {
                        if v.get("id").and_then(Value::as_u64) == Some(id) || v.get("result").is_some() {
                            found = Some(v);
                            break;
                        }
                    }
                }
            }
            found.ok_or_else(|| anyhow!("Invalid JSON-RPC payload from MCP HTTP server"))?
        };
        if parsed.get("error").is_some_and(Value::is_object) {
            bail!(
                "{}",
                parsed["error"]["message"]
                    .as_str()
                    .unwrap_or("MCP HTTP server error")
            );
        }
        Ok(parsed.get("result").cloned().unwrap_or(Value::Null))
    }

    fn send_notification(&self, method: &str, params: Value) {
        let body = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        });
        let url = self.url.clone();
        let headers = self.headers.clone();
        let existing_sid = self.session_id.lock().unwrap().clone();
        let _ = run_async_isolated(async move {
            let client = reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_secs(5))
                .build()?;
            let mut req = client
                .post(&url)
                .header("Content-Type", "application/json");
            if let Some(sid) = existing_sid {
                req = req.header("Mcp-Session-Id", sid);
            }
            for (k, v) in headers {
                req = req.header(&k, &v);
            }
            let _ = req.json(&body).send().await;
            Ok::<(), anyhow::Error>(())
        });
    }
}

enum TransportKind {
    Stdio(Arc<StdioTransport>),
    Http(Arc<HttpTransport>),
}

impl TransportKind {
    fn request(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
        cancel: &CancellationToken,
    ) -> Result<Value> {
        match self {
            TransportKind::Stdio(t) => t.request(method, params, timeout, cancel),
            TransportKind::Http(t) => t.request(method, params, timeout, cancel),
        }
    }

    fn notify(&self, method: &str, params: Value) {
        match self {
            TransportKind::Stdio(t) => {
                let _ = t.send_notification(method, params);
            }
            TransportKind::Http(t) => t.send_notification(method, params),
        }
    }

    fn close(&self) {
        if let TransportKind::Stdio(t) = self {
            t.close();
        }
    }
}

struct ServerEntry {
    transport: TransportKind,
    tools: Vec<McpTool>,
}

#[derive(Default)]
struct HostState {
    servers: HashMap<String, ServerEntry>,
    starting: HashSet<String>,
    errors: HashMap<String, String>,
}

pub struct McpHost {
    state: Mutex<HostState>,
}

impl Default for McpHost {
    fn default() -> Self {
        Self::new()
    }
}

impl McpHost {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(HostState::default()),
        }
    }

    pub fn connect(
        &self,
        name: &str,
        config: &McpConfig,
        cwd: Option<&Path>,
        cancel: &CancellationToken,
    ) -> Result<Vec<McpTool>> {
        if config.disabled == Some(true) {
            bail!("MCP server '{name}' is disabled in configuration");
        }
        match route_cli_or_mcp(name, &[]) {
            CliOrMcpRoute::Cli { reason, .. } => bail!("CLI_OVER_MCP_BLOCKED: {reason}"),
            CliOrMcpRoute::Denied { reason, .. }
                if std::env::var("MEG_STRICT_MCP_ALLOWLIST").ok().as_deref() == Some("1") =>
            {
                bail!("MCP_ALLOWLIST_BLOCKED: {reason}");
            }
            _ => {}
        }

        {
            let mut st = self.state.lock().unwrap();
            if let Some(entry) = st.servers.get(name) {
                return Ok(entry.tools.clone());
            }
            if st.starting.contains(name) {
                bail!("MCP server '{name}' is already connecting");
            }
            st.starting.insert(name.to_string());
        }

        let outcome = self.open_inner(name, config, cwd, cancel);
        let mut st = self.state.lock().unwrap();
        st.starting.remove(name);
        match outcome {
            Ok((transport, tools)) => {
                st.errors.remove(name);
                st.servers.insert(
                    name.to_string(),
                    ServerEntry {
                        transport,
                        tools: tools.clone(),
                    },
                );
                Ok(tools)
            }
            Err(err) => {
                let message = format!(
                    "Could not initialize MCP server '{name}'; check its command, arguments and environment ({err})"
                );
                st.errors.insert(name.to_string(), message.clone());
                bail!("{message}");
            }
        }
    }

    fn open_inner(
        &self,
        name: &str,
        config: &McpConfig,
        cwd: Option<&Path>,
        cancel: &CancellationToken,
    ) -> Result<(TransportKind, Vec<McpTool>)> {
        let transport = if let Some(url) = &config.url {
            let expanded = expand_environment(config.headers.as_ref())?;
            TransportKind::Http(HttpTransport::new(url, expanded)?)
        } else {
            let cmd = config
                .command
                .as_deref()
                .filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow!("MCP '{name}' needs a command or URL"))?;
            let args = config.args.clone().unwrap_or_default();
            let expanded = expand_environment(config.env.as_ref())?;
            TransportKind::Stdio(StdioTransport::spawn(cmd, &args, cwd, &expanded)?)
        };

        let init_res = transport.request(
            "initialize",
            json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": { "name": "meg", "version": "0.2.0" }
            }),
            Duration::from_secs(15),
            cancel,
        );
        if let Err(err) = init_res {
            transport.close();
            return Err(err);
        }
        transport.notify("notifications/initialized", json!({}));

        let mut tools = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let params = match &cursor {
                Some(c) => json!({ "cursor": c }),
                None => json!({}),
            };
            let page = match transport.request("tools/list", params, Duration::from_secs(15), cancel) {
                Ok(v) => v,
                Err(err) => {
                    transport.close();
                    return Err(err);
                }
            };
            if let Some(arr) = page.get("tools").and_then(Value::as_array) {
                for item in arr {
                    let Some(tname) = item.get("name").and_then(Value::as_str) else {
                        continue;
                    };
                    let read_only = item
                        .get("annotations")
                        .and_then(|a| a.get("readOnlyHint"))
                        .and_then(Value::as_bool)
                        .unwrap_or(false);
                    tools.push(McpTool {
                        server: name.to_string(),
                        name: tname.to_string(),
                        alias: sanitize_mcp_alias(name, tname),
                        description: item
                            .get("description")
                            .and_then(Value::as_str)
                            .map(|s| s.to_string()),
                        input_schema: item
                            .get("inputSchema")
                            .cloned()
                            .unwrap_or_else(|| json!({"type": "object"})),
                        read_only,
                    });
                }
            }
            if tools.len() > 500 {
                transport.close();
                bail!("MCP server advertises too many tools");
            }
            cursor = page
                .get("nextCursor")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string());
            if cursor.is_none() {
                break;
            }
        }
        Ok((transport, tools))
    }

    pub fn call_tool(
        &self,
        server: &str,
        tool: &str,
        args: Value,
        cancel: &CancellationToken,
    ) -> Result<Value> {
        let transport = {
            let st = self.state.lock().unwrap();
            let entry = st
                .servers
                .get(server)
                .ok_or_else(|| anyhow!("MCP tool is not connected: {server}/{tool}"))?;
            if !entry.tools.iter().any(|t| t.name == tool) {
                bail!("MCP tool is not connected: {server}/{tool}");
            }
            match &entry.transport {
                TransportKind::Stdio(t) => TransportKind::Stdio(t.clone()),
                TransportKind::Http(t) => TransportKind::Http(t.clone()),
            }
        };
        transport.request(
            "tools/call",
            json!({ "name": tool, "arguments": args }),
            Duration::from_secs(60),
            cancel,
        )
    }

    pub fn all_tools(&self) -> Vec<McpTool> {
        let st = self.state.lock().unwrap();
        let mut names: Vec<&String> = st.servers.keys().collect();
        names.sort();
        let mut out = Vec::new();
        for name in names {
            if let Some(entry) = st.servers.get(name) {
                out.extend(entry.tools.clone());
            }
        }
        out
    }

    pub fn status(&self, name: &str) -> McpStatus {
        let st = self.state.lock().unwrap();
        let connecting = st.starting.contains(name);
        let connected = st.servers.contains_key(name) && !connecting;
        McpStatus {
            connected,
            connecting,
            error: st.errors.get(name).cloned(),
        }
    }

    pub fn stop_server(&self, name: &str) {
        let removed = self.state.lock().unwrap().servers.remove(name);
        if let Some(entry) = removed {
            entry.transport.close();
        }
    }

    pub fn stop_all(&self) {
        let entries: Vec<ServerEntry> = {
            let mut st = self.state.lock().unwrap();
            st.servers.drain().map(|(_, v)| v).collect()
        };
        for entry in entries {
            entry.transport.close();
        }
    }
}

impl Drop for McpHost {
    fn drop(&mut self) {
        self.stop_all();
    }
}

pub fn expand_environment(values: Option<&HashMap<String, String>>) -> Result<HashMap<String, String>> {
    let mut out = HashMap::new();
    let Some(map) = values else {
        return Ok(out);
    };
    let re = regex::Regex::new(r"\$\{([A-Za-z_][A-Za-z0-9_]*)\}").expect("valid env regex");
    for (k, v) in map {
        let mut replaced = String::with_capacity(v.len());
        let mut last = 0;
        for cap in re.captures_iter(v) {
            let m = cap.get(0).unwrap();
            let var_name = cap.get(1).unwrap().as_str();
            let val = std::env::var(var_name)
                .map_err(|_| anyhow!("Missing MCP environment variable: {var_name}"))?;
            replaced.push_str(&v[last..m.start()]);
            replaced.push_str(&val);
            last = m.end();
        }
        replaced.push_str(&v[last..]);
        out.insert(k.clone(), replaced);
    }
    Ok(out)
}

pub(crate) fn kill_child_tree(child: &mut Child) {
    if child.try_wait().ok().flatten().is_some() {
        return;
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let pid = child.id();
        if pid > 0 {
            let _ = Command::new("taskkill")
                .args(["/PID", &pid.to_string(), "/T", "/F"])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .creation_flags(CREATE_NO_WINDOW)
                .status();
        }
    }
    let _ = child.kill();
}

fn run_async_isolated<F, T>(fut: F) -> Result<T>
where
    F: std::future::Future<Output = Result<T>> + Send + 'static,
    T: Send + 'static,
{
    if tokio::runtime::Handle::try_current().is_ok() {
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            rt.block_on(fut)
        })
        .join()
        .map_err(|_| anyhow!("MCP HTTP worker thread panicked"))?
    } else {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        rt.block_on(fut)
    }
}
