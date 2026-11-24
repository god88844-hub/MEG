//! Hashline reader/patcher (`42#f9a`), exact str_replace, and unified-diff fallback.
//! Preserves UTF-8/UTF-16 BOM and CRLF/LF line endings; writes atomically via temp file.

use anyhow::{bail, ensure, Result};
use serde_json::Value;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::fs_tools::resolve;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TextEncoding {
    Utf8,
    Utf8Bom,
    Utf16LeBom,
    Utf16BeBom,
}

fn decode_with_bom(bytes: &[u8]) -> Result<(String, TextEncoding)> {
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        let s = std::str::from_utf8(&bytes[3..])?.to_string();
        return Ok((s, TextEncoding::Utf8Bom));
    }
    if bytes.starts_with(&[0xFF, 0xFE]) && bytes.len() % 2 == 0 {
        let u16s: Vec<u16> = bytes[2..]
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        return Ok((String::from_utf16(&u16s)?, TextEncoding::Utf16LeBom));
    }
    if bytes.starts_with(&[0xFE, 0xFF]) && bytes.len() % 2 == 0 {
        let u16s: Vec<u16> = bytes[2..]
            .chunks_exact(2)
            .map(|c| u16::from_be_bytes([c[0], c[1]]))
            .collect();
        return Ok((String::from_utf16(&u16s)?, TextEncoding::Utf16BeBom));
    }
    Ok((std::str::from_utf8(bytes)?.to_string(), TextEncoding::Utf8))
}

fn encode_with_bom(text: &str, enc: TextEncoding) -> Vec<u8> {
    match enc {
        TextEncoding::Utf8 => text.as_bytes().to_vec(),
        TextEncoding::Utf8Bom => {
            let mut out = Vec::with_capacity(3 + text.len());
            out.extend_from_slice(&[0xEF, 0xBB, 0xBF]);
            out.extend_from_slice(text.as_bytes());
            out
        }
        TextEncoding::Utf16LeBom => {
            let mut out = vec![0xFF, 0xFE];
            for u in text.encode_utf16() {
                out.extend_from_slice(&u.to_le_bytes());
            }
            out
        }
        TextEncoding::Utf16BeBom => {
            let mut out = vec![0xFE, 0xFF];
            for u in text.encode_utf16() {
                out.extend_from_slice(&u.to_be_bytes());
            }
            out
        }
    }
}

fn write_atomic(full: &Path, bytes: &[u8]) -> Result<()> {
    static TMP_SEQ: AtomicU64 = AtomicU64::new(0);
    let seq = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let file_name = full
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("edit");
    let tmp = full.with_file_name(format!(".{file_name}.megtmp.{}.{seq}", std::process::id()));
    if let Err(err) = std::fs::write(&tmp, bytes).and_then(|_| std::fs::rename(&tmp, full)) {
        let _ = std::fs::remove_file(&tmp);
        return Err(err.into());
    }
    Ok(())
}

/// Deterministic 3-hex FNV-1a content hash for a single line (stripped of `\r`).
pub fn line_hash3(line: &str) -> String {
    let clean = line.strip_suffix('\r').unwrap_or(line);
    let mut h: u32 = 0x811c9dc5;
    for &b in clean.as_bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(0x01000193);
    }
    format!("{:03x}", h & 0x0fff)
}

/// Format lines with `line#hash | content` tags (1-based `start_line`).
pub fn format_hashlines(lines: &[&str], start_line: usize) -> String {
    let mut out = String::new();
    for (idx, line) in lines.iter().enumerate() {
        let n = start_line + idx;
        let h = line_hash3(line);
        out.push_str(&format!("{n}#{h} | {line}\n"));
    }
    out.trim_end_matches('\n').to_string()
}

fn parse_anchor(anchor: &str) -> Result<(usize, String)> {
    let (num_s, hash_s) = anchor
        .trim()
        .split_once('#')
        .ok_or_else(|| anyhow::anyhow!("invalid hashline anchor '{anchor}': expected '<line>#<3-hex>'"))?;
    let line_no: usize = num_s
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid line number in anchor '{anchor}'"))?;
    ensure!(line_no >= 1, "line number must be >= 1 in anchor '{anchor}'");
    let hash = hash_s.trim().to_ascii_lowercase();
    ensure!(
        hash.len() == 3 && hash.chars().all(|c| c.is_ascii_hexdigit()),
        "hash must be 3 hex chars in anchor '{anchor}'"
    );
    Ok((line_no, hash))
}

