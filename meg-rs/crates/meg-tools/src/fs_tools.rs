//! Filesystem tools: read (raw, 100-line bounded viewer, or hashline-tagged),
//! repeated unchanged-read suppression (`ReadCache`), write, list_dir, and
//! 50-hit capped glob search. Strictly workspace-confined.

use anyhow::{bail, ensure, Result};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

const MAX_READ_BYTES: usize = 50_000;
const MAX_LOAD_FILE_BYTES: u64 = 4 * 1024 * 1024;
const MAX_STREAM_SCAN_BYTES: usize = 16 * 1024 * 1024;
const MAX_LIST_SCAN_ENTRIES: usize = 10_000;
pub const DEFAULT_VIEW_LINES: usize = 100;
pub const MAX_SEARCH_HITS: usize = 50;

/// Tracks content hashes of previously read file slices to suppress wasteful
/// repeated identical reads while always returning full content on change or `force: true`.
type ReadCacheKey = (PathBuf, usize, usize, bool);
type ReadCacheEntry = (u64, usize, usize);

#[derive(Default)]
pub struct ReadCache {
    seen: Mutex<HashMap<ReadCacheKey, ReadCacheEntry>>,
}

impl ReadCache {
    pub fn new() -> Self {
        Self::default()
    }
}

fn fnv1a_64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

pub fn resolve(workspace: &Path, candidate: &str) -> Result<PathBuf> {
    resolve_inner(workspace, candidate)
}

fn resolve_inner(workspace: &Path, candidate: &str) -> Result<PathBuf> {
    let p = Path::new(candidate);
    if p.components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        bail!("path escapes workspace or contains parent traversal: {candidate}");
    }
    let full = if p.is_absolute() {
        PathBuf::from(candidate)
    } else {
        workspace.join(p)
    };
    fn strip_unc(p: PathBuf) -> PathBuf {
        let s = p.as_os_str().to_string_lossy().to_string();
        match s.strip_prefix(r"\\?\") {
            Some(stripped) => PathBuf::from(stripped),
            None => p,
        }
    }
    let mut ancestor = full.as_path();
    let mut suffix = Vec::new();
    while !ancestor.exists() {
        suffix.push(
            ancestor
                .file_name()
                .ok_or_else(|| anyhow::anyhow!("Invalid path"))?
                .to_owned(),
        );
        ancestor = ancestor
            .parent()
            .ok_or_else(|| anyhow::anyhow!("Invalid path"))?;
    }
    let mut resolved = ancestor.canonicalize()?;
    for component in suffix.iter().rev() {
        resolved.push(component);
    }
    let canon = strip_unc(resolved);
    let ws = strip_unc(
        workspace
            .canonicalize()
            .unwrap_or_else(|_| workspace.to_path_buf()),
    );
    if !canon.starts_with(&ws) {
        bail!("path escapes workspace: {candidate}");
    }
    Ok(canon)
}

/// Open a workspace file read-only with symlink TOCTOU verification:
/// checks `symlink_metadata` before open, opens the handle, and verifies the opened
/// handle's identity (`dev`/`ino` on Unix) and post-open canonical workspace confinement.
pub(crate) fn open_confined_readonly(workspace: &Path, full: &Path) -> Result<std::fs::File> {
    let pre_meta = std::fs::symlink_metadata(full)?;
    if pre_meta.file_type().is_symlink() {
        let target = std::fs::canonicalize(full)?;
        let ws = workspace
            .canonicalize()
            .unwrap_or_else(|_| workspace.to_path_buf());
        if !target.starts_with(&ws) {
            bail!("SYMLINK_TOCTOU_BLOCKED: symlink escapes workspace: {}", full.display());
        }
    }
    let file = std::fs::File::open(full)?;
    let opened_meta = file.metadata()?;
    if !opened_meta.is_file() {
        bail!("not a regular file: {}", full.display());
    }
    #[cfg(unix)]
    if !pre_meta.file_type().is_symlink() {
        use std::os::unix::fs::MetadataExt;
        if pre_meta.dev() != opened_meta.dev() || pre_meta.ino() != opened_meta.ino() {
            bail!(
                "SYMLINK_TOCTOU_BLOCKED: file identity changed between resolve and open: {}",
                full.display()
            );
        }
    }
    // Re-confirm canonical confinement after opening handle
    if let (Ok(post_canon), Ok(ws_canon)) = (std::fs::canonicalize(full), std::fs::canonicalize(workspace)) {
        if !post_canon.starts_with(&ws_canon) {
            bail!("SYMLINK_TOCTOU_BLOCKED: path escaped workspace after open: {}", full.display());
        }
    }
    Ok(file)
}

