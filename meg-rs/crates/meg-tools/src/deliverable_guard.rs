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
        } else if (lower.ends_with(".caffemodel")
            || lower.ends_with(".bin")
            || lower.ends_with(".ftz")
            || lower.ends_with(".pt")
            || lower.ends_with(".pth")
            || lower.ends_with(".pkl")
            || lower.ends_with(".rds"))
            && size < 4096
        {
            problems.push(format!(
                "{name}: weights file is {size} bytes (< 4096) — a dummy file"
            ));
        }
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems)
    }
}

// Whitelist single-file sweep helper magic bytes.
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

// ===========================================================================
// P1-10: Competitor Bug Guards, Sealed-File Wall, Privacy Pin & .hot.* Linter
// ===========================================================================

/// Pure-Rust SHA-256 digest (hex lowercase), zero external crate dependencies.
pub fn sha256_hex(input: &[u8]) -> String {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
        0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
        0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
        0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
        0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
        0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
        0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a,
        0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
    ];
    let bit_len = (input.len() as u64).wrapping_mul(8);
    let mut msg = input.to_vec();
    msg.push(0x80);
    while (msg.len() % 64) != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());

    for chunk in msg.as_chunks::<64>().0 {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                chunk[4 * i],
                chunk[4 * i + 1],
                chunk[4 * i + 2],
                chunk[4 * i + 3],
            ]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (slot, delta) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
            *slot = slot.wrapping_add(delta);
        }
    }
    let mut out = String::with_capacity(64);
    for word in h {
        out.push_str(&format!("{word:08x}"));
    }
    out
}

/// Protected path prefixes restored by the post-run wall (`git checkout <base> -- PROTECTED`).
pub const PROTECTED_PATHS: &[&str] = &["tests/", "migrations/", "scripts/check."];

/// Ring-0 read-only lock flipped outside the LLM on Esc or stop phrases.
#[derive(Default)]
pub struct ReadOnlyLock {
    locked: std::sync::atomic::AtomicBool,
}

impl ReadOnlyLock {
    pub const fn new() -> Self {
        Self {
            locked: std::sync::atomic::AtomicBool::new(false),
        }
    }
    pub fn engage(&self) {
        self.locked.store(true, std::sync::atomic::Ordering::SeqCst);
    }
    pub fn release(&self) {
        self.locked.store(false, std::sync::atomic::Ordering::SeqCst);
    }
    pub fn is_locked(&self) -> bool {
        self.locked.load(std::sync::atomic::Ordering::SeqCst)
    }
    /// Inspect user input outside the LLM for stop phrases and flip lock immediately.
    pub fn check_stop_phrase(&self, text: &str) -> bool {
        let low = text.trim().to_ascii_lowercase();
        let triggered = matches!(
            low.as_str(),
            "stop" | "halt" | "abort" | "freeze" | "read-only" | "read only" | "stop editing"
        ) || low.starts_with("stop ")
            || low.starts_with("do not edit");
        if triggered {
            self.engage();
        }
        triggered
    }
    pub fn guard_mutation(&self, tool: &str) -> Result<(), String> {
        if self.is_locked() && matches!(tool, "write_file" | "edit_file" | "bash") {
            return Err(format!(
                "RING0_READ_ONLY_LOCK: tool '{tool}' blocked until operator unlocks"
            ));
        }
        Ok(())
    }
}

/// AHE Publish-State Shell Guard: hard-blocks `rm`, `git reset`, `git clean`
/// targeting non-`/tmp` verified deliverables.
pub fn check_publish_state_shell(command: &str, deliverables: &[std::path::PathBuf]) -> Result<(), String> {
    let low = command.to_ascii_lowercase();
    if low.contains("git reset --hard") || low.contains("git clean -f") || low.contains("git clean -fd") {
        return Err("PUBLISH_STATE_GUARD: destructive git reset/clean blocked after verification".into());
    }
    let is_rm = low.contains("rm ") || low.contains("remove-item ") || low.contains("del ");
    if is_rm {
        for d in deliverables {
            let s = d.to_string_lossy().replace('\\', "/");
            if s.starts_with("/tmp/") || s.starts_with("tmp/") {
                continue;
            }
            let name = d.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if (!name.is_empty() && command.contains(name)) || command.contains(&s) {
                return Err(format!(
                    "PUBLISH_STATE_GUARD: refusing destructive shell command targeting verified deliverable '{s}'"
                ));
            }
        }
    }
    Ok(())
}

