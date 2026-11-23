//! Pre-turn workspace indexer (Sentra Code Memory pattern, AHE): Turn 1
//! starts with full repository recon in 0 extra API calls.

use std::fs;
use std::path::Path;
use std::time::{Duration, Instant};

const IGNORED_DIRS: &[&str] = &[".git", "target", "node_modules", "__pycache__", ".venv", ".meg"];
const MAX_FILE_BYTES: u64 = 512 * 1024;
const MAX_MAP_CHARS: usize = 4_000;
const SIGNATURE_PREFIXES: &[&str] = &[
    "fn ", "pub fn ", "def ", "class ", "export function ", "struct ", "int main",
];

fn is_ignored_dir(name: &str) -> bool {
    IGNORED_DIRS.contains(&name)
}

fn extract_signatures_from_text(text: &str, max: usize) -> Vec<String> {
    let mut sigs = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim_start();
        if SIGNATURE_PREFIXES.iter().any(|p| trimmed.starts_with(p)) {
            sigs.push(trimmed.chars().take(80).collect());
            if sigs.len() >= max {
                break;
            }
        }
    }
    sigs
}

struct WalkBudget {
    remaining_files: usize,
    remaining_visits: usize,
    estimated_chars: usize,
    deadline: Option<Instant>,
    truncated: bool,
}

impl WalkBudget {
    fn exhausted(&mut self) -> bool {
        if self.remaining_files == 0 || self.remaining_visits == 0 || self.estimated_chars >= MAX_MAP_CHARS + 512 {
            return true;
        }
        if let Some(dl) = self.deadline {
            if Instant::now() >= dl {
                self.truncated = true;
                return true;
            }
        }
        false
    }
}

/// Index the workspace into a compact `<workspace_map>` string: relative path,
/// byte size, line count, up to 5 top-level signatures per file. Capped at
/// `max_files` files and 4,000 chars total. Also warns when procps is missing
/// on Linux (the TB2 container defect that silently broke process tools).
pub fn index_workspace(root: &Path, max_files: usize) -> String {
    index_workspace_inner(root, max_files, None)
}

/// Time-bounded variant for live Turn 1 recon so huge directories never stall startup.
pub fn index_workspace_with_budget(root: &Path, max_files: usize, budget: Duration) -> String {
    index_workspace_inner(root, max_files, Some(Instant::now() + budget))
}

fn index_workspace_inner(root: &Path, max_files: usize, deadline: Option<Instant>) -> String {
    let mut entries: Vec<(String, u64, usize, Vec<String>)> = Vec::new();
    let mut budget = WalkBudget {
        remaining_files: max_files,
        remaining_visits: max_files.saturating_mul(10).max(500),
        estimated_chars: 0,
        deadline,
        truncated: false,
    };
    walk(root, root, &mut budget, &mut entries, 0);

    let mut out = format!("<workspace_map root=\"{}\">\n", root.display());
    let mut hit_char_cap = false;
    for (rel, size, lines, sigs) in &entries {
        if out.len() >= MAX_MAP_CHARS {
            out.push_str("...[workspace_map truncated at 4000 chars]\n");
            hit_char_cap = true;
            break;
        }
        out.push_str(&format!("{rel} ({size}B, {lines}L)\n"));
        for sig in sigs {
            out.push_str(&format!("  {sig}\n"));
        }
    }
    if !hit_char_cap && budget.truncated {
        out.push_str("...[workspace_map truncated by time budget]\n");
    }
    if cfg!(target_os = "linux") && !procps_present() {
        out.push_str("[ENV_WARN: procps (/bin/ps) missing]\n");
    }
    out.push_str("</workspace_map>");
    out
}

fn procps_present() -> bool {
    Path::new("/bin/ps").exists() || Path::new("/usr/bin/ps").exists()
}

