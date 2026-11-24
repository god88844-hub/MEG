//! Ring-0 security guards:
//! 1. Stage-by-stage compound bash gate (`&&`, `||`, `;`, `|`, `$()`, backticks)
//! 2. `.env*` gitignore lock + direct `.env` read/write block
//! 3. Shannon-entropy + token-prefix (`sk-`, `ghp_`, `AIza`) pre-commit scanner
//! 4. SAL observation scrubber (`<SURROGATE_...>`, `disable_codebase_upload = true`)
//! 5. `ChildAuthority::clamp` bitmask intersection (`Child = Parent & Requested`)

use anyhow::{bail, Result};
use regex::Regex;
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::OnceLock;

pub const DISABLE_CODEBASE_UPLOAD: bool = true;

/// Split a shell command into individual stages across `&&`, `||`, `;`, `|`,
/// plus any `$()` or backtick subshells.
pub fn split_compound_stages(command: &str) -> Vec<String> {
    let mut stages = Vec::new();
    let mut current = String::new();
    let chars: Vec<char> = command.chars().collect();
    let mut i = 0;
    let mut in_single = false;
    let mut in_double = false;

    while i < chars.len() {
        let ch = chars[i];
        if ch == '\'' && !in_double {
            in_single = !in_single;
            current.push(ch);
            i += 1;
            continue;
        }
        if ch == '"' && !in_single {
            in_double = !in_double;
            current.push(ch);
            i += 1;
            continue;
        }
        if !in_single && !in_double {
            // Subshell `$(...)`
            if ch == '$' && i + 1 < chars.len() && chars[i + 1] == '(' {
                let mut depth = 1usize;
                let mut sub = String::new();
                i += 2;
                while i < chars.len() && depth > 0 {
                    if chars[i] == '(' {
                        depth += 1;
                    } else if chars[i] == ')' {
                        depth -= 1;
                        if depth == 0 {
                            i += 1;
                            break;
                        }
                    }
                    sub.push(chars[i]);
                    i += 1;
                }
                if !sub.trim().is_empty() {
                    stages.extend(split_compound_stages(&sub));
                }
                continue;
            }
            // Backtick subshell
            if ch == '`' {
                let mut sub = String::new();
                i += 1;
                while i < chars.len() && chars[i] != '`' {
                    sub.push(chars[i]);
                    i += 1;
                }
                if i < chars.len() {
                    i += 1;
                }
                if !sub.trim().is_empty() {
                    stages.extend(split_compound_stages(&sub));
                }
                continue;
            }
            // Top-level separators: &&, ||, ;, |
            if (ch == '&' && i + 1 < chars.len() && chars[i + 1] == '&')
                || (ch == '|' && i + 1 < chars.len() && chars[i + 1] == '|')
            {
                if !current.trim().is_empty() {
                    stages.push(current.trim().to_string());
                }
                current.clear();
                i += 2;
                continue;
            }
            if ch == ';' || ch == '|' {
                if !current.trim().is_empty() {
                    stages.push(current.trim().to_string());
                }
                current.clear();
                i += 1;
                continue;
            }
        }
        current.push(ch);
        i += 1;
    }
    if !current.trim().is_empty() {
        stages.push(current.trim().to_string());
    }
    stages
}

const DENIED_STAGE_PATTERNS: &[&str] = &[
    "rm -rf /",
    "rm -fr /",
    "mkfs.",
    "dd if=",
    ":(){",
    "shutdown",
    "reboot",
    "format c:",
    "del /s /q c:\\",
    "git push --force",
    "git push -f",
    "gh pr merge",
    "npm publish",
];

/// Validate EVERY stage of a compound shell command before Stage 1 runs.
/// If ANY stage is denied, the entire command is aborted.
pub fn validate_compound_bash(command: &str) -> Result<()> {
    for stage in split_compound_stages(command) {
        let low = stage.to_ascii_lowercase();
        for denied in DENIED_STAGE_PATTERNS {
            if low.contains(denied) {
                bail!("compound bash gate denied stage '{stage}' (matched '{denied}')");
            }
        }
        if low.contains(".env")
            && (low.starts_with("cat ")
                || low.starts_with("type ")
                || low.starts_with("get-content ")
                || low.contains("curl ")
                || low.contains("wget "))
        {
            bail!("compound bash gate denied secret-airlock access to .env in stage '{stage}'");
        }
    }
    Ok(())
}