/// Read a workspace file. Supports `offset`, `limit` (default 100 lines when paginated),
/// `line_numbers: true`, and `hashlines: true` (`42#f9a | line`).
pub fn read_file(workspace: &Path, args: &Value) -> Result<String> {
    read_file_cached(workspace, args, None)
}

/// Same as `read_file`, plus repeated unchanged-read suppression via `ReadCache`.
pub fn read_file_cached(
    workspace: &Path,
    args: &Value,
    cache: Option<&ReadCache>,
) -> Result<String> {
    let path = args
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing 'path'"))?;
    let full = resolve(workspace, path)?;
    if crate::security_guards::is_blocked_env_path(&full) {
        bail!(
            "SECRET_AIRLOCK: direct read of '{path}' is blocked (.env* files cannot enter model context; use .env.example)"
        );
    }
    let offset = args.get("offset").and_then(Value::as_u64).map(|n| n as usize);
    let limit = args.get("limit").and_then(Value::as_u64).map(|n| n as usize);
    let hashlines = args
        .get("hashlines")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let force = args.get("force").and_then(Value::as_bool).unwrap_or(false);
    let numbered = hashlines
        || args
            .get("line_numbers")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        || offset.is_some()
        || limit.is_some();

    let file_len = std::fs::metadata(&full)?.len();
    if file_len > MAX_LOAD_FILE_BYTES {
        return read_oversized_file(&full, path, file_len, offset, limit, hashlines, numbered);
    }

    let raw_bytes = {
        use std::io::Read;
        let mut f = open_confined_readonly(workspace, &full)?;
        let mut buf = Vec::new();
        f.read_to_end(&mut buf)?;
        buf
    };
    let (text, _) = crate::edit::decode_with_bom(&raw_bytes)?;

    let content_hash = fnv1a_64(text.as_bytes());
    let total_lines = text.lines().count();
    let cache_key = (
        full.clone(),
        offset.unwrap_or(0),
        limit.unwrap_or(if numbered { DEFAULT_VIEW_LINES } else { 0 }),
        hashlines,
    );
    if !force {
        if let Some(c) = cache {
            let map = c.seen.lock().unwrap();
            if let Some(&(prev_hash, prev_lines, prev_bytes)) = map.get(&cache_key) {
                if prev_hash == content_hash {
                    return Ok(format!(
                        "[unchanged since last read: {path} (hash #{:06x}, {prev_lines} lines, {prev_bytes}B); pass force: true to re-read]",
                        content_hash & 0xffffff
                    ));
                }
            }
        }
    }
    if let Some(c) = cache {
        c.seen
            .lock()
            .unwrap()
            .insert(cache_key, (content_hash, total_lines, text.len()));
    }

    if numbered {
        let lines: Vec<&str> = text.lines().collect();
        let total = lines.len();
        if total == 0 {
            return Ok(format!("[{path}: empty file (0 lines)]"));
        }
        let start = offset.unwrap_or(1).max(1);
        if start > total {
            return Ok(format!(
                "[{path}: offset {start} is past end of file ({total} lines)]"
            ));
        }
        let count = limit.unwrap_or(DEFAULT_VIEW_LINES).clamp(1, 2_000);
        let end = (start - 1 + count).min(total);
        let mut out = format!("[{path}: lines {start}-{end} of {total}]\n");
        if hashlines {
            out.push_str(&crate::edit::format_hashlines(&lines[(start - 1)..end], start));
        } else {
            for (idx, line) in lines[(start - 1)..end].iter().enumerate() {
                out.push_str(&format!("{:>4} | {line}\n", start + idx));
            }
        }
        let out = out.trim_end_matches('\n').to_string();
        return Ok(cap_utf8(crate::security_guards::scrub_sal_observation(&out), MAX_READ_BYTES));
    }

    Ok(cap_utf8(crate::security_guards::scrub_sal_observation(&text), MAX_READ_BYTES))
}

