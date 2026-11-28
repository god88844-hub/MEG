//! Native loopback JSON-RPC 2.0 client (`meg attach`) for `meg serve`.
//!
//! Supports both one-shot streaming goals (`meg attach --addr 127.0.0.1:PORT --token TOK "goal"`)
//! with live `Ctrl+C` -> `session.cancel` forwarding, and an interactive REPL when invoked
//! without a positional goal.

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::io::Write;
use std::path::PathBuf;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;

#[derive(Debug, Clone)]
pub(crate) struct AttachOptions {
    pub(crate) addr: String,
    pub(crate) token: String,
    pub(crate) session_id: Option<String>,
    pub(crate) workspace: Option<PathBuf>,
    pub(crate) mode: Option<String>,
    pub(crate) json: bool,
    pub(crate) goal: Option<String>,
}

fn normalize_attach_addr(raw: &str) -> Result<String> {
    let trimmed = raw.trim();
    anyhow::ensure!(
        !trimmed.is_empty(),
        "attach requires --addr <host:port> (or MEG_SERVE_ADDR)"
    );
    let normalized = if trimmed.chars().all(|c| c.is_ascii_digit()) {
        format!("127.0.0.1:{trimmed}")
    } else {
        trimmed.to_string()
    };
    let host = if normalized.starts_with('[') {
        normalized
            .split(']')
            .next()
            .unwrap_or("")
            .trim_start_matches('[')
    } else {
        normalized.rsplit_once(':').map(|(h, _)| h).unwrap_or("")
    };
    let is_loopback = matches!(host, "127.0.0.1" | "localhost" | "::1");
    let allow_remote = std::env::var("MEG_ATTACH_ALLOW_REMOTE")
        .ok()
        .is_some_and(|v| matches!(v.as_str(), "1" | "true" | "TRUE"));
    if !is_loopback && !allow_remote {
        bail!(
            "Refusing to send daemon token to non-loopback address '{normalized}' (set MEG_ATTACH_ALLOW_REMOTE=1 to override)"
        );
    }
    Ok(normalized)
}

pub(crate) fn parse_attach_options(args: &[String]) -> Result<AttachOptions> {
    let mut addr = std::env::var("MEG_SERVE_ADDR")
        .ok()
        .filter(|s| !s.trim().is_empty());
    let mut token = std::env::var("MEG_SERVE_TOKEN")
        .ok()
        .filter(|s| !s.trim().is_empty());
    let mut session_id: Option<String> = None;
    let mut workspace: Option<PathBuf> = None;
    let mut mode: Option<String> = None;
    let mut json_mode = false;
    let mut goal_parts: Vec<String> = Vec::new();

    let mut i = 2;
    let mut literal = false;
    while i < args.len() {
        let arg = args[i].as_str();
        if literal {
            goal_parts.push(args[i].clone());
            i += 1;
            continue;
        }
        match arg {
            "--" => {
                literal = true;
                i += 1;
            }
            "--addr" | "-a" => {
                let val = args.get(i + 1).context("--addr requires <host:port>")?;
                addr = Some(val.clone());
                i += 2;
            }
            "--token" | "-t" => {
                let val = args.get(i + 1).context("--token requires a daemon token")?;
                token = Some(val.clone());
                i += 2;
            }
            "--session" | "-s" => {
                let val = args.get(i + 1).context("--session requires a session id")?;
                session_id = Some(val.clone());
                i += 2;
            }
            "-C" | "--cd" => {
                let val = args.get(i + 1).context("-C/--cd requires a workspace path")?;
                workspace = Some(PathBuf::from(val));
                i += 2;
            }
            "--mode" => {
                let val = args.get(i + 1).context("--mode requires 'plan' or 'build'")?;
                let _ = crate::bridge::parse_bridge_mode(val)?;
                mode = Some(val.clone());
                i += 2;
            }
            "--json" => {
                json_mode = true;
                i += 1;
            }
            other if other.starts_with('-') => {
                bail!("Unknown option for 'meg attach': {other}");
            }
            _ => {
                goal_parts.push(args[i].clone());
                i += 1;
            }
        }
    }

    let addr = normalize_attach_addr(
        addr.as_deref()
            .ok_or_else(|| anyhow::anyhow!("meg attach requires --addr <host:port> (or MEG_SERVE_ADDR)"))?,
    )?;
    let token = token
        .filter(|t| !t.trim().is_empty())
        .ok_or_else(|| anyhow::anyhow!("meg attach requires --token <token> (or MEG_SERVE_TOKEN)"))?;

    let goal = if goal_parts.is_empty() {
        None
    } else {
        Some(goal_parts.join(" "))
    };

    Ok(AttachOptions {
        addr,
        token,
        session_id,
        workspace,
        mode,
        json: json_mode,
        goal,
    })
}

