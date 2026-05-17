use ratatui::{
    prelude::*,
    widgets::*,
};

use crate::app::{App, DisplayMessage, RenderSegment};
use crate::markdown_tui;

/// Render the scrollable message area.
pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let mut lines: Vec<Line> = Vec::new();

    for msg in &app.messages {
        match msg {
            DisplayMessage::User { text } => {
                lines.push(Line::from(vec![
                    Span::styled("> ", Style::default().fg(Color::Cyan)),
                    Span::styled(text.clone(), Style::default().add_modifier(Modifier::BOLD)),
                ]));
                lines.push(Line::from(""));
            }
            DisplayMessage::Assistant { segments } => {
                for segment in segments {
                    match segment {
                        RenderSegment::Text(text) => {
                            // Parse markdown with code block support
                            let rendered = render_markdown_with_code_blocks(text);
                            lines.extend(rendered);
                        }
                    }
                }
                lines.push(Line::from(""));
            }
            DisplayMessage::ToolCall {
                name,
                activity,
                is_error,
                summary,
            } => {
                let mut header = vec![
                    Span::styled("  ⏺ ", Style::default().fg(Color::Cyan)),
                    Span::styled(
                        name.clone(),
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ),
                ];
                if !activity.is_empty() {
                    header.push(Span::styled(
                        format!(": {}", activity),
                        Style::default().fg(Color::DarkGray),
                    ));
                }
                lines.push(Line::from(header));

                if !summary.is_empty() {
                    let color = if *is_error { Color::Red } else { Color::Green };
                    lines.push(Line::from(vec![
                        Span::styled("  ⏺ ", Style::default().fg(color)),
                        Span::styled(summary.clone(), Style::default().fg(Color::DarkGray)),
                    ]));
                }
                lines.push(Line::from(""));
            }
            DisplayMessage::Error { text } => {
                lines.push(Line::from(vec![
                    Span::styled(
                        "  ⏺ Error: ",
                        Style::default()
                            .fg(Color::Red)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(text.clone(), Style::default().fg(Color::Red)),
                ]));
            }
            DisplayMessage::Status { text } => {
                lines.push(Line::from(vec![
                    Span::styled("  ⏺ ", Style::default().fg(Color::DarkGray)),
                    Span::styled(text.clone(), Style::default().fg(Color::DarkGray)),
                ]));
            }
        }
    }

    let paragraph = Paragraph::new(lines).wrap(Wrap { trim: false });

    let scroll = if app.auto_scroll {
        (0, 0)
    } else {
        (app.scroll_offset, 0)
    };

    let paragraph = paragraph.scroll(scroll);

    frame.render_widget(paragraph, area);
}

/// Render markdown text with code block support.
fn render_markdown_with_code_blocks(text: &str) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let mut in_code_block = false;
    let mut code_lang = String::new();

    for line in text.split('\n') {
        // Check for code block start/end
        if line.starts_with("```") {
            if in_code_block {
                // End of code block
                in_code_block = false;
                lines.push(Line::from(Span::styled(
                    "  ────────────────────────────────────",
                    Style::default().fg(Color::DarkGray),
                )));
            } else {
                // Start of code block
                in_code_block = true;
                code_lang = line[3..].trim().to_string();
                lines.push(Line::from(Span::styled(
                    format!("  ┌─ {} ─────────────────────────────", if code_lang.is_empty() { "code" } else { &code_lang }),
                    Style::default().fg(Color::DarkGray),
                )));
            }
            continue;
        }

        if in_code_block {
            // Render code line with syntax highlighting
            let highlighted = markdown_tui::highlight_code_line(line, &code_lang);
            let mut spans = vec![Span::styled("  │ ", Style::default().fg(Color::DarkGray))];
            spans.extend(highlighted);
            lines.push(Line::from(spans));
        } else {
            // Regular markdown line
            lines.push(render_markdown_line(line));
        }
    }

    lines
}

/// Render a single markdown line with inline formatting.
fn render_markdown_line(line: &str) -> Line<'static> {
    // Headers
    if let Some(content) = line.strip_prefix("#### ") {
        return Line::from(Span::styled(
            content.to_string(),
            Style::default()
                .fg(Color::Magenta)
                .add_modifier(Modifier::BOLD),
        ));
    }
    if let Some(content) = line.strip_prefix("### ") {
        return Line::from(Span::styled(
            content.to_string(),
            Style::default()
                .fg(Color::Magenta)
                .add_modifier(Modifier::BOLD),
        ));
    }
    if let Some(content) = line.strip_prefix("## ") {
        return Line::from(Span::styled(
            content.to_string(),
            Style::default()
                .fg(Color::Blue)
                .add_modifier(Modifier::BOLD),
        ));
    }
    if let Some(content) = line.strip_prefix("# ") {
        return Line::from(Span::styled(
            content.to_string(),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ));
    }

    // Horizontal rule
    let trimmed = line.trim();
    if trimmed == "---" || trimmed == "***" || trimmed == "___" {
        return Line::from(Span::styled(
            "─".repeat(40),
            Style::default().fg(Color::DarkGray),
        ));
    }

    // Unordered list
    if let Some(content) = line.strip_prefix("- ").or_else(|| line.strip_prefix("* ")) {
        let mut spans = vec![Span::styled("  • ", Style::default().fg(Color::Cyan))];
        spans.extend(markdown_tui::parse_inline(content));
        return Line::from(spans);
    }

    // Indented list
    if let Some(rest) = line
        .strip_prefix("  - ")
        .or_else(|| line.strip_prefix("  * "))
    {
        let mut spans = vec![Span::styled("    ◦ ", Style::default().fg(Color::Cyan))];
        spans.extend(markdown_tui::parse_inline(rest));
        return Line::from(spans);
    }

    // Ordered list
    if let Some(dot_pos) = line.find(". ") {
        if dot_pos <= 3 && line[..dot_pos].chars().all(|c| c.is_ascii_digit()) {
            let num = &line[..dot_pos];
            let content = &line[dot_pos + 2..];
            let mut spans = vec![Span::styled(
                format!("  {}. ", num),
                Style::default().fg(Color::Cyan),
            )];
            spans.extend(markdown_tui::parse_inline(content));
            return Line::from(spans);
        }
    }

    // Blockquote
    if let Some(content) = line.strip_prefix("> ") {
        return Line::from(vec![
            Span::styled("  │ ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                content.to_string(),
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::ITALIC),
            ),
        ]);
    }

    // Regular text with inline formatting
    let spans = markdown_tui::parse_inline(line);
    Line::from(spans)
}
