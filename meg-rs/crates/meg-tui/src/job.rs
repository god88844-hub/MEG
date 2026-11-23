//! Active Job Execution Indicator widget and status tracker.
//!
//! Provides single-line docked job status above the composer, animated spinner
//! cycling across braille glyphs (`⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏`), compact elapsed timer,
//! folded subprocess log output (`  └ ... (+N lines)`), and cancellation states.

use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::{sanitize_ansi, Palette};

/// Braille spinner sequence (10 frames) cycling every 80-100ms.
pub const SPINNER_FRAMES: [char; 10] = [
    '\u{280b}', // ⠋
    '\u{2819}', // ⠙
    '\u{2839}', // ⠹
    '\u{2838}', // ⠸
    '\u{283c}', // ⠼
    '\u{2834}', // ⠴
    '\u{2826}', // ⠦
    '\u{2827}', // ⠧
    '\u{2807}', // ⠇
    '\u{280f}', // ⠏
];

/// Branch glyph prefix for folded output details.
pub const DETAILS_PREFIX: &str = "  \u{2514} ";

/// Format elapsed seconds into a compact human-friendly form.
/// Examples: 0s, 12s, 1m 05s, 1h 02m.
pub fn fmt_elapsed_compact(elapsed_secs: u64) -> String {
    if elapsed_secs < 60 {
        format!("{elapsed_secs}s")
    } else if elapsed_secs < 3600 {
        let mins = elapsed_secs / 60;
        let secs = elapsed_secs % 60;
        format!("{mins}m {secs:02}s")
    } else {
        let hours = elapsed_secs / 3600;
        let mins = (elapsed_secs % 3600) / 60;
        format!("{hours}h {mins:02}m")
    }
}

/// State of an active or recently completed background tool job.
#[derive(Debug, Clone)]
pub struct JobState {
    pub id: String,
    pub description: String,
    pub status: String,
    pub status_line: String,
    pub output_lines: Vec<String>,
    pub spinner_frame: usize,
    pub spinner_tick: usize,
    pub spinner_timer: f32,
    pub elapsed_secs: f32,
    pub is_active: bool,
    pub is_failed: bool,
    pub is_cancelled: bool,
    pub error_msg: Option<String>,
}

/// Type alias for parity with interface specifications.
pub type JobIndicator = JobState;

impl Default for JobState {
    fn default() -> Self {
        Self {
            id: String::new(),
            description: String::new(),
            status: String::new(),
            status_line: String::new(),
            output_lines: Vec::new(),
            spinner_frame: 0,
            spinner_tick: 0,
            spinner_timer: 0.0,
            elapsed_secs: 0.0,
            is_active: false,
            is_failed: false,
            is_cancelled: false,
            error_msg: None,
        }
    }
}

impl JobState {
    /// Create a new JobState with given ID and description and start it immediately.
    pub fn new(id: String, description: String) -> Self {
        let mut st = Self::default();
        st.start(id, description);
        st
    }

    /// Start a new job lifecycle, resetting elapsed time, ticks, and output lines.
    pub fn start(&mut self, id: String, desc: String) {
        self.id = id;
        self.description = desc;
        self.status = "running".into();
        self.status_line.clear();
        self.output_lines.clear();
        self.spinner_frame = 0;
        self.spinner_tick = 0;
        self.spinner_timer = 0.0;
        self.elapsed_secs = 0.0;
        self.is_active = true;
        self.is_failed = false;
        self.is_cancelled = false;
        self.error_msg = None;
    }

    /// Advance spinner frame and elapsed seconds by delta.
    pub fn tick(&mut self, delta_secs: f32) {
        self.spinner_timer += delta_secs;
        self.spinner_tick = (self.spinner_tick + 1) % SPINNER_FRAMES.len();
        self.spinner_frame = self.spinner_tick;
        self.elapsed_secs += delta_secs;
    }

    /// Return current spinner braille glyph.
    pub fn spinner_glyph(&self) -> char {
        SPINNER_FRAMES[self.spinner_tick % SPINNER_FRAMES.len()]
    }