pub(crate) struct DaemonClient {
    reader: BufReader<tokio::net::tcp::OwnedReadHalf>,
    writer: tokio::net::tcp::OwnedWriteHalf,
    next_id: u64,
}

#[derive(Debug, Clone)]
pub(crate) struct TurnOutcome {
    pub(crate) ok: bool,
    #[allow(dead_code)]
    pub(crate) answer: String,
    pub(crate) error: Option<String>,
    #[allow(dead_code)]
    pub(crate) turns: u64,
}

impl DaemonClient {
    pub(crate) async fn connect_and_auth(addr: &str, token: &str) -> Result<Self> {
        let normalized = normalize_attach_addr(addr)?;
        let stream = TcpStream::connect(&normalized)
            .await
            .with_context(|| format!("Failed to connect to MEG daemon at {normalized}"))?;
        let (rh, wh) = stream.into_split();
        let mut client = Self {
            reader: BufReader::new(rh),
            writer: wh,
            next_id: 1,
        };
        let res = client.call("auth", json!({ "token": token })).await?;
        anyhow::ensure!(
            res["authenticated"].as_bool() == Some(true),
            "Daemon authentication failed"
        );
        Ok(client)
    }

    async fn send_rpc(&mut self, method: &str, params: Value) -> Result<u64> {
        let id = self.next_id;
        self.next_id += 1;
        let payload = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        let mut line = serde_json::to_string(&payload)?;
        line.push('\n');
        self.writer.write_all(line.as_bytes()).await?;
        self.writer.flush().await?;
        Ok(id)
    }

    async fn read_message(&mut self) -> Result<Value> {
        let mut line = String::new();
        let n = self.reader.read_line(&mut line).await?;
        if n == 0 {
            bail!("Connection to MEG daemon closed unexpectedly");
        }
        serde_json::from_str(line.trim()).context("Invalid JSON-RPC frame from daemon")
    }

    pub(crate) async fn call(&mut self, method: &str, params: Value) -> Result<Value> {
        let id = self.send_rpc(method, params).await?;
        loop {
            let msg = self.read_message().await?;
            if msg.get("id").and_then(Value::as_u64) == Some(id) {
                if let Some(err) = msg.get("error") {
                    let text = err["message"]
                        .as_str()
                        .unwrap_or("Daemon returned JSON-RPC error");
                    bail!("Daemon error ({method}): {text}");
                }
                return Ok(msg["result"].clone());
            }
        }
    }

