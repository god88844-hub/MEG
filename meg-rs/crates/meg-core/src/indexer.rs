//! Pre-turn workspace indexer, PageRank RepoMap (~1,000-token cap),
//! Aider `ROOT_IMPORTANT_FILES` priority list, and downstream caller lookup.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::path::Path;
use std::time::{Duration, Instant};

const IGNORED_DIRS: &[&str] = &[".git", "target", "node_modules", "__pycache__", ".venv", ".meg"];
const MAX_FILE_BYTES: u64 = 512 * 1024;
const MAX_MAP_CHARS: usize = 4_000;
const MAX_WALK_DEPTH: usize = 16;

const SIGNATURE_PREFIXES: &[&str] = &[
    "fn ",
    "pub fn ",
    "pub async fn ",
    "async fn ",
    "def ",
    "class ",
    "export function ",
    "export async function ",
    "export class ",
    "export interface ",
    "struct ",
    "pub struct ",
    "enum ",
    "pub enum ",
    "trait ",
    "pub trait ",
    "int main",
];

/// Aider-style root important files prioritized at the top of RepoMap.
pub const ROOT_IMPORTANT_FILES: &[&str] = &[
    "AGENTS.md",
    "CLAUDE.md",
    "README.md",
    "Cargo.toml",
    "package.json",
    "pyproject.toml",
    "go.mod",
    "Makefile",
    "Dockerfile",
    "docker-compose.yml",
    "tsconfig.json",
    ".github/workflows/ci.yml",
    "docs/PLAN.md",
    "docs/STATE.md",
    "docs/ARCHITECTURE.md",
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

fn extract_symbol_name(sig: &str) -> Option<String> {
    let stripped = sig
        .trim_start_matches("pub ")
        .trim_start_matches("async ")
        .trim_start_matches("export ");
    let rest = stripped
        .strip_prefix("fn ")
        .or_else(|| stripped.strip_prefix("def "))
        .or_else(|| stripped.strip_prefix("class "))
        .or_else(|| stripped.strip_prefix("function "))
        .or_else(|| stripped.strip_prefix("interface "))
        .or_else(|| stripped.strip_prefix("struct "))
        .or_else(|| stripped.strip_prefix("enum "))
        .or_else(|| stripped.strip_prefix("trait "))?;
    let name: String = rest
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect();
    if name.len() >= 2 {
        Some(name)
    } else {
        None
    }
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
        if self.remaining_files == 0
            || self.remaining_visits == 0
            || self.estimated_chars >= MAX_MAP_CHARS + 512
        {
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

/// Index the workspace into a compact `<workspace_map>` string.
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

    // Prioritize ROOT_IMPORTANT_FILES first while preserving deterministic ordering.
    entries.sort_by_key(|(rel, _, _, _)| {
        let imp = ROOT_IMPORTANT_FILES
            .iter()
            .position(|&p| p == rel)
            .unwrap_or(usize::MAX);
        (imp, rel.clone())
    });

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

/// Build a PageRank-ranked RepoMap capped at `max_tokens` (~4 chars/token; default 1,000 tokens).
/// Ranks files by incoming cross-file symbol references + `ROOT_IMPORTANT_FILES` boost.
pub fn build_repo_map(root: &Path, max_tokens: usize) -> String {
    let char_cap = max_tokens.saturating_mul(4).clamp(400, 16_000);
    let mut files: Vec<(String, String, Vec<String>)> = Vec::new();
    collect_source_files(root, root, 0, 200, &mut files);

    // Map symbol -> defining file indices.
    let mut defs: HashMap<String, Vec<usize>> = HashMap::new();
    for (idx, (_, _, sigs)) in files.iter().enumerate() {
        for sig in sigs {
            if let Some(name) = extract_symbol_name(sig) {
                defs.entry(name).or_default().push(idx);
            }
        }
    }

    // Build directed reference graph: file i -> file j if i mentions a symbol defined in j.
    let n = files.len();
    let mut out_edges: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); n];
    for (i, (_, text, _)) in files.iter().enumerate() {
        for (sym, owners) in &defs {
            if text.contains(sym.as_str()) {
                for &j in owners {
                    if i != j {
                        out_edges[i].insert(j);
                    }
                }
            }
        }
    }

    // 8 iterations of damped PageRank (d = 0.85).
    let mut rank = vec![if n > 0 { 1.0 / n as f64 } else { 0.0 }; n];
    for _ in 0..8 {
        let mut next = vec![if n > 0 { 0.15 / n as f64 } else { 0.0 }; n];
        for i in 0..n {
            if !out_edges[i].is_empty() {
                let share = 0.85 * rank[i] / out_edges[i].len() as f64;
                for &j in &out_edges[i] {
                    next[j] += share;
                }
            }
        }
        rank = next;
    }

    // Add a small priority boost for ROOT_IMPORTANT_FILES.
    let mut scored: Vec<(usize, f64)> = (0..n)
        .map(|i| {
            let boost = if ROOT_IMPORTANT_FILES.contains(&files[i].0.as_str()) {
                0.5
            } else {
                0.0
            };
            (i, rank[i] + boost)
        })
        .collect();
    scored.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| files[a.0].0.cmp(&files[b.0].0))
    });

    let mut out = format!("<repo_map root=\"{}\" max_tokens=\"{max_tokens}\">\n", root.display());
    for (idx, score) in scored {
        let (rel, _, sigs) = &files[idx];
        let mut block = format!("{rel} [rank={score:.3}]\n");
        for sig in sigs.iter().take(6) {
            block.push_str(&format!("  {sig}\n"));
        }
        if out.len() + block.len() + 20 > char_cap {
            out.push_str("...[repo_map truncated at token cap]\n");
            break;
        }
        out.push_str(&block);
    }
    out.push_str("</repo_map>");
    out
}

