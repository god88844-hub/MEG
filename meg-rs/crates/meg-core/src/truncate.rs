//! Observation truncation: fit to budget, keep head AND tail, preserve
//! error lines (the headroom/critical-line contract from the Python engine).

const CRITICAL: &[&str] = &[
    "Traceback", "Error", "ERROR", "FATAL", "Fatal", "FAIL", "Exception",
    "exit code", "exit with", "Segmentation fault",
];

/// Truncate `text` to `limit` bytes on valid UTF-8 boundaries, keeping head +
/// tail and pinning critical lines (error/traceback lines) so they always reach
/// the model.
pub fn truncate_observation(text: &str, limit: usize) -> String {
    if text.len() <= limit {
        return text.to_string();
    }
    let lines: Vec<&str> = text.lines().collect();

    let head = limit * 70 / 100;
    let head_end = text.floor_char_boundary(head.min(text.len()));
    let tail = limit.saturating_sub(head).saturating_sub(200).max(200);

    // Pin dropped critical lines verbatim (headroom contract).
    let pinned_block: String = {
        let missing: Vec<&&str> = lines
            .iter()
            .filter(|l| CRITICAL.iter().any(|c| l.contains(c)))
            .filter(|l| {
                // only lines that the head window won't already keep
                text.find(**l).map(|idx| idx >= head_end).unwrap_or(true)
            })
            .take(20)
            .collect();
        if missing.is_empty() {
            String::new()
        } else {
            format!(
                "\n--- critical lines preserved ---\n{}\n",
                missing.iter().map(|l| **l).collect::<Vec<_>>().join("\n")
            )
        }
    };

    let last = last_chars(text, tail);
    let elided = text.len().saturating_sub(head_end + last.len());

    // Build with a shrinking head so the budget is met WITHOUT cutting the
    // tail or the pinned critical block (naive end-truncation would drop
    // exactly the lines this function exists to preserve).
    let mut out = format!(
        "{}\n...[{} chars elided]...\n{}{}",
        &text[..head_end],
        elided,
        last,
        pinned_block
    );
    let mut head_keep = head;
    while out.len() > limit && head_keep > 100 {
        head_keep /= 2;
        let keep_end = text.floor_char_boundary(head_keep.min(text.len()));
        out = format!(
            "{}\n...[{} chars elided]...\n{}{}",
            &text[..keep_end],
            text.len().saturating_sub(keep_end + last.len()),
            last,
            pinned_block
        );
    }
    if out.len() > limit {
        let final_end = out.floor_char_boundary(limit);
        out.truncate(final_end);
    }
    out
}

fn last_chars(s: &str, n: usize) -> &str {
    if s.len() <= n {
        return s;
    }
    let start = s.ceil_char_boundary(s.len() - n);
    &s[start..]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_head_and_tail() {
        let body = format!("{}\nexit code 1\n{}", "a".repeat(2000), "b".repeat(2000));
        let out = truncate_observation(&body, 1000);
        assert!(out.contains("exit code 1"));
        assert!(out.contains("chars elided"));
        assert!(out.len() <= 1100);
    }

    #[test]
    fn pins_dropped_fatal_line() {
        let mut body = String::new();
        for i in 0..500 {
            body.push_str(&format!("row {i}\n"));
        }
        body.push_str("FATAL: database connection refused");
        let out = truncate_observation(&body, 600);
        assert!(out.contains("FATAL: database connection refused"));
    }

    #[test]
    fn short_text_untouched() {
        assert_eq!(truncate_observation("hello", 100), "hello");
    }

    #[test]
    fn handles_multibyte_utf8_without_panicking() {
        let mut body = String::new();
        for i in 0..200 {
            body.push_str(&format!("line {i} \u{4E16}\u{754C} \u{1F680}\n"));
        }
        body.push_str("FATAL: \u{0924}\u{094D}\u{0930}\u{0941}\u{091F}\u{093F}");
        for limit in [17, 50, 101, 601, 1003] {
            let out = truncate_observation(&body, limit);
            assert!(out.len() <= limit, "len {} exceeded limit {limit}", out.len());
        }
        let preserved = truncate_observation(&body, 601);
        assert!(preserved.contains("FATAL: \u{0924}\u{094D}\u{0930}\u{0941}\u{091F}\u{093F}"));
    }
}