    pub(crate) async fn ensure_session(
        &mut self,
        session_id: Option<&str>,
        workspace: Option<&PathBuf>,
        mode: Option<&str>,
    ) -> Result<String> {
        if let Some(sid) = session_id {
            if let Some(m) = mode {
                let _ = self
                    .call("session.set_mode", json!({ "session_id": sid, "mode": m }))
                    .await?;
            }
            return Ok(sid.to_string());
        }
        let mut params = json!({});
        if let Some(ws) = workspace {
            params["workspace"] = json!(ws.display().to_string());
        }
        if let Some(m) = mode {
            params["mode"] = json!(m);
        }
        let created = self.call("session.create", params).await?;
        created["session_id"]
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| anyhow::anyhow!("Daemon session.create did not return session_id"))
    }

    pub(crate) async fn run_turn(
        &mut self,
        session_id: &str,
        goal: &str,
        json_mode: bool,
        cancel_on_ctrlc: bool,
    ) -> Result<TurnOutcome> {
        let ack = self
            .call(
                "session.run",
                json!({ "session_id": session_id, "goal": goal }),
            )
            .await?;
        anyhow::ensure!(
            ack["accepted"].as_bool() == Some(true),
            "Daemon did not accept session.run"
        );
        let run_id = ack["run_id"].as_str().unwrap_or("").to_string();
        if json_mode {
            println!(
                "{}",
                json!({ "event": "accepted", "session_id": session_id, "run_id": run_id })
            );
        }

        let mut streamed_text = String::new();
        let mut cancel_sent = false;
        loop {
            let msg = if cancel_on_ctrlc && !cancel_sent {
                tokio::select! {
                    res = self.read_message() => res?,
                    sig = tokio::signal::ctrl_c() => {
                        if sig.is_ok() {
                            cancel_sent = true;
                            let _ = self.send_rpc("session.cancel", json!({ "session_id": session_id })).await;
                            if !json_mode {
                                eprintln!("\n[cancelling remote run...]");
                            }
                        }
                        continue;
                    }
                }
            } else {
                self.read_message().await?
            };

            if msg.get("method").and_then(Value::as_str) != Some("session.event") {
                continue;
            }
            let params = &msg["params"];
            if params["session_id"].as_str() != Some(session_id) {
                continue;
            }
            let payload = &params["payload"];
            if json_mode {
                println!("{payload}");
            }
            match payload["event"].as_str() {
                Some("chunk") => {
                    if let Some(text) = payload["text"].as_str() {
                        streamed_text.push_str(text);
                        if !json_mode {
                            print!("{text}");
                            let _ = std::io::stdout().flush();
                        }
                    }
                }
                Some("tool_start") if !json_mode => {
                    let name = payload["name"].as_str().unwrap_or("tool");
                    eprintln!("\n[tool: {name}]");
                }
                Some("done") => {
                    let ok = payload["ok"].as_bool().unwrap_or(false);
                    let turns = payload["turns"].as_u64().unwrap_or(0);
                    let answer = payload["answer"]
                        .as_str()
                        .map(str::to_string)
                        .unwrap_or_else(|| streamed_text.clone());
                    let error = payload["error"].as_str().map(str::to_string);
                    if !json_mode {
                        if ok && streamed_text.is_empty() && !answer.is_empty() {
                            println!("{answer}");
                        } else if !streamed_text.is_empty() && !streamed_text.ends_with('\n') {
                            println!();
                        }
                    }
                    return Ok(TurnOutcome {
                        ok,
                        answer,
                        error,
                        turns,
                    });
                }
                _ => {}
            }
        }
    }
}

