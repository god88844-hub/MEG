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

/// Provenance metadata recorded alongside workspace tree hashes in `.meg/receipt.json`.
#[derive(Debug, Clone, Default)]
pub struct ReceiptMeta {
    pub verify_cmd: Option<String>,
    pub reviewer_verdict: Option<String>,
    pub edited_count: usize,
}

fn hex_to_bytes_32(hex: &str) -> [u8; 32] {
    let mut out = [0u8; 32];
    let bytes = hex.as_bytes();
    for i in 0..32 {
        let hi = (bytes[i * 2] as char).to_digit(16).unwrap_or(0) as u8;
        let lo = (bytes[i * 2 + 1] as char).to_digit(16).unwrap_or(0) as u8;
        out[i] = (hi << 4) | lo;
    }
    out
}

/// Compute RFC 2104 HMAC-SHA256 hex digest using the workspace SHA-256 primitive.
pub fn hmac_sha256_hex(key: &[u8], msg: &[u8]) -> String {
    let mut k_block = [0u8; 64];
    if key.len() > 64 {
        let hashed = hex_to_bytes_32(&sha256_hex(key));
        k_block[..32].copy_from_slice(&hashed);
    } else {
        k_block[..key.len()].copy_from_slice(key);
    }

    let mut ipad = Vec::with_capacity(64 + msg.len());
    let mut opad = Vec::with_capacity(64 + 32);
    for b in k_block {
        ipad.push(b ^ 0x36);
        opad.push(b ^ 0x5c);
    }
    ipad.extend_from_slice(msg);
    let inner = hex_to_bytes_32(&sha256_hex(&ipad));
    opad.extend_from_slice(&inner);
    sha256_hex(&opad)
}

