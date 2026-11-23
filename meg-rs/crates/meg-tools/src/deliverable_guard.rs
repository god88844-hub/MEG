//! Publish-State Deliverable Guard (AHE iterations 5 & 8): once a deliverable
//! passes verification it is LOCKED — write/edit/destructive-bash attempts
//! against it are blocked until explicitly unlocked. Also the single-file
//! layout cleanliness check (polyglot-rust-c / polyglot-c-py fail on stray
//! a.out/.o/.pyc).

use std::collections::HashSet;
use std::path::{Path, PathBuf};

const DESTRUCTIVE_BASH_TOKENS: &[&str] = &[
    "rm ",
    "git clean",
    "git checkout",
    "git reset --hard",
    "git restore",
    "> ",
];

pub struct DeliverableGuard {
    pub workspace: PathBuf,
    pub target_paths: HashSet<PathBuf>,
    pub locked_paths: HashSet<PathBuf>,
    pub single_file_mode: bool,
}

impl DeliverableGuard {
    /// Extract deliverable paths from the goal: `/app/x` and bare filenames.
    /// Container-convention `app/` prefixes are stripped when `app` is not a
    /// real subdirectory of the workspace (the Python _resolve_artifact rule).
    pub fn from_goal(workspace: &Path, goal: &str) -> Self {
        let mut targets = HashSet::new();
        for token in goal.split(|c: char| c.is_whitespace() || c == ',' || c == '"') {
            let token = token.trim();
            let candidate = if let Some(rest) = token.strip_prefix("/app/") {
                rest.to_string()
            } else if let Some(rest) = token.strip_prefix("/app") {
                rest.to_string()
            } else {
                token.to_string()
            };
            if candidate.is_empty() || !candidate.contains('.') {
                continue;
            }
            // Reject tokens with characters a deliverable path would not have.
            if candidate
                .chars()
                .any(|c| c.is_whitespace() || c == '(' || c == ')')
            {
                continue;
            }
            targets.insert(workspace.join(&candidate));
        }
        let single_file_mode = {
            let g = goal.to_lowercase();
            g.contains("single file")
                || g.contains("polyglot-rust-c")
                || g.contains("polyglot-c-py")
        };
        Self {
            workspace: workspace.to_path_buf(),
            target_paths: targets,
            locked_paths: HashSet::new(),
            single_file_mode,
        }
    }

    pub fn lock_verified_deliverable(&mut self, path: &Path) {
        self.locked_paths.insert(path.to_path_buf());
    }

    pub fn unlock_deliverable(&mut self, path: &Path) {
        self.locked_paths.remove(path);
    }

    /// None = allowed; Some(reason) = blocked.
    pub fn check_write(&self, path: &Path) -> Option<String> {
        if self.locked_paths.contains(path) {
            return Some(
                "ERROR: Deliverable is LOCKED by Publish-State Guard. Pass unlock_deliverable=true to modify."
                    .into(),
            );
        }
        None
    }

    /// None = allowed; Some(reason) = blocked (destructive bash against a locked path).
    pub fn check_bash(&self, command: &str) -> Option<String> {
        let lower = command.to_lowercase();
        for locked in &self.locked_paths {
            let lock_str = locked.to_string_lossy().to_lowercase();
            for token in DESTRUCTIVE_BASH_TOKENS {
                if lower.contains(token) && lower.contains(&lock_str) {
                    return Some(
                        "ERROR: Deliverable is LOCKED by Publish-State Guard. Pass unlock_deliverable=true to modify."
                            .into(),
                    );
                }
            }
            // rm/git-clean targeting the locked file name at all
            if (lower.contains("rm ")
                || lower.contains("git clean")
                || lower.contains("git checkout"))
                && lock_str
                    .rsplit(['/', '\\'])
                    .next()
                    .map(|f| lower.contains(f))
                    .unwrap_or(false)
            {
                return Some(
                    "ERROR: Deliverable is LOCKED by Publish-State Guard. Pass unlock_deliverable=true to modify."
                        .into(),
                );
            }
        }
        None
    }
}

/// Single-file layout cleanliness: stray compilation artifacts fail.
pub fn check_single_file_cleanliness(workspace: &Path) -> Result<(), Vec<String>> {
    let mut stray = Vec::new();
    const ARTIFACTS: &[&str] = &["a.out", "tmp_*"];
    const ARTIFACT_EXTS: &[&str] = &["o", "so", "pyc"];
    let Ok(read) = std::fs::read_dir(workspace) else {
        return Ok(());
    };
    for entry in read.filter_map(|e| e.ok()) {
        let name = entry.file_name().to_string_lossy().to_string();
        if ARTIFACTS.iter().any(|a| glob_simple(&name, a)) {
            stray.push(name);
        } else if let Some(ext) = entry.path().extension().and_then(|e| e.to_str()) {
            if ARTIFACT_EXTS.contains(&ext) {
                stray.push(name);
            }
        } else if name == "__pycache__" {
            stray.push(name);
        }
    }
    if stray.is_empty() {
        Ok(())
    } else {
        Err(stray)
    }
}