/// Apply validated hashline edits atomically. Rejects stale/mismatched anchors before writing.
pub fn apply_hashline_patch(workspace: &Path, args: &Value) -> Result<String> {
    let path = args
        .get("path")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("missing 'path'"))?;
    let edits = args
        .get("edits")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow::anyhow!("missing 'edits' array"))?;
    ensure!(!edits.is_empty(), "'edits' array cannot be empty");

    let full = resolve(workspace, path)?;
    let raw_bytes = std::fs::read(&full)?;
    let (content, enc) = decode_with_bom(&raw_bytes)?;
    let uses_crlf = content.contains("\r\n");
    let trailing_nl = content.ends_with('\n');
    let normalized = if uses_crlf {
        content.replace("\r\n", "\n")
    } else {
        content
    };
    let mut lines: Vec<String> = normalized.lines().map(str::to_string).collect();

    // Parse & validate ALL anchors up front before mutating anything.
    let mut parsed_ops: Vec<(usize, usize, Vec<String>)> = Vec::with_capacity(edits.len());
    for op in edits {
        let start_raw = op
            .get("start_anchor")
            .or_else(|| op.get("anchor"))
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("each hashline edit requires 'start_anchor' (or 'anchor')"))?;
        let (start_no, start_hash) = parse_anchor(start_raw)?;
        ensure!(
            start_no <= lines.len(),
            "stale anchor '{start_raw}': file {path} has only {} lines",
            lines.len()
        );
        let actual_start = line_hash3(&lines[start_no - 1]);
        ensure!(
            actual_start == start_hash,
            "stale hashline anchor '{start_raw}' in {path}: actual is {start_no}#{actual_start}"
        );

        let (end_no, end_hash) = match op.get("end_anchor").and_then(Value::as_str) {
            Some(e) => parse_anchor(e)?,
            None => (start_no, start_hash.clone()),
        };
        ensure!(
            end_no >= start_no && end_no <= lines.len(),
            "invalid end_anchor line {end_no} for range starting at {start_no}"
        );
        let actual_end = line_hash3(&lines[end_no - 1]);
        ensure!(
            actual_end == end_hash,
            "stale end_anchor '{end_no}#{end_hash}' in {path}: actual is {end_no}#{actual_end}"
        );

        let new_content = op
            .get("new_content")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("each hashline edit requires 'new_content'"))?;
        let replacement: Vec<String> = if new_content.is_empty() {
            Vec::new()
        } else {
            new_content
                .replace("\r\n", "\n")
                .lines()
                .map(str::to_string)
                .collect()
        };
        parsed_ops.push((start_no, end_no, replacement));
    }

    // Reject overlapping ranges and apply bottom-up so line indices stay stable.
    parsed_ops.sort_by_key(|a| a.0);
    for w in parsed_ops.windows(2) {
        ensure!(
            w[0].1 < w[1].0,
            "overlapping hashline edit ranges: {}..={} and {}..={}",
            w[0].0,
            w[0].1,
            w[1].0,
            w[1].1
        );
    }
    let op_count = parsed_ops.len();
    for (start_no, end_no, replacement) in parsed_ops.into_iter().rev() {
        lines.splice((start_no - 1)..end_no, replacement);
    }

    let mut joined = lines.join("\n");
    if trailing_nl && !joined.is_empty() {
        joined.push('\n');
    }
    let final_text = if uses_crlf {
        joined.replace('\n', "\r\n")
    } else {
        joined
    };
    write_atomic(&full, &encode_with_bom(&final_text, enc))?;
    Ok(format!("hashline-patched {path} ({op_count} edit(s))"))
}

