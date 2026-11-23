//! Observation truncation with:
//! 1. RTK log deduplicator (`compact_text` folding >2 repeated lines),
//! 2. Scratch-file spill (`.meg/scratch/cmd_<id>.log` when >150 lines or >4,000 chars,
//!    returning `head=2400, tail=900`),
//! 3. Critical-line preservation (`FATAL`, `ERROR`, `Traceback`, `FAILED`, etc.),
//! 4. Auto-bypass once `first_completion_claimed == true`.

use std::path::Path;

const CRITICAL_MARKERS: &[&str] = &[
    "FATAL",
    "ERROR",
    "Exception",
    "Traceback",
    "FAILED",
    "AssertionError",
    "SyntaxError",
    "ModuleNotFoundError",
    "PermissionError",
    "FileNotFoundError",
    "panic",
];

pub const SCRATCH_LINE_THRESHOLD: usize = 150;
pub const SCRATCH_CHAR_THRESHOLD: usize = 4_000;
pub const SCRATCH_HEAD_CHARS: usize = 2_400;
pub const SCRATCH_TAIL_CHARS: usize = 900;

/// RTK log deduplicator: folds runs of >2 identical consecutive lines into
/// `... [previous line repeated N more times]`.
pub fn compact_text(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() <= 2 {
        return text.to_string();
    }
    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let mut run = 1usize;
        while i + run < lines.len() && lines[i + run] == line {
            run += 1;
        }
        if run > 2 {
            out.push(line.to_string());
            out.push(format!("... [previous line repeated {} more times]", run - 1));
        } else {
            for _ in 0..run {
                out.push(line.to_string());
            }
        }
        i += run;
    }
    let mut joined = out.join("\n");
    if text.ends_with('\n') && !joined.ends_with('\n') {
        joined.push('\n');
    }
    joined
}

/// Truncate with RTK deduplication + `.meg/scratch/cmd_<id>.log` spill.
/// Auto-bypasses truncation once `first_completion_claimed == true`.
pub fn truncate_with_scratch(
    workspace: &Path,
    cmd_id: &str,
    text: &str,
    first_completion_claimed: bool,
) -> String {
    if first_completion_claimed {
        return text.to_string();
    }
    let compacted = compact_text(text);
    let line_count = compacted.lines().count();
    if line_count <= SCRATCH_LINE_THRESHOLD && compacted.len() <= SCRATCH_CHAR_THRESHOLD {
        return compacted;
    }

    let safe_id: String = cmd_id
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' })
        .collect();
    let scratch_dir = workspace.join(".meg").join("scratch");
    let scratch_file = scratch_dir.join(format!("cmd_{safe_id}.log"));
    let spill_note = if std::fs::create_dir_all(&scratch_dir)
        .and_then(|_| std::fs::write(&scratch_file, text))
        .is_ok()
    {
        format!("\n...[full output spilled to .meg/scratch/cmd_{safe_id}.log ({line_count} lines, {} bytes)]", text.len())
    } else {
        String::new()
    };

    let head_end = compacted.floor_char_boundary(SCRATCH_HEAD_CHARS.min(compacted.len()));
    let tail_start = compacted.ceil_char_boundary(compacted.len().saturating_sub(SCRATCH_TAIL_CHARS));
    if head_end >= tail_start {
        return format!("{compacted}{spill_note}");
    }
    let head = &compacted[..head_end];
    let tail = &compacted[tail_start..];
    let middle = &compacted[head_end..tail_start];
    let mut pinned = String::new();
    for line in middle.lines() {
        if CRITICAL_MARKERS.iter().any(|m| line.contains(m)) {
            let clipped = if line.len() > 200 {
                &line[..line.floor_char_boundary(200)]
            } else {
                line
            };
            pinned.push_str("\n[PINNED] ");
            pinned.push_str(clipped);
            if pinned.len() > 600 {
                break;
            }
        }
    }
    format!(
        "{head}\n...[truncated {} bytes]{pinned}{spill_note}\n{tail}",
        middle.len()
    )
}