fn ct_eq_str(a: &str, b: &str) -> bool {
    let ab = a.as_bytes();
    let bb = b.as_bytes();
    if ab.len() != bb.len() {
        return false;
    }
    let mut diff = 0u8;
    for (&x, &y) in ab.iter().zip(bb.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Read an optional receipt HMAC signing key from `MEG_RECEIPT_KEY`.
pub fn receipt_key_from_env() -> Option<Vec<u8>> {
    std::env::var("MEG_RECEIPT_KEY")
        .ok()
        .filter(|s| !s.is_empty())
        .map(|s| s.into_bytes())
}

fn provenance_digest(
    task_id: &str,
    tree_hash: &str,
    checks_passed: bool,
    verify_cmd: &str,
    reviewer_verdict: &str,
    edited_count: usize,
) -> String {
    let material = format!(
        "v2|{task_id}|{tree_hash}|{checks_passed}|{verify_cmd}|{reviewer_verdict}|{edited_count}"
    );
    sha256_hex(material.as_bytes())
}

/// Write a tree-hash bound execution receipt with provenance metadata and optional HMAC-SHA256 attestation.
pub fn write_receipt_with_options(
    workspace: &Path,
    task_id: &str,
    checks_passed: bool,
    meta: &ReceiptMeta,
    hmac_key: Option<&[u8]>,
) -> Result<Value> {
    let tree_hash = compute_workspace_tree_hash(workspace)?;
    let sig_input = format!("v1|{task_id}|{tree_hash}|{checks_passed}");
    let receipt_hash = sha256_hex(sig_input.as_bytes());
    let verify_cmd_str = meta.verify_cmd.as_deref().unwrap_or("");
    let reviewer_str = meta.reviewer_verdict.as_deref().unwrap_or("");
    let prov_hash = provenance_digest(
        task_id,
        &tree_hash,
        checks_passed,
        verify_cmd_str,
        reviewer_str,
        meta.edited_count,
    );
    let (integrity_mode, hmac_val) = match hmac_key.filter(|k| !k.is_empty()) {
        Some(key) => {
            let mac_input = format!("hmac-v1|{receipt_hash}|{prov_hash}");
            ("hmac-sha256", Some(hmac_sha256_hex(key, mac_input.as_bytes())))
        }
        None => ("unkeyed-sha256", None),
    };
    let mut receipt = json!({
        "version": 1,
        "task_id": task_id,
        "tree_hash": tree_hash,
        "checks_passed": checks_passed,
        "receipt_hash": receipt_hash,
        "provenance_hash": prov_hash,
        "provenance": {
            "verify_cmd": meta.verify_cmd,
            "reviewer_verdict": meta.reviewer_verdict,
            "edited_count": meta.edited_count,
            "integrity_mode": integrity_mode,
        },
    });
    if let Some(mac) = hmac_val {
        receipt["hmac_sha256"] = Value::String(mac);
    }
    let dir = workspace.join(".meg");
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join("receipt.json"), serde_json::to_vec_pretty(&receipt)?)?;
    Ok(receipt)
}

/// Write a tree-hash bound execution receipt to `<workspace>/.meg/receipt.json`.
pub fn write_receipt(workspace: &Path, task_id: &str, checks_passed: bool) -> Result<Value> {
    let key = receipt_key_from_env();
    write_receipt_with_options(
        workspace,
        task_id,
        checks_passed,
        &ReceiptMeta::default(),
        key.as_deref(),
    )
}

/// Verify `<workspace>/.meg/receipt.json` against the live workspace tree hash, provenance, and optional HMAC key.
pub fn verify_receipt_with_key(workspace: &Path, hmac_key: Option<&[u8]>) -> Result<String> {
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
    if !ct_eq_str(recorded_sig, &expected_sig) {
        bail!("receipt signature mismatch (tampered receipt.json)");
    }
    let recorded_prov_hash = v["provenance_hash"].as_str();
    if let Some(prov_sig) = recorded_prov_hash {
        let verify_cmd_str = v["provenance"]["verify_cmd"].as_str().unwrap_or("");
        let reviewer_str = v["provenance"]["reviewer_verdict"].as_str().unwrap_or("");
        let edited_count = v["provenance"]["edited_count"].as_u64().unwrap_or(0) as usize;
        let expected_prov = provenance_digest(
            task_id,
            recorded_tree,
            checks_passed,
            verify_cmd_str,
            reviewer_str,
            edited_count,
        );
        if !ct_eq_str(prov_sig, &expected_prov) {
            bail!("receipt provenance hash mismatch (tampered provenance metadata)");
        }
    }
    let recorded_mac = v["hmac_sha256"].as_str();
    match (hmac_key.filter(|k| !k.is_empty()), recorded_mac) {
        (Some(key), Some(mac)) => {
            let prov_part = recorded_prov_hash.unwrap_or("");
            let mac_input = format!("hmac-v1|{recorded_sig}|{prov_part}");
            let expected_mac = hmac_sha256_hex(key, mac_input.as_bytes());
            if !ct_eq_str(mac, &expected_mac) {
                bail!("receipt HMAC-SHA256 mismatch (invalid key or tampered receipt)");
            }
        }
        (Some(_), None) => {
            bail!("receipt is unkeyed but MEG_RECEIPT_KEY requires HMAC-SHA256 attestation");
        }
        (None, Some(_)) => {
            bail!("receipt carries HMAC-SHA256 attestation; supply MEG_RECEIPT_KEY to verify");
        }
        (None, None) => {}
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

/// Verify `<workspace>/.meg/receipt.json` against the live workspace tree hash and signature.
pub fn verify_receipt(workspace: &Path) -> Result<String> {
    let key = receipt_key_from_env();
    verify_receipt_with_key(workspace, key.as_deref())
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

    #[test]
    fn hmac_sha256_matches_rfc4231_and_keyed_receipt_blocks_tampering_and_downgrade() {
        // RFC 4231 Test Case 1 & Test Case 2
        assert_eq!(
            hmac_sha256_hex(&[0x0bu8; 20], b"Hi There"),
            "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
        );
        assert_eq!(
            hmac_sha256_hex(b"Jefe", b"what do ya want for nothing?"),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );

        let ws = tmpdir();
        std::fs::write(ws.join("app.rs"), "pub fn ok() -> bool { true }\n").unwrap();
        let key = b"super-secret-ci-attestation-key";
        let meta = ReceiptMeta {
            verify_cmd: Some("cargo test --locked".into()),
            reviewer_verdict: Some("PASS".into()),
            edited_count: 1,
        };
        let r = write_receipt_with_options(&ws, "phase6a", true, &meta, Some(key)).unwrap();
        assert_eq!(r["provenance"]["integrity_mode"], "hmac-sha256");
        assert!(verify_receipt_with_key(&ws, Some(key)).is_ok());

        // Wrong key must fail
        assert!(verify_receipt_with_key(&ws, Some(b"wrong-key")).is_err());
        // Missing key (downgrade attempt) must fail closed
        assert!(verify_receipt_with_key(&ws, None).is_err());

        // Tampering provenance metadata without recomputing HMAC must fail
        let receipt_file = ws.join(".meg").join("receipt.json");
        let mut tampered: Value =
            serde_json::from_slice(&std::fs::read(&receipt_file).unwrap()).unwrap();
        tampered["provenance"]["verify_cmd"] = Value::String("echo fake".into());
        std::fs::write(&receipt_file, serde_json::to_vec_pretty(&tampered).unwrap()).unwrap();
        assert!(verify_receipt_with_key(&ws, Some(key)).is_err());

        // Unkeyed receipt must be rejected when verification requires a key
        write_receipt_with_options(&ws, "phase6a-unkeyed", true, &meta, None).unwrap();
        assert!(verify_receipt_with_key(&ws, None).is_ok());
        assert!(verify_receipt_with_key(&ws, Some(key)).is_err());

        let _ = std::fs::remove_dir_all(&ws);
    }
}
