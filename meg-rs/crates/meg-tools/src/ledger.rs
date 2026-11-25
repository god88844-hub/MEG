//! Append-only event ledger (`session.v1.jsonl`) logging failed generations
//! off-history as `assistant/attempt`, OpenTelemetry (`OTel`) span exporter,
//! and tree-hash bound `.meg/receipt.json` generator & verifier.

use anyhow::{bail, Result};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::deliverable_guard::sha256_hex;

#[derive(Debug, Clone)]
pub struct LedgerEntry {
    pub seq: u64,
    pub kind: String,
    pub payload: Value,
}

pub struct EventLedger {
    file: PathBuf,
    next_seq: u64,
}

impl EventLedger {
    pub fn open(workspace: &Path, session_id: &str) -> Result<Self> {
        let safe: String = session_id
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
            .collect();
        let dir = workspace.join(".meg").join("ledger");
        std::fs::create_dir_all(&dir)?;
        let file = dir.join(format!("{safe}.v1.jsonl"));
        let next_seq = if file.exists() {
            std::fs::read_to_string(&file)?.lines().count() as u64 + 1
        } else {
            1
        };
        Ok(Self { file, next_seq })
    }

    pub fn append(&mut self, kind: &str, payload: Value) -> Result<u64> {
        let seq = self.next_seq;
        self.next_seq += 1;
        let record = json!({
            "seq": seq,
            "kind": kind,
            "payload": payload,
        });
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.file)?;
        writeln!(f, "{}", record)?;
        Ok(seq)
    }

    /// Log a failed generation off-history as `assistant/attempt`.
    pub fn append_failed_attempt(&mut self, raw: &str, reason: &str) -> Result<u64> {
        self.append(
            "assistant/attempt",
            json!({ "raw": raw, "reason": reason, "off_history": true }),
        )
    }

    /// Read all ledger entries; set `include_failed_attempts = false` to keep
    /// `assistant/attempt` records out of active prompt history.
    pub fn read_entries(&self, include_failed_attempts: bool) -> Result<Vec<LedgerEntry>> {
        if !self.file.exists() {
            return Ok(Vec::new());
        }
        let mut out = Vec::new();
        for line in std::fs::read_to_string(&self.file)?.lines() {
            if line.trim().is_empty() {
                continue;
            }
            let v: Value = serde_json::from_str(line)?;
            let kind = v["kind"].as_str().unwrap_or("").to_string();
            if !include_failed_attempts && kind == "assistant/attempt" {
                continue;
            }
            out.push(LedgerEntry {
                seq: v["seq"].as_u64().unwrap_or(0),
                kind,
                payload: v["payload"].clone(),
            });
        }
        Ok(out)
    }
}

/// Minimal OpenTelemetry (`OTel`) span record for offline trace export.
#[derive(Debug, Clone)]
pub struct OtelSpan {
    pub trace_id: String,
    pub span_id: String,
    pub name: String,
    pub start_ms: u64,
    pub end_ms: u64,
    pub status_ok: bool,
    pub attributes: BTreeMap<String, String>,
}

pub fn export_otel_spans(spans: &[OtelSpan]) -> Value {
    let items: Vec<Value> = spans
        .iter()
        .map(|s| {
            json!({
                "traceId": s.trace_id,
                "spanId": s.span_id,
                "name": s.name,
                "startTimeUnixNano": (s.start_ms as u128) * 1_000_000,
                "endTimeUnixNano": (s.end_ms as u128) * 1_000_000,
                "status": { "code": if s.status_ok { 1 } else { 2 } },
                "attributes": s.attributes,
            })
        })
        .collect();
    json!({
        "resourceSpans": [{
            "resource": { "attributes": [{ "key": "service.name", "value": { "stringValue": "meg-rs" } }] },
            "scopeSpans": [{ "spans": items }]
        }]
    })
}

/// Compute a deterministic SHA-256 tree hash across tracked workspace files (excluding `.git`, `.meg`, `target`, `node_modules`).
pub fn compute_workspace_tree_hash(workspace: &Path) -> Result<String> {
    let mut entries: Vec<(String, String)> = Vec::new();
    for entry in walkdir::WalkDir::new(workspace)
        .sort_by_file_name()
        .into_iter()
        .filter_entry(|e| !crate::fs_tools::is_ignored(e))
        .filter_map(|e| e.ok())
    {
        if entry.file_type().is_file() {
            let rel = entry
                .path()
                .strip_prefix(workspace)
                .unwrap_or(entry.path())
                .to_string_lossy()
                .replace('\\', "/");
            let bytes = std::fs::read(entry.path())?;
            entries.push((rel, sha256_hex(&bytes)));
        }
    }
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    let mut material = String::new();
    for (rel, h) in entries {
        material.push_str(&format!("{rel}:{h}\n"));
    }
    Ok(sha256_hex(material.as_bytes()))
}

