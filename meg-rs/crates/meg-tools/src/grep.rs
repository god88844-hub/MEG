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

struct GitIgnoreRule {
    negated: bool,
    dir_only: bool,
    anchored: bool,
    matcher: regex::Regex,
}

fn compile_gitignore_pattern(pat: &str) -> Option<regex::Regex> {
    let mut rx = String::from("^");
    let chars: Vec<char> = pat.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '*' if i + 1 < chars.len() && chars[i + 1] == '*' => {
                rx.push_str(".*");
                i += 2;
                if i < chars.len() && chars[i] == '/' {
                    rx.push_str("(?:/)?");
                    i += 1;
                }
            }
            '*' => {
                rx.push_str("[^/]*");
                i += 1;
            }
            '?' => {
                rx.push_str("[^/]");
                i += 1;
            }
            c => {
                rx.push_str(&regex::escape(&c.to_string()));
                i += 1;
            }
        }
    }
    rx.push('$');
    regex::Regex::new(&rx).ok()
}

fn load_gitignore_rules(workspace: &Path) -> Vec<GitIgnoreRule> {
    let mut rules = Vec::new();
    for name in [".gitignore", ".ignore"] {
        let p = workspace.join(name);
        let Ok(content) = std::fs::read_to_string(&p) else {
            continue;
        };
        for raw_line in content.lines() {
            let line = raw_line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (negated, rest) = match line.strip_prefix('!') {
                Some(r) => (true, r.trim()),
                None => (false, line),
            };
            if rest.is_empty() {
                continue;
            }
            let dir_only = rest.ends_with('/');
            let rest = rest.trim_end_matches('/');
            let anchored = rest.starts_with('/') || rest.contains('/');
            let clean = rest.trim_start_matches('/');
            if let Some(matcher) = compile_gitignore_pattern(clean) {
                rules.push(GitIgnoreRule {
                    negated,
                    dir_only,
                    anchored,
                    matcher,
                });
            }
        }
    }
    rules
}

