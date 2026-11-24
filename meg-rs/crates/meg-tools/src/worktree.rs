//! Isolated CoW worktree manager (`~/.meg/worktrees/`) with 4-state lease log
//! (`requested -> prepared -> lease_active -> scope_activated`, `cleanup: remove_if_clean`)
//! and zero-reinstall cache linking (`.env`, `node_modules`, `.venv`, `target`).

use anyhow::{bail, Context, Result};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::process::Command;

pub const LINKABLE_CACHES: &[&str] = &[".env", "node_modules", ".venv", "target"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeaseState {
    Requested,
    Prepared,
    LeaseActive,
    ScopeActivated,
    Cleaned,
}

impl LeaseState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Requested => "requested",
            Self::Prepared => "prepared",
            Self::LeaseActive => "lease_active",
            Self::ScopeActivated => "scope_activated",
            Self::Cleaned => "cleaned",
        }
    }
}

#[derive(Debug, Clone)]
pub struct WorktreeLease {
    pub id: String,
    pub repo_root: PathBuf,
    pub path: PathBuf,
    pub branch: String,
    pub state: LeaseState,
    pub cleanup_policy: String,
    pub linked_caches: Vec<String>,
    pub transitions: Vec<LeaseState>,
    lease_log: PathBuf,
}

pub fn worktrees_root() -> PathBuf {
    std::env::var_os("MEG_WORKTREES_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let home = std::env::var_os("USERPROFILE")
                .or_else(|| std::env::var_os("HOME"))
                .map(PathBuf::from)
                .unwrap_or_else(std::env::temp_dir);
            home.join(".meg").join("worktrees")
        })
}

fn append_lease_event(log: &Path, id: &str, state: LeaseState, path: &Path, cleanup: &str) -> Result<()> {
    if let Some(parent) = log.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let line = json!({
        "id": id,
        "state": state.as_str(),
        "path": path.to_string_lossy(),
        "cleanup": cleanup,
    })
    .to_string();
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(log)?;
    writeln!(f, "{line}")?;
    Ok(())
}

fn link_cache_entry(src: &Path, dst: &Path) -> bool {
    if !src.exists() || dst.exists() {
        return false;
    }
    #[cfg(windows)]
    {
        if src.is_dir() {
            // Directory junction requires no admin privileges on Windows 10.
            let status = Command::new("cmd")
                .args(["/C", "mklink", "/J"])
                .arg(dst)
                .arg(src)
                .output();
            if status.is_ok_and(|o| o.status.success()) {
                return true;
            }
            std::os::windows::fs::symlink_dir(src, dst).is_ok()
        } else {
            std::fs::hard_link(src, dst)
                .or_else(|_| std::fs::copy(src, dst).map(|_| ()))
                .is_ok()
        }
    }
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(src, dst).is_ok()
    }
}

impl WorktreeLease {
    /// Create an isolated git worktree under `~/.meg/worktrees/<id>` and advance
    /// through the 4-state lease log: requested -> prepared -> lease_active -> scope_activated.
    pub fn create(repo_root: &Path, id: &str) -> Result<Self> {
        if id.is_empty()
            || !id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            bail!("invalid worktree id '{id}'");
        }
        let base = worktrees_root();
        let wt_path = base.join(id);
        let lease_log = base.join("leases.jsonl");
        let branch = format!("meg-wt-{id}");
        let cleanup_policy = "remove_if_clean".to_string();

        append_lease_event(&lease_log, id, LeaseState::Requested, &wt_path, &cleanup_policy)?;
        let mut transitions = vec![LeaseState::Requested];