/// True for `.env`, `.env.local`, `.env.production`, etc. (allows `.env.example` / `.env.sample`).
pub fn is_blocked_env_path(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    let low = name.to_ascii_lowercase();
    if low == ".env.example" || low == ".env.sample" || low == ".env.template" {
        return false;
    }
    low == ".env" || low.starts_with(".env.")
}

/// Ensure `.env*` is present in `<workspace>/.gitignore`.
pub fn ensure_env_gitignored(workspace: &Path) -> Result<()> {
    let gi = workspace.join(".gitignore");
    let existing = if gi.exists() {
        std::fs::read_to_string(&gi)?
    } else {
        String::new()
    };
    if existing.lines().any(|l| l.trim() == ".env*" || l.trim() == ".env") {
        return Ok(());
    }
    let mut updated = existing;
    if !updated.is_empty() && !updated.ends_with('\n') {
        updated.push('\n');
    }
    updated.push_str(".env*\n!.env.example\n");
    std::fs::write(&gi, updated)?;
    Ok(())
}

/// Shannon entropy in bits per character.
pub fn shannon_entropy(s: &str) -> f64 {
    if s.is_empty() {
        return 0.0;
    }
    let mut counts: BTreeMap<u8, usize> = BTreeMap::new();
    for &b in s.as_bytes() {
        *counts.entry(b).or_insert(0) += 1;
    }
    let len = s.len() as f64;
    let mut h = 0.0;
    for &c in counts.values() {
        let p = c as f64 / len;
        h -= p * p.log2();
    }
    h
}

fn secret_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?x)
            (sk-[A-Za-z0-9_\-]{16,}) |
            (ghp_[A-Za-z0-9]{20,}) |
            (AIza[0-9A-Za-z\-_]{20,}) |
            (AKIA[0-9A-Z]{16})
            ",
        )
        .unwrap()
    })
}

/// Pre-commit secret scanner: detects known token prefixes (`sk-`, `ghp_`, `AIza`, `AKIA`)
/// and high-entropy (>= 4.5 bits/char, len >= 32) secret-like tokens.
pub fn scan_for_secrets(text: &str) -> Vec<String> {
    let mut findings = Vec::new();
    for m in secret_regex().find_iter(text) {
        findings.push(format!("known token prefix ({} chars)", m.as_str().len()));
    }
    for tok in text.split(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '=' | ':' | ',' | ';' | '(' | ')' | '{' | '}')) {
        if tok.len() >= 32
            && tok.len() <= 128
            && !tok.contains('/')
            && !tok.contains('\\')
            && tok.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '+' | '/'))
            && shannon_entropy(tok) >= 4.5
        {
            findings.push(format!("high-entropy token ({} chars, H={:.2})", tok.len(), shannon_entropy(tok)));
        }
    }
    findings
}

/// SAL membrane: scrubs detected secrets in tool observations into `<SURROGATE_SECRET_N>`.
pub fn scrub_sal_observation(text: &str) -> String {
    let mut idx = 0usize;
    secret_regex()
        .replace_all(text, |_caps: &regex::Captures| {
            idx += 1;
            format!("<SURROGATE_SECRET_{idx}>")
        })
        .into_owned()
}

/// Monotonic authority bitmask for delegated child workers (`Child = Parent & Requested`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChildAuthority(pub u32);

impl ChildAuthority {
    pub const READ: u32 = 1 << 0;
    pub const WRITE: u32 = 1 << 1;
    pub const EXEC: u32 = 1 << 2;
    pub const NETWORK: u32 = 1 << 3;
    pub const DELEGATE: u32 = 1 << 4;

    pub const fn clamp(parent: Self, requested: Self) -> Self {
        Self(parent.0 & requested.0)
    }

