//! Token-authenticated headless JSON-RPC 2.0 daemon (`meg serve`).
//!
//! Binds strictly to `127.0.0.1` with a per-launch 256-bit bearer token so
//! local CLI, TUI, Web, or IDE clients can share stateful sessions, stream
//! live model/tool events, and cancel runs mid-flight without exposing an
//! unauthenticated loopback control plane.

use crate::bridge::parse_bridge_mode;
use crate::runner::{run_request_detailed, start_plane, RunExtras};
use anyhow::{bail, Result};
use meg_tui::RunnerMsg;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, Notify};

const MAX_RPC_LINE_BYTES: usize = 1024 * 1024;
const MAX_AUTH_FAILURES: usize = 3;

#[derive(Clone)]
pub(crate) struct ServeConfig {
    pub(crate) workspace: PathBuf,
    pub(crate) port: u16,
    pub(crate) token: String,
    pub(crate) default_provider: Option<meg_core::providers::ProviderProfile>,
    pub(crate) default_model: String,
    pub(crate) default_base_url: String,
}

impl ServeConfig {
    pub(crate) fn from_env(workspace: PathBuf, port: u16) -> Result<Self> {
        let token = match std::env::var("MEG_SERVE_TOKEN") {
            Ok(t) if t.trim().len() >= 16 => t.trim().to_string(),
            Ok(_) => bail!("MEG_SERVE_TOKEN must be at least 16 characters"),
            Err(_) => generate_daemon_token(),
        };
        let default_provider = if std::env::var_os("MEG_TUI_BASE_URL").is_some() {
            Some(meg_core::providers::ProviderProfile::environment())
        } else {
            meg_core::providers::ProviderStore::global()
                .load()
                .ok()
                .and_then(|s| {
                    s.profiles
                        .iter()
                        .find(|p| p.id == s.active)
                        .or_else(|| s.profiles.first())
                        .cloned()
                })
        };
        let default_model = std::env::var("MEG_TUI_MODEL")
            .ok()
            .or_else(|| default_provider.as_ref().map(|p| p.default_model.clone()))
            .unwrap_or_else(meg_core::shim_client::default_model);
        let default_base_url = std::env::var("MEG_TUI_BASE_URL")
            .ok()
            .or_else(|| default_provider.as_ref().map(|p| p.base_url.clone()))
            .unwrap_or_else(meg_core::shim_client::default_base_url);
        Ok(Self {
            workspace,
            port,
            token,
            default_provider,
            default_model,
            default_base_url,
        })
    }
}

pub(crate) fn generate_daemon_token() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

fn constant_time_eq(a: &str, b: &str) -> bool {
    let ab = a.as_bytes();
    let bb = b.as_bytes();
    if ab.len() != bb.len() {
        return false;
    }
    let mut diff: u8 = 0;
    for (&x, &y) in ab.iter().zip(bb.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

struct DaemonSession {
    id: String,
    workspace: PathBuf,
    provider: Option<meg_core::providers::ProviderProfile>,
    model: String,
    base_url: String,
    mode: meg_tui::Mode,
    effort: String,
    pair_mode: Option<String>,
    executor_model: Option<String>,
    reviewer_model: Option<String>,
    history: Arc<Mutex<Vec<meg_core::ChatMessage>>>,
    active_cancel: Arc<Mutex<Option<meg_core::CancellationToken>>>,
}

pub(crate) struct DaemonState {
    config: ServeConfig,
    plane: Option<Arc<meg_core::services::ServicePlane>>,
    sessions: Mutex<HashMap<String, DaemonSession>>,
    pub(crate) shutdown: Notify,
}

pub(crate) struct DaemonHandle {
    pub(crate) addr: SocketAddr,
    pub(crate) token: String,
    pub(crate) task: tokio::task::JoinHandle<()>,
    pub(crate) state: Arc<DaemonState>,
}

impl DaemonHandle {
    #[cfg(test)]
    pub(crate) async fn stop(self) {
        self.state.shutdown.notify_waiters();
        let _ = self.task.await;
    }
}

pub(crate) async fn spawn_daemon(config: ServeConfig) -> Result<DaemonHandle> {
    let bind_addr = format!("127.0.0.1:{}", config.port);
    let listener = TcpListener::bind(&bind_addr).await?;
    let addr = listener.local_addr()?;
    let plane = if std::env::var("MEG_NO_PLANE").as_deref() == Ok("1")
        || std::env::var("MEG_PLANE_DISABLED").as_deref() == Ok("1")
    {
        None
    } else {
        start_plane().ok()
    };
    let token = config.token.clone();
    let state = Arc::new(DaemonState {
        config,
        plane,
        sessions: Mutex::new(HashMap::new()),
        shutdown: Notify::new(),
    });
    let state_loop = state.clone();
    let task = tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = state_loop.shutdown.notified() => {
                    let sessions = state_loop.sessions.lock().unwrap();
                    for s in sessions.values() {
                        if let Some(tok) = s.active_cancel.lock().unwrap().as_ref() {
                            tok.cancel();
                        }
                    }
                    break;
                }
                accepted = listener.accept() => {
                    match accepted {
                        Ok((stream, _peer)) => {
                            let st = state_loop.clone();
                            tokio::spawn(async move {
                                let _ = handle_client(stream, st).await;
                            });
                        }
                        Err(_) => break,
                    }
                }
            }
        }
    });
    Ok(DaemonHandle {
        addr,
        token,
        task,
        state,
    })
}