fn read_oversized_file(
    full: &Path,
    path: &str,
    file_len: u64,
    offset: Option<usize>,
    limit: Option<usize>,
    hashlines: bool,
    numbered: bool,
) -> Result<String> {
    use std::io::{BufRead, BufReader, Read};
    let mut file = std::fs::File::open(full)?;
    if numbered {
        let start = offset.unwrap_or(1).max(1);
        let count = limit.unwrap_or(DEFAULT_VIEW_LINES).clamp(1, 2_000);
        let target_end = start.saturating_add(count).saturating_sub(1);
        let mut reader = BufReader::new(file);
        let mut collected: Vec<String> = Vec::new();
        let mut line_no = 0usize;
        let mut scanned_bytes = 0usize;
        let mut raw_line = Vec::new();
        let mut hit_scan_cap = false;
        while line_no < target_end {
            raw_line.clear();
            let n = reader.read_until(b'\n', &mut raw_line)?;
            if n == 0 {
                break;
            }
            scanned_bytes = scanned_bytes.saturating_add(n);
            line_no += 1;
            if line_no >= start {
                let s = String::from_utf8_lossy(&raw_line)
                    .trim_end_matches(&['\r', '\n'][..])
                    .to_string();
                collected.push(s);
            }
            if scanned_bytes >= MAX_STREAM_SCAN_BYTES {
                hit_scan_cap = true;
                break;
            }
        }
        if collected.is_empty() {
            if hit_scan_cap {
                return Ok(format!(
                    "[{path}: file is {file_len} bytes; stream scan reached {} MiB limit at line {line_no} before offset {start}; use grep_files to narrow down]",
                    MAX_STREAM_SCAN_BYTES / (1024 * 1024)
                ));
            }
            return Ok(format!(
                "[{path}: offset {start} is past end of file ({line_no} lines)]"
            ));
        }
        let end_no = start + collected.len() - 1;
        let mut out = format!("[{path}: lines {start}-{end_no} (streamed from {file_len}B file)]\n");
        let refs: Vec<&str> = collected.iter().map(String::as_str).collect();
        if hashlines {
            out.push_str(&crate::edit::format_hashlines(&refs, start));
        } else {
            for (idx, line) in refs.iter().enumerate() {
                out.push_str(&format!("{:>4} | {line}\n", start + idx));
            }
        }
        let out = out.trim_end_matches('\n').to_string();
        return Ok(cap_utf8(
            crate::security_guards::scrub_sal_observation(&out),
            MAX_READ_BYTES,
        ));
    }

    let mut buf = Vec::with_capacity(MAX_READ_BYTES.min(file_len as usize));
    file.by_ref().take(MAX_READ_BYTES as u64).read_to_end(&mut buf)?;
    let prefix = String::from_utf8_lossy(&buf).into_owned();
    let scrubbed = crate::security_guards::scrub_sal_observation(&prefix);
    let clipped = cap_utf8(scrubbed, MAX_READ_BYTES.saturating_sub(256));
    Ok(format!(
        "{clipped}\n...[file is {file_len} bytes (> {} MiB full-load cap); showing bounded prefix ? use offset/limit or grep_files]",
        MAX_LOAD_FILE_BYTES / (1024 * 1024)
    ))
}

