//! meg-ffi -- Bridge 1: the meg-rs hot path as a C-ABI cdylib, loaded
//! in-process by Bun's `bun:ffi` (dlopen). Zero fork, zero subprocess:
//! the control plane calls grep/edit/truncate/parse in ~us.
//!
//! Memory contract: every returned char* is heap-allocated with CString and
//! MUST be released by the caller via meg_free after copying. All functions
//! are `extern "C"`, UTF-8 in/out.

use std::ffi::{c_char, CStr, CString};
use std::path::PathBuf;

fn take_str(ptr: *const c_char) -> String {
    if ptr.is_null() {
        return String::new();
    }
    unsafe { CStr::from_ptr(ptr).to_string_lossy().into_owned() }
}

fn put_str(s: String) -> *mut c_char {
    // Interior NULs cannot survive a C string; replace to keep the contract.
    match CString::new(s.replace('\0', " ")) {
        Ok(c) => c.into_raw(),
        Err(_) => CString::default().into_raw(),
    }
}

fn ws(p: *const c_char) -> PathBuf {
    PathBuf::from(take_str(p))
}

/// Release a pointer previously returned by this library.
#[no_mangle]
pub extern "C" fn meg_free(ptr: *mut c_char) {
    if !ptr.is_null() {
        unsafe { drop(CString::from_raw(ptr)) };
    }
}

/// Glob search confined to the workspace. Returns newline-separated paths,
/// or an "error: ..." string (FFI errors travel in-band, Python-harness style).
#[no_mangle]
pub extern "C" fn meg_search_files(workspace: *const c_char, pattern: *const c_char) -> *mut c_char {
    let args = serde_json::json!({ "pattern": take_str(pattern) });
    let result = meg_tools::ToolRegistry::with_core_tools(&ws(workspace))
        .execute("search_files", &args);
    put_str(if result.ok { result.output } else { format!("error: {}", result.output) })
}

/// Regex content search confined to the workspace.
#[no_mangle]
pub extern "C" fn meg_grep_files(
    workspace: *const c_char,
    pattern: *const c_char,
    glob: *const c_char,
    max_results: u32,
) -> *mut c_char {
    let glob_str = take_str(glob);
    let mut args = serde_json::json!({
        "pattern": take_str(pattern),
        "max_results": if max_results == 0 { 50 } else { max_results },
    });
    if !glob_str.trim().is_empty() {
        args["glob"] = serde_json::Value::String(glob_str);
    }
    let result = meg_tools::ToolRegistry::with_core_tools(&ws(workspace))
        .execute("grep_files", &args);
    put_str(if result.ok { result.output } else { format!("error: {}", result.output) })
}

/// Bounded directory listing confined to the workspace.
#[no_mangle]
pub extern "C" fn meg_list_dir(workspace: *const c_char, rel_path: *const c_char) -> *mut c_char {
    let rel = take_str(rel_path);
    let path_arg = if rel.trim().is_empty() { ".".to_string() } else { rel };
    let args = serde_json::json!({ "path": path_arg });
    let result = meg_tools::ToolRegistry::with_core_tools(&ws(workspace))
        .execute("list_dir", &args);
    put_str(if result.ok { result.output } else { format!("error: {}", result.output) })
}

/// Read a file (workspace-confined), capped at `cap` bytes on a valid UTF-8 boundary.
#[no_mangle]
pub extern "C" fn meg_read_file(workspace: *const c_char, path: *const c_char, cap: u32) -> *mut c_char {
    let args = serde_json::json!({ "path": take_str(path) });
    let result = meg_tools::ToolRegistry::with_core_tools(&ws(workspace))
        .execute("read_file", &args);
    if !result.ok {
        return put_str(format!("error: {}", result.output));
    }
    let cap = cap as usize;
    if result.output.len() > cap {
        let end = result.output.floor_char_boundary(cap);
        let truncated = format!(
            "{}\n...[truncated {} chars]",
            &result.output[..end],
            result.output.len() - end
        );
        put_str(truncated)
    } else {
        put_str(result.output)
    }
}