    pub const fn has(self, flag: u32) -> bool {
        (self.0 & flag) == flag
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compound_bash_gate_aborts_if_any_stage_or_subshell_is_denied() {
        assert!(validate_compound_bash("echo hello && ls -la").is_ok());
        assert!(validate_compound_bash("echo ok && git push --force origin main").is_err());
        assert!(validate_compound_bash("echo $(cat .env) | wc -l").is_err());
        assert!(validate_compound_bash("echo `gh pr merge 12`").is_err());
    }

    #[test]
    fn env_airlock_blocks_dotenv_and_locks_gitignore() {
        assert!(is_blocked_env_path(Path::new(".env")));
        assert!(is_blocked_env_path(Path::new("app/.env.local")));
        assert!(!is_blocked_env_path(Path::new(".env.example")));

        let ws = std::env::temp_dir().join(format!("meg-sec-gi-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&ws);
        std::fs::create_dir_all(&ws).unwrap();
        ensure_env_gitignored(&ws).unwrap();
        let gi = std::fs::read_to_string(ws.join(".gitignore")).unwrap();
        assert!(gi.contains(".env*"));
        let _ = std::fs::remove_dir_all(&ws);
    }

    #[test]
    fn secret_scanner_and_sal_scrubber_catch_prefixes_and_high_entropy() {
        let sample = "token=sk-proj-1234567890abcdef1234567890 and ghp_1234567890abcdefghij1234567890";
        let hits = scan_for_secrets(sample);
        assert!(hits.len() >= 2, "{hits:?}");
        let scrubbed = scrub_sal_observation(sample);
        assert!(scrubbed.contains("<SURROGATE_SECRET_1>"), "{scrubbed}");
        assert!(scrubbed.contains("<SURROGATE_SECRET_2>"), "{scrubbed}");
        assert!(!scrubbed.contains("sk-proj-"));
        const { assert!(DISABLE_CODEBASE_UPLOAD) };
    }

    #[test]
    fn child_authority_clamp_never_escalates_privileges() {
        let parent = ChildAuthority(ChildAuthority::READ | ChildAuthority::EXEC);
        let requested = ChildAuthority(ChildAuthority::READ | ChildAuthority::WRITE | ChildAuthority::NETWORK);
        let child = ChildAuthority::clamp(parent, requested);
        assert!(child.has(ChildAuthority::READ));
        assert!(!child.has(ChildAuthority::WRITE));
        assert!(!child.has(ChildAuthority::EXEC));
        assert!(!child.has(ChildAuthority::NETWORK));
    }
}
#[cfg(test)]
mod live_ring0_wiring_tests {
    use serde_json::json;

    #[test]
    fn live_tools_enforce_compound_bash_sal_scrub_env_airlock_and_hot_linter() {
        let ws = std::env::temp_dir().join(format!("meg-live-ring0-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&ws);
        std::fs::create_dir_all(&ws).unwrap();
        std::fs::write(ws.join(".env"), "API_KEY=supersecret\n").unwrap();
        std::fs::write(ws.join(".env.example"), "API_KEY=changeme\n").unwrap();
        std::fs::write(ws.join("config.txt"), "leaked=sk-proj-1234567890abcdef1234567890\n").unwrap();

        // 1. Live bash blocks compound denied stage BEFORE Stage 1 runs
        let denied_bash = crate::bash::run(
            &ws,
            &json!({"command": "echo stage1 > pwned.txt && git push --force origin main"}),
        );
        assert!(denied_bash.is_err());
        assert!(!ws.join("pwned.txt").exists(), "Stage 1 must NEVER run when Stage 2 is denied");

        // 2. Live bash scrubs secrets from stdout via SAL membrane
        let scrubbed_bash = crate::bash::run(
            &ws,
            &json!({"command": "echo sk-proj-1234567890abcdef1234567890"}),
        )
        .unwrap();
        assert!(scrubbed_bash.contains("<SURROGATE_SECRET_1>"), "{scrubbed_bash}");
        assert!(!scrubbed_bash.contains("sk-proj-"), "{scrubbed_bash}");

        // 3. Live read_file, write_file, edit_file enforce .env airlock while allowing .env.example
        assert!(crate::fs_tools::read_file(&ws, &json!({"path": ".env"})).is_err());
        assert!(crate::fs_tools::read_file(&ws, &json!({"path": ".env.example"})).is_ok());
        assert!(crate::fs_tools::write_file(&ws, &json!({"path": ".env.local", "content": "X=1"})).is_err());
        assert!(crate::edit::str_replace(&ws, &json!({"path": ".env", "old_string": "super", "new_string": "x"})).is_err());

        // 4. Live read_file scrubs secrets via SAL membrane
        let read_cfg = crate::fs_tools::read_file(&ws, &json!({"path": "config.txt"})).unwrap();
        assert!(read_cfg.contains("<SURROGATE_SECRET_1>"), "{read_cfg}");
        assert!(!read_cfg.contains("sk-proj-"), "{read_cfg}");

        // 5. Live write_file enforces .hot.* zero-alloc / zero-unseeded-RNG linter
        let hot_err = crate::fs_tools::write_file(
            &ws,
            &json!({"path": "physics.hot.ts", "content": "const x = Math.random();\n"}),
        );
        assert!(hot_err.is_err());
        assert!(!ws.join("physics.hot.ts").exists());

        let _ = std::fs::remove_dir_all(&ws);
    }
}