fn cap_utf8(text: String, max_bytes: usize) -> String {
    if text.len() > max_bytes {
        let end = text.floor_char_boundary(max_bytes);
        format!("{}\n...[truncated {} bytes]", &text[..end], text.len() - end)
    } else {
        text
    }
}

pub fn write_file(workspace: &Path, args: &Value) -> Result<String> {
    let path = args
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing 'path'"))?;
    let content = args
        .get("content")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing 'content'"))?;
    let full = resolve(workspace, path)?;
    if crate::security_guards::is_blocked_env_path(&full) {
        bail!(
            "SECRET_AIRLOCK: direct write to '{path}' is blocked (.env* files are airlocked)"
        );
    }
    let hot_violations = crate::deliverable_guard::lint_hot_frame_file(&full, content);
    if !hot_violations.is_empty() {
        bail!("HOT_FRAME_LINTER_BLOCKED:\n{}", hot_violations.join("\n"));
    }
    if let Ok(meta) = std::fs::symlink_metadata(&full) {
        if meta.file_type().is_symlink() {
            bail!("SYMLINK_WRITE_BLOCKED: refusing to overwrite symlink '{path}'");
        }
    }
    if let Some(parent) = full.parent() {
        std::fs::create_dir_all(parent)?;
    }
    crate::edit::write_atomic(&full, content.as_bytes())?;
    let mut msg = format!("wrote {} bytes to {}", content.len(), path);
    if let Some(diags) = crate::lsp::collect_write_through_diagnostics(workspace, &full, content) {
        msg.push_str(&diags);
    }
    Ok(msg)
}

/// Bounded directory listing confined to the workspace.
pub fn list_dir(workspace: &Path, args: &Value) -> Result<String> {
    let rel_path = args.get("path").and_then(Value::as_str).unwrap_or(".");
    let include_ignored = args
        .get("include_ignored")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let max_entries = args
        .get("max_entries")
        .and_then(Value::as_u64)
        .unwrap_or(200)
        .clamp(1, 500) as usize;

    let full = resolve(workspace, rel_path)?;
    ensure!(full.is_dir(), "not a directory: {rel_path}");

    let mut items: Vec<(bool, String, u64)> = Vec::new();
    let mut scan_capped = false;
    for entry in std::fs::read_dir(&full)?.filter_map(|e| e.ok()) {
        if items.len() >= MAX_LIST_SCAN_ENTRIES {
            scan_capped = true;
            break;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if !include_ignored && is_ignored_name(&name) {
            continue;
        }
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        items.push((!meta.is_dir(), name, meta.len()));
    }
    items.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.to_lowercase().cmp(&b.1.to_lowercase())));

    if items.is_empty() {
        return Ok("(empty directory)".into());
    }
    let total = items.len();
    let mut lines = Vec::new();
    if scan_capped {
        lines.push(format!(
            "[directory scan capped at {MAX_LIST_SCAN_ENTRIES} entries; narrow path or use search_files]"
        ));
    }
    for (is_file, name, size) in items.into_iter().take(max_entries) {
        if is_file {
            lines.push(format!("file {name} ({size}B)"));
        } else {
            lines.push(format!("dir  {name}/"));
        }
    }
    if total > max_entries {
        lines.push(format!("...[truncated {} more entries]", total - max_entries));
    }
    Ok(lines.join("\n"))
}

pub fn search(workspace: &Path, args: &Value) -> Result<String> {
    search_cancellable(workspace, args, &std::sync::atomic::AtomicBool::new(false))
}

