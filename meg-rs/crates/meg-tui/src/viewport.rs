use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Wrap},
    Frame,
};

pub use crate::stream::render_streaming_view;
use crate::{Palette, WorkstationState};

/// Strips all ANSI escape sequences (CSI and OSC) from an input string.
pub fn sanitize_ansi(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            if let Some(&next) = chars.peek() {
                if next == '[' {
                    chars.next(); // consume '['
                                  // CSI parameter bytes (0x30-0x3F)
                    while let Some(&b) = chars.peek() {
                        if (b as u32) >= 0x30 && (b as u32) <= 0x3F {
                            chars.next();
                        } else {
                            break;
                        }
                    }
                    // Optional intermediate bytes (0x20-0x2F except space)
                    while let Some(&b) = chars.peek() {
                        if (b as u32) > 0x20 && (b as u32) <= 0x2F {
                            chars.next();
                        } else {
                            break;
                        }
                    }
                    // Final command byte (0x40-0x7E)
                    if let Some(&b) = chars.peek() {
                        if (b as u32) >= 0x40 && (b as u32) <= 0x7E {
                            chars.next();
                        }
                    }
                } else if next == ']' {
                    chars.next(); // consume ']'
                                  // OSC sequence: consume until BEL (\x07) or ST (\x1b\)
                    while let Some(&b) = chars.peek() {
                        chars.next();
                        if b == '\x07' {
                            break;
                        }
                        if b == '\x1b' && chars.peek() == Some(&'\\') {
                            chars.next();
                            break;
                        }
                    }
                } else {
                    chars.next();
                }
            }
        } else if !c.is_control() || c == '\n' || c == '\t' {
            out.push(c);
        }
    }
    out
}

/// Alias for `sanitize_ansi`.
pub fn strip_ansi(input: &str) -> String {
    sanitize_ansi(input)
}

/// Converts an ANSI-containing string into sanitized Ratatui `Span`s.
pub fn sanitize_to_spans(input: &str, base_style: Style) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut cur_text = String::new();
    let mut cur_style = base_style;
    let mut chars = input.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '\x1b' {
            if let Some(&next) = chars.peek() {
                if next == '[' {
                    chars.next(); // consume '['
                    let mut code = String::new();
                    while let Some(&b) = chars.peek() {
                        if (b as u32) >= 0x30 && (b as u32) <= 0x3F {
                            code.push(b);
                            chars.next();
                        } else {
                            break;
                        }
                    }
                    while let Some(&b) = chars.peek() {
                        if (b as u32) > 0x20 && (b as u32) <= 0x2F {
                            code.push(b);
                            chars.next();
                        } else {
                            break;
                        }
                    }
                    if let Some(&b) = chars.peek() {
                        if (b as u32) >= 0x40 && (b as u32) <= 0x7E {
                            code.push(b);
                            chars.next();
                        }
                    }
                    if !cur_text.is_empty() {
                        spans.push(Span::styled(std::mem::take(&mut cur_text), cur_style));
                    }
                    // Handle SGR color/style commands
                    if code.ends_with('m') {
                        let inner = &code[..code.len() - 1];
                        for part in inner.split(';') {
                            match part {
                                "0" | "" => cur_style = base_style,
                                "1" => cur_style = cur_style.add_modifier(Modifier::BOLD),
                                "2" => cur_style = cur_style.add_modifier(Modifier::DIM),
                                "3" => cur_style = cur_style.add_modifier(Modifier::ITALIC),
                                "4" => cur_style = cur_style.add_modifier(Modifier::UNDERLINED),
                                "7" => cur_style = cur_style.add_modifier(Modifier::REVERSED),
                                // Standard 3/4-bit foreground
                                "30" => cur_style = cur_style.fg(Color::Black),
                                "31" => cur_style = cur_style.fg(Color::Red),
                                "32" => cur_style = cur_style.fg(Color::Green),
                                "33" => cur_style = cur_style.fg(Color::Yellow),
                                "34" => cur_style = cur_style.fg(Color::Blue),
                                "35" => cur_style = cur_style.fg(Color::Magenta),
                                "36" => cur_style = cur_style.fg(Color::Cyan),
                                "37" => cur_style = cur_style.fg(Color::White),
                                "39" => {
                                    cur_style = cur_style.fg(base_style.fg.unwrap_or(Color::Reset))
                                }
                                // Standard background
                                "40" => cur_style = cur_style.bg(Color::Black),
                                "41" => cur_style = cur_style.bg(Color::Red),
                                "42" => cur_style = cur_style.bg(Color::Green),
                                "43" => cur_style = cur_style.bg(Color::Yellow),
                                "44" => cur_style = cur_style.bg(Color::Blue),
                                "45" => cur_style = cur_style.bg(Color::Magenta),
                                "46" => cur_style = cur_style.bg(Color::Cyan),
                                "47" => cur_style = cur_style.bg(Color::White),
                                "49" => {
                                    cur_style = cur_style.bg(base_style.bg.unwrap_or(Color::Reset))
                                }
                                // High-intensity bright foreground
                                "90" => cur_style = cur_style.fg(Color::DarkGray),
                                "91" => cur_style = cur_style.fg(Color::LightRed),
                                "92" => cur_style = cur_style.fg(Color::LightGreen),
                                "93" => cur_style = cur_style.fg(Color::LightYellow),
                                "94" => cur_style = cur_style.fg(Color::LightBlue),
                                "95" => cur_style = cur_style.fg(Color::LightMagenta),
                                "96" => cur_style = cur_style.fg(Color::LightCyan),
                                "97" => cur_style = cur_style.fg(Color::White),
                                // High-intensity bright background
                                "100" => cur_style = cur_style.bg(Color::DarkGray),
                                "101" => cur_style = cur_style.bg(Color::LightRed),
                                "102" => cur_style = cur_style.bg(Color::LightGreen),
                                "103" => cur_style = cur_style.bg(Color::LightYellow),
                                "104" => cur_style = cur_style.bg(Color::LightBlue),
                                "105" => cur_style = cur_style.bg(Color::LightMagenta),
                                "106" => cur_style = cur_style.bg(Color::LightCyan),
                                "107" => cur_style = cur_style.bg(Color::White),
                                _ => {}
                            }
                        }
                    }
                } else if next == ']' {
                    chars.next(); // OSC
                    while let Some(&b) = chars.peek() {
                        chars.next();
                        if b == '\x07' || (b == '\x1b' && chars.peek() == Some(&'\\')) {
                            if b == '\x1b' {
                                chars.next();
                            }
                            break;
                        }
                    }
                }
            }
        } else if !c.is_control() || c == '\n' || c == '\t' {
            cur_text.push(c);
        }
    }
    if !cur_text.is_empty() {
        spans.push(Span::styled(cur_text, cur_style));
    }
    spans
}