/// Find downstream callers/importers of `symbol` across the workspace (excluding
/// definition lines) so agents never delete an imported symbol blindly.
pub fn find_downstream_callers(root: &Path, symbol: &str) -> Vec<(String, usize, String)> {
    if symbol.trim().is_empty() {
        return Vec::new();
    }
    let mut files: Vec<(String, String, Vec<String>)> = Vec::new();
    collect_source_files(root, root, 0, 400, &mut files);
    let mut hits: BTreeMap<(String, usize), String> = BTreeMap::new();
    for (rel, text, _) in files {
        for (idx, line) in text.lines().enumerate() {
            if !contains_identifier(line, symbol) {
                continue;
            }
            let trimmed = line.trim_start();
            let is_def = SIGNATURE_PREFIXES
                .iter()
                .any(|p| trimmed.starts_with(p) && extract_symbol_name(trimmed).as_deref() == Some(symbol));
            if !is_def {
                hits.insert((rel.clone(), idx + 1), trimmed.chars().take(120).collect());
            }
        }
    }
    hits.into_iter().map(|((f, l), s)| (f, l, s)).collect()
}

fn contains_identifier(line: &str, ident: &str) -> bool {
    let bytes = line.as_bytes();
    let mut start = 0;
    while let Some(pos) = line[start..].find(ident) {
        let abs = start + pos;
        let before_ok = abs == 0
            || !(bytes[abs - 1].is_ascii_alphanumeric() || bytes[abs - 1] == b'_');
        let end = abs + ident.len();
        let after_ok = end == bytes.len()
            || !(bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_');
        if before_ok && after_ok {
            return true;
        }
        start = abs + ident.len();
    }
    false
}

fn collect_source_files(
    root: &Path,
    dir: &Path,
    depth: usize,
    max_files: usize,
    out: &mut Vec<(String, String, Vec<String>)>,
) {
    if out.len() >= max_files || depth > MAX_WALK_DEPTH {
        return;
    }
    let Ok(read) = fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = read.filter_map(|e| e.ok()).collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        if out.len() >= max_files {
            return;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        let path = entry.path();
        let Ok(meta) = entry.metadata() else { continue };
        if meta.is_dir() {
            if !is_ignored_dir(&name) && !is_reparse(&path) {
                collect_source_files(root, &path, depth + 1, max_files, out);
            }
            continue;
        }
        if meta.len() > MAX_FILE_BYTES {
            continue;
        }
        let Ok(rel) = path.strip_prefix(root) else { continue };
        let rel_str = rel.to_string_lossy().replace('\\', "/");
        if let Ok(text) = fs::read_to_string(&path) {
            let sigs = extract_signatures_from_text(&text, 8);
            out.push((rel_str, text, sigs));
        }
    }
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
        budget.estimated_chars +=
            rel_str.len() + 24 + sigs.iter().map(|s| s.len() + 3).sum::<usize>();
        out.push((rel_str, meta.len(), lines, sigs));
        budget.remaining_files = budget.remaining_files.saturating_sub(1);
    }
}

#[cfg(windows)]
fn is_reparse(path: &Path) -> bool {
    use std::os::windows::fs::MetadataExt;
    match fs::symlink_metadata(path) {
        Ok(meta) => meta.file_attributes() & 0x400 != 0,
        Err(_) => true,
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

    #[test]
    fn pagerank_repo_map_ranks_referenced_core_and_finds_downstream_callers() {
        let root = std::env::temp_dir().join(format!("meg-idx-pr-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("Cargo.toml"), "[package]\nname = \"demo\"\n").unwrap();
        fs::write(root.join("src/core.rs"), "pub fn compute_hash() {}\n").unwrap();
        fs::write(root.join("src/a.rs"), "fn use_a() { compute_hash(); }\n").unwrap();
        fs::write(root.join("src/b.rs"), "fn use_b() { compute_hash(); }\n").unwrap();
        fs::write(root.join("src/leaf.rs"), "fn unused_leaf() {}\n").unwrap();

        let map = build_repo_map(&root, 1000);
        assert!(map.starts_with("<repo_map"), "{map}");
        let pos_core = map.find("src/core.rs").expect("core.rs in map");
        let pos_leaf = map.find("src/leaf.rs").expect("leaf.rs in map");
        assert!(pos_core < pos_leaf, "referenced core.rs should outrank leaf.rs:\n{map}");
        assert!(map.find("Cargo.toml").unwrap() < pos_leaf, "ROOT_IMPORTANT_FILES boosted");

        let callers = find_downstream_callers(&root, "compute_hash");
        let caller_files: Vec<&str> = callers.iter().map(|(f, _, _)| f.as_str()).collect();
        assert_eq!(caller_files, vec!["src/a.rs", "src/b.rs"]);
        let _ = fs::remove_dir_all(&root);
    }
}