pub fn search_cancellable(
    workspace: &Path,
    args: &Value,
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<String> {
    let pattern = args
        .get("pattern")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing 'pattern'"))?;
    let max_hits = args
        .get("max_results")
        .and_then(Value::as_u64)
        .map(|n| (n as usize).clamp(1, 200))
        .unwrap_or(MAX_SEARCH_HITS);
    let mut matches = Vec::new();
    let mut overflow = 0usize;
    let glob_set = globset_lite(pattern)?;
    for entry in walkdir::WalkDir::new(workspace)
        .sort_by_file_name()
        .into_iter()
        .filter_entry(|e| !is_ignored(e))
        .filter_map(|e| e.ok())
    {
        anyhow::ensure!(
            !cancel.load(std::sync::atomic::Ordering::SeqCst),
            "Run cancelled"
        );
        if entry.file_type().is_file() {
            let rel = entry.path().strip_prefix(workspace).unwrap_or(entry.path());
            let rel_str = rel.to_string_lossy().replace('\\', "/");
            if glob_set(&rel_str) {
                if matches.len() < max_hits {
                    matches.push(rel_str);
                } else {
                    overflow += 1;
                }
            }
        }
    }
    if matches.is_empty() {
        return Ok("no matches".into());
    }
    if overflow > 0 {
        matches.push(format!(
            "...[capped at {max_hits} hits (+{overflow} more); narrow your pattern]"
        ));
    }
    Ok(matches.join("\n"))
}

pub(crate) fn is_ignored_name(name: &str) -> bool {
    matches!(
        name,
        ".git"
            | "target"
            | "node_modules"
            | "__pycache__"
            | ".venv"
            | "venv"
            | ".meg"
            | ".loop"
            | ".cache"
            | ".pytest_cache"
            | ".mypy_cache"
            | ".ruff_cache"
    )
}

pub(crate) fn is_ignored(entry: &walkdir::DirEntry) -> bool {
    entry.file_name().to_str().map(is_ignored_name).unwrap_or(false)
}

/// Glob matcher: `*` within a segment, `**` across zero or more segments, `?` single non-slash char.
pub(crate) fn globset_lite(pattern: &str) -> Result<impl Fn(&str) -> bool> {
    let normalized = pattern.replace('\\', "/");
    let chars: Vec<char> = normalized.chars().collect();
    let mut regex = String::from("^");
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '*' && i + 1 < chars.len() && chars[i + 1] == '*' {
            if i + 2 < chars.len() && chars[i + 2] == '/' {
                regex.push_str("(?:.*/)?");
                i += 3;
            } else {
                regex.push_str(".*");
                i += 2;
            }
            continue;
        }
        match chars[i] {
            '*' => regex.push_str("[^/]*"),
            '?' => regex.push_str("[^/]"),
            '.' | '(' | ')' | '+' | '|' | '^' | '$' | '[' | ']' | '{' | '}' | '\\' => {
                regex.push('\\');
                regex.push(chars[i]);
            }
            ch => regex.push(ch),
        }
        i += 1;
    }
    regex.push('$');
    let re = regex::Regex::new(&regex)?;
    Ok(move |s: &str| re.is_match(s))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn read_write_round_trip() {
        let ws = tempdir();
        write_file(&ws, &json!({"path": "a/b.txt", "content": "hello"})).unwrap();
        let out = read_file(&ws, &json!({"path": "a/b.txt"})).unwrap();
        assert_eq!(out, "hello");
    }

    #[test]
    fn read_file_supports_line_numbered_pagination() {
        let ws = tempdir();
        write_file(
            &ws,
            &json!({"path": "nums.txt", "content": "one\ntwo\nthree\nfour\nfive\n"}),
        )
        .unwrap();
        let slice = read_file(&ws, &json!({"path": "nums.txt", "offset": 2, "limit": 2})).unwrap();
        assert!(slice.starts_with("[nums.txt: lines 2-3 of 5]"), "{slice}");
        assert!(slice.contains("   2 | two"), "{slice}");
        assert!(slice.contains("   3 | three"), "{slice}");
        assert!(!slice.contains("four"), "{slice}");

        let past = read_file(&ws, &json!({"path": "nums.txt", "offset": 99})).unwrap();
        assert!(past.contains("past end of file"), "{past}");
    }

    #[test]
    fn aci_100_line_default_hashlines_and_repeated_read_suppression() {
        let ws = tempdir();
        let body: String = (1..=140).map(|i| format!("line-{i}\n")).collect();
        write_file(&ws, &json!({"path": "big.txt", "content": body})).unwrap();

        let cache = ReadCache::new();
        let first = read_file_cached(
            &ws,
            &json!({"path": "big.txt", "offset": 1, "hashlines": true}),
            Some(&cache),
        )
        .unwrap();
        assert!(first.starts_with("[big.txt: lines 1-100 of 140]"), "{first}");
        assert!(first.contains("1#"), "{first}");
        assert!(!first.contains("101#"), "default view must cap at 100 lines");

        // Second identical read is suppressed with a short pointer.
        let second = read_file_cached(
            &ws,
            &json!({"path": "big.txt", "offset": 1, "hashlines": true}),
            Some(&cache),
        )
        .unwrap();
        assert!(second.starts_with("[unchanged since last read: big.txt"), "{second}");

        // Force bypasses suppression.
        let forced = read_file_cached(
            &ws,
            &json!({"path": "big.txt", "offset": 1, "hashlines": true, "force": true}),
            Some(&cache),
        )
        .unwrap();
        assert!(forced.starts_with("[big.txt: lines 1-100 of 140]"));
    }

    #[test]
    fn search_summarizes_at_50_hits() {
        let ws = tempdir();
        for i in 0..60 {
            write_file(&ws, &json!({"path": format!("f{i:02}.rs"), "content": ""})).unwrap();
        }
        let out = search(&ws, &json!({"pattern": "**/*.rs"})).unwrap();
        assert!(out.contains("capped at 50 hits (+10 more)"), "{out}");
    }

    #[test]
    fn list_dir_sorts_dirs_first_and_skips_ignored() {
        let ws = tempdir();
        std::fs::create_dir_all(ws.join("src")).unwrap();
        std::fs::create_dir_all(ws.join("target")).unwrap();
        std::fs::write(ws.join("Cargo.toml"), "[workspace]\n").unwrap();
        let out = list_dir(&ws, &json!({})).unwrap();
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 2, "{out}");
        assert_eq!(lines[0], "dir  src/");
        assert!(lines[1].starts_with("file Cargo.toml ("), "{out}");
        assert!(!out.contains("target"), "{out}");
    }

    #[test]
    fn workspace_escape_rejected() {
        let ws = tempdir();
        let err = read_file(&ws, &json!({"path": "../outside.txt"}));
        assert!(err.is_err());
        assert!(list_dir(&ws, &json!({"path": ".."})).is_err());
    }

    #[test]
    fn glob_search() {
        let ws = tempdir();
        write_file(
            &ws,
            &json!({"path": "src/main.rs", "content": "fn main(){}"}),
        )
        .unwrap();
        let out = search(&ws, &json!({"pattern": "**/*.rs"})).unwrap();
        assert!(out.contains("src/main.rs"));
    }

    #[test]
    fn recursive_glob_matches_root_shallow_and_deep_paths() {
        let ws = tempdir();
        write_file(&ws, &json!({"path": "root.rs", "content": ""})).unwrap();
        write_file(&ws, &json!({"path": "src/lib.rs", "content": ""})).unwrap();
        write_file(&ws, &json!({"path": "src/a/b/c/deep.rs", "content": ""})).unwrap();
        write_file(&ws, &json!({"path": "src/a/b/c/notes.txt", "content": ""})).unwrap();

        let all_rs = search(&ws, &json!({"pattern": "**/*.rs"})).unwrap();
        assert!(all_rs.contains("root.rs"), "missing root.rs: {all_rs}");
        assert!(all_rs.contains("src/lib.rs"), "missing src/lib.rs: {all_rs}");
        assert!(all_rs.contains("src/a/b/c/deep.rs"), "missing deep.rs: {all_rs}");
        assert!(!all_rs.contains("notes.txt"), "unexpected txt: {all_rs}");

        let src_only = search(&ws, &json!({"pattern": "src/**/*.rs"})).unwrap();
        assert!(!src_only.contains("root.rs"), "root.rs should not match src/**/*.rs: {src_only}");
        assert!(src_only.contains("src/lib.rs"));
        assert!(src_only.contains("src/a/b/c/deep.rs"));

        let root_only = search(&ws, &json!({"pattern": "*.rs"})).unwrap();
        assert!(root_only.contains("root.rs"));
        assert!(!root_only.contains("src/lib.rs"), "single * must not cross /: {root_only}");
    }

    fn tempdir() -> PathBuf {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("meg-tools-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn adversarial_read_file_decodes_utf16le_bom() {
        let dir = std::env::temp_dir().join(format!(
            "meg-utf16-read-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let mut bytes = vec![0xFFu8, 0xFEu8];
        for u in "hello utf16\r\nline 2\r\n".encode_utf16() {
            bytes.extend_from_slice(&u.to_le_bytes());
        }
        std::fs::write(dir.join("note.txt"), &bytes).unwrap();
        let out = read_file(&dir, &serde_json::json!({ "path": "note.txt" }))
            .expect("read_file must support UTF-16LE BOM files");
        assert!(out.contains("hello utf16"), "{out}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn oversized_file_uses_bounded_prefix_and_streamed_pagination() {
        let dir = std::env::temp_dir().join(format!("meg-oversized-read-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let big = dir.join("huge.log");
        let line = "alpha beta gamma delta epsilon zeta eta theta iota kappa";
        let repeats = (MAX_LOAD_FILE_BYTES as usize / line.len()) + 50;
        let mut f = std::fs::File::create(&big).unwrap();
        use std::io::Write;
        for i in 1..=repeats {
            writeln!(f, "line-{i}: {line}").unwrap();
        }
        drop(f);

        let raw = read_file(&dir, &serde_json::json!({"path": "huge.log"})).unwrap();
        assert!(raw.contains("full-load cap"), "{raw}");
        assert!(raw.len() <= MAX_READ_BYTES + 256);

        let page = read_file(
            &dir,
            &serde_json::json!({"path": "huge.log", "offset": 10, "limit": 3, "hashlines": true}),
        )
        .unwrap();
        assert!(page.contains("streamed from"), "{page}");
        assert!(page.contains("line-10:"), "{page}");
        assert!(page.contains("line-12:"), "{page}");
        assert!(!page.contains("line-13:"), "{page}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn symlink_escape_and_write_blocked() {
        let ws = std::env::temp_dir().join(format!("meg-toctou-ws-{}", std::process::id()));
        let outside = std::env::temp_dir().join(format!("meg-toctou-out-{}.txt", std::process::id()));
        let _ = std::fs::remove_dir_all(&ws);
        std::fs::create_dir_all(&ws).unwrap();
        std::fs::write(&outside, "outside secret").unwrap();
        let link = ws.join("escape_link.txt");
        #[cfg(unix)]
        let linked = std::os::unix::fs::symlink(&outside, &link).is_ok();
        #[cfg(windows)]
        let linked = std::os::windows::fs::symlink_file(&outside, &link).is_ok();
        if linked {
            assert!(
                read_file(&ws, &json!({"path": "escape_link.txt"})).is_err(),
                "symlink escaping workspace must be rejected on read"
            );
            assert!(
                write_file(&ws, &json!({"path": "escape_link.txt", "content": "x"})).is_err(),
                "symlink escaping workspace must be rejected on write"
            );
        }
        let _ = std::fs::remove_dir_all(&ws);
        let _ = std::fs::remove_file(&outside);
    }
}