/// `gh` target-repo privacy pin & irreversible remote/publish command blocker.
pub fn check_gh_privacy_and_publish(command: &str, allowed_repo: Option<&str>) -> Result<(), String> {
    let low = command.to_ascii_lowercase();
    for banned in [
        "gh pr merge",
        "git push --force",
        "git push -f",
        "git tag",
        "npm publish",
        "cargo publish",
    ] {
        if low.contains(banned) {
            return Err(format!(
                "PRIVACY_PUBLISH_GUARD: autonomous '{banned}' is blocked; requires human operator"
            ));
        }
    }
    // Pin `gh ... --repo <owner/name>` or `-R <owner/name>` to allowed_repo.
    let tokens: Vec<&str> = command.split_whitespace().collect();
    if tokens.first().copied() == Some("gh") {
        for (i, tok) in tokens.iter().enumerate() {
            let repo_arg = if *tok == "--repo" || *tok == "-R" {
                tokens.get(i + 1).copied()
            } else {
                tok.strip_prefix("--repo=")
            };
            if let Some(target) = repo_arg {
                let target_clean = target.trim_matches(|c| c == '"' || c == '\'');
                match allowed_repo {
                    Some(allowed) if allowed.eq_ignore_ascii_case(target_clean) => {}
                    Some(allowed) => {
                        return Err(format!(
                            "GH_PRIVACY_PIN: external --repo '{target_clean}' blocked (pinned to '{allowed}')"
                        ));
                    }
                    None => {
                        return Err(format!(
                            "GH_PRIVACY_PIN: explicit --repo '{target_clean}' blocked when no target repo is pinned"
                        ));
                    }
                }
            }
        }
    }
    Ok(())
}

/// SHA-256 sealed-file hash manifest (`scalar-loop`) protecting immutable files.
#[derive(Debug, Clone, Default)]
pub struct SealedManifest {
    hashes: std::collections::BTreeMap<String, String>,
}

impl SealedManifest {
    pub fn seal(workspace: &std::path::Path, rel_paths: &[&str]) -> anyhow::Result<Self> {
        let mut hashes = std::collections::BTreeMap::new();
        for &rel in rel_paths {
            let full = workspace.join(rel);
            let bytes = std::fs::read(&full)?;
            hashes.insert(rel.replace('\\', "/"), sha256_hex(&bytes));
        }
        Ok(Self { hashes })
    }

    pub fn verify(&self, workspace: &std::path::Path) -> Result<(), Vec<String>> {
        let mut violations = Vec::new();
        for (rel, expected) in &self.hashes {
            let full = workspace.join(rel);
            match std::fs::read(&full) {
                Ok(bytes) => {
                    let actual = sha256_hex(&bytes);
                    if &actual != expected {
                        violations.push(format!("sealed file modified: {rel}"));
                    }
                }
                Err(_) => violations.push(format!("sealed file missing/deleted: {rel}")),
            }
        }
        if violations.is_empty() {
            Ok(())
        } else {
            Err(violations)
        }
    }

    pub fn len(&self) -> usize {
        self.hashes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.hashes.is_empty()
    }

    pub fn contains_rel_path(&self, rel: &str) -> bool {
        self.hashes.contains_key(&rel.replace('\\', "/"))
    }

    /// Seal all existing protected files under `tests/`, `migrations/`, and `scripts/check.*`.
    pub fn seal_protected_paths(workspace: &std::path::Path) -> Self {
        let mut hashes = std::collections::BTreeMap::new();
        for fixed in ["scripts/check.ps1", "scripts/check.sh", "scripts/selftest.ps1", "scripts/selftest.sh"] {
            let full = workspace.join(fixed);
            if full.is_file() {
                if let Ok(bytes) = std::fs::read(&full) {
                    hashes.insert(fixed.to_string(), sha256_hex(&bytes));
                }
            }
        }
        for dir_name in ["tests", "migrations"] {
            let root = workspace.join(dir_name);
            if !root.is_dir() {
                continue;
            }
            let mut stack = vec![root];
            while let Some(dir) = stack.pop() {
                let Ok(entries) = std::fs::read_dir(&dir) else { continue };
                for entry in entries.flatten() {
                    let p = entry.path();
                    if p.is_dir() {
                        stack.push(p);
                    } else if p.is_file() {
                        if let (Ok(rel), Ok(bytes)) = (p.strip_prefix(workspace), std::fs::read(&p)) {
                            let key = rel.to_string_lossy().replace('\\', "/");
                            hashes.insert(key, sha256_hex(&bytes));
                        }
                    }
                }
            }
        }
        Self { hashes }
    }
}

