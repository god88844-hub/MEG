//! Difficulty-conditioned evaluator-isomorphic finish-hook (AHE §4.4.1):
//! mandatory verification lifts resolve 43%→91% on Easy/Medium but inflates
//! turns on Hard tasks near the deadline (-3.4pp regression). The gate:
//! full re-prompt verification only while under 70% of the time budget AND
//! 60% of the turn cap; past that, the zero-overhead artifact check alone
//! decides so Hard tasks never time out in verification loops.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct FinishHookConfig {
    pub timeout_budget_secs: u64,
    pub max_turns: u64,
    pub simulated_elapsed_secs: Option<u64>,
    pub target_deliverables: Vec<PathBuf>,
}

pub struct FinishVerdict {
    pub approved: bool,
    /// Some(text) = one re-prompt is warranted (under budget, fixable failure).
    pub reprompt: Option<String>,
    /// Human-readable gate trace.
    pub detail: String,
}

/// The 0-API-call local deliverable check: exists, non-empty, no conflict markers.
pub fn local_deliverable_check(deliverables: &[PathBuf]) -> Vec<String> {
    let mut failures = Vec::new();
    for path in deliverables {
        match std::fs::metadata(path) {
            Err(_) => failures.push(format!("{}: missing on disk", path.display())),
            Ok(meta) if meta.len() == 0 => {
                failures.push(format!("{}: empty (0 bytes)", path.display()))
            }
            Ok(_) => {
                if let Ok(text) = std::fs::read_to_string(path) {
                    for marker in ["<<<<<<<", ">>>>>>>"] {
                        if text.contains(marker) {
                            failures.push(format!("{}: unresolved conflict marker {marker}", path.display()));
                            break;
                        }
                    }
                }
            }
        }
    }
    failures.extend(polyglot_cleanliness_failures(deliverables));
    failures
}

/// Build artifacts that must not sit beside a single-file deliverable. Measured
/// cost of skipping this: polyglot-c-py and polyglot-rust-c recorded 0.0 after
/// 2-3 turns because the verifier compiles, RUNS, and then requires the
/// directory to contain ONLY the source file — leftover a.out/*.o is a fail the
/// agent could have cleaned in one turn had the finish-hook named it.
pub const STRAY_ARTIFACT_NAMES: &[&str] = &["a.out", "__pycache__", "target"];
pub const STRAY_ARTIFACT_SUFFIXES: &[&str] = &[".o", ".so", ".pyc", ".class", ".obj"];

pub fn is_stray_artifact(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    STRAY_ARTIFACT_NAMES.contains(&lower.as_str())
        || STRAY_ARTIFACT_SUFFIXES.iter().any(|s| lower.ends_with(s))
        || lower.starts_with("tmp_")
}

