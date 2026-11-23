//! Live Token Streaming & Thinking Display.
//!
//! Provides non-blocking streaming state buffers, live telemetry (tok/s, token count,
//! elapsed duration), and collapsible thinking blocks with responsive folding.

use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::{sanitize_ansi, sanitize_to_spans, Palette};

/// State of an active or completed streaming LLM generation.
#[derive(Debug, Clone)]
pub struct StreamState {
    pub thinking_buffer: String,
    pub content_buffer: String,
    pub is_thinking_collapsed: bool,
    pub user_pinned_thinking: bool,
    pub tok_per_sec: f32,
    pub elapsed_secs: f32,
    pub token_count: usize,
    pub is_complete: bool,
}

/// Thinking state specification alias.
pub type ThinkingState = StreamState;

impl Default for StreamState {
    fn default() -> Self {
        Self {
            thinking_buffer: String::new(),
            content_buffer: String::new(),
            is_thinking_collapsed: false,
            user_pinned_thinking: false,
            tok_per_sec: 0.0,
            elapsed_secs: 0.0,
            token_count: 0,
            is_complete: false,
        }
    }
}

impl StreamState {
    /// A single-cell spinner while waiting for or receiving reasoning tokens.
    /// Use the existing UI clock so rendering never needs a timer or a sleep.
    pub fn thinking_frame(&self) -> Option<&'static str> {
        const FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
        if self.is_complete || !self.content_buffer.is_empty() {
            return None;
        }
        let frame = (self.elapsed_secs.max(0.0) * 8.0) as usize % FRAMES.len();
        Some(FRAMES[frame])
    }

    /// Append a chunk of reasoning/thinking tokens to the thinking buffer.
    pub fn push_thinking(&mut self, text: &str) {
        self.thinking_buffer.push_str(text);
    }

    /// Append a chunk of response/content tokens to the content buffer.
    /// Automatically collapses the thinking block unless the user explicitly
    /// pinned it open.
    pub fn push_content(&mut self, text: &str) {
        if !self.thinking_buffer.is_empty() && !self.user_pinned_thinking {
            self.is_thinking_collapsed = true;
        }
        self.content_buffer.push_str(text);
    }

    /// Toggle thinking block collapse state.
    /// If user expands it, it marks the thinking block as user-pinned so
    /// subsequent content tokens will not collapse it.
    pub fn toggle_thinking_collapse(&mut self) {
        self.is_thinking_collapsed = !self.is_thinking_collapsed;
        self.user_pinned_thinking = !self.is_thinking_collapsed;
    }

    /// Alias for toggle_thinking_collapse.
    pub fn toggle_thinking_collapsed(&mut self) {
        self.toggle_thinking_collapse();
    }

    /// Format elapsed time into human-friendly string (`{:.1}s` under 60s, `{mins}m {secs}s` above 60s).
    pub fn formatted_elapsed(&self) -> String {
        if self.elapsed_secs < 60.0 {
            format!("{:.1}s", self.elapsed_secs)
        } else {
            let mins = (self.elapsed_secs / 60.0).floor() as u64;
            let secs = (self.elapsed_secs % 60.0) as u64;
            format!("{}m {}s", mins, secs)
        }
    }

    /// Safe tokens-per-second calculation with division-by-zero protection.
    pub fn calculate_rate(token_count: usize, elapsed_secs: f32) -> f32 {
        if elapsed_secs <= 0.0001 {
            0.0
        } else {
            token_count as f32 / elapsed_secs
        }
    }
}