fn walk(
    root: &Path,
    dir: &Path,
    budget: &mut WalkBudget,
    out: &mut Vec<(String, u64, usize, Vec<String>)>,
    depth: usize,
) {
    if budget.exhausted() {
        return;
    }
    let Ok(read) = fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = read.filter_map(|e| e.ok()).collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        if budget.exhausted() {
            return;
        }
        budget.remaining_visits = budget.remaining_visits.saturating_sub(1);
        let name = entry.file_name().to_string_lossy().to_string();
        let path = entry.path();
        // Metadata WITHOUT following links: a junction here is a cycle risk
        // ("Application Data" -> "AppData") and often unreadable (WinError 5).
        // Skip reparse points entirely; never recurse into them.
        let Ok(meta) = entry.metadata() else { continue };
        if meta.is_dir() {
            if !is_ignored_dir(&name) && !is_reparse(&path) && depth < MAX_WALK_DEPTH {
                walk(root, &path, budget, out, depth + 1);
            }
            continue;
        }
        if meta.len() > MAX_FILE_BYTES {
            continue;
        }
        let Ok(rel) = path.strip_prefix(root) else { continue };
        let rel_str = rel.to_string_lossy().replace('\\', "/");
        // Single read per file: compute both line count and top-level signatures.
        let (lines, sigs) = match fs::read(&path) {
            Ok(bytes) => {
                let line_count = bytes.iter().filter(|&&b| b == b'\n').count();
                let sigs = match std::str::from_utf8(&bytes) {
                    Ok(text) => extract_signatures_from_text(text, 5),
                    Err(_) => Vec::new(),
                };
                (line_count, sigs)
            }
            Err(_) => (0, Vec::new()),
        };
        budget.estimated_chars += rel_str.len() + 24 + sigs.iter().map(|s| s.len() + 3).sum::<usize>();
        out.push((rel_str, meta.len(), lines, sigs));
        budget.remaining_files = budget.remaining_files.saturating_sub(1);
    }
}

/// Depth cap: NTFS junction cycles recursed forever and hung a home-directory
/// launch; 16 levels is far more than any real source tree needs.
const MAX_WALK_DEPTH: usize = 16;

#[cfg(windows)]
fn is_reparse(path: &Path) -> bool {
    use std::os::windows::fs::MetadataExt;
    match fs::symlink_metadata(path) {
        Ok(meta) => meta.file_attributes() & 0x400 != 0, // FILE_ATTRIBUTE_REPARSE_POINT
        Err(_) => true, // unreadable = skip, never hang
    }
}

#[cfg(not(windows))]
fn is_reparse(path: &Path) -> bool {
    match fs::symlink_metadata(path) {
        Ok(meta) => meta.file_type().is_symlink(),
        Err(_) => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn indexes_files_with_signatures_and_sizes() {
        let root = std::env::temp_dir().join(format!("meg-idx-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let src = root.join("src");
        fs::create_dir_all(&src).unwrap();
        fs::write(
            src.join("lib.rs"),
            "pub fn alpha() {}\nfn beta() {}\nconst X: u8 = 1;\npub fn gamma() {}\n",
        )
        .unwrap();
        fs::write(root.join("README.md"), "# test\n").unwrap();

        let map = index_workspace(&root, 200);
        assert!(map.starts_with("<workspace_map"), "{map}");
        assert!(map.contains("src/lib.rs"), "MAP WAS:\n{map}");
        assert!(map.contains("pub fn alpha"));
        assert!(map.contains("README.md"));
        assert!(map.ends_with("</workspace_map>"));
        assert!(map.len() < 4_000);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn max_files_cap_respected() {
        let root = std::env::temp_dir().join(format!("meg-idx-cap-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        for i in 0..12 {
            fs::write(root.join(format!("f{i}.txt")), "x\n").unwrap();
        }
        let map = index_workspace(&root, 5);
        let file_lines = map
            .lines()
            .filter(|l| l.contains(".txt") && !l.contains("workspace_map"))
            .count();
        assert!(file_lines <= 5, "{file_lines} files indexed with cap 5");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn ignored_dirs_skipped() {
        let root = std::env::temp_dir().join(format!("meg-idx-ign-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let git = root.join(".git");
        fs::create_dir_all(&git).unwrap();
        fs::write(git.join("config"), "should not appear\n").unwrap();
        fs::write(root.join("real.txt"), "appears\n").unwrap();
        let map = index_workspace(&root, 200);
        assert!(!map.contains("should not appear"));
        assert!(map.contains("real.txt"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn budget_bounds_walk_and_handles_binary_files() {
        let root = std::env::temp_dir().join(format!("meg-idx-bgt-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("a.rs"), "pub fn ok() {}\n").unwrap();
        fs::write(root.join("blob.bin"), [0xffu8, 0x00, 0xfe, b'\n']).unwrap();
        let map = index_workspace_with_budget(&root, 200, Duration::from_millis(200));
        assert!(map.contains("a.rs"), "{map}");
        assert!(map.contains("pub fn ok"), "{map}");
        assert!(map.ends_with("</workspace_map>"));
        let _ = fs::remove_dir_all(&root);
    }
}