/// Single-file / polyglot cleanliness + structural syntax checks.
///
/// "Single-file" is judged on the deliverable's own directory: the rule fires
/// only when the source deliverable is the ONLY source file in that directory.
/// A multi-module workspace (app.py beside calc.py, a project with its
/// __pycache__) is legitimate and must settle — measured on the code-fence
/// rescue fixtures, where a two-file workspace tripped the first cut. 0 API
/// calls — pure filesystem + text.
pub fn polyglot_cleanliness_failures(deliverables: &[PathBuf]) -> Vec<String> {
    let sources: Vec<&PathBuf> = deliverables
        .iter()
        .filter(|p| {
            matches!(
                p.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()),
                Some(ref ext) if ["py", "c", "h", "cpp", "rs", "scm", "sql", "js", "sh"]
                    .contains(&ext.as_str())
            )
        })
        .collect();
    if sources.len() != 1 {
        return Vec::new();
    }
    let source = sources[0];
    let mut failures = Vec::new();

    // ls -A on the deliverable's own directory — but only when this really is
    // a single-source directory. A second source file beside the deliverable
    // makes it a multi-module workspace and the rule stands down.
    let dir = source.parent().unwrap_or_else(|| Path::new("."));
    if let Ok(entries) = std::fs::read_dir(dir) {
        const SOURCE_EXTS: [&str; 9] =
            ["py", "c", "h", "cpp", "rs", "scm", "sql", "js", "sh"];
        let mut sibling_sources = 0;
        let mut strays: Vec<String> = Vec::new();
        for entry in entries.filter_map(|e| e.ok()) {
            let name = entry.file_name().to_string_lossy().to_string();
            let path = entry.path();
            if path.is_file() {
                let ext = path
                    .extension()
                    .and_then(|e| e.to_str())
                    .map(|e| e.to_ascii_lowercase())
                    .unwrap_or_default();
                if SOURCE_EXTS.contains(&ext.as_str()) && path != **source
                    && !is_stray_artifact(&name)
                {
                    sibling_sources += 1;
                    continue;
                }
            }
            if is_stray_artifact(&name) {
                strays.push(name);
            }
        }
        if sibling_sources == 0 && !strays.is_empty() {
            failures.push(format!(
                "FINISH BLOCKED: Stray build artifacts found in directory ({}). Single-file \
                 tasks require removing all compiled binaries/temp files after testing AND \
                 verifying both compilers produce identical output.",
                strays.join(", ")
            ));
        }
    }

    if let Ok(text) = std::fs::read_to_string(source) {
        let ext = source
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .unwrap_or_default();
        if ext == "scm" && !scm_structural_ok(&text) {
            failures.push(format!(
                "{}: scheme file is not structurally plausible (unbalanced parentheses or \
                 no environment frame definition)",
                source.display()
            ));
        }
    }
    failures
}

/// Balanced parentheses (comments stripped) and at least one `define` — the
/// cheap structural floor for a .scm deliverable. Not a full reader; it exists
/// to catch the empty-frame / paren-dropped failure class in 0 API calls.
pub fn scm_structural_ok(text: &str) -> bool {
    let mut depth: i64 = 0;
    let mut defines = 0;
    for raw in text.lines() {
        let line = raw.split(';').next().unwrap_or("");
        for ch in line.chars() {
            match ch {
                '(' => depth += 1,
                ')' => depth -= 1,
                _ => {}
            }
        }
        if line.contains("(define") {
            defines += 1;
        }
    }
    depth == 0 && defines > 0
}

