//! meg-ffi — Bridge 1: the meg-rs hot path as a C-ABI cdylib, loaded
//! in-process by Bun's `bun:ffi` (dlopen). Zero fork, zero subprocess:
//! the control plane calls grep/edit/truncate/parse in ~µs.
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

/// Read a file (workspace-confined), capped at `cap` bytes.
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
        let truncated = format!(
            "{}\n...[truncated {} chars]",
            &result.output[..cap],
            result.output.len() - cap
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
}

/// Pre-turn workspace indexer: compact <workspace_map> for Turn 1 context.
#[no_mangle]
pub extern "C" fn meg_index_workspace(root_ptr: *const c_char) -> *mut c_char {
    let root = take_str(root_ptr);
    put_str(meg_core::indexer::index_workspace(std::path::Path::new(&root), 200))
}