/// Exact str_replace edit (uniqueness law). Returns a summary or "error: ...".
#[no_mangle]
pub extern "C" fn meg_edit_file(
    workspace: *const c_char,
    path: *const c_char,
    old_string: *const c_char,
    new_string: *const c_char,
    replace_all: bool,
) -> *mut c_char {
    let args = serde_json::json!({
        "path": take_str(path),
        "old_string": take_str(old_string),
        "new_string": take_str(new_string),
        "replace_all": replace_all,
    });
    let result = meg_tools::ToolRegistry::with_core_tools(&ws(workspace))
        .execute("edit_file", &args);
    put_str(if result.ok { result.output } else { format!("error: {}", result.output) })
}

/// Observation truncation with the critical-line guarantee.
#[no_mangle]
pub extern "C" fn meg_truncate_observation(text: *const c_char, limit: u32) -> *mut c_char {
    put_str(meg_core::truncate::truncate_observation(&take_str(text), limit as usize))
}

/// Tolerant action parse. Returns the JSON variant tag ("tool"|"batch"|"final"
/// |"unknown") or null when nothing parseable exists.
#[no_mangle]
pub extern "C" fn meg_parse_action_variant(raw: *const c_char) -> *mut c_char {
    match meg_core::parse_model_output(&take_str(raw)) {
        Some(action) => put_str(
            match action {
                meg_core::Action::Tool { .. } => "tool",
                meg_core::Action::Batch { .. } => "batch",
                meg_core::Action::Final { .. } => "final",
                meg_core::Action::Unknown(_) => "unknown",
            }
            .to_string(),
        ),
        None => std::ptr::null_mut(),
    }
}

/// Pre-turn workspace indexer: compact <workspace_map> for Turn 1 context.
#[no_mangle]
pub extern "C" fn meg_index_workspace(root_ptr: *const c_char) -> *mut c_char {
    let root = take_str(root_ptr);
    put_str(meg_core::indexer::index_workspace(std::path::Path::new(&root), 200))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_via_c_strings() {
        let ws = CString::new(std::env::temp_dir().to_string_lossy().to_string()).unwrap();
        let pattern = CString::new("**/*.txt").unwrap();
        let ptr = meg_search_files(ws.as_ptr(), pattern.as_ptr());
        let out = take_str(ptr);
        meg_free(ptr);
        assert!(out.starts_with("error:") || !out.contains('\0'));
    }

    #[test]
    fn truncate_preserves_fatal() {
        let mut body = String::new();
        for i in 0..300 {
            body.push_str(&format!("row {i}\n"));
        }
        body.push_str("FATAL: refused");
        let text = CString::new(body).unwrap();
        let ptr = meg_truncate_observation(text.as_ptr(), 600);
        let out = take_str(ptr);
        meg_free(ptr);
        assert!(out.contains("FATAL: refused"));
    }

    #[test]
    fn read_file_truncates_safely_on_multibyte_utf8_boundary() {
        let dir = std::env::temp_dir().join(format!("meg-ffi-utf8-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("u.txt"), "\u{1F680}\u{1F525}\u{1F31F}\u{2728}").unwrap();
        let ws_c = CString::new(dir.to_string_lossy().to_string()).unwrap();
        let path_c = CString::new("u.txt").unwrap();
        let ptr = meg_read_file(ws_c.as_ptr(), path_c.as_ptr(), 5);
        let out = take_str(ptr);
        meg_free(ptr);
        assert!(out.starts_with("\u{1F680}\n...[truncated"), "got: {out}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn grep_and_list_dir_round_trip() {
        let dir = std::env::temp_dir().join(format!("meg-ffi-grep-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("src/app.rs"), "pub fn hello_ffi() {}\n").unwrap();

        let ws_c = CString::new(dir.to_string_lossy().to_string()).unwrap();
        let pat_c = CString::new("hello_ffi").unwrap();
        let glob_c = CString::new("**/*.rs").unwrap();
        let ptr = meg_grep_files(ws_c.as_ptr(), pat_c.as_ptr(), glob_c.as_ptr(), 20);
        let grep_out = take_str(ptr);
        meg_free(ptr);
        assert!(grep_out.contains("src/app.rs:1:pub fn hello_ffi()"), "{grep_out}");

        let rel_c = CString::new(".").unwrap();
        let ptr2 = meg_list_dir(ws_c.as_ptr(), rel_c.as_ptr());
        let ls_out = take_str(ptr2);
        meg_free(ptr2);
        assert!(ls_out.contains("dir  src/"), "{ls_out}");

        let _ = std::fs::remove_dir_all(&dir);
    }
}