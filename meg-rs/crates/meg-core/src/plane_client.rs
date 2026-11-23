//! Multiplexed JSON-RPC seam to the TypeScript control plane. Owns its process
//! tree; request payloads travel over pipes, never shell command strings.
use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::io::Read;
use std::{
    collections::HashMap,
    io::{BufRead, BufReader, Write},
    path::Path,
    process::{Child, ChildStdin, Command, Stdio},
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc, Arc, Mutex,
    },
    time::{Duration, Instant},
};
enum Incoming {
    Event(Value),
    Result(std::result::Result<Value, String>),
}
pub struct PlaneClient {
    stdin: Mutex<ChildStdin>,
    child: Mutex<Child>,
    pending: Arc<Mutex<HashMap<u64, mpsc::Sender<Incoming>>>>,
    sequence: AtomicU64,
    reader: Mutex<Option<std::thread::JoinHandle<()>>>,
    #[cfg(windows)]
    job: Option<std::os::windows::io::OwnedHandle>,
}
impl PlaneClient {
    pub fn spawn(entry: &Path, native: &Path, builtin_skills: Option<&Path>) -> Result<Arc<Self>> {
        anyhow::ensure!(entry.is_file(), "MEG control-plane bundle is missing");
        let mut command =
            Command::new(std::env::var_os("MEG_BUN_BIN").unwrap_or_else(|| "bun".into()));
        command
            .arg("run")
            .arg(entry)
            .env("MEG_NATIVE_EXE", native)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        if let Some(path) = builtin_skills {
            command.env("MEG_BUILTIN_SKILLS", path);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000200);
        }
        let mut child = command
            .spawn()
            .context("Could not start the MEG control plane; Bun must be installed and on PATH")?;
        #[cfg(windows)]
        let job = match own_process_tree(&child) {
            Ok(job) => Some(job),
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
        };
        let stdin = child
            .stdin
            .take()
            .context("Control-plane stdin is unavailable")?;
        let stdout = child
            .stdout
            .take()
            .context("Control-plane stdout is unavailable")?;
        let pending: Arc<Mutex<HashMap<u64, mpsc::Sender<Incoming>>>> = Default::default();
        let responses = pending.clone();
        let reader = std::thread::spawn(move || {
            let mut input = BufReader::new(stdout);
            loop {
                let mut line = Vec::new();
                match input
                    .by_ref()
                    .take(4 * 1024 * 1024 + 1)
                    .read_until(b'\n', &mut line)
                {
                    Ok(0) | Err(_) => break,
                    _ => {}
                }
                if line.len() > 4 * 1024 * 1024 {
                    break;
                }
                let Ok(value) = serde_json::from_slice::<Value>(&line) else {
                    continue;
                };
                if let Some(id) = value["id"].as_u64() {
                    if let Some(tx) = responses.lock().unwrap().remove(&id) {
                        let result = if value["error"].is_object() {
                            Err(value["error"]["message"]
                                .as_str()
                                .unwrap_or("Control-plane request failed")
                                .to_string())
                        } else {
                            Ok(value["result"].clone())
                        };
                        let _ = tx.send(Incoming::Result(result));
                    }
                } else if value["method"].as_str() == Some("agent.event") {
                    if let Some(id) = value["params"]["requestId"].as_u64() {
                        if let Some(tx) = responses.lock().unwrap().get(&id) {
                            let _ = tx.send(Incoming::Event(value["params"].clone()));
                        }
                    }
                }
            }
            for (_, tx) in responses.lock().unwrap().drain() {
                let _ = tx.send(Incoming::Result(Err(
                    "MEG control plane exited; restart MEG or run /doctor".into(),
                )));
            }
        });
        Ok(Arc::new(Self {
            stdin: Mutex::new(stdin),
            child: Mutex::new(child),
            pending,
            sequence: AtomicU64::new(1),
            reader: Mutex::new(Some(reader)),
            #[cfg(windows)]
            job,
        }))
    }
    fn send(&self, value: &Value) -> Result<()> {
        let mut bytes = serde_json::to_vec(value)?;
        anyhow::ensure!(
            bytes.len() <= 4 * 1024 * 1024,
            "Control-plane request is too large"
        );
        bytes.push(b'\n');
        let mut stdin = self.stdin.lock().unwrap();
        stdin.write_all(&bytes)?;
        stdin.flush()?;
        Ok(())
    }
    pub fn call(
        &self,
        method: &str,
        params: Value,
        cancel: &crate::CancellationToken,
    ) -> Result<Value> {
        self.call_with_events(method, params, cancel, |_| {})
    }
    pub fn call_with_events(
        &self,
        method: &str,
        params: Value,
        cancel: &crate::CancellationToken,
        mut event: impl FnMut(Value),
    ) -> Result<Value> {
        let id = self.sequence.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = mpsc::channel();
        self.pending.lock().unwrap().insert(id, tx);
        if let Err(error) =
            self.send(&json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}))
        {
            self.pending.lock().unwrap().remove(&id);
            return Err(error);
        }
        let deadline = Instant::now() + Duration::from_secs(300);
        loop {
            if cancel.is_cancelled() || Instant::now() >= deadline {
                let _ = self.send(
                    &json!({"jsonrpc":"2.0","method":"runtime.cancel","params":{"requestId":id}}),
                );
                let grace = Instant::now() + Duration::from_millis(250);
                while Instant::now() < grace {
                    match rx.recv_timeout(Duration::from_millis(10)) {
                        Ok(Incoming::Event(value)) => event(value),
                        Ok(Incoming::Result(_)) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                            break
                        }
                        Err(_) => {}
                    }
                }
                self.pending.lock().unwrap().remove(&id);
                anyhow::bail!(if cancel.is_cancelled() {
                    "Operation cancelled"
                } else {
                    "Control-plane operation timed out"
                });
            }
            match rx.recv_timeout(Duration::from_millis(25)) {
                Ok(Incoming::Event(value)) => event(value),
                Ok(Incoming::Result(result)) => return result.map_err(anyhow::Error::msg),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(_) => {
                    self.pending.lock().unwrap().remove(&id);
                    anyhow::bail!("Control plane disconnected");
                }
            }
        }
    }
}
impl Drop for PlaneClient {
    fn drop(&mut self) {
        #[cfg(windows)]
        drop(self.job.take());
        if let Ok(child) = self.child.get_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
        if let Ok(reader) = self.reader.get_mut() {
            if let Some(reader) = reader.take() {
                let _ = reader.join();
            }
        }
    }
}

#[cfg(windows)]
fn own_process_tree(child: &Child) -> Result<std::os::windows::io::OwnedHandle> {
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use windows_sys::Win32::System::JobObjects::*;
    unsafe {
        let handle = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        anyhow::ensure!(
            !handle.is_null(),
            "Could not own the control-plane process tree"
        );
        let owned = OwnedHandle::from_raw_handle(handle);
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        anyhow::ensure!(
            SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                &info as *const _ as _,
                std::mem::size_of_val(&info) as u32
            ) != 0
                && AssignProcessToJobObject(handle, child.as_raw_handle() as _) != 0,
            "Could not isolate the control-plane process tree"
        );
        Ok(owned)
    }
}