/// Edit a file: dispatches to hashline patch when `edits` is present, otherwise
/// performs exact `str_replace` with uniqueness enforcement and BOM/CRLF preservation.
pub fn str_replace(workspace: &Path, args: &Value) -> Result<String> {
    if args.get("edits").is_some() {
        return apply_hashline_patch(workspace, args);
    }
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

    ensure!(!old.is_empty(), "old_string cannot be empty");
    let full = resolve(workspace, path)?;
    let raw_bytes = std::fs::read(&full)?;
    let (content, enc) = decode_with_bom(&raw_bytes)?;

    let (count, updated) = match content.matches(old).count() {
        n if n > 0 => {
            if n > 1 && !replace_all {
                bail!(
                    "old_string appears {} times in {}; include more surrounding context or pass replace_all: true",
                    n,
                    path
                );
            }
            let updated = if replace_all {
                content.replace(old, new)
            } else {
                content.replacen(old, new, 1)
            };
            (n, updated)
        }
        _ if content.contains("\r\n") && !old.contains("\r\n") => {
            let norm_content = content.replace("\r\n", "\n");
            let norm_old = old.replace("\r\n", "\n");
            let n = norm_content.matches(&norm_old).count();
            if n == 0 {
                bail!("old_string not found in {} (check exact whitespace/casing)", path);
            }
            if n > 1 && !replace_all {
                bail!(
                    "old_string appears {} times in {}; include more surrounding context or pass replace_all: true",
                    n,
                    path
                );
            }
            let norm_new = new.replace("\r\n", "\n");
            let replaced = if replace_all {
                norm_content.replace(&norm_old, &norm_new)
            } else {
                norm_content.replacen(&norm_old, &norm_new, 1)
            };
            (n, replaced.replace('\n', "\r\n"))
        }
        _ => bail!("old_string not found in {} (check exact whitespace/casing)", path),
    };

    write_atomic(&full, &encode_with_bom(&updated, enc))?;
    Ok(format!(
        "edited {} ({} replacement{})",
        path,
        if replace_all { count } else { 1 },
        if replace_all && count > 1 { "s" } else { "" }
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::path::PathBuf;

    fn tmp() -> PathBuf {
        static N: AtomicU64 = AtomicU64::new(0);
        let d = std::env::temp_dir().join(format!(
            "meg-edit-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn replaces_unique_match() {
        let ws = tmp();
        std::fs::write(ws.join("f.py"), "x = 1\ny = 2\n").unwrap();
        let out = str_replace(
            &ws,
            &json!({"path": "f.py", "old_string": "y = 2", "new_string": "y = 99"}),
        )
        .unwrap();
        assert!(out.contains("edited"));
        assert_eq!(std::fs::read_to_string(ws.join("f.py")).unwrap(), "x = 1\ny = 99\n");
        let _ = std::fs::remove_dir_all(&ws);
    }

    #[test]
    fn rejects_non_unique_without_replace_all() {
        let ws = tmp();
        std::fs::write(ws.join("f.py"), "a = 1\na = 1\n").unwrap();
        let err = str_replace(
            &ws,
            &json!({"path": "f.py", "old_string": "a = 1", "new_string": "a = 2"}),
        )
        .unwrap_err();
        assert!(err.to_string().contains("2 times"));
        let _ = std::fs::remove_dir_all(&ws);
    }

    #[test]
    fn replace_all_flag_works() {
        let ws = tmp();
        std::fs::write(ws.join("f.py"), "a = 1\na = 1\n").unwrap();
        str_replace(
            &ws,
            &json!({"path": "f.py", "old_string": "a = 1", "new_string": "a = 2", "replace_all": true}),
        )
        .unwrap();
        assert_eq!(std::fs::read_to_string(ws.join("f.py")).unwrap(), "a = 2\na = 2\n");
        let _ = std::fs::remove_dir_all(&ws);
    }

    #[test]
    fn not_found_error_is_actionable() {
        let ws = tmp();
        std::fs::write(ws.join("f.py"), "hello\n").unwrap();
        let err = str_replace(
            &ws,
            &json!({"path": "f.py", "old_string": "nope", "new_string": "x"}),
        )
        .unwrap_err();
        assert!(err.to_string().contains("not found"));
        let _ = std::fs::remove_dir_all(&ws);
    }

    #[test]
    fn crlf_file_accepts_lf_edit_and_preserves_crlf() {
        let ws = tmp();
        std::fs::write(ws.join("win.rs"), "fn main() {\r\n    let x = 1;\r\n}\r\n").unwrap();
        let out = str_replace(
            &ws,
            &json!({
                "path": "win.rs",
                "old_string": "fn main() {\n    let x = 1;\n}",
                "new_string": "fn main() {\n    let x = 42;\n}"
            }),
        )
        .unwrap();
        assert!(out.contains("edited"));
        assert_eq!(
            std::fs::read_to_string(ws.join("win.rs")).unwrap(),
            "fn main() {\r\n    let x = 42;\r\n}\r\n"
        );
        let _ = std::fs::remove_dir_all(&ws);
    }

    #[test]
    fn crlf_uniqueness_still_enforced() {
        let ws = tmp();
        std::fs::write(ws.join("dup.txt"), "a\r\nb\r\na\r\nb\r\n").unwrap();
        let err = str_replace(
            &ws,
            &json!({"path": "dup.txt", "old_string": "a\nb", "new_string": "c\nd"}),
        )
        .unwrap_err();
        assert!(err.to_string().contains("2 times"));
        let _ = std::fs::remove_dir_all(&ws);
    }

    #[test]
    fn hashline_patch_validates_hashes_and_rejects_stale_edits() {
        let ws = tmp();
        std::fs::write(ws.join("h.rs"), "fn a() {}\r\nfn b() {}\r\nfn c() {}\r\n").unwrap();
        let h2 = line_hash3("fn b() {}");
        let stale = apply_hashline_patch(
            &ws,
            &json!({
                "path": "h.rs",
                "edits": [{ "start_anchor": "2#000", "new_content": "fn b_new() {}" }]
            }),
        );
        if h2 != "000" {
            assert!(stale.unwrap_err().to_string().contains("stale hashline anchor"));
        }
        // Disk untouched on stale rejection.
        assert_eq!(
            std::fs::read_to_string(ws.join("h.rs")).unwrap(),
            "fn a() {}\r\nfn b() {}\r\nfn c() {}\r\n"
        );
        let ok = str_replace(
            &ws,
            &json!({
                "path": "h.rs",
                "edits": [{ "start_anchor": format!("2#{h2}"), "new_content": "fn b_new() {}" }]
            }),
        )
        .unwrap();
        assert!(ok.contains("hashline-patched"));
        assert_eq!(
            std::fs::read_to_string(ws.join("h.rs")).unwrap(),
            "fn a() {}\r\nfn b_new() {}\r\nfn c() {}\r\n"
        );
        let _ = std::fs::remove_dir_all(&ws);
    }

    #[test]
    fn preserves_utf8_and_utf16le_bom_during_edits() {
        let ws = tmp();
        // UTF-8 BOM
        let mut utf8_bom = vec![0xEF, 0xBB, 0xBF];
        utf8_bom.extend_from_slice(b"hello = 1\n");
        std::fs::write(ws.join("bom8.txt"), &utf8_bom).unwrap();
        str_replace(
            &ws,
            &json!({"path": "bom8.txt", "old_string": "hello = 1", "new_string": "hello = 2"}),
        )
        .unwrap();
        let after8 = std::fs::read(ws.join("bom8.txt")).unwrap();
        assert!(after8.starts_with(&[0xEF, 0xBB, 0xBF]));
        assert_eq!(&after8[3..], b"hello = 2\n");

        // UTF-16 LE BOM
        let mut utf16 = vec![0xFF, 0xFE];
        for u in "alpha = 1\r\n".encode_utf16() {
            utf16.extend_from_slice(&u.to_le_bytes());
        }
        std::fs::write(ws.join("bom16.txt"), &utf16).unwrap();
        str_replace(
            &ws,
            &json!({"path": "bom16.txt", "old_string": "alpha = 1", "new_string": "alpha = 9"}),
        )
        .unwrap();
        let after16 = std::fs::read(ws.join("bom16.txt")).unwrap();
        assert!(after16.starts_with(&[0xFF, 0xFE]));
        let (decoded, _) = decode_with_bom(&after16).unwrap();
        assert_eq!(decoded, "alpha = 9\r\n");
        let _ = std::fs::remove_dir_all(&ws);
    }
}