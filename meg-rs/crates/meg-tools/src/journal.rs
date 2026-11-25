//! Multi-step undo journal, out-of-tree `side-git` bare shadow snapshots
//! (`~/.meg/snapshots/<repo-hash>/`), and local `Keep Rate` tracker.
//! Never pollutes the workspace's own `.git` directory.

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

const MAX_UNDO_DEPTH: usize = 20;

fn journal_path(workspace: &Path) -> PathBuf {
    workspace.join(".meg").join("tui").join("undo.json")
}

fn keep_rate_path(workspace: &Path) -> PathBuf {
    workspace.join(".meg").join("tui").join("keep_rate.jsonl")
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

    /// Commit the undo entry; if saving the journal fails, restore the file to its
    /// pre-edit state so a mutation never lands without a matching undo record.
    pub fn commit_or_rollback(self, workspace: &Path) -> Result<()> {
        let path = self.path.clone();
        let before = self.before.clone();
        if let Err(err) = self.commit(workspace) {
            match before {
                Some(bytes) => {
                    let _ = std::fs::write(&path, bytes);
                }
                None => {
                    let _ = std::fs::remove_file(&path);
                }
            }
            return Err(err);
        }
        Ok(())
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
        let _ = record_keep_rate_event(workspace, &path, KeepOutcome::Overwritten);
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
    let _ = record_keep_rate_event(workspace, &path, KeepOutcome::Reverted);
    Ok(format!(
        "Undid MEG's last file mutation: {} ({} remaining)",
        path.display(),
        entries.len()
    ))
}

// ---------------------------------------------------------------------------
// Out-of-tree `side-git` bare shadow snapshots (~/.meg/snapshots/<repo-hash>/)
// ---------------------------------------------------------------------------

fn fnv1a_hex(s: &str) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for &b in s.as_bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")
}

/// Resolve the out-of-tree bare shadow git directory for `workspace`.
pub fn side_git_dir(workspace: &Path) -> PathBuf {
    let base = std::env::var_os("MEG_SNAPSHOTS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let home = std::env::var_os("USERPROFILE")
                .or_else(|| std::env::var_os("HOME"))
                .map(PathBuf::from)
                .unwrap_or_else(std::env::temp_dir);
            home.join(".meg").join("snapshots")
        });
    let canon = workspace
        .canonicalize()
        .unwrap_or_else(|_| workspace.to_path_buf());
    base.join(fnv1a_hex(&canon.to_string_lossy()))
}