fn glob_simple(name: &str, pattern: &str) -> bool {
    if let Some(prefix) = pattern.strip_suffix('*') {
        return name.starts_with(prefix);
    }
    name == pattern
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_app_prefixed_deliverables() {
        let ws = Path::new("/app");
        let g =
            GuardFixture::goal_guard(ws, "Call your program /app/gpt2.c and write /app/out.txt");
        assert!(g.target_paths.contains(&ws.join("gpt2.c")));
        assert!(g.target_paths.contains(&ws.join("out.txt")));
    }

    #[test]
    fn locked_write_is_blocked_until_unlock() {
        let mut g = GuardFixture::goal_guard(Path::new("/app"), "write /app/gpt2.c");
        let path = Path::new("/app/gpt2.c");
        g.lock_verified_deliverable(path);
        assert!(g.check_write(path).is_some());
        g.unlock_deliverable(path);
        assert!(g.check_write(path).is_none());
    }

    #[test]
    fn destructive_bash_on_locked_path_blocked() {
        let mut g = GuardFixture::goal_guard(Path::new("/app"), "write /app/gpt2.c");
        let path = Path::new("/app/gpt2.c");
        g.lock_verified_deliverable(path);
        assert!(g.check_bash("rm /app/gpt2.c").is_some());
        assert!(g.check_bash("git checkout -- gpt2.c").is_some());
        assert!(g.check_bash("git clean -f gpt2.c").is_some());
        // benign command passes
        assert!(g.check_bash("ls -la").is_none());
    }

    #[test]
    fn single_file_cleanliness_detects_stray_artifacts() {
        let dir = std::env::temp_dir().join(format!("meg-guard-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("main.c"), "int main(){}").unwrap();
        assert!(check_single_file_cleanliness(&dir).is_ok());
        std::fs::write(dir.join("a.out"), b"ELF").unwrap();
        std::fs::write(dir.join("main.o"), b"obj").unwrap();
        let err = check_single_file_cleanliness(&dir).unwrap_err();
        assert!(err.contains(&"a.out".to_string()));
        assert!(err.contains(&"main.o".to_string()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    struct GuardFixture;
    impl GuardFixture {
        fn goal_guard(ws: &Path, goal: &str) -> DeliverableGuard {
            DeliverableGuard::from_goal(ws, goal)
        }
    }
}

/// Deep artifact sanity (2026-11-12 T1.4 port): reject hollow deliverables
/// before a trial settles. Text must be non-trivial, images need the right
/// magic header and a real size, model weights must exceed a dummy-file floor.
pub fn deep_artifact_sanity(workspace: &Path, artifacts: &[PathBuf]) -> Result<(), Vec<String>> {
    let mut problems: Vec<String> = Vec::new();
    for rel in artifacts {
        let path = if rel.is_absolute() {
            rel.clone()
        } else {
            workspace.join(rel)
        };
        if !path.is_file() {
            continue; // existence is the artifact gate's finding
        }
        let lower = path.to_string_lossy().to_lowercase();
        let meta = match std::fs::metadata(&path) {
            Ok(m) => m,
            Err(_) => continue,
        };
        let size = meta.len();
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        if lower.ends_with(".txt") || lower.ends_with(".json") || lower.ends_with(".csv") {
            match std::fs::read_to_string(&path) {
                Ok(text) => {
                    let hollow = matches!(
                        text.trim().to_lowercase().as_str(),
                        "" | "todo" | "none" | "null" | "n/a" | "placeholder" | "tbd"
                    );
                    if hollow {
                        problems.push(format!(
                            "{name}: hollow text deliverable (empty/placeholder)"
                        ));
                    }
                }
                Err(_) => problems.push(format!("{name}: unreadable text deliverable")),
            }
        } else if lower.ends_with(".bmp") {
            if size <= 1024 {
                problems.push(format!("{name}: image is {size} bytes (< 1024)"));
            }
            match std::fs::read(&path) {
                Ok(bytes) => {
                    if !bytes.starts_with(b"BM") {
                        problems.push(format!("{name}: missing BM header"));
                    }
                }
                Err(_) => problems.push(format!("{name}: unreadable image")),
            }
        } else if lower.ends_with(".png") {
            if size <= 1024 {
                problems.push(format!("{name}: image is {size} bytes (< 1024)"));
            }
            match std::fs::read(&path) {
                Ok(bytes) => {
                    if !bytes.starts_with(byte_png_magic()) {
                        problems.push(format!("{name}: missing PNG magic"));
                    }
                }
                Err(_) => problems.push(format!("{name}: unreadable image")),
            }
        } else if lower.ends_with(".caffemodel")
            || lower.ends_with(".bin")
            || lower.ends_with(".ftz")
            || lower.ends_with(".pt")
            || lower.ends_with(".pth")
            || lower.ends_with(".pkl")
            || lower.ends_with(".rds")
        {
            if size < 4096 {
                problems.push(format!(
                    "{name}: weights file is {size} bytes (< 4096) — a dummy file"
                ));
            }
        }
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems)
    }
}

/// Whitelist single-file sweep (2026-11-12 T1.3 port): when the deliverable
/// is the ONLY source file in its directory, remove EVERY other entry so
/// os.listdir() == [expected] by construction. Returns removed names.

fn byte_png_magic() -> &'static [u8] {
    &[0x89, b'P', b'N', b'G']
}

pub fn whitelist_sweep_single_file(workspace: &Path, artifacts: &[PathBuf]) -> Vec<String> {
    let source_exts = ["py", "c", "h", "cpp", "rs", "scm", "sql", "js", "sh"];
    let mut sources: Vec<PathBuf> = Vec::new();
    for rel in artifacts {
        let text = rel.to_string_lossy().to_string();
        let ext = text.rsplit('.').next().unwrap_or("").to_lowercase();
        if source_exts.contains(&ext.as_str()) {
            sources.push(workspace.join(text));
        }
    }
    if sources.len() != 1 {
        return Vec::new();
    }
    let source = &sources[0];
    if !source.is_file() {
        return Vec::new();
    }
    let dir = match source.parent() {
        Some(d) => d,
        None => return Vec::new(),
    };
    let mut removed: Vec<String> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path == *source {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            if path.is_dir() {
                if std::fs::remove_dir_all(&path).is_ok() {
                    removed.push(format!("{name}/"));
                }
            } else if std::fs::remove_file(&path).is_ok() {
                removed.push(name);
            }
        }
    }
    removed
}

#[cfg(test)]
mod closure_guard_tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn deep_sanity_rejects_hollow_text() {
        let dir = std::env::temp_dir().join("meg-dg-hollow");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("out.txt");
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(b"   \n").unwrap();
        let problems = deep_artifact_sanity(&dir, &[PathBuf::from("out.txt")]).unwrap_err();
        assert!(problems.iter().any(|p| p.contains("hollow")));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn deep_sanity_rejects_bad_bmp_and_small_weights() {
        let dir = std::env::temp_dir().join("meg-dg-magic");
        let _ = std::fs::create_dir_all(&dir);
        let bmp = dir.join("frame.bmp");
        std::fs::write(&bmp, b"NOPE").unwrap();
        let weights = dir.join("model.bin");
        std::fs::write(&weights, vec![0u8; 32]).unwrap();
        let problems = deep_artifact_sanity(
            &dir,
            &[PathBuf::from("frame.bmp"), PathBuf::from("model.bin")],
        )
        .unwrap_err();
        assert!(problems.iter().any(|p| p.contains("BM header")));
        assert!(problems.iter().any(|p| p.contains("dummy file")));
        let _ = std::fs::remove_file(&bmp);
        let _ = std::fs::remove_file(&weights);
    }

    #[test]
    fn deep_sanity_accepts_real_artifacts() {
        let dir = std::env::temp_dir().join("meg-dg-real");
        let _ = std::fs::create_dir_all(&dir);
        let mut bmp = dir.join("frame.bmp");
        let mut bytes = b"BM".to_vec();
        bytes.extend(vec![0u8; 2048]);
        std::fs::write(&mut bmp, &bytes).unwrap();
        let out = dir.join("out.txt");
        std::fs::write(&out, b"flag{real_content}\n").unwrap();
        assert!(deep_artifact_sanity(
            &dir,
            &[PathBuf::from("frame.bmp"), PathBuf::from("out.txt")]
        )
        .is_ok());
        let _ = std::fs::remove_file(&bmp);
        let _ = std::fs::remove_file(&out);
    }

    #[test]
    fn whitelist_sweep_leaves_exactly_the_source() {
        let dir = std::env::temp_dir().join("meg-dg-sweep");
        let _ = std::fs::create_dir_all(&dir);
        let source = dir.join("main.py.c");
        std::fs::write(&source, b"#define sdf b /*").unwrap();
        std::fs::write(dir.join("cmain"), b"ELF").unwrap();
        std::fs::write(dir.join(".hidden"), b"x").unwrap();
        std::fs::create_dir_all(dir.join("__pycache__")).unwrap();
        let removed = whitelist_sweep_single_file(&dir, &[PathBuf::from("main.py.c")]);
        assert!(
            removed.contains(&"cmain".to_string()),
            "removed: {removed:?}"
        );
        assert!(removed.contains(&".hidden".to_string()));
        let left: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        assert_eq!(left, vec!["main.py.c".to_string()]);
        let _ = std::fs::remove_file(&source);
        let _ = std::fs::remove_dir(&dir);
    }
}