        if let Some(parent) = wt_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let out = Command::new("git")
            .current_dir(repo_root)
            .env("GIT_TERMINAL_PROMPT", "0")
            .args(["worktree", "add", "-b", &branch])
            .arg(&wt_path)
            .arg("HEAD")
            .output()
            .context("failed to run git worktree add")?;
        if !out.status.success() {
            bail!(
                "git worktree add failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        append_lease_event(&lease_log, id, LeaseState::Prepared, &wt_path, &cleanup_policy)?;
        transitions.push(LeaseState::Prepared);

        let mut linked_caches = Vec::new();
        for name in LINKABLE_CACHES {
            let src = repo_root.join(name);
            let dst = wt_path.join(name);
            if link_cache_entry(&src, &dst) {
                linked_caches.push((*name).to_string());
            }
        }
        append_lease_event(&lease_log, id, LeaseState::LeaseActive, &wt_path, &cleanup_policy)?;
        transitions.push(LeaseState::LeaseActive);

        append_lease_event(&lease_log, id, LeaseState::ScopeActivated, &wt_path, &cleanup_policy)?;
        transitions.push(LeaseState::ScopeActivated);

        Ok(Self {
            id: id.to_string(),
            repo_root: repo_root.to_path_buf(),
            path: wt_path,
            branch,
            state: LeaseState::ScopeActivated,
            cleanup_policy,
            linked_caches,
            transitions,
            lease_log,
        })
    }

    /// True when the worktree has no modified/untracked files (excluding linked caches).
    pub fn is_clean(&self) -> Result<bool> {
        let out = Command::new("git")
            .current_dir(&self.path)
            .args(["status", "--porcelain"])
            .output()?;
        if !out.status.success() {
            return Ok(false);
        }
        let dirty = String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter(|l| {
                let path_part = l.get(3..).unwrap_or(l).trim().trim_end_matches('/');
                !self.linked_caches.iter().any(|c| c == path_part)
            })
            .count();
        Ok(dirty == 0)
    }

    /// Remove the worktree if clean (`cleanup: remove_if_clean`). Returns true if removed.
    pub fn remove_if_clean(&mut self) -> Result<bool> {
        if !self.is_clean()? {
            return Ok(false);
        }
        // Unlink junctions/files before git worktree remove so caches in repo_root stay intact.
        for name in &self.linked_caches {
            let dst = self.path.join(name);
            if dst.exists() {
                if dst.is_dir() {
                    let _ = std::fs::remove_dir(&dst);
                } else {
                    let _ = std::fs::remove_file(&dst);
                }
            }
        }
        let out = Command::new("git")
            .current_dir(&self.repo_root)
            .args(["worktree", "remove", "--force"])
            .arg(&self.path)
            .output()?;
        if !out.status.success() {
            bail!(
                "git worktree remove failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        let _ = Command::new("git")
            .current_dir(&self.repo_root)
            .args(["branch", "-D", &self.branch])
            .output();
        self.state = LeaseState::Cleaned;
        self.transitions.push(LeaseState::Cleaned);
        append_lease_event(
            &self.lease_log,
            &self.id,
            LeaseState::Cleaned,
            &self.path,
            &self.cleanup_policy,
        )?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn tmpdir(tag: &str) -> PathBuf {
        static N: AtomicU64 = AtomicU64::new(0);
        let d = std::env::temp_dir().join(format!(
            "meg-wt-{tag}-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn worktree_lifecycle_links_caches_logs_four_states_and_cleans_if_clean() {
        let repo = tmpdir("repo");
        let wt_base = tmpdir("base");
        std::env::set_var("MEG_WORKTREES_DIR", &wt_base);

        // Init tiny git repo with one commit + .env + node_modules cache.
        assert!(Command::new("git").current_dir(&repo).arg("init").output().unwrap().status.success());
        std::fs::write(repo.join("README.md"), "# test\n").unwrap();
        std::fs::write(repo.join(".env"), "KEY=val\n").unwrap();
        std::fs::create_dir_all(repo.join("node_modules/pkg")).unwrap();
        std::fs::write(repo.join("node_modules/pkg/index.js"), "module.exports = 1;\n").unwrap();
        assert!(Command::new("git").current_dir(&repo).args(["add", "README.md"]).output().unwrap().status.success());
        assert!(Command::new("git")
            .current_dir(&repo)
            .args(["-c", "user.name=MEG", "-c", "user.email=meg@local", "commit", "-m", "init"])
            .output()
            .unwrap()
            .status
            .success());

        let mut lease = WorktreeLease::create(&repo, "task-01").unwrap();
        assert_eq!(
            lease.transitions,
            vec![
                LeaseState::Requested,
                LeaseState::Prepared,
                LeaseState::LeaseActive,
                LeaseState::ScopeActivated
            ]
        );
        assert!(lease.linked_caches.contains(&".env".to_string()));
        assert!(lease.linked_caches.contains(&"node_modules".to_string()));
        assert!(lease.path.join("node_modules/pkg/index.js").exists());

        // Dirty worktree is NOT removed by remove_if_clean.
        std::fs::write(lease.path.join("dirty.txt"), "uncommitted").unwrap();
        assert!(!lease.remove_if_clean().unwrap());

        // Clean worktree IS removed and original repo cache stays intact.
        std::fs::remove_file(lease.path.join("dirty.txt")).unwrap();
        assert!(lease.remove_if_clean().unwrap());
        assert!(!lease.path.exists());
        assert!(repo.join("node_modules/pkg/index.js").exists());

        std::env::remove_var("MEG_WORKTREES_DIR");
        let _ = std::fs::remove_dir_all(&repo);
        let _ = std::fs::remove_dir_all(&wt_base);
    }
}