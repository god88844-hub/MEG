//! Multi-step undo journal for mutations made through MEG's file tools.
//! Undoes only if the file still matches MEG's last recorded output; never
//! overwrites subsequent manual edits. Backward-compatible with legacy
//! single-entry `undo.json`.

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

const MAX_UNDO_DEPTH: usize = 20;

fn journal_path(workspace: &Path) -> PathBuf {
    workspace.join(".meg").join("tui").join("undo.json")
}

fn load_entries(journal: &Path) -> Result<Vec<Value>> {
    if !journal.exists() {
        return Ok(Vec::new());
    }
    let raw = std::fs::read(journal)?;
    let value: Value = serde_json::from_slice(&raw)?;
    if let Some(arr) = value.get("entries").and_then(Value::as_array) {
        return Ok(arr.clone());
    }
    // Legacy v1 single-entry object format.
    if value.is_object() && value.get("path").is_some() {
        return Ok(vec![value]);
    }
    bail!("Invalid undo journal")
}

fn save_entries_atomic(journal: &Path, entries: &[Value]) -> Result<()> {
    static TMP_SEQ: AtomicU64 = AtomicU64::new(0);
    if entries.is_empty() {
        if journal.exists() {
            std::fs::remove_file(journal)?;
        }
        return Ok(());
    }
    if let Some(parent) = journal.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let seq = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let tmp = journal.with_extension(format!("tmp.{}.{seq}", std::process::id()));
    let payload = serde_json::to_vec(&json!({ "version": 2, "entries": entries }))?;
    if let Err(err) = std::fs::write(&tmp, payload).and_then(|_| std::fs::rename(&tmp, journal)) {
        let _ = std::fs::remove_file(&tmp);
        return Err(err.into());
    }
    Ok(())
}

pub struct FileCheckpoint {
    path: PathBuf,
    before: Option<Vec<u8>>,
}

impl FileCheckpoint {
    pub fn capture(workspace: &Path, args: &Value) -> Result<Self> {
        let path =
            crate::fs_tools::resolve(workspace, args["path"].as_str().context("Missing path")?)?;
        let before = if path.exists() {
            Some(std::fs::read(&path)?)
        } else {
            None
        };
        Ok(Self { path, before })
    }

    pub fn commit(self, workspace: &Path) -> Result<()> {
        let after = std::fs::read(&self.path)?;
        let journal = journal_path(workspace);
        let mut entries = load_entries(&journal).unwrap_or_default();
        entries.push(json!({
            "path": self.path.to_string_lossy(),
            "before": self.before,
            "after": after,
        }));
        if entries.len() > MAX_UNDO_DEPTH {
            let drop_count = entries.len() - MAX_UNDO_DEPTH;
            entries.drain(..drop_count);
        }
        save_entries_atomic(&journal, &entries)
    }
}

pub fn depth(workspace: &Path) -> usize {
    load_entries(&journal_path(workspace))
        .map(|v| v.len())
        .unwrap_or(0)
}

pub fn undo(workspace: &Path) -> Result<String> {
    let journal = journal_path(workspace);
    if !journal.exists() {
        bail!("No recorded file mutation to undo");
    }
    let mut entries = load_entries(&journal)?;
    let Some(record) = entries.last().cloned() else {
        let _ = std::fs::remove_file(&journal);
        bail!("No recorded file mutation to undo");
    };
    let path = crate::fs_tools::resolve(
        workspace,
        record["path"].as_str().context("Invalid undo journal")?,
    )?;
    let after: Vec<u8> = serde_json::from_value(record["after"].clone())?;
    if std::fs::read(&path)? != after {
        bail!("File changed after MEG's edit; undo refused to preserve those changes");
    }
    let before: Option<Vec<u8>> = serde_json::from_value(record["before"].clone())?;
    if let Some(before) = before {
        std::fs::write(&path, before)?;
    } else {
        std::fs::remove_file(&path)?;
    }
    entries.pop();
    save_entries_atomic(&journal, &entries)?;
    Ok(format!(
        "Undid MEG's last file mutation: {} ({} remaining)",
        path.display(),
        entries.len()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tempdir() -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("meg-undo-test-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn undo_restores_recorded_edits_but_refuses_later_manual_changes() {
        let dir = tempdir();
        let path = dir.join("file.txt");
        std::fs::write(&path, "before").unwrap();
        let checkpoint = FileCheckpoint::capture(&dir, &json!({"path":"file.txt"})).unwrap();
        std::fs::write(&path, "after").unwrap();
        checkpoint.commit(&dir).unwrap();
        std::fs::write(&path, "manual edit").unwrap();
        assert!(undo(&dir).unwrap_err().to_string().contains("refused"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "manual edit");
        std::fs::write(&path, "after").unwrap();
        undo(&dir).unwrap();
        assert_eq!(std::fs::read_to_string(path).unwrap(), "before");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn multistep_undo_stack_rolls_back_multiple_edits_in_order() {
        let dir = tempdir();
        let a = dir.join("a.txt");
        let b = dir.join("b.txt");
        std::fs::write(&a, "v0").unwrap();

        let cp1 = FileCheckpoint::capture(&dir, &json!({"path": "a.txt"})).unwrap();
        std::fs::write(&a, "v1").unwrap();
        cp1.commit(&dir).unwrap();

        let cp2 = FileCheckpoint::capture(&dir, &json!({"path": "b.txt"})).unwrap();
        std::fs::write(&b, "created").unwrap();
        cp2.commit(&dir).unwrap();

        let cp3 = FileCheckpoint::capture(&dir, &json!({"path": "a.txt"})).unwrap();
        std::fs::write(&a, "v2").unwrap();
        cp3.commit(&dir).unwrap();

        assert_eq!(depth(&dir), 3);
        assert!(undo(&dir).unwrap().contains("2 remaining"));
        assert_eq!(std::fs::read_to_string(&a).unwrap(), "v1");

        assert!(undo(&dir).unwrap().contains("1 remaining"));
        assert!(!b.exists());

        assert!(undo(&dir).unwrap().contains("0 remaining"));
        assert_eq!(std::fs::read_to_string(&a).unwrap(), "v0");
        assert_eq!(depth(&dir), 0);
        assert!(undo(&dir).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn stack_caps_at_max_depth() {
        let dir = tempdir();
        let p = dir.join("seq.txt");
        std::fs::write(&p, "0").unwrap();
        for i in 1..=25 {
            let cp = FileCheckpoint::capture(&dir, &json!({"path": "seq.txt"})).unwrap();
            std::fs::write(&p, format!("{i}")).unwrap();
            cp.commit(&dir).unwrap();
        }
        assert_eq!(depth(&dir), MAX_UNDO_DEPTH);
        let _ = std::fs::remove_dir_all(&dir);
    }
}