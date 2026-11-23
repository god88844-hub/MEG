//! Content search (ripgrep-style) confined to the workspace.
//! Skips ignored directories, binary files, and oversized files; supports
//! regex, optional file glob, case-insensitive flag, context lines, and
//! cooperative cancellation.

use anyhow::{ensure, Result};
use regex::RegexBuilder;
use serde_json::Value;
use std::collections::BTreeSet;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

const MAX_FILE_BYTES: u64 = 512 * 1024;
const MAX_LINE_BYTES: usize = 300;
const BINARY_SNIFF_BYTES: usize = 8192;

pub fn grep_files(workspace: &Path, args: &Value) -> Result<String> {
    grep_files_cancellable(workspace, args, &AtomicBool::new(false))
}

pub fn grep_files_cancellable(
    workspace: &Path,
    args: &Value,
    cancel: &AtomicBool,
) -> Result<String> {
    let pattern = args
        .get("pattern")
        .or_else(|| args.get("query"))
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("missing 'pattern'"))?;
    ensure!(!pattern.is_empty(), "pattern cannot be empty");

    let case_insensitive = args
        .get("case_insensitive")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let context_lines = args
        .get("context_lines")
        .and_then(Value::as_u64)
        .unwrap_or(0)
        .min(5) as usize;
    let max_results = args
        .get("max_results")
        .and_then(Value::as_u64)
        .unwrap_or(50)
        .clamp(1, 200) as usize;

    let search_root = match args.get("path").and_then(Value::as_str) {
        Some(p) if !p.trim().is_empty() => crate::fs_tools::resolve(workspace, p)?,
        _ => workspace.to_path_buf(),
    };
    ensure!(
        search_root.exists(),
        "search path does not exist: {}",
        search_root.display()
    );

    let re = RegexBuilder::new(pattern)
        .case_insensitive(case_insensitive)
        .build()?;
    let glob_filter = match args.get("glob").and_then(Value::as_str) {
        Some(g) if !g.trim().is_empty() => Some(crate::fs_tools::globset_lite(g)?),
        _ => None,
    };

    let mut out_lines: Vec<String> = Vec::new();
    let mut match_count = 0usize;
    let mut truncated = false;

    for entry in walkdir::WalkDir::new(&search_root)
        .sort_by_file_name()
        .into_iter()
        .filter_entry(|e| e.depth() == 0 || !crate::fs_tools::is_ignored(e))
        .filter_map(|e| e.ok())
    {
        ensure!(!cancel.load(Ordering::SeqCst), "Run cancelled");
        if !entry.file_type().is_file() {
            continue;
        }
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if meta.len() > MAX_FILE_BYTES {
            continue;
        }
        let rel = entry.path().strip_prefix(workspace).unwrap_or(entry.path());
        let rel_str = rel.to_string_lossy().replace('\\', "/");
        if let Some(ref filter) = glob_filter {
            if !filter(&rel_str) {
                continue;
            }
        }
        let Ok(bytes) = std::fs::read(entry.path()) else {
            continue;
        };
        if bytes.iter().take(BINARY_SNIFF_BYTES).any(|&b| b == 0) {
            continue;
        }
        let Ok(text) = std::str::from_utf8(&bytes) else {
            continue;
        };
        let lines: Vec<&str> = text.lines().collect();
        let mut hit_indices: Vec<usize> = Vec::new();
        for (idx, line) in lines.iter().enumerate() {
            if re.is_match(line) {
                hit_indices.push(idx);
                match_count += 1;
                if match_count >= max_results {
                    truncated = true;
                    break;
                }
            }
        }
        if hit_indices.is_empty() {
            continue;
        }
        let hit_set: BTreeSet<usize> = hit_indices.iter().copied().collect();
        let mut emit_indices: BTreeSet<usize> = BTreeSet::new();
        for &idx in &hit_indices {
            let start = idx.saturating_sub(context_lines);
            let end = (idx + context_lines).min(lines.len().saturating_sub(1));
            for i in start..=end {
                emit_indices.insert(i);
            }
        }
        let mut prev: Option<usize> = None;
        for idx in emit_indices {
            if context_lines > 0 {
                if let Some(p) = prev {
                    if idx > p + 1 {
                        out_lines.push("--".to_string());
                    }
                }
            }
            let sep = if hit_set.contains(&idx) { ':' } else { '-' };
            let snippet = clip_line(lines[idx]);
            out_lines.push(format!("{rel_str}{sep}{}{sep}{snippet}", idx + 1));
            prev = Some(idx);
        }
        if truncated {
            break;
        }
    }

    if out_lines.is_empty() {
        return Ok("no matches".into());
    }
    if truncated {
        out_lines.push(format!("...[truncated at {max_results} matches]"));
    }
    Ok(out_lines.join("\n"))
}

fn clip_line(line: &str) -> String {
    if line.len() <= MAX_LINE_BYTES {
        line.to_string()
    } else {
        let end = line.floor_char_boundary(MAX_LINE_BYTES);
        format!("{}...", &line[..end])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::path::PathBuf;

    fn tempdir() -> PathBuf {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("meg-grep-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn finds_regex_matches_with_glob_and_skips_binary_and_ignored() {
        let ws = tempdir();
        std::fs::create_dir_all(ws.join("src")).unwrap();
        std::fs::create_dir_all(ws.join("target")).unwrap();
        std::fs::write(
            ws.join("src/main.rs"),
            "fn alpha() {}\nfn beta_target() {}\nfn gamma() {}\n",
        )
        .unwrap();
        std::fs::write(ws.join("src/notes.md"), "beta_target in docs\n").unwrap();
        std::fs::write(ws.join("src/bin.dat"), b"\0\x01beta_target\0").unwrap();
        std::fs::write(ws.join("target/gen.rs"), "fn beta_target() {}\n").unwrap();

        let out = grep_files(
            &ws,
            &json!({"pattern": "beta_[a-z]+", "glob": "**/*.rs"}),
        )
        .unwrap();
        assert_eq!(out, "src/main.rs:2:fn beta_target() {}");
        let _ = std::fs::remove_dir_all(&ws);
    }

    #[test]
    fn supports_case_insensitive_context_and_max_results() {
        let ws = tempdir();
        std::fs::write(
            ws.join("log.txt"),
            "line1\nERROR one\nline3\nline4\nerror two\nline6\n",
        )
        .unwrap();
        let out = grep_files(
            &ws,
            &json!({"pattern": "error", "case_insensitive": true, "context_lines": 1}),
        )
        .unwrap();
        assert!(out.contains("log.txt-1-line1"), "{out}");
        assert!(out.contains("log.txt:2:ERROR one"), "{out}");
        assert!(out.contains("log.txt:5:error two"), "{out}");

        let capped = grep_files(
            &ws,
            &json!({"pattern": "error", "case_insensitive": true, "max_results": 1}),
        )
        .unwrap();
        assert!(capped.contains("truncated at 1 matches"), "{capped}");
        let _ = std::fs::remove_dir_all(&ws);
    }

    #[test]
    fn respects_cancellation_and_workspace_confinement() {
        let ws = tempdir();
        std::fs::write(ws.join("a.txt"), "needle\n").unwrap();
        let cancel = AtomicBool::new(true);
        assert!(grep_files_cancellable(&ws, &json!({"pattern": "needle"}), &cancel)
            .unwrap_err()
            .to_string()
            .contains("cancelled"));
        assert!(grep_files(&ws, &json!({"pattern": "x", "path": "../outside"})).is_err());
        let _ = std::fs::remove_dir_all(&ws);
    }
}