/// Build the post-run restore argv (`git checkout <base> -- <protected_paths>`).
pub fn protected_restore_argv(base_ref: &str, paths: &[String]) -> Vec<String> {
    let mut argv = vec!["git".into(), "checkout".into(), base_ref.into(), "--".into()];
    argv.extend(paths.iter().cloned());
    argv
}

/// Lauren Tan `.hot.*` 60-FPS frame linter: flags heap allocations and unseeded
/// RNG inside `.hot.*` hot-loop files.
pub fn lint_hot_frame_file(path: &std::path::Path, content: &str) -> Vec<String> {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    if !name.contains(".hot.") {
        return Vec::new();
    }
    const FORBIDDEN: &[(&str, &str)] = &[
        ("Math.random(", "unseeded RNG (Math.random) in .hot.* frame path"),
        ("rand::random", "unseeded RNG (rand::random) in .hot.* frame path"),
        ("thread_rng(", "unseeded RNG (thread_rng) in .hot.* frame path"),
        ("new Array(", "per-frame heap allocation (new Array) in .hot.*"),
        ("new Map(", "per-frame heap allocation (new Map) in .hot.*"),
        ("new Set(", "per-frame heap allocation (new Set) in .hot.*"),
        ("Vec::new(", "per-frame heap allocation (Vec::new) in .hot.*"),
        ("HashMap::new(", "per-frame heap allocation (HashMap::new) in .hot.*"),
        ("JSON.parse(", "per-frame allocation (JSON.parse) in .hot.*"),
    ];
    let mut hits = Vec::new();
    for (idx, line) in content.lines().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("//") {
            continue;
        }
        for (pat, reason) in FORBIDDEN {
            if trimmed.contains(pat) {
                hits.push(format!("{name}:{}: {reason}", idx + 1));
            }
        }
    }
    hits
}

#[cfg(test)]
mod p1_10_tests {
    use super::*;

    #[test]
    fn sha256_matches_nist_vector_and_sealed_manifest_detects_tampering() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        let ws = std::env::temp_dir().join(format!("meg-seal-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&ws);
        std::fs::create_dir_all(ws.join("tests")).unwrap();
        std::fs::write(ws.join("tests/spec.rs"), "assert!(true);\n").unwrap();

        let manifest = SealedManifest::seal(&ws, &["tests/spec.rs"]).unwrap();
        assert!(manifest.verify(&ws).is_ok());

        std::fs::write(ws.join("tests/spec.rs"), "// weakened\n").unwrap();
        let err = manifest.verify(&ws).unwrap_err();
        assert!(err[0].contains("sealed file modified"), "{err:?}");
        let _ = std::fs::remove_dir_all(&ws);
    }

    #[test]
    fn privacy_pin_publish_guard_readonly_lock_and_hot_linter() {
        assert!(check_gh_privacy_and_publish("gh issue list --repo god88844-hub/MEG", Some("god88844-hub/MEG")).is_ok());
        assert!(check_gh_privacy_and_publish("gh issue create --repo other/public", Some("god88844-hub/MEG")).is_err());
        assert!(check_gh_privacy_and_publish("gh pr merge 5", Some("god88844-hub/MEG")).is_err());
        assert!(check_gh_privacy_and_publish("git push --force", None).is_err());

        let deliv = vec![std::path::PathBuf::from("dist/app.bin")];
        assert!(check_publish_state_shell("rm -f dist/app.bin", &deliv).is_err());
        assert!(check_publish_state_shell("git reset --hard", &deliv).is_err());
        assert!(check_publish_state_shell("rm -f /tmp/scratch.txt", &deliv).is_ok());

        let lock = ReadOnlyLock::new();
        assert!(lock.guard_mutation("edit_file").is_ok());
        assert!(lock.check_stop_phrase("stop editing"));
        assert!(lock.guard_mutation("edit_file").is_err());
        assert!(lock.guard_mutation("read_file").is_ok());

        let hot_hits = lint_hot_frame_file(
            std::path::Path::new("physics.hot.ts"),
            "const r = Math.random();\nconst a = new Array(16);\n",
        );
        assert_eq!(hot_hits.len(), 2, "{hot_hits:?}");
        assert!(lint_hot_frame_file(std::path::Path::new("setup.ts"), "const r = Math.random();").is_empty());
    }
}