/// Write a tree-hash bound execution receipt to `<workspace>/.meg/receipt.json`.
pub fn write_receipt(workspace: &Path, task_id: &str, checks_passed: bool) -> Result<Value> {
    let tree_hash = compute_workspace_tree_hash(workspace)?;
    let sig_input = format!("v1|{task_id}|{tree_hash}|{checks_passed}");
    let receipt_hash = sha256_hex(sig_input.as_bytes());
    let receipt = json!({
        "version": 1,
        "task_id": task_id,
        "tree_hash": tree_hash,
        "checks_passed": checks_passed,
        "receipt_hash": receipt_hash,
    });
    let dir = workspace.join(".meg");
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join("receipt.json"), serde_json::to_vec_pretty(&receipt)?)?;
    Ok(receipt)
}

/// Verify `<workspace>/.meg/receipt.json` against the live workspace tree hash and signature.
pub fn verify_receipt(workspace: &Path) -> Result<String> {
    let file = workspace.join(".meg").join("receipt.json");
    if !file.exists() {
        bail!("receipt missing: .meg/receipt.json");
    }
    let v: Value = serde_json::from_slice(&std::fs::read(&file)?)?;
    let task_id = v["task_id"].as_str().unwrap_or("");
    let recorded_tree = v["tree_hash"].as_str().unwrap_or("");
    let checks_passed = v["checks_passed"].as_bool().unwrap_or(false);
    let recorded_sig = v["receipt_hash"].as_str().unwrap_or("");

    let expected_sig = sha256_hex(format!("v1|{task_id}|{recorded_tree}|{checks_passed}").as_bytes());
    if recorded_sig != expected_sig {
        bail!("receipt signature mismatch (tampered receipt.json)");
    }
    if !checks_passed {
        bail!("receipt records checks_passed = false");
    }
    let live_tree = compute_workspace_tree_hash(workspace)?;
    if live_tree != recorded_tree {
        bail!("workspace tree hash drifted since receipt was signed");
    }
    Ok(format!("verified:{task_id}:{live_tree}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn tmpdir() -> PathBuf {
        static N: AtomicU64 = AtomicU64::new(0);
        let d = std::env::temp_dir().join(format!(
            "meg-ledger-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn ledger_keeps_failed_attempts_off_history_and_receipt_detects_drift() {
        let ws = tmpdir();
        std::fs::write(ws.join("main.rs"), "fn main() {}\n").unwrap();

        let mut ledger = EventLedger::open(&ws, "s-01").unwrap();
        ledger.append("user", json!({"text": "build"})).unwrap();
        ledger.append_failed_attempt("{broken", "json parse error").unwrap();
        ledger.append("assistant", json!({"text": "done"})).unwrap();

        assert_eq!(ledger.read_entries(false).unwrap().len(), 2);
        assert_eq!(ledger.read_entries(true).unwrap().len(), 3);

        let otel = export_otel_spans(&[OtelSpan {
            trace_id: "t1".into(),
            span_id: "s1".into(),
            name: "tool.edit_file".into(),
            start_ms: 100,
            end_ms: 112,
            status_ok: true,
            attributes: BTreeMap::new(),
        }]);
        assert_eq!(otel["resourceSpans"][0]["scopeSpans"][0]["spans"][0]["name"], "tool.edit_file");

        write_receipt(&ws, "P1-12", true).unwrap();
        assert!(verify_receipt(&ws).unwrap().starts_with("verified:P1-12:"));

        // Mutating workspace after receipt invalidates verification.
        std::fs::write(ws.join("main.rs"), "fn main() { panic!(); }\n").unwrap();
        assert!(verify_receipt(&ws).unwrap_err().to_string().contains("drifted"));
        let _ = std::fs::remove_dir_all(&ws);
    }

    #[test]
    fn adversarial_loop_state_does_not_invalidate_receipt() {
        let dir = std::env::temp_dir().join(format!(
            "meg-ledger-loop-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(dir.join(".loop")).unwrap();
        std::fs::write(dir.join("main.rs"), "fn main() {}\n").unwrap();
        write_receipt(&dir, "meg-loop", true).unwrap();
        // Updating .loop/state.json after receipt write must not cause drift
        std::fs::write(dir.join(".loop").join("state.json"), r#"{"status":"green"}"#).unwrap();
        verify_receipt(&dir).expect("Receipt must remain valid after .loop/state.json update");
        let _ = std::fs::remove_dir_all(&dir);
    }
}