    /// Format elapsed duration. Under 60 seconds produces `{:.1}s`, at or above 60 seconds
    /// produces compact `{mins}m {secs:02}s`.
    pub fn formatted_duration(&self) -> String {
        if self.elapsed_secs < 60.0 {
            format!("{:.1}s", self.elapsed_secs)
        } else {
            fmt_elapsed_compact(self.elapsed_secs.round() as u64)
        }
    }

    /// Mark the job as cancelled by the user.
    pub fn cancel(&mut self) {
        self.is_cancelled = true;
        self.is_failed = false;
        self.is_active = false;
        self.status = "cancelled".into();
        self.status_line = "[cancelled by user]".into();
    }

    /// Mark the job as failed with an error message.
    pub fn fail(&mut self, err: String) {
        self.is_failed = true;
        self.is_cancelled = false;
        self.is_active = false;
        self.status = "failed".into();
        self.status_line = format!("[error: {}]", err);
        self.error_msg = Some(err);
    }

    /// Mark the job as cleanly completed.
    pub fn complete(&mut self) {
        self.is_active = false;
        self.status = "completed".into();
    }

    /// Append a subprocess output line to the job's log.
    pub fn push_output(&mut self, line: String) {
        self.output_lines.push(line);
    }
}

/// Computes the required layout height for the job indicator widget.
/// - Returns 2 if active with at least one output line.
/// - Returns 1 if active with no output lines, or if showing cancelled/failed status.
/// - Returns 0 if inactive/completed or None.
pub fn job_indicator_height(state: Option<&JobState>) -> u16 {
    match state {
        Some(j) if j.is_active => {
            if j.output_lines.is_empty() {
                1
            } else {
                2
            }
        }
        Some(j) if j.is_cancelled || j.is_failed => 1,
        _ => 0,
    }
}

