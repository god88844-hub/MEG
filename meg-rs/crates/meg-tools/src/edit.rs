//! edit_file — exact str_replace with uniqueness law (Claude Code style):
//! old_string must match exactly and be unique; helpful errors when not.

use anyhow::{bail, Result};
use serde_json::Value;
use std::path::Path;

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
    let count = content.matches(old).count();
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
        content.replace(old, new)
    } else {
        content.replacen(old, new, 1)
    };
    std::fs::write(&full, &updated)?;
    let lines_new = new.lines().count();
    let lines_old = old.lines().count();
    Ok(format!(
        "edited {path}: {lines_old} line(s) replaced with {lines_new} line(s)"
    ))
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
        let text = std::fs::read_to_string(ws.join("calc.py")).unwrap();
        assert!(text.contains("return a + b"));
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