fn run_side_git(git_dir: &Path, workspace: &Path, args: &[&str]) -> Result<String> {
    let out = Command::new("git")
        .env("GIT_DIR", git_dir)
        .env("GIT_WORK_TREE", workspace)
        .env("GIT_TERMINAL_PROMPT", "0")
        .args(["-c", "core.autocrlf=false"])
        .args(args)
        .output()
        .context("failed to execute git for side-git snapshot")?;
    if !out.status.success() {
        bail!(
            "side-git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Capture an out-of-tree pre-edit snapshot of `workspace` in `~/.meg/snapshots/<repo-hash>/`.
/// Returns the snapshot tree/commit SHA without touching `<workspace>/.git`.
pub fn side_git_snapshot(workspace: &Path, label: &str) -> Result<String> {
    let git_dir = side_git_dir(workspace);
    if !git_dir.join("HEAD").exists() {
        std::fs::create_dir_all(&git_dir)?;
        let out = Command::new("git")
            .arg("init")
            .arg("--bare")
            .arg(&git_dir)
            .output()
            .context("failed to init bare side-git repo")?;
        if !out.status.success() {
            bail!(
                "side-git init failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
    }
    // Stage workspace files excluding .git and .meg.
    run_side_git(
        &git_dir,
        workspace,
        &["add", "-A", "--", ".", ":!.git", ":!.meg", ":!.loop", ":!target", ":!node_modules", ":!.env", ":!.env.*"],
    )?;
    let tree = run_side_git(&git_dir, workspace, &["write-tree"])?;
    let parent = run_side_git(&git_dir, workspace, &["rev-parse", "--verify", "HEAD"]).ok();
    if let Some(ref p) = parent {
        if let Ok(prev_tree) = run_side_git(&git_dir, workspace, &["rev-parse", "HEAD^{tree}"]) {
            if prev_tree == tree {
                return Ok(p.clone());
            }
        }
    }
    let commit_count: usize = run_side_git(&git_dir, workspace, &["rev-list", "--count", "HEAD"])
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let parent = if commit_count >= 20 { None } else { parent };
    let mut cmd = Command::new("git");
    cmd.env("GIT_DIR", &git_dir)
        .env("GIT_WORK_TREE", workspace)
        .env("GIT_AUTHOR_NAME", "MEG")
        .env("GIT_AUTHOR_EMAIL", "meg@local")
        .env("GIT_COMMITTER_NAME", "MEG")
        .env("GIT_COMMITTER_EMAIL", "meg@local")
        .args(["commit-tree", &tree, "-m", label]);
    if let Some(p) = &parent {
        cmd.args(["-p", p]);
    }
    let out = cmd.output()?;
    if !out.status.success() {
        bail!(
            "side-git commit-tree failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    let commit = String::from_utf8_lossy(&out.stdout).trim().to_string();
    run_side_git(&git_dir, workspace, &["update-ref", "HEAD", &commit])?;
    Ok(commit)
}

/// Restore `workspace` files from a `side-git` snapshot SHA (or `"HEAD"`).
pub fn side_git_restore(workspace: &Path, rev: &str) -> Result<()> {
    let git_dir = side_git_dir(workspace);
    if !git_dir.join("HEAD").exists() {
        bail!("no side-git snapshots exist for workspace");
    }
    run_side_git(&git_dir, workspace, &["checkout", rev, "--", "."])?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Local Keep-Rate Tracker
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeepOutcome {
    Kept,
    Reverted,
    Overwritten,
}

impl KeepOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Kept => "kept",
            Self::Reverted => "reverted",
            Self::Overwritten => "overwritten",
        }
    }
}

#[derive(Debug, Clone)]
pub struct KeepRateSummary {
    pub kept: usize,
    pub reverted: usize,
    pub overwritten: usize,
    pub keep_rate: f64,
}

pub fn record_keep_rate_event(workspace: &Path, path: &Path, outcome: KeepOutcome) -> Result<()> {
    let file = keep_rate_path(workspace);
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let line = json!({
        "path": path.to_string_lossy(),
        "outcome": outcome.as_str(),
    })
    .to_string();
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&file)?;
    writeln!(f, "{line}")?;
    Ok(())
}

pub fn keep_rate_summary(workspace: &Path) -> KeepRateSummary {
    let file = keep_rate_path(workspace);
    let Ok(raw) = std::fs::read_to_string(&file) else {
        return KeepRateSummary {
            kept: 0,
            reverted: 0,
            overwritten: 0,
            keep_rate: 1.0,
        };
    };
    let (mut kept, mut reverted, mut overwritten) = (0usize, 0usize, 0usize);
    for line in raw.lines() {
        if let Ok(v) = serde_json::from_str::<Value>(line) {
            match v["outcome"].as_str() {
                Some("kept") => kept += 1,
                Some("reverted") => reverted += 1,
                Some("overwritten") => overwritten += 1,
                _ => {}
            }
        }
    }
    let total = kept + reverted + overwritten;
    let keep_rate = if total == 0 {
        1.0
    } else {
        kept as f64 / total as f64
    };
    KeepRateSummary {
        kept,
        reverted,
        overwritten,
        keep_rate,
    }
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
        let summary = keep_rate_summary(&dir);
        assert_eq!(summary.overwritten, 1);
        assert_eq!(summary.reverted, 1);
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

    #[test]
    fn side_git_snapshot_and_restore_without_polluting_workspace_git() {
        let dir = tempdir();
        let snap_root = tempdir();
        std::env::set_var("MEG_SNAPSHOTS_DIR", &snap_root);
        let file = dir.join("app.rs");
        std::fs::write(&file, "fn v1() {}\n").unwrap();

        let sha1 = side_git_snapshot(&dir, "pre-edit").unwrap();
        assert!(!dir.join(".git").exists(), "workspace .git must not be created");
        std::fs::write(&file, "fn v2_broken() {}\n").unwrap();

        side_git_restore(&dir, &sha1).unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "fn v1() {}\n");

        record_keep_rate_event(&dir, &file, KeepOutcome::Kept).unwrap();
        record_keep_rate_event(&dir, &file, KeepOutcome::Reverted).unwrap();
        let summary = keep_rate_summary(&dir);
        assert_eq!(summary.kept, 1);
        assert_eq!(summary.reverted, 1);
        assert!((summary.keep_rate - 0.5).abs() < 1e-6);

        std::env::remove_var("MEG_SNAPSHOTS_DIR");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&snap_root);
    }

    #[test]
    fn adversarial_journal_failure_rolls_back_file_edit() {
        let dir = std::env::temp_dir().join(format!("meg-journal-rb-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("code.rs");
        std::fs::write(&file, "fn original() {}\n").unwrap();
        let cp = FileCheckpoint::capture(&dir, &json!({"path": "code.rs"})).unwrap();
        // Mutate the file, then sabotage the journal path with a directory so save fails
        std::fs::write(&file, "fn mutated() {}\n").unwrap();
        std::fs::create_dir_all(journal_path(&dir)).unwrap();
        let err = cp.commit_or_rollback(&dir);
        assert!(err.is_err());
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            "fn original() {}\n",
            "Failed journal commit must roll back the file mutation"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}