/// Renders the job indicator docked above the composer.
pub fn render_job_indicator(frame: &mut Frame, area: Rect, state: &JobState, pal: &Palette) {
    if area.is_empty() || area.height == 0 {
        return;
    }
    if !state.is_active && !state.is_failed && !state.is_cancelled {
        return;
    }

    let mut lines: Vec<Line<'static>> = Vec::new();

    if state.is_active {
        let glyph = state.spinner_glyph();
        let dur_str = state.formatted_duration();
        let clean_desc = sanitize_ansi(&state.description);

        let header = Line::from(vec![
            Span::styled(
                format!(" {} ", glyph),
                Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                clean_desc,
                Style::default()
                    .fg(pal.viewport_fg)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(" ({} \u{2022} esc to interrupt)", dur_str),
                Style::default().fg(pal.chrome_fg),
            ),
        ]);
        lines.push(header);

        if area.height >= 2 {
            if let Some(last_output) = state.output_lines.last() {
                let clean_output = sanitize_ansi(last_output);
                let count = state.output_lines.len();
                let folded_msg = if count > 1 {
                    format!("  \u{2514} {} (+{} lines)", clean_output, count - 1)
                } else {
                    format!("  \u{2514} {}", clean_output)
                };
                lines.push(Line::from(Span::styled(
                    folded_msg,
                    Style::default().fg(pal.chrome_fg),
                )));
            }
        }
    } else if state.is_cancelled {
        let clean_desc = sanitize_ansi(&state.description);
        lines.push(Line::from(vec![
            Span::styled(" \u{2717} ", Style::default().fg(pal.warning)),
            Span::styled(
                format!("{} \u{2014} cancelled", clean_desc),
                Style::default().fg(pal.warning),
            ),
        ]));
    } else if state.is_failed {
        let clean_desc = sanitize_ansi(&state.description);
        let clean_status = sanitize_ansi(&state.status_line);
        lines.push(Line::from(vec![
            Span::styled(" \u{2717} ", Style::default().fg(pal.error)),
            Span::styled(
                format!("{} \u{2014} {}", clean_desc, clean_status),
                Style::default().fg(pal.error),
            ),
        ]));
    }

    frame.render_widget(Paragraph::new(lines), area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::DARK_SLATE;
    use ratatui::{backend::TestBackend, Terminal};

    #[test]
    fn test_compact_elapsed_formatting() {
        assert_eq!(fmt_elapsed_compact(0), "0s");
        assert_eq!(fmt_elapsed_compact(12), "12s");
        assert_eq!(fmt_elapsed_compact(59), "59s");
        assert_eq!(fmt_elapsed_compact(60), "1m 00s");
        assert_eq!(fmt_elapsed_compact(65), "1m 05s");
        assert_eq!(fmt_elapsed_compact(3599), "59m 59s");
        assert_eq!(fmt_elapsed_compact(3600), "1h 00m");
        assert_eq!(fmt_elapsed_compact(3725), "1h 02m");
    }

    #[test]
    fn test_job_state_lifecycle_and_spinner_cycling() {
        let mut job = JobState::default();
        assert!(!job.is_active);
        assert_eq!(job_indicator_height(Some(&job)), 0);

        job.start("job-1".into(), "Cargo check".into());
        assert!(job.is_active);
        assert_eq!(job_indicator_height(Some(&job)), 1);

        let initial_glyph = job.spinner_glyph();
        assert_eq!(initial_glyph, SPINNER_FRAMES[0]);

        // Advance 10 ticks to complete a full cycle
        for i in 1..=10 {
            job.tick(0.08);
            assert_eq!(
                job.spinner_glyph(),
                SPINNER_FRAMES[i % SPINNER_FRAMES.len()]
            );
        }
        assert_eq!(job.spinner_glyph(), SPINNER_FRAMES[0]);

        // Output lines expand height to 2
        job.push_output("Checking crates...".into());
        assert_eq!(job_indicator_height(Some(&job)), 2);

        // Completion clears active status and collapses height to 0
        job.complete();
        assert!(!job.is_active);
        assert_eq!(job_indicator_height(Some(&job)), 0);
    }

    #[test]
    fn test_job_indicator_render_active_and_folded() {
        let backend = TestBackend::new(80, 5);
        let mut terminal = Terminal::new(backend).unwrap();

        let mut job = JobState::default();
        job.start("job-check".into(), "Testing tool".into());
        job.push_output("output line 1".into());
        job.push_output("output line 2".into());
        job.push_output("output line 3".into());
        job.elapsed_secs = 2.5;

        terminal
            .draw(|f| {
                render_job_indicator(f, Rect::new(0, 0, 80, 2), &job, &DARK_SLATE);
            })
            .unwrap();

        let buf = terminal.backend().buffer();
        let row0: String = (0..80).map(|x| buf[(x, 0)].symbol()).collect();
        let row1: String = (0..80).map(|x| buf[(x, 1)].symbol()).collect();

        assert!(row0.contains("Testing tool"));
        assert!(row0.contains("2.5s"));
        assert!(row0.contains("esc to interrupt"));
        assert!(row1.contains("output line 3"));
        assert!(row1.contains("(+2 lines)"));
        assert!(row1.contains('\u{2514}'));
    }

    #[test]
    fn test_job_cancellation_and_failure_rendering() {
        let backend = TestBackend::new(80, 5);
        let mut terminal = Terminal::new(backend).unwrap();

        let mut job = JobState::default();
        job.start("job-c".into(), "Long run".into());
        job.cancel();
        assert_eq!(job_indicator_height(Some(&job)), 1);

        terminal
            .draw(|f| {
                render_job_indicator(f, Rect::new(0, 0, 80, 1), &job, &DARK_SLATE);
            })
            .unwrap();

        let buf = terminal.backend().buffer();
        let row0: String = (0..80).map(|x| buf[(x, 0)].symbol()).collect();
        assert!(row0.contains("Long run — cancelled"));
        assert!(row0.contains('\u{2717}'));

        job.fail("timeout after 30s".into());
        terminal
            .draw(|f| {
                render_job_indicator(f, Rect::new(0, 0, 80, 1), &job, &DARK_SLATE);
            })
            .unwrap();

        let buf2 = terminal.backend().buffer();
        let row0_fail: String = (0..80).map(|x| buf2[(x, 0)].symbol()).collect();
        assert!(row0_fail.contains("Long run — [error: timeout after 30s]"));
    }
}
