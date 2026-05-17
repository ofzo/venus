use ratatui::{
    prelude::*,
    widgets::*,
};

/// Unicode figures
const POINTER: &str = "\u{276F}";  // ❯
const MIDDOT: &str = "\u{00B7}";   // ·

/// Render permission prompt matching Claude Code's PermissionPrompt exactly.
///
/// Layout:
///   {question text}
///   ❯ Option One  description
///     Option Two  description
///     Option Three  description
///
///   Esc to cancel · Tab to amend
pub fn render(
    frame: &mut Frame,
    area: Rect,
    tool_name: &str,
    description: &str,
    selected_option: usize,
) {
    // Center the content
    let content_width = area.width.min(70);
    let x = (area.width.saturating_sub(content_width)) / 2;
    let start_y = (area.height.saturating_sub(12)) / 2;

    // Clear background area
    let bg_area = Rect::new(x, start_y, content_width, 12);
    let bg = Block::default().style(Style::default().bg(Color::Black));
    frame.render_widget(bg, bg_area);

    let content_x = x + 2; // paddingX=2
    let inner_width = content_width.saturating_sub(4);
    let mut current_y = start_y + 1;

    // 1. Question text (default color, no special styling)
    let question_area = Rect::new(content_x, current_y, inner_width, 1);
    frame.render_widget(
        Paragraph::new(Line::from(Span::raw("Do you want to proceed?"))),
        question_area,
    );
    current_y += 1;

    // 2. Tool info
    let tool_area = Rect::new(content_x, current_y, inner_width, 1);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(tool_name, Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
            Span::raw("  "),
            Span::styled(description, Style::default().fg(Color::DarkGray)),
        ])),
        tool_area,
    );
    current_y += 2;

    // 3. Select options (compact layout, inlineDescriptions=true)
    let options = [
        ("Yes, proceed", "allow this operation"),
        ("Yes, always allow", "skip future prompts for this tool"),
        ("No", "deny this operation"),
    ];

    for (i, (label, desc)) in options.iter().enumerate() {
        let is_focused = i == selected_option;
        let opt_area = Rect::new(content_x, current_y, inner_width, 1);

        let mut spans = Vec::new();

        // Indicator: ❯ for focused, space otherwise
        if is_focused {
            spans.push(Span::styled(POINTER, Style::default().fg(Color::Cyan)));
        } else {
            spans.push(Span::raw(" "));
        }

        // Gap (1 space)
        spans.push(Span::raw(" "));

        // Label text (suggestion color when focused)
        if is_focused {
            spans.push(Span::styled(label.to_string(), Style::default().fg(Color::Cyan)));
        } else {
            spans.push(Span::raw(label.to_string()));
        }

        // Inline description (inactive color, dimColor)
        spans.push(Span::raw("  "));
        spans.push(Span::styled(desc.to_string(), Style::default().fg(Color::DarkGray)));

        frame.render_widget(Paragraph::new(Line::from(spans)), opt_area);
        current_y += 1;
    }

    current_y += 1; // marginTop=1

    // 4. Footer: "Esc to cancel · Tab to amend"
    let hint_area = Rect::new(content_x, current_y, inner_width, 1);
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            format!("Esc to cancel {} Tab to amend", MIDDOT),
            Style::default().fg(Color::DarkGray),
        ))),
        hint_area,
    );
}