/// Truncate `text` to at most `limit` bytes while preserving head, tail, and
/// any critical error lines that fell into the dropped middle.
pub fn truncate_observation(text: &str, limit: usize) -> String {
    if text.len() <= limit || limit < 120 {
        return text.to_string();
    }
    let head_len = (limit as f32 * 0.55) as usize;
    let tail_len = (limit as f32 * 0.30) as usize;
    let head_end = text.floor_char_boundary(head_len.min(text.len()));
    let tail_start = text.ceil_char_boundary(text.len().saturating_sub(tail_len));
    if head_end >= tail_start {
        return text.to_string();
    }
    let head = &text[..head_end];
    let tail = &text[tail_start..];
    let middle = &text[head_end..tail_start];

    let mut pinned = String::new();
    let pin_budget = limit.saturating_sub(head.len() + tail.len() + 64);
    for line in middle.lines() {
        if CRITICAL_MARKERS.iter().any(|m| line.contains(m)) {
            if pinned.len() + line.len() + 1 > pin_budget {
                break;
            }
            if !pinned.is_empty() {
                pinned.push('\n');
            }
            pinned.push_str(line);
        }
    }

    let dropped = middle.len();
    if pinned.is_empty() {
        format!("{head}\n...[truncated {dropped} chars]...\n{tail}")
    } else {
        format!("{head}\n...[truncated {dropped} chars; critical lines preserved]:\n{pinned}\n...[end preserved]...\n{tail}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_text_untouched() {
        assert_eq!(truncate_observation("hello", 1000), "hello");
    }

    #[test]
    fn keeps_head_and_tail() {
        let text = format!("{}{}{}", "A".repeat(500), "M".repeat(2000), "Z".repeat(500));
        let out = truncate_observation(&text, 600);
        assert!(out.starts_with("AAAA"));
        assert!(out.ends_with("ZZZZ"));
        assert!(out.contains("truncated"));
    }

    #[test]
    fn pins_dropped_fatal_line() {
        let mut s = String::new();
        for i in 0..200 {
            s.push_str(&format!("line {i} normal output\n"));
            if i == 100 {
                s.push_str("FATAL: database refused connection\n");
            }
        }
        let out = truncate_observation(&s, 800);
        assert!(out.contains("FATAL: database refused connection"), "got: {out}");
    }

    #[test]
    fn handles_multibyte_utf8_without_panicking() {
        let text = format!(
            "{}\nFATAL: boom \u{1F680}\n{}",
            "\u{4E16}".repeat(300),
            "\u{754C}".repeat(300)
        );
        let out = truncate_observation(&text, 400);
        assert!(out.contains("truncated"));
    }

    #[test]
    fn rtk_compact_text_folds_repeated_lines_and_scratch_spill_bypasses_after_claim() {
        let mut log = String::new();
        for _ in 0..10 {
            log.push_str("polling status...\n");
        }
        log.push_str("done\n");
        let compacted = compact_text(&log);
        assert!(compacted.contains("previous line repeated 9 more times"), "{compacted}");
        assert!(compacted.contains("done"));

        let ws = std::env::temp_dir().join(format!("meg-scratch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&ws);
        std::fs::create_dir_all(&ws).unwrap();
        let big = (0..200).map(|i| format!("unique line {i}: {}\n", "x".repeat(25))).collect::<String>();

        let spilled = truncate_with_scratch(&ws, "t1", &big, false);
        assert!(spilled.contains(".meg/scratch/cmd_t1.log"), "{spilled}");
        assert!(ws.join(".meg/scratch/cmd_t1.log").is_file());

        // Auto-bypass once first_completion_claimed == true.
        let bypassed = truncate_with_scratch(&ws, "t2", &big, true);
        assert_eq!(bypassed, big);
        let _ = std::fs::remove_dir_all(&ws);
    }
}