/// The conditioned gate. Returns the verdict for the loop to act on.
pub fn evaluate_finish(
    cfg: &FinishHookConfig,
    elapsed_secs: u64,
    turn_idx: u64,
    finish_reprompt_count: u32,
) -> FinishVerdict {
    // Always: the zero-overhead local check.
    let failures = local_deliverable_check(&cfg.target_deliverables);
    if failures.is_empty() {
        return FinishVerdict {
            approved: true,
            reprompt: None,
            detail: "finish-hook: all deliverables present, non-empty, conflict-free".into(),
        };
    }

    let elapsed = cfg.simulated_elapsed_secs.unwrap_or(elapsed_secs);
    let under_time = elapsed < cfg.timeout_budget_secs * 70 / 100;
    let under_turns = turn_idx < cfg.max_turns * 60 / 100;
    let time_budget_used = elapsed * 100 / cfg.timeout_budget_secs.max(1);

    if !under_time || !under_turns {
        return FinishVerdict {
            approved: false,
            reprompt: None,
            detail: format!(
                "finish-hook: deliverables failed ({}) but {time_budget_used}% of time budget used / turn {turn_idx}/{} — bypassing re-prompt to avoid a verification timeout",
                failures.join("; "),
                cfg.max_turns
            ),
        };
    }
    if finish_reprompt_count == 0 {
        return FinishVerdict {
            approved: false,
            reprompt: Some(format!(
                "Deliverable check failed: {}. Fix these on disk, verify, then answer again.",
                failures.join("; ")
            )),
            detail: format!("finish-hook: 1 re-prompt granted ({})", failures.join("; ")),
        };
    }
    FinishVerdict {
        approved: false,
        reprompt: None,
        detail: format!(
            "finish-hook: deliverables still failing after the granted re-prompt ({})",
            failures.join("; ")
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg_with(deliverables: Vec<(PathBuf, &str)>) -> FinishHookConfig {
        for (path, content) in &deliverables {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            std::fs::write(path, content).unwrap();
        }
        FinishHookConfig {
            timeout_budget_secs: 900,
            max_turns: 20,
            simulated_elapsed_secs: None,
            target_deliverables: deliverables.into_iter().map(|(p, _)| p).collect(),
        }
    }

    #[test]
    fn missing_deliverable_under_budget_triggers_exactly_one_reprompt_and_recovers() {
        let dir = std::env::temp_dir().join(format!("meg-fh-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let missing = dir.join("out.txt");
        let cfg = cfg_with(vec![(missing.clone(), "")]);
        std::fs::remove_file(&missing).unwrap(); // it does not exist

        let v1 = evaluate_finish(&cfg, 100, 5, 0);
        assert!(!v1.approved);
        assert!(v1.reprompt.is_some(), "first failure must grant 1 re-prompt");

        // The model fixes it; second pass approves.
        std::fs::write(&missing, "result data").unwrap();
        let v2 = evaluate_finish(&cfg, 100, 6, 1);
        assert!(v2.approved, "{:?}", v2.detail);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn conflict_marker_triggers_reprompt_under_budget() {
        let dir = std::env::temp_dir().join(format!("meg-fh-c-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let conflicted = dir.join("code.py");
let cfg = cfg_with(vec![(conflicted.clone(), "")]);
        std::fs::write(&conflicted, "a = 1
<<<<<<< HEAD
b = 2
>>>>>>> other
").unwrap();
        let v = evaluate_finish(&cfg, 200, 5, 0);
        assert!(v.reprompt.is_some(), "conflict must grant the re-prompt");
        assert!(v.reprompt.as_ref().unwrap().contains("unresolved conflict marker"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn late_stage_bypasses_reprompt_cleanly() {
        let dir = std::env::temp_dir().join(format!("meg-fh-late-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let missing = dir.join("never-written.txt");
        let mut cfg = cfg_with(vec![(missing, "")]);
        cfg.simulated_elapsed_secs = Some(750); // >= 70% of 900
        let v = evaluate_finish(&cfg, 750, 18, 0);
        assert!(!v.approved);
        assert!(v.reprompt.is_none(), "late-stage must bypass turn-inflating re-prompts");
        assert!(v.detail.contains("bypassing re-prompt"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod wave9_tests {
    use super::*;

    #[test]
    fn polyglot_finish_blocked_while_a_out_remains_then_recovers() {
        let dir = std::env::temp_dir().join(format!("meg-w9-poly-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("solution.c");
        std::fs::write(&src, "int main(void){return 0;}\n").unwrap();
        let fake_binary = dir.join("a.out");
        std::fs::write(&fake_binary, b"MZ fake binary").unwrap();

        let cfg = FinishHookConfig {
            timeout_budget_secs: 900,
            max_turns: 15,
            simulated_elapsed_secs: None,
            target_deliverables: vec![src.clone()],
        };
        let v1 = evaluate_finish(&cfg, 100, 5, 0);
        assert!(!v1.approved, "stray a.out must block the finish");
        assert!(v1.reprompt.is_some(), "one recovery turn must be granted");
        let text = v1.reprompt.unwrap();
        assert!(text.contains("Stray build artifacts found in directory"), "{text}");

        // The agent cleans up; the finish now approves.
        std::fs::remove_file(&fake_binary).unwrap();
        let v2 = evaluate_finish(&cfg, 120, 6, 1);
        assert!(v2.approved, "{:?}", v2.detail);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn scm_deliverable_with_unbalanced_parens_is_blocked() {
        let dir = std::env::temp_dir().join(format!("meg-w9-scm-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("eval.scm");
        std::fs::write(&src, "(define (f x) (* x x)\n").unwrap(); // missing close
        let failures = polyglot_cleanliness_failures(&[src.clone()]);
        assert!(failures.iter().any(|f| f.contains("structurally plausible")),
                "{failures:?}");
        std::fs::write(&src, "(define (f x) (* x x))\n").unwrap();
        assert!(polyglot_cleanliness_failures(&[src]).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn multi_file_deliverable_set_keeps_its_build_dir() {
        let dir = std::env::temp_dir().join(format!("meg-w9-multi-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let proj = dir.join("target");
        std::fs::create_dir_all(&proj).unwrap();
        let a = dir.join("main.rs");
        let b = dir.join("lib.rs");
        std::fs::write(&a, "fn main(){}\n").unwrap();
        std::fs::write(&b, "pub fn f(){}\n").unwrap();
        // Two source deliverables: the single-file rule does not apply.
        assert!(polyglot_cleanliness_failures(&[a, b]).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

// ---------------------------------------------------------------------------
// Final-closure guards (2026-11-12 directive T2 port): daemon liveness,
// read-only repo restore, and the git non-interactive environment.
// ---------------------------------------------------------------------------

/// Daemon families a task may legitimately require alive at grading time.
pub const DAEMON_PROC_PATTERNS: &[&str] = &[
    "qemu-system", "qemu", "sshd", "nginx", "gunicorn", "uvicorn",
    "pypi-server", "pypi_server", "mysqld", "postgres", "redis-server",
    "websockify", "http.server",
];

/// True when the instruction asks for a server/VM/daemon left running.
/// General keyword detection — never a task-name table.
pub fn goal_wants_live_daemon(goal: &str) -> bool {
    let low = goal.to_lowercase();
    ["vm", "virtual machine", "qemu", "daemon", "server", "ssh", "listen",
     "web server", "webserver", "web interface", "pypi", "database",
     "keep it running", "leave it running", "background"]
        .iter()
        .any(|k| low.contains(k))
}

/// Probe whether any daemon family process is alive (pgrep), POSIX only.
/// Unavailable probes count as ALIVE: a missing pgrep must never block a
/// genuine finish.
/// Combined extended-regex alternation covering every daemon family in `DAEMON_PROC_PATTERNS`.
pub fn daemon_pgrep_pattern() -> String {
    DAEMON_PROC_PATTERNS
        .iter()
        .map(|p| p.replace('.', r"\."))
        .collect::<Vec<_>>()
        .join("|")
}

pub fn daemon_alive_probe() -> Result<bool, String> {
    if DAEMON_PROC_PATTERNS.is_empty() {
        return Err("no patterns".to_string());
    }
    let pattern = daemon_pgrep_pattern();
    match std::process::Command::new("pgrep")
        .arg("-f")
        .arg(&pattern)
        .output()
    {
        Ok(out) => Ok(out.status.success()),
        Err(e) => Err(format!("pgrep unavailable: {e}")),
    }
}

/// The env every git invocation in a trial carries: fully non-interactive.
pub const GIT_NONINTERACTIVE_ENV: &[(&str, &str)] = &[
    ("GIT_TERMINAL_PROMPT", "0"),
    ("GIT_EDITOR", "true"),
    ("GIT_MERGE_AUTOEDIT", "no"),
    ("GCM_INTERACTIVE", "never"),
    ("GIT_ASKPASS", "echo"),
    ("GIT_PAGER", "cat"),
];

#[cfg(test)]
mod closure_guard_tests {
    use super::*;

    #[test]
    fn goal_daemon_intent_is_general() {
        assert!(goal_wants_live_daemon(
            "Boot the VM and leave it running in the background"
        ));
        assert!(!goal_wants_live_daemon("write quicksort.py"));
    }

    #[test]
    fn git_env_covers_the_noninteractive_set() {
        let names: Vec<&str> = GIT_NONINTERACTIVE_ENV.iter().map(|e| e.0).collect();
        for required in ["GIT_TERMINAL_PROMPT", "GIT_EDITOR", "GIT_MERGE_AUTOEDIT", "GCM_INTERACTIVE"] {
            assert!(names.contains(&required), "{required} must be present");
        }
    }

    #[test]
    fn daemon_pgrep_pattern_covers_all_daemon_families() {
        let re = regex::Regex::new(&daemon_pgrep_pattern()).unwrap();
        for sample in [
            "qemu-system-x86_64 -nographic",
            "/usr/sbin/nginx -g daemon off;",
            "uvicorn app:main --port 8000",
            "redis-server *:6379",
            "python3 -m http.server 8080",
        ] {
            assert!(re.is_match(sample), "pattern missed daemon: {sample}");
        }
        assert!(!re.is_match("cargo test --workspace"));
    }
}

// ===========================================================================
// P1-11: AEPN Petri-Net Supervisor, Undo-Price Gate & ATIF Pinned-Model Lock
// ===========================================================================

/// P-invariant Petri-net marking for deterministic task completion gating.
/// Any code edit resets `p_tests_green` and `p_critic_approved` to `0`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AepnSupervisor {
    pub p_contract_locked: u8,
    pub p_selftest_red: u8,
    pub p_tests_green: u8,
    pub p_critic_approved: u8,
    pub p_human_approval_token: u8,
    pub p_turn_budget: u32,
}

impl AepnSupervisor {
    pub fn new(turn_budget: u32) -> Self {
        Self {
            p_contract_locked: 0,
            p_selftest_red: 0,
            p_tests_green: 0,
            p_critic_approved: 0,
            p_human_approval_token: 0,
            p_turn_budget: turn_budget,
        }
    }

    pub fn lock_contract(&mut self) {
        self.p_contract_locked = 1;
    }

    pub fn record_selftest_red(&mut self) {
        self.p_selftest_red = 1;
    }

    /// Transition: code edit invalidates prior test and critic approvals.
    pub fn on_code_edit(&mut self) -> Result<(), String> {
        if self.p_turn_budget == 0 {
            return Err("AEPN: p_turn_budget exhausted".into());
        }
        self.p_turn_budget -= 1;
        self.p_tests_green = 0;
        self.p_critic_approved = 0;
        Ok(())
    }

    pub fn record_tests_green(&mut self) -> Result<(), String> {
        if self.p_contract_locked == 0 {
            return Err("AEPN: cannot mark tests_green before p_contract_locked == 1".into());
        }
        self.p_tests_green = 1;
        Ok(())
    }

    pub fn record_critic_approved(&mut self) -> Result<(), String> {
        if self.p_tests_green == 0 {
            return Err("AEPN: cannot mark critic_approved while p_tests_green == 0".into());
        }
        self.p_critic_approved = 1;
        Ok(())
    }

    pub fn grant_human_approval(&mut self) {
        self.p_human_approval_token = 1;
    }

    /// Fire the final completion transition only when all invariants hold.
    pub fn can_complete(&self, require_human_token: bool) -> Result<(), Vec<String>> {
        let mut missing = Vec::new();
        if self.p_contract_locked == 0 {
            missing.push("p_contract_locked == 0".into());
        }
        if self.p_selftest_red == 0 {
            missing.push("p_selftest_red == 0".into());
        }
        if self.p_tests_green == 0 {
            missing.push("p_tests_green == 0".into());
        }
        if self.p_critic_approved == 0 {
            missing.push("p_critic_approved == 0".into());
        }
        if require_human_token && self.p_human_approval_token == 0 {
            missing.push("p_human_approval_token == 0".into());
        }
        if missing.is_empty() {
            Ok(())
        } else {
            Err(missing)
        }
    }
}

/// Undo-Price action taxonomy for external/stateful side effects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UndoPrice {
    OneClickUndo,
    CleanupUndo,
    NoUndo,
}

pub fn classify_undo_price(action: &str) -> UndoPrice {
    let low = action.to_ascii_lowercase();
    if [
        "push --force",
        "gh pr merge",
        "publish",
        "drop table",
        "delete_prod",
        "send_email",
        "wire_transfer",
        "deploy_prod",
    ]
    .iter()
    .any(|k| low.contains(k))
    {
        return UndoPrice::NoUndo;
    }
    if [
        "create_pr",
        "create_branch",
        "write_file",
        "edit_file",
        "mkdir",
        "docker run",
        "git commit",
    ]
    .iter()
    .any(|k| low.contains(k))
    {
        return UndoPrice::CleanupUndo;
    }
    UndoPrice::OneClickUndo
}

/// Timeout Destination Pre-Check: before retrying an external write after a
/// timeout, verify whether the destination already recorded the effect.
pub fn should_retry_after_timeout(
    price: UndoPrice,
    destination_already_applied: bool,
) -> Result<bool, String> {
    if destination_already_applied {
        return Ok(false);
    }
    if price == UndoPrice::NoUndo {
        return Err(
            "UNDO_PRICE_GATE: NO_UNDO action timed out; destination pre-check required with explicit human confirmation before retry".into(),
        );
    }
    Ok(true)
}

/// ATIF single-model / single-effort lock active under `--pinned-model`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AtifPinnedModelLock {
    pub pinned_model: String,
    pub pinned_effort: Option<String>,
}

impl AtifPinnedModelLock {
    pub fn new(model: impl Into<String>, effort: Option<String>) -> Self {
        Self {
            pinned_model: model.into(),
            pinned_effort: effort,
        }
    }

    pub fn validate(&self, requested_model: &str, requested_effort: Option<&str>) -> Result<(), String> {
        if requested_model != self.pinned_model {
            return Err(format!(
                "ATIF_PINNED_MODEL_LOCK: model switch '{requested_model}' denied (pinned to '{}')",
                self.pinned_model
            ));
        }
        if let Some(ref locked_effort) = self.pinned_effort {
            if requested_effort != Some(locked_effort.as_str()) {
                return Err(format!(
                    "ATIF_PINNED_MODEL_LOCK: effort change '{requested_effort:?}' denied (pinned to '{locked_effort}')"
                ));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod p1_11_tests {
    use super::*;

    #[test]
    fn aepn_petri_net_resets_tests_and_critic_on_edit() {
        let mut net = AepnSupervisor::new(10);
        assert!(net.record_tests_green().is_err());
        net.lock_contract();
        net.record_selftest_red();
        net.record_tests_green().unwrap();
        net.record_critic_approved().unwrap();
        assert!(net.can_complete(false).is_ok());
        assert!(net.can_complete(true).is_err());

        // Any code edit resets tests_green & critic_approved to 0.
        net.on_code_edit().unwrap();
        assert_eq!(net.p_tests_green, 0);
        assert_eq!(net.p_critic_approved, 0);
        assert!(net.can_complete(false).is_err());

        net.record_tests_green().unwrap();
        net.record_critic_approved().unwrap();
        net.grant_human_approval();
        assert!(net.can_complete(true).is_ok());
    }

    #[test]
    fn undo_price_timeout_precheck_and_atif_lock() {
        assert_eq!(classify_undo_price("read_file"), UndoPrice::OneClickUndo);
        assert_eq!(classify_undo_price("edit_file"), UndoPrice::CleanupUndo);
        assert_eq!(classify_undo_price("gh pr merge 1"), UndoPrice::NoUndo);

        assert!(!should_retry_after_timeout(UndoPrice::CleanupUndo, true).unwrap());
        assert!(should_retry_after_timeout(UndoPrice::CleanupUndo, false).unwrap());
        assert!(should_retry_after_timeout(UndoPrice::NoUndo, false).is_err());

        let lock = AtifPinnedModelLock::new("claude-sonnet-4", Some("medium".into()));
        assert!(lock.validate("claude-sonnet-4", Some("medium")).is_ok());
        assert!(lock.validate("gpt-4o", Some("medium")).is_err());
        assert!(lock.validate("claude-sonnet-4", Some("high")).is_err());
    }
}