pub(crate) async fn run_attach(opts: &AttachOptions) -> Result<()> {
    let mut client = DaemonClient::connect_and_auth(&opts.addr, &opts.token).await?;
    let sid = client
        .ensure_session(
            opts.session_id.as_deref(),
            opts.workspace.as_ref(),
            opts.mode.as_deref(),
        )
        .await?;

    if let Some(goal) = &opts.goal {
        let outcome = client.run_turn(&sid, goal, opts.json, true).await?;
        if !outcome.ok {
            bail!(
                "{}",
                outcome
                    .error
                    .unwrap_or_else(|| "Remote daemon run failed".into())
            );
        }
        return Ok(());
    }

    if opts.json {
        println!(
            "{}",
            json!({ "event": "attached", "addr": opts.addr, "session_id": sid })
        );
    } else {
        eprintln!("Attached to MEG daemon at {} (session {sid})", opts.addr);
        eprintln!("Commands: /mode <plan|build>, /sessions, /history, /quit");
    }

    let stdin = tokio::io::BufReader::new(tokio::io::stdin());
    let mut lines = stdin.lines();
    loop {
        if !opts.json {
            eprint!("meg[{sid}]> ");
            let _ = std::io::stderr().flush();
        }
        let Some(raw) = lines.next_line().await? else {
            break;
        };
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }
        if matches!(trimmed, "/quit" | "/exit") {
            break;
        }
        if let Some(mode_arg) = trimmed.strip_prefix("/mode ") {
            let res = client
                .call(
                    "session.set_mode",
                    json!({ "session_id": sid, "mode": mode_arg.trim() }),
                )
                .await;
            match res {
                Ok(v) => println!(
                    "{}",
                    if opts.json {
                        v.to_string()
                    } else {
                        format!("Mode set: {}", v["mode"].as_str().unwrap_or(mode_arg.trim()))
                    }
                ),
                Err(e) => eprintln!("Error: {e}"),
            }
            continue;
        }
        if trimmed == "/sessions" {
            let res = client.call("session.list", json!({})).await?;
            println!("{}", serde_json::to_string_pretty(&res)?);
            continue;
        }
        if trimmed == "/history" {
            let res = client
                .call("session.history", json!({ "session_id": sid }))
                .await?;
            println!("{}", serde_json::to_string_pretty(&res)?);
            continue;
        }
        let outcome = client.run_turn(&sid, trimmed, opts.json, true).await?;
        if !outcome.ok {
            if let Some(err) = outcome.error {
                eprintln!("Error: {err}");
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::serve::{generate_daemon_token, spawn_daemon, ServeConfig};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn attach_option_parser_rejects_remote_addr_by_default_and_parses_flags() {
        let args: Vec<String> = vec![
            "meg".into(),
            "attach".into(),
            "--addr".into(),
            "10.0.0.5:9000".into(),
            "--token".into(),
            "0123456789abcdef0123456789abcdef".into(),
            "hello".into(),
        ];
        assert!(
            parse_attach_options(&args).is_err(),
            "non-loopback attach address must fail closed by default"
        );

        let ok_args: Vec<String> = vec![
            "meg".into(),
            "attach".into(),
            "--addr".into(),
            "127.0.0.1:41234".into(),
            "--token".into(),
            "0123456789abcdef0123456789abcdef".into(),
            "--mode".into(),
            "plan".into(),
            "--json".into(),
            "inspect".into(),
            "repo".into(),
        ];
        let parsed = parse_attach_options(&ok_args).unwrap();
        assert_eq!(parsed.addr, "127.0.0.1:41234");
        assert_eq!(parsed.mode.as_deref(), Some("plan"));
        assert!(parsed.json);
        assert_eq!(parsed.goal.as_deref(), Some("inspect repo"));
    }

    #[tokio::test]
    async fn attach_client_round_trips_live_serve_daemon_and_reattaches_session() {
        let mock = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"data": [{"id": "fixture"}]})),
            )
            .mount(&mock)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "choices": [{"message": {"content": "{\"final\":\"Attached reply ok\"}"}}],
                "usage": {"prompt_tokens": 12, "completion_tokens": 4}
            })))
            .mount(&mock)
            .await;

        let ws = std::env::temp_dir().join(format!("meg-attach-test-{}", uuid::Uuid::new_v4()));
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
        let addr = handle.addr.to_string();

        assert!(
            DaemonClient::connect_and_auth(&addr, "wrong-token-value-12345")
                .await
                .is_err()
        );

        let mut client1 = DaemonClient::connect_and_auth(&addr, &handle.token)
            .await
            .unwrap();
        let sid = client1
            .ensure_session(None, Some(&ws), Some("plan"))
            .await
            .unwrap();
        let out = client1
            .run_turn(&sid, "Summarize workspace", false, false)
            .await
            .unwrap();
        assert!(out.ok, "expected ok outcome: {out:?}");
        assert!(out.answer.contains("Attached reply ok"), "answer={}", out.answer);
        assert!(out.turns >= 1);

        let mut client2 = DaemonClient::connect_and_auth(&addr, &handle.token)
            .await
            .unwrap();
        let hist = client2
            .call("session.history", json!({ "session_id": sid }))
            .await
            .unwrap();
        assert_eq!(hist["messages"].as_array().unwrap().len(), 2);

        handle.state.shutdown.notify_waiters();
        let _ = handle.task.await;
        let _ = std::fs::remove_dir_all(&ws);
    }
}
