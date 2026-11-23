//! Filesystem tools: read, write, search (glob). Workspace-confined.

use anyhow::{bail, Result};
use serde_json::Value;
use std::path::{Path, PathBuf};

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
    // Windows canonicalize() returns \\?\-prefixed paths for existing files
    // while non-existent paths stay plain - normalize both before comparing.
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
    Ok(full)
}

pub fn read_file(workspace: &Path, args: &Value) -> Result<String> {
    let path = args
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing 'path'"))?;
    let full = resolve(workspace, path)?;
    let text = std::fs::read_to_string(&full)?;
    if text.len() > 50_000 {
        let end = text.floor_char_boundary(50_000);
        let out = format!(
            "{}\n...[truncated {} bytes]",
            &text[..end],
            text.len() - end
        );
        return Ok(out);
    }
    Ok(text)
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
    if let Some(parent) = full.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&full, content)?;
    Ok(format!("wrote {} bytes to {}", content.len(), path))
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
    let mut matches = Vec::new();
    let glob_set = globset_lite(pattern)?;
    for entry in walkdir::WalkDir::new(workspace)
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
                matches.push(rel_str);
                if matches.len() >= 100 {
                    break;
                }
            }
        }
    }
    if matches.is_empty() {
        return Ok("no matches".into());
    }
    Ok(matches.join("\n"))
}

fn is_ignored(entry: &walkdir::DirEntry) -> bool {
    entry
        .file_name()
        .to_str()
        .map(|n| {
            matches!(
                n,
                ".git"
                    | "target"
                    | "node_modules"
                    | "__pycache__"
                    | ".venv"
                    | "venv"
                    | ".meg"
                    | ".cache"
            )
        })
        .unwrap_or(false)
}

/// Tiny glob: `*` within a segment, `**` across segments, `?` single char.
fn globset_lite(pattern: &str) -> Result<impl Fn(&str) -> bool> {
    let mut regex = String::from("^");
    for ch in pattern.chars() {
        match ch {
            '*' => regex.push_str("[^/]*"),
            '?' => regex.push('.'),
            '.' | '(' | ')' | '+' | '|' | '^' | '$' | '[' | ']' | '{' | '}' | '\\' => {
                regex.push('\\');
                regex.push(ch);
            }
            _ => regex.push(ch),
        }
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
    fn workspace_escape_rejected() {
        let ws = tempdir();
        let err = read_file(&ws, &json!({"path": "../outside.txt"}));
        assert!(err.is_err());
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

    fn tempdir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("meg-tools-{}", std::process::id()));
        let dir = dir.join(format!(
            "{:?}",
            std::time::SystemTime::now().elapsed().unwrap().as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }
}