fn is_gitignored(rules: &[GitIgnoreRule], rel_slash: &str, is_dir: bool) -> bool {
    let mut ignored = false;
    let file_name = rel_slash.rsplit('/').next().unwrap_or(rel_slash);
    for rule in rules {
        if rule.dir_only && !is_dir {
            continue;
        }
        let matched = if rule.anchored {
            rule.matcher.is_match(rel_slash)
        } else {
            rule.matcher.is_match(file_name) || rule.matcher.is_match(rel_slash)
        };
        if matched {
            ignored = !rule.negated;
        }
    }
    ignored
}

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
    let mut skipped_oversized = 0usize;
    let mut skipped_binary = 0usize;
    let mut skipped_non_utf8 = 0usize;
    let mut skipped_unreadable = 0usize;

    if crate::security_guards::is_blocked_env_path(&search_root) {
        anyhow::bail!(
            "SECRET_AIRLOCK: grep_files of '{}' is blocked (.env* files cannot enter model context)",
            search_root.display()
        );
    }

    let gitignore_rules = load_gitignore_rules(workspace);
    for entry in walkdir::WalkDir::new(&search_root)
        .sort_by_file_name()
        .into_iter()
        .filter_entry(|e| {
            if e.depth() == 0 {
                return true;
            }
            if crate::fs_tools::is_ignored(e) {
                return false;
            }
            if !gitignore_rules.is_empty() {
                let rel = e.path().strip_prefix(workspace).unwrap_or(e.path());
                let rel_slash = rel.to_string_lossy().replace('\\', "/");
                if is_gitignored(&gitignore_rules, &rel_slash, e.file_type().is_dir()) {
                    return false;
                }
            }
            true
        })
        .filter_map(|e| e.ok())
    {
        ensure!(!cancel.load(Ordering::SeqCst), "Run cancelled");
        if !entry.file_type().is_file() || crate::security_guards::is_blocked_env_path(entry.path()) {
            continue;
        }
        let Ok(meta) = entry.metadata() else {
            skipped_unreadable += 1;
            continue;
        };
        let rel = entry.path().strip_prefix(workspace).unwrap_or(entry.path());
        let rel_str = rel.to_string_lossy().replace('\\', "/");
        if let Some(ref filter) = glob_filter {
            if !filter(&rel_str) {
                continue;
            }
        }
        if meta.len() > MAX_FILE_BYTES {
            skipped_oversized += 1;
            continue;
        }
        let Ok(bytes) = std::fs::read(entry.path()) else {
            skipped_unreadable += 1;
            continue;
        };
        if bytes.iter().take(BINARY_SNIFF_BYTES).any(|&b| b == 0) {
            skipped_binary += 1;
            continue;
        }
        let Ok(text) = std::str::from_utf8(&bytes) else {
            skipped_non_utf8 += 1;
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
            let snippet = crate::security_guards::scrub_sal_observation(&clip_line(lines[idx]));
            out_lines.push(format!("{rel_str}{sep}{}{sep}{snippet}", idx + 1));
            prev = Some(idx);
        }
        if truncated {
            break;
        }
    }

    let mut skip_parts = Vec::new();
    if skipped_oversized > 0 {
        skip_parts.push(format!("{skipped_oversized} oversized (>512 KiB)"));
    }
    if skipped_binary > 0 {
        skip_parts.push(format!("{skipped_binary} binary"));
    }
    if skipped_non_utf8 > 0 {
        skip_parts.push(format!("{skipped_non_utf8} non-UTF-8"));
    }
    if skipped_unreadable > 0 {
        skip_parts.push(format!("{skipped_unreadable} unreadable"));
    }
    if out_lines.is_empty() {
        if skip_parts.is_empty() {
            return Ok("no matches".into());
        }
        return Ok(format!("no matches [skipped: {}]", skip_parts.join(", ")));
    }
    if truncated {
        out_lines.push(format!("...[truncated at {max_results} matches]"));
    }
    if !skip_parts.is_empty() {
        out_lines.push(format!("[note: skipped {}]", skip_parts.join(", ")));
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

    #[test]
    fn adversarial_grep_blocks_dotenv_and_scrubs_secrets() {
        let dir = std::env::temp_dir().join(format!("meg-grep-sec-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(".env"), "OPENAI_API_KEY=sk-proj-secret1234567890abcdef12345678\n").unwrap();
        std::fs::write(dir.join(".env.local"), "DB_PASS=super_secret_env_local_value\n").unwrap();
        std::fs::write(
            dir.join("app.rs"),
            "const TOKEN: &str = \"ghp_1234567890abcdef1234567890abcdef1234\";\n",
        )
        .unwrap();

        // 1. Direct grep targeting .env must be rejected
        let direct_err = grep_files(&dir, &serde_json::json!({"pattern": "OPENAI", "path": ".env"}));
        assert!(direct_err.is_err());
        assert!(direct_err.unwrap_err().to_string().contains("SECRET_AIRLOCK"));

        // 2. Workspace-wide grep must never return .env or .env.local lines
        let env_scan = grep_files(&dir, &serde_json::json!({"pattern": "OPENAI|DB_PASS"})).unwrap();
        assert!(!env_scan.contains("sk-proj-"));
        assert!(!env_scan.contains("super_secret_env_local_value"));

        // 3. Hardcoded secrets in regular source files must be SAL-scrubbed
        let src_scan = grep_files(&dir, &serde_json::json!({"pattern": "TOKEN"})).unwrap();
        assert!(src_scan.contains("app.rs:1:"));
        assert!(!src_scan.contains("ghp_1234567890abcdef1234567890abcdef1234"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn adversarial_grep_respects_gitignore_and_reports_skipped_files() {
        let dir = std::env::temp_dir().join(format!("meg-grep-c05-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("ignored_dir")).unwrap();
        std::fs::write(dir.join(".gitignore"), "ignored_dir/\n*.gen.txt\n").unwrap();
        std::fs::write(dir.join("ignored_dir/a.txt"), "NEEDLE_HIDDEN\n").unwrap();
        std::fs::write(dir.join("skip.gen.txt"), "NEEDLE_HIDDEN\n").unwrap();
        std::fs::write(dir.join("keep.txt"), "NEEDLE_VISIBLE\n").unwrap();
        std::fs::write(dir.join("blob.bin"), b"\x00\x01NEEDLE_BIN").unwrap();
        std::fs::write(dir.join("huge.txt"), "A".repeat(600 * 1024)).unwrap();

        let out = grep_files(&dir, &serde_json::json!({"pattern": "NEEDLE"})).unwrap();
        assert!(out.contains("keep.txt:1:NEEDLE_VISIBLE"), "{out}");
        assert!(!out.contains("NEEDLE_HIDDEN"), "gitignored files leaked: {out}");
        assert!(out.contains("1 oversized"), "missing oversized skip note: {out}");
        assert!(out.contains("1 binary"), "missing binary skip note: {out}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