pub(crate) async fn run_serve(workspace: &Path, port: u16) -> Result<()> {
    let cfg = ServeConfig::from_env(workspace.to_path_buf(), port)?;
    let handle = spawn_daemon(cfg).await?;
    println!(
        "{}",
        json!({
            "event": "listening",
            "protocol": "meg-serve/v1",
            "addr": handle.addr.to_string(),
            "token": handle.token,
            "workspace": workspace.display().to_string(),
            "native_plane": handle.state.plane.as_ref().is_some_and(|p| p.is_native()),
        })
    );
    let state_ctrlc = handle.state.clone();
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            state_ctrlc.shutdown.notify_waiters();
        }
    });
    let _ = handle.task.await;
    Ok(())
}

fn rpc_ok(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn rpc_err(id: Value, code: i32, message: impl Into<String>) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message.into() }
    })
}

fn rpc_notify(method: &str, params: Value) -> Value {
    json!({ "jsonrpc": "2.0", "method": method, "params": params })
}

async fn handle_client(stream: TcpStream, state: Arc<DaemonState>) -> Result<()> {
    let (read_half, mut write_half) = stream.into_split();
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<String>();

    let writer = tokio::spawn(async move {
        while let Some(line) = out_rx.recv().await {
            if write_half.write_all(line.as_bytes()).await.is_err() {
                break;
            }
            if write_half.write_all(b"\n").await.is_err() {
                break;
            }
            if write_half.flush().await.is_err() {
                break;
            }
        }
    });

    let send_val = |v: Value| {
        if let Ok(s) = serde_json::to_string(&v) {
            let _ = out_tx.send(s);
        }
    };

    let mut reader = BufReader::new(read_half);
    let mut buf = Vec::new();
    let mut authed = false;
    let mut auth_failures = 0usize;

    loop {
        buf.clear();
        let n = reader.read_until(b'\n', &mut buf).await?;
        if n == 0 {
            break;
        }
        if buf.len() > MAX_RPC_LINE_BYTES {
            send_val(rpc_err(Value::Null, -32600, "Frame exceeds 1 MiB limit"));
            break;
        }
        let line = String::from_utf8_lossy(&buf);
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let req: Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(e) => {
                send_val(rpc_err(Value::Null, -32700, format!("Parse error: {e}")));
                continue;
            }
        };
        let id = req.get("id").cloned().unwrap_or(Value::Null);
        let method = req.get("method").and_then(Value::as_str).unwrap_or("");
        let params = req.get("params").cloned().unwrap_or_else(|| json!({}));

        if method == "auth" {
            let supplied = params.get("token").and_then(Value::as_str).unwrap_or("");
            if constant_time_eq(supplied, &state.config.token) {
                authed = true;
                auth_failures = 0;
                send_val(rpc_ok(
                    id,
                    json!({ "authenticated": true, "protocol": "meg-serve/v1" }),
                ));
            } else {
                auth_failures += 1;
                send_val(rpc_err(id, -32001, "Unauthorized: invalid daemon token"));
                if auth_failures >= MAX_AUTH_FAILURES {
                    break;
                }
            }
            continue;
        }

        if !authed {
            let inline_tok = req.get("token").and_then(Value::as_str).unwrap_or("");
            if !inline_tok.is_empty() && constant_time_eq(inline_tok, &state.config.token) {
                authed = true;
            } else {
                auth_failures += 1;
                send_val(rpc_err(
                    id,
                    -32001,
                    "Unauthorized: authenticate with 'auth' method first",
                ));
                if auth_failures >= MAX_AUTH_FAILURES {
                    break;
                }
                continue;
            }
        }

        match method {
            "health" => {
                let count = state.sessions.lock().unwrap().len();
                send_val(rpc_ok(
                    id,
                    json!({
                        "status": "ok",
                        "protocol": "meg-serve/v1",
                        "version": meg_tui::APP_VERSION,
                        "workspace": state.config.workspace.display().to_string(),
                        "sessions": count,
                        "native_plane": state.plane.as_ref().is_some_and(|p| p.is_native()),
                    }),
                ));
            }
            "session.create" => {
                let ws = match params.get("workspace").and_then(Value::as_str) {
                    Some(p) if !p.trim().is_empty() => match PathBuf::from(p).canonicalize() {
                        Ok(c) if c.is_dir() => c,
                        _ => {
                            send_val(rpc_err(id, -32602, format!("Invalid workspace directory: {p}")));
                            continue;
                        }
                    },
                    _ => state.config.workspace.clone(),
                };
                let mode = match params.get("mode").and_then(Value::as_str) {
                    Some(raw) => match parse_bridge_mode(raw) {
                        Ok(m) => m,
                        Err(e) => {
                            send_val(rpc_err(id, -32602, e.to_string()));
                            continue;
                        }
                    },
                    None => meg_tui::Mode::Plan,
                };
                let model = params
                    .get("model")
                    .and_then(Value::as_str)
                    .filter(|s| !s.trim().is_empty())
                    .map(str::to_string)
                    .unwrap_or_else(|| state.config.default_model.clone());
                let base_url = params
                    .get("base_url")
                    .and_then(Value::as_str)
                    .filter(|s| !s.trim().is_empty())
                    .map(str::to_string)
                    .unwrap_or_else(|| state.config.default_base_url.clone());
                let effort = params
                    .get("effort")
                    .and_then(Value::as_str)
                    .unwrap_or("medium")
                    .to_string();
                let provider = if params.get("base_url").and_then(Value::as_str).is_some() {
                    None
                } else {
                    state.config.default_provider.clone()
                };
                let sid = format!("sess_{}", &uuid::Uuid::new_v4().simple().to_string()[..12]);
                let sess = DaemonSession {
                    id: sid.clone(),
                    workspace: ws.clone(),
                    provider,
                    model: model.clone(),
                    base_url,
                    mode,
                    effort: effort.clone(),
                    pair_mode: params.get("pair_mode").and_then(Value::as_str).map(str::to_string),
                    executor_model: params.get("executor_model").and_then(Value::as_str).map(str::to_string),
                    reviewer_model: params.get("reviewer_model").and_then(Value::as_str).map(str::to_string),
                    history: Arc::new(Mutex::new(Vec::new())),
                    active_cancel: Arc::new(Mutex::new(None)),
                };
                state.sessions.lock().unwrap().insert(sid.clone(), sess);
                send_val(rpc_ok(
                    id,
                    json!({
                        "session_id": sid,
                        "workspace": ws.display().to_string(),
                        "model": model,
                        "mode": mode.label(),
                        "effort": effort,
                    }),
                ));
            }
            "session.list" => {
                let sessions = state.sessions.lock().unwrap();
                let list: Vec<Value> = sessions
                    .values()
                    .map(|s| {
                        json!({
                            "session_id": s.id,
                            "workspace": s.workspace.display().to_string(),
                            "model": s.model,
                            "mode": s.mode.label(),
                            "running": s.active_cancel.lock().unwrap().is_some(),
                            "history_len": s.history.lock().unwrap().len(),
                        })
                    })
                    .collect();
                send_val(rpc_ok(id, json!({ "sessions": list })));
            }
            "session.history" => {
                let sid = params.get("session_id").and_then(Value::as_str).unwrap_or("");
                let sessions = state.sessions.lock().unwrap();
                let Some(sess) = sessions.get(sid) else {
                    send_val(rpc_err(id, -32004, format!("Unknown session '{sid}'")));
                    continue;
                };
                let msgs: Vec<Value> = sess
                    .history
                    .lock()
                    .unwrap()
                    .iter()
                    .map(|m| json!({ "role": m.role, "content": m.content }))
                    .collect();
                send_val(rpc_ok(id, json!({ "session_id": sid, "messages": msgs })));
            }
            "session.set_mode" => {
                let sid = params.get("session_id").and_then(Value::as_str).unwrap_or("");
                let raw_mode = params.get("mode").and_then(Value::as_str).unwrap_or("");
                let parsed = match parse_bridge_mode(raw_mode) {
                    Ok(m) => m,
                    Err(e) => {
                        send_val(rpc_err(id, -32602, e.to_string()));
                        continue;
                    }
                };
                let mut sessions = state.sessions.lock().unwrap();
                let Some(sess) = sessions.get_mut(sid) else {
                    send_val(rpc_err(id, -32004, format!("Unknown session '{sid}'")));
                    continue;
                };
                sess.mode = parsed;
                send_val(rpc_ok(
                    id,
                    json!({ "session_id": sid, "mode": sess.mode.label() }),
                ));
            }
            "session.cancel" => {
                let sid = params.get("session_id").and_then(Value::as_str).unwrap_or("");
                let sessions = state.sessions.lock().unwrap();
                let Some(sess) = sessions.get(sid) else {
                    send_val(rpc_err(id, -32004, format!("Unknown session '{sid}'")));
                    continue;
                };
                let cancelled = if let Some(tok) = sess.active_cancel.lock().unwrap().as_ref() {
                    tok.cancel();
                    true
                } else {
                    false
                };
                send_val(rpc_ok(
                    id,
                    json!({ "session_id": sid, "cancelled": cancelled }),
                ));
            }
            "session.delete" => {
                let sid = params.get("session_id").and_then(Value::as_str).unwrap_or("");
                let removed = state.sessions.lock().unwrap().remove(sid);
                if let Some(sess) = removed {
                    if let Some(tok) = sess.active_cancel.lock().unwrap().take() {
                        tok.cancel();
                    }
                    send_val(rpc_ok(id, json!({ "session_id": sid, "deleted": true })));
                } else {
                    send_val(rpc_err(id, -32004, format!("Unknown session '{sid}'")));
                }
            }
            "session.run" => {
                let sid = params.get("session_id").and_then(Value::as_str).unwrap_or("").to_string();
                let goal = params
                    .get("goal")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim()
                    .to_string();
                if goal.is_empty() {
                    send_val(rpc_err(id, -32602, "session.run requires non-empty 'goal'"));
                    continue;
                }
                if std::env::var("MEG_OFFLINE").as_deref() == Ok("1") {
                    send_val(rpc_err(id, -32002, "MEG_OFFLINE=1: inference disabled"));
                    continue;
                }

                let cancel_tok = meg_core::CancellationToken::default();
                let (run_req, extras, hist_arc, cancel_slot) = {
                    let sessions = state.sessions.lock().unwrap();
                    let Some(sess) = sessions.get(&sid) else {
                        send_val(rpc_err(id, -32004, format!("Unknown session '{sid}'")));
                        continue;
                    };
                    let mut slot = sess.active_cancel.lock().unwrap();
                    if slot.is_some() {
                        send_val(rpc_err(
                            id,
                            -32003,
                            "Session already has an active run; call session.cancel first",
                        ));
                        continue;
                    }
                    *slot = Some(cancel_tok.clone());
                    let prior_history = sess.history.lock().unwrap().clone();
                    let run_req = meg_tui::RunRequest {
                        provider: sess.provider.clone(),
                        goal: goal.clone(),
                        workspace: sess.workspace.clone(),
                        model: sess.model.clone(),
                        base_url: sess.base_url.clone(),
                        history: prior_history,
                        mode: sess.mode,
                        effort: params
                            .get("effort")
                            .and_then(Value::as_str)
                            .unwrap_or(&sess.effort)
                            .to_string(),
                        cancel: cancel_tok,
                        pair_mode: sess.pair_mode.clone(),
                        executor_model: sess.executor_model.clone(),
                        reviewer_model: sess.reviewer_model.clone(),
                        attachments: Vec::new(),
                    };
                    let extras = RunExtras {
                        max_steps: params.get("max_steps").and_then(Value::as_u64).map(|n| n as usize),
                        verify_cmd: params.get("verify_cmd").and_then(Value::as_str).map(str::to_string),
                        require_verify: params.get("require_verify").and_then(Value::as_bool).unwrap_or(false),
                        pair_mode: sess.pair_mode.clone(),
                        executor_model: sess.executor_model.clone(),
                        reviewer_model: sess.reviewer_model.clone(),
                    };
                    (run_req, extras, sess.history.clone(), sess.active_cancel.clone())
                };

                let run_id = format!("run_{}", &uuid::Uuid::new_v4().simple().to_string()[..10]);
                send_val(rpc_ok(
                    id,
                    json!({ "accepted": true, "session_id": sid, "run_id": run_id }),
                ));

                let (tx, rx) = std::sync::mpsc::channel::<RunnerMsg>();
                let ev_tx = out_tx.clone();
                let ev_sid = sid.clone();
                let ev_run_id = run_id.clone();
                let forwarder = std::thread::spawn(move || {
                    for msg in rx {
                        let ev = match msg {
                            RunnerMsg::StreamStart => json!({"event": "stream_start"}),
                            RunnerMsg::ThinkingChunk(t) => json!({"event": "thinking", "text": t}),
                            RunnerMsg::ContentChunk(c) => json!({"event": "chunk", "text": c}),
                            RunnerMsg::ToolStarted { id, name, args } => {
                                json!({"event": "tool_start", "id": id, "name": name, "args": args})
                            }
                            RunnerMsg::ToolFinished { id, ok, output } => {
                                json!({"event": "tool_end", "id": id, "ok": ok, "output": output})
                            }
                            RunnerMsg::Usage { input, output, cached } => {
                                json!({"event": "usage", "input": input, "output": output, "cached": cached})
                            }
                            _ => continue,
                        };
                        let note = rpc_notify(
                            "session.event",
                            json!({
                                "session_id": ev_sid,
                                "run_id": ev_run_id,
                                "payload": ev,
                            }),
                        );
                        if let Ok(s) = serde_json::to_string(&note) {
                            let _ = ev_tx.send(s);
                        }
                    }
                });

                let plane_clone = state.plane.clone();
                let done_tx = out_tx.clone();
                tokio::spawn(async move {
                    let res = run_request_detailed(run_req, Some(tx), plane_clone, extras).await;
                    let _ = forwarder.join();
                    *cancel_slot.lock().unwrap() = None;
                    let payload = match res {
                        Ok((outcomes, answer, telemetry)) => {
                            if let Some(ref ans) = answer {
                                let mut h = hist_arc.lock().unwrap();
                                h.push(meg_core::ChatMessage {
                                    role: "user".into(),
                                    content: goal,
                                });
                                h.push(meg_core::ChatMessage {
                                    role: "assistant".into(),
                                    content: ans.clone(),
                                });
                            }
                            json!({
                                "event": "done",
                                "ok": true,
                                "turns": outcomes.len(),
                                "answer": answer,
                                "planner_turns": telemetry.planner_turns,
                                "executor_turns": telemetry.executor_turns,
                                "reviewer_verdict": telemetry.reviewer_verdict,
                            })
                        }
                        Err(e) => json!({
                            "event": "done",
                            "ok": false,
                            "error": e.to_string(),
                        }),
                    };
                    let note = rpc_notify(
                        "session.event",
                        json!({
                            "session_id": sid,
                            "run_id": run_id,
                            "payload": payload,
                        }),
                    );
                    if let Ok(s) = serde_json::to_string(&note) {
                        let _ = done_tx.send(s);
                    }
                });
            }
            "shutdown" => {
                send_val(rpc_ok(id, json!({ "shutting_down": true })));
                state.shutdown.notify_waiters();
                break;
            }
            other => {
                send_val(rpc_err(id, -32601, format!("Method not found: '{other}'")));
            }
        }
    }

    drop(out_tx);
    let _ = writer.await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    async fn send_line(w: &mut tokio::net::tcp::OwnedWriteHalf, v: &Value) {
        let s = serde_json::to_string(v).unwrap();
        w.write_all(s.as_bytes()).await.unwrap();
        w.write_all(b"\n").await.unwrap();
        w.flush().await.unwrap();
    }

    async fn read_json(r: &mut BufReader<tokio::net::tcp::OwnedReadHalf>) -> Value {
        let mut line = String::new();
        let n = r.read_line(&mut line).await.unwrap();
        assert!(n > 0, "unexpected EOF from daemon");
        serde_json::from_str(line.trim()).unwrap()
    }

    #[tokio::test]
    async fn serve_enforces_auth_persists_sessions_and_cancels_in_flight() {
        let mock = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": [{"id": "fixture"}]})))
            .mount(&mock)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "choices": [{"message": {"content": "{\"final\":\"Turn 1 ok\"}"}}],
                "usage": {"prompt_tokens": 20, "completion_tokens": 5}
            })))
            .up_to_n_times(1)
            .mount(&mock)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_delay(std::time::Duration::from_secs(5))
                    .set_body_json(json!({"choices": [{"message": {"content": "{\"final\":\"late\"}"}}]})),
            )
            .mount(&mock)
            .await;

        let ws = std::env::temp_dir().join(format!("meg-serve-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&ws).unwrap();
        let cfg = ServeConfig {
            workspace: ws.clone(),
            port: 0,
            token: generate_daemon_token(),
            default_provider: None,
            default_model: "fixture".into(),
            default_base_url: format!("{}/v1", mock.uri()),
        };
        let handle = spawn_daemon(cfg).await.unwrap();

        let stream = TcpStream::connect(handle.addr).await.unwrap();
        let (rh, mut wh) = stream.into_split();
        let mut reader = BufReader::new(rh);

        // 1. Unauthenticated request MUST fail closed (-32001)
        send_line(&mut wh, &json!({"jsonrpc": "2.0", "id": 1, "method": "health"})).await;
        let unauth = read_json(&mut reader).await;
        assert_eq!(unauth["error"]["code"], -32001);

        // 2. Authenticate with daemon token
        send_line(
            &mut wh,
            &json!({"jsonrpc": "2.0", "id": 2, "method": "auth", "params": {"token": handle.token}}),
        )
        .await;
        let auth_ok = read_json(&mut reader).await;
        assert_eq!(auth_ok["result"]["authenticated"], true);

        // 3. Create session & check invalid mode fails closed
        send_line(
            &mut wh,
            &json!({"jsonrpc": "2.0", "id": 3, "method": "session.create", "params": {"mode": "plan"}}),
        )
        .await;
        let created = read_json(&mut reader).await;
        let sid = created["result"]["session_id"].as_str().unwrap().to_string();

        send_line(
            &mut wh,
            &json!({"jsonrpc": "2.0", "id": 4, "method": "session.set_mode", "params": {"session_id": sid, "mode": "bogus"}}),
        )
        .await;
        let bad_mode = read_json(&mut reader).await;
        assert_eq!(bad_mode["error"]["code"], -32602);

        // 4. Run Turn 1 and wait for streaming done event
        send_line(
            &mut wh,
            &json!({"jsonrpc": "2.0", "id": 5, "method": "session.run", "params": {"session_id": sid, "goal": "Remember cobalt"}}),
        )
        .await;
        let ack = read_json(&mut reader).await;
        assert_eq!(ack["result"]["accepted"], true);

        let done_ev = loop {
            let msg = read_json(&mut reader).await;
            if msg["method"] == "session.event" && msg["params"]["payload"]["event"] == "done" {
                break msg;
            }
        };
        assert_eq!(done_ev["params"]["payload"]["ok"], true);
        assert_eq!(done_ev["params"]["payload"]["answer"], "Turn 1 ok");

        // 5. Verify session history persisted Turn 1
        send_line(
            &mut wh,
            &json!({"jsonrpc": "2.0", "id": 6, "method": "session.history", "params": {"session_id": sid}}),
        )
        .await;
        let hist = read_json(&mut reader).await;
        assert_eq!(hist["result"]["messages"].as_array().unwrap().len(), 2);

        // 6. Start Turn 2 (5s delay) and cancel mid-flight on the same connection
        let started = std::time::Instant::now();
        send_line(
            &mut wh,
            &json!({"jsonrpc": "2.0", "id": 7, "method": "session.run", "params": {"session_id": sid, "goal": "Slow turn"}}),
        )
        .await;
        let ack2 = read_json(&mut reader).await;
        assert_eq!(ack2["result"]["accepted"], true);

        tokio::time::sleep(std::time::Duration::from_millis(80)).await;
        send_line(
            &mut wh,
            &json!({"jsonrpc": "2.0", "id": 8, "method": "session.cancel", "params": {"session_id": sid}}),
        )
        .await;

        let mut saw_cancel_resp = false;
        let mut saw_cancel_done = false;
        while !(saw_cancel_resp && saw_cancel_done) {
            let msg = read_json(&mut reader).await;
            if msg["id"] == 8 {
                assert_eq!(msg["result"]["cancelled"], true);
                saw_cancel_resp = true;
            } else if msg["method"] == "session.event"
                && msg["params"]["payload"]["event"] == "done"
            {
                assert_eq!(msg["params"]["payload"]["ok"], false);
                saw_cancel_done = true;
            }
        }
        assert!(
            started.elapsed() < std::time::Duration::from_secs(2),
            "cancellation must abort promptly without waiting for 5s endpoint"
        );

        handle.stop().await;
        let _ = std::fs::remove_dir_all(&ws);
    }
}