/// Render both recorded messages and live deltas through the same wrapped viewport.
pub fn render_viewport(frame: &mut Frame, area: Rect, state: &WorkstationState, pal: &Palette) {
    if area.is_empty() {
        return;
    }
    let inner = crate::dashboard::panel(frame, area, "Chat / Agent", pal);
    if inner.is_empty() {
        return;
    }
    let mut lines: Vec<Line<'static>> = Vec::new();
    let start = state.transcript.len().saturating_sub(30_000);
    for raw in &state.transcript[start..] {
        let clean = sanitize_ansi(raw).replace('\t', "    ");
        if let Some(thought) = clean.strip_prefix("[thinking] ") {
            if !lines.is_empty() {
                lines.push(Line::raw(""));
            }
            let folded = state
                .stream
                .as_ref()
                .map(|s| s.is_thinking_collapsed)
                .unwrap_or(true);
            thought_lines(&mut lines, thought, folded, None, "", pal);
            continue;
        }
        let (role, content, color) = if let Some(rest) = clean
            .strip_prefix("❯ ")
            .or_else(|| clean.strip_prefix("> "))
        {
            ("user: ", rest, pal.viewport_fg)
        } else if let Some(rest) = clean
            .strip_prefix("✓ ")
            .or_else(|| clean.strip_prefix("✔ "))
            .or_else(|| clean.strip_prefix("[MEG] "))
        {
            ("meg:  ", rest, pal.accent)
        } else if let Some(rest) = clean.strip_prefix("[system] ") {
            ("", rest, pal.chrome_fg)
        } else if let Some(rest) = clean
            .strip_prefix("[error] ")
            .or_else(|| clean.strip_prefix("[tool failed] "))
        {
            ("error: ", rest, pal.error)
        } else if let Some(rest) = clean.strip_prefix("[tool ok] ") {
            ("tool: ", rest, pal.success)
        } else {
            ("", clean.as_str(), pal.chrome_fg)
        };
        if !lines.is_empty() {
            lines.push(Line::raw(""));
        }
        message_lines(&mut lines, role, content, color, pal);
    }
    if let Some(stream) = &state.stream {
        if !stream.thinking_buffer.is_empty() {
            if !lines.is_empty() {
                lines.push(Line::raw(""));
            }
            thought_lines(
                &mut lines,
                &stream.thinking_buffer,
                stream.is_thinking_collapsed,
                stream.thinking_frame(),
                &stream.formatted_elapsed(),
                pal,
            );
        }
        if !stream.content_buffer.is_empty() {
            if !lines.is_empty() {
                lines.push(Line::raw(""));
            }
            message_lines(
                &mut lines,
                "meg:  ",
                &stream.content_buffer,
                pal.accent,
                pal,
            );
        } else if !stream.is_complete && stream.thinking_buffer.is_empty() {
            lines.push(Line::styled(
                format!("meg:  {} thinking…", stream.thinking_frame().unwrap_or(" ")),
                Style::default().fg(pal.accent),
            ));
        }
    }
    if lines.is_empty() {
        lines.push(Line::raw(""));
        lines.push(Line::from(vec![
            Span::styled(
                "meg:  ",
                Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
            ),
            Span::styled("Ready when you are.", Style::default().fg(pal.viewport_fg)),
        ]));
        lines.push(Line::raw(""));
        lines.push(Line::styled(
            "Describe a task, ask a question, or use / for commands.",
            Style::default().fg(pal.chrome_fg),
        ));
        if !state.online {
            lines.push(Line::styled(
                "Brain offline · /doctor shows connection details.",
                Style::default().fg(pal.warning),
            ));
        }
    }
    let paragraph = Paragraph::new(lines)
        .style(Style::default().fg(pal.viewport_fg).bg(pal.viewport_bg))
        .wrap(Wrap { trim: false });
    let tail = paragraph
        .line_count(inner.width)
        .saturating_sub(inner.height as usize);
    let scroll = tail
        .saturating_sub(state.transcript_scroll as usize)
        .min(u16::MAX as usize) as u16;
    frame.render_widget(paragraph.scroll((scroll, 0)), inner);
}

fn thought_lines(
    lines: &mut Vec<Line<'static>>,
    text: &str,
    folded: bool,
    thinking_frame: Option<&str>,
    elapsed: &str,
    pal: &Palette,
) {
    lines.push(Line::from(vec![
        Span::styled(
            if folded { "  ▸ " } else { "  ▾ " },
            Style::default().fg(pal.accent),
        ),
        Span::styled(
            thinking_frame.map_or_else(|| "Thought".to_owned(), |s| format!("{s} thinking…")),
            Style::default()
                .fg(pal.viewport_fg)
                .add_modifier(Modifier::ITALIC),
        ),
        Span::styled(
            format!(
                "  {}Ctrl+T to {}",
                if elapsed.is_empty() {
                    String::new()
                } else {
                    format!("{elapsed} · ")
                },
                if folded { "expand" } else { "collapse" }
            ),
            Style::default().fg(pal.chrome_fg),
        ),
    ]));
    if !folded {
        for line in text.lines() {
            lines.push(Line::from(vec![
                Span::styled("  │  ", Style::default().fg(pal.accent)),
                Span::styled(
                    sanitize_ansi(line),
                    Style::default()
                        .fg(pal.chrome_fg)
                        .add_modifier(Modifier::ITALIC),
                ),
            ]));
        }
    }
}

fn message_lines(
    lines: &mut Vec<Line<'static>>,
    role: &str,
    content: &str,
    color: Color,
    pal: &Palette,
) {
    let mut in_code = false;
    for (i, text) in sanitize_ansi(content)
        .replace('\t', "    ")
        .split('\n')
        .enumerate()
    {
        let mut spans = vec![Span::styled(
            if i == 0 {
                role.to_owned()
            } else {
                " ".repeat(role.len())
            },
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        )];
        if text.trim_start().starts_with("```") {
            in_code = !in_code;
            spans.push(Span::styled(
                text.to_owned(),
                Style::default().fg(pal.chrome_fg),
            ));
        } else if in_code {
            spans.push(Span::styled(
                text.to_owned(),
                Style::default().fg(pal.accent),
            ));
        } else if text.starts_with("# ") || text.starts_with("## ") || text.starts_with("### ") {
            spans.push(Span::styled(
                text.trim_start_matches('#').trim_start().to_owned(),
                Style::default()
                    .fg(pal.viewport_fg)
                    .add_modifier(Modifier::BOLD),
            ));
        } else {
            let mut bold = false;
            for part in text.split("**") {
                let mut code = false;
                for chunk in part.split('`') {
                    let mut style = Style::default().fg(if code {
                        pal.accent
                    } else if role.is_empty() {
                        pal.chrome_fg
                    } else {
                        pal.viewport_fg
                    });
                    if bold {
                        style = style.add_modifier(Modifier::BOLD);
                    }
                    spans.push(Span::styled(chunk.to_owned(), style));
                    code = !code;
                }
                bold = !bold;
            }
        }
        lines.push(Line::from(spans));
    }
}