/// Renders the streaming response including thinking block, divider, and content.
pub fn render_streaming_view(frame: &mut Frame, area: Rect, state: &StreamState, pal: &Palette) {
    if area.is_empty() {
        return;
    }
    let mut lines = Vec::new();

    // 1. Thinking block region
    if !state.thinking_buffer.is_empty() {
        let duration_str = state.formatted_elapsed();
        let rate_str = format!("{:.1} tok/s", state.tok_per_sec);

        if state.is_thinking_collapsed {
            let collapsed_line = Line::from(vec![Span::styled(
                format!(
                    " {} {} for {} ({} tokens) [Ctrl+T to expand] ",
                    state.thinking_frame().unwrap_or("▸"),
                    if state.thinking_frame().is_some() {
                        "Thinking"
                    } else {
                        "Thought"
                    },
                    duration_str,
                    state.token_count
                ),
                Style::default()
                    .fg(pal.chrome_fg)
                    .add_modifier(Modifier::ITALIC),
            )]);
            lines.push(collapsed_line);
        } else {
            let header = Line::from(vec![
                Span::styled(
                    format!(
                        " {} {} ",
                        state.thinking_frame().unwrap_or("▾"),
                        if state.thinking_frame().is_some() {
                            "Thinking"
                        } else {
                            "Thought"
                        }
                    ),
                    Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!(
                        "({} \u{2022} {} \u{2022} {} tokens) ",
                        duration_str, rate_str, state.token_count
                    ),
                    Style::default().fg(pal.chrome_fg),
                ),
                Span::styled(
                    "[Ctrl+T to collapse] ",
                    Style::default().fg(pal.panel_border),
                ),
            ]);
            lines.push(header);

            for t_line in state.thinking_buffer.lines().take(10) {
                lines.push(Line::from(Span::styled(
                    format!("   {}", sanitize_ansi(t_line)),
                    Style::default()
                        .fg(pal.chrome_fg)
                        .add_modifier(Modifier::ITALIC),
                )));
            }

            lines.push(Line::from(Span::styled(
                "\u{2500}".repeat(area.width as usize),
                Style::default().fg(pal.panel_border),
            )));
        }
    }

    // 2. Content region
    for c_line in state.content_buffer.lines() {
        let spans = sanitize_to_spans(c_line, Style::default().fg(pal.viewport_fg));
        lines.push(Line::from(spans));
    }

    frame.render_widget(Paragraph::new(lines), area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::DARK_SLATE;
    use ratatui::{backend::TestBackend, Terminal};

    #[test]
    fn test_stream_thinking_and_content_chunks() {
        let mut st = StreamState::default();
        st.push_thinking("Thinking line 1\n");
        st.push_thinking("Thinking line 2\n");
        assert_eq!(st.thinking_buffer, "Thinking line 1\nThinking line 2\n");
        assert!(!st.is_thinking_collapsed);

        st.push_content("Content line 1\n");
        assert!(
            st.is_thinking_collapsed,
            "push_content auto-collapses thinking"
        );
        assert_eq!(st.content_buffer, "Content line 1\n");
    }

    #[test]
    fn test_stream_user_pinned_thinking_prevents_collapse() {
        let mut st = StreamState::default();
        st.push_thinking("Keep this open\n");
        st.user_pinned_thinking = true;

        st.push_content("Answer line\n");
        assert!(!st.is_thinking_collapsed, "User pinned thinking stays open");
    }

    #[test]
    fn test_stream_metrics_and_formatting() {
        let mut st = StreamState::default();
        st.elapsed_secs = 2.4;
        assert_eq!(st.formatted_elapsed(), "2.4s");

        st.elapsed_secs = 75.0;
        assert_eq!(st.formatted_elapsed(), "1m 15s");

        st.elapsed_secs = 3600.0;
        assert_eq!(st.formatted_elapsed(), "60m 0s");

        let rate = StreamState::calculate_rate(521, 10.0);
        assert!((rate - 52.1).abs() < 0.05);

        let div_zero = StreamState::calculate_rate(100, 0.0);
        assert_eq!(div_zero, 0.0);
        assert!(!div_zero.is_nan());
        assert!(!div_zero.is_infinite());
    }

    #[test]
    fn test_render_streaming_view_expanded_and_collapsed() {
        let backend = TestBackend::new(80, 15);
        let mut terminal = Terminal::new(backend).unwrap();

        let mut st = StreamState::default();
        st.push_thinking("Step 1 reasoning\nStep 2 reasoning\n");
        st.elapsed_secs = 3.2;
        st.tok_per_sec = 45.0;
        st.token_count = 144;
        st.user_pinned_thinking = true;
        st.push_content("Hello from model\n");

        // Expanded render
        terminal
            .draw(|f| {
                render_streaming_view(f, Rect::new(0, 0, 80, 10), &st, &DARK_SLATE);
            })
            .unwrap();

        let buf = terminal.backend().buffer();
        let mut has_header = false;
        let mut has_reasoning = false;
        let mut has_content = false;
        for y in 0..10 {
            let row: String = (0..80).map(|x| buf[(x, y)].symbol()).collect();
            if row.contains("Thought") && row.contains("45.0 tok/s") {
                has_header = true;
            }
            if row.contains("Step 1 reasoning") {
                has_reasoning = true;
            }
            if row.contains("Hello from model") {
                has_content = true;
            }
        }
        assert!(has_header);
        assert!(has_reasoning);
        assert!(has_content);

        // Toggle to collapsed
        st.toggle_thinking_collapsed();
        assert!(st.is_thinking_collapsed);

        terminal
            .draw(|f| {
                render_streaming_view(f, Rect::new(0, 0, 80, 10), &st, &DARK_SLATE);
            })
            .unwrap();

        let buf2 = terminal.backend().buffer();
        let mut has_collapsed_badge = false;
        let mut has_hidden_reasoning = false;
        for y in 0..10 {
            let row: String = (0..80).map(|x| buf2[(x, y)].symbol()).collect();
            if row.contains("Thought for 3.2s (144 tokens)") {
                has_collapsed_badge = true;
            }
            if row.contains("Step 1 reasoning") {
                has_hidden_reasoning = true;
            }
        }
        assert!(has_collapsed_badge);
        assert!(!has_hidden_reasoning);
    }
}
