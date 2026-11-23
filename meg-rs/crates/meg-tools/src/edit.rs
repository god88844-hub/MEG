//! edit_file -- exact str_replace with uniqueness law (Claude Code style):
//! old_string must match exactly and be unique; helpful errors when not.
//! Tolerates CRLF/LF mismatch on Windows while preserving the file's original
//! line endings.

use anyhow::{bail, Result};
use serde_json::Value;
use std::path::Path;

fn compact_preview(prefix: char, text: &str) -> String {
    const MAX_PREVIEW_LINES: usize = 6;
    let lines: Vec<&str> = text.lines().collect();
    let mut out = String::new();
    for line in lines.iter().take(MAX_PREVIEW_LINES) {
        out.push(prefix);
        out.push(' ');
        out.push_str(line);
        out.push('\n');
    }
    if lines.len() > MAX_PREVIEW_LINES {
        out.push_str(&format!(
            "{prefix} ... ({} more line(s))\n",
            lines.len() - MAX_PREVIEW_LINES
        ));
    }
    out
}

pub fn str_replace(workspace: &Path, args: &Value) -> Result<String> {
    let path = args
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing 'path'"))?;
    let old = args
        .get("old_string")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing 'old_string'"))?;
    let new = args
        .get("new_string")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing 'new_string'"))?;
    let replace_all = args
        .get("replace_all")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let full = super::fs_tools::resolve(workspace, path)?;
    let content = std::fs::read_to_string(&full)?;

    if old.is_empty() {
        bail!("old_string cannot be empty; use write_file for intentional full replacement");
    }

    // Prefer exact byte match; fall back to adapting line endings (LF <-> CRLF)
    // while preserving the file's original line-ending style.
    let exact = content.matches(old).count();
    let (effective_old, effective_new, count) = if exact > 0 {
        (old.to_string(), new.to_string(), exact)
    } else {
        let crlf_old = old.replace("\r\n", "\n").replace('\n', "\r\n");
        let crlf_count = if crlf_old != old {
            content.matches(&crlf_old).count()
        } else {
            0
        };
        if crlf_count > 0 {
            let crlf_new = new.replace("\r\n", "\n").replace('\n', "\r\n");
            (crlf_old, crlf_new, crlf_count)
        } else {
            let lf_old = old.replace("\r\n", "\n");
            let lf_count = if lf_old != old {
                content.matches(&lf_old).count()
            } else {
                0
            };
            if lf_count > 0 {
                let lf_new = new.replace("\r\n", "\n");
                (lf_old, lf_new, lf_count)
            } else {
                (old.to_string(), new.to_string(), 0)
            }
        }
    };

    if count == 0 {
        bail!(
            "old_string not found in {path}. It must match exactly, including whitespace and line endings."
        );
    }
    if count > 1 && !replace_all {
        bail!(
            "old_string matches {count} times in {path}. Make it unique or pass replace_all: true."
        );
    }
    let updated = if replace_all {
        content.replace(&effective_old, &effective_new)
    } else {
        content.replacen(&effective_old, &effective_new, 1)
    };
    std::fs::write(&full, &updated)?;
    let lines_new = new.lines().count();
    let lines_old = old.lines().count();
    let mut summary = format!(
        "edited {path}: {lines_old} line(s) replaced with {lines_new} line(s)"
    );
    let removed = compact_preview('-', &effective_old);
    let added = compact_preview('+', &effective_new);
    if !removed.is_empty() || !added.is_empty() {
        summary.push('\n');
        summary.push_str(&removed);
        summary.push_str(&added);
    }
    Ok(summary.trim_end().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::path::PathBuf;

    fn setup() -> PathBuf {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("meg-edit-{}-{}", std::process::id(), n));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("calc.py"), "def add(a, b):\n    return a - b\n").unwrap();
        dir
    }

    #[test]
    fn replaces_unique_match() {
        let ws = setup();
        let out = str_replace(
            &ws,
            &json!({
                "path": "calc.py", "old_string": "return a - b", "new_string": "return a + b"
            }),
        )
        .unwrap();
        assert!(out.contains("1 line"));
        assert!(out.contains("- return a - b"));
        assert!(out.contains("+ return a + b"));
        let text = std::fs::read_to_string(ws.join("calc.py")).unwrap();
        assert!(text.contains("return a + b"));
    }

    #[test]
    fn crlf_file_accepts_lf_edit_and_preserves_crlf() {
        let ws = setup();
        std::fs::write(
            ws.join("win.py"),
            "def add(a, b):\r\n    return a - b\r\n",
        )
        .unwrap();
        let out = str_replace(
            &ws,
            &json!({
                "path": "win.py",
                "old_string": "def add(a, b):\n    return a - b",
                "new_string": "def add(a, b):\n    return a + b"
            }),
        )
        .unwrap();
        assert!(out.contains("edited win.py"), "{out}");
        let raw = std::fs::read_to_string(ws.join("win.py")).unwrap();
        assert_eq!(raw, "def add(a, b):\r\n    return a + b\r\n");
    }

    #[test]
    fn crlf_uniqueness_still_enforced() {
        let ws = setup();
        std::fs::write(ws.join("dup.py"), "x = 1\r\ny = 2\r\nx = 1\r\ny = 2\r\n").unwrap();
        let err = str_replace(
            &ws,
            &json!({
                "path": "dup.py",
                "old_string": "x = 1\ny = 2",
                "new_string": "x = 3\ny = 4"
            }),
        );
        assert!(err.unwrap_err().to_string().contains("matches 2 times"));
    }

    #[test]
    fn rejects_non_unique_without_replace_all() {
        let ws = setup();
        std::fs::write(ws.join("calc.py"), "x = 1\nx = 1\n").unwrap();
        let err = str_replace(
            &ws,
            &json!({"path": "calc.py", "old_string": "x = 1", "new_string": "x = 2"}),
        );
        assert!(err.is_err());
    }

    #[test]
    fn replace_all_flag_works() {
        let ws = setup();
        std::fs::write(ws.join("calc.py"), "x = 1\nx = 1\n").unwrap();
        str_replace(&ws, &json!({"path": "calc.py", "old_string": "x = 1", "new_string": "x = 2", "replace_all": true})).unwrap();
        let text = std::fs::read_to_string(ws.join("calc.py")).unwrap();
        assert_eq!(text.matches("x = 2").count(), 2);
    }

    #[test]
    fn not_found_error_is_actionable() {
        let ws = setup();
        let err = str_replace(
            &ws,
            &json!({"path": "calc.py", "old_string": "NOPE", "new_string": "x"}),
        );
        let msg = err.unwrap_err().to_string();
        assert!(msg.contains("must match exactly"));
    }
}