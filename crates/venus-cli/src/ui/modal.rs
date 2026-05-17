use ratatui::{
    prelude::*,
    widgets::*,
};

const POINTER: &str = "\u{276F}";  // ❯
const MIDDOT: &str = "\u{00B7}";   // ·

/// Render permission prompt matching Claude Code's PermissionPrompt exactly.
///
/// Layout (from JSX):
///   <Box flexDirection="column">
///     <Text>{question}</Text>
///     <Select inlineDescriptions={true} options={...} />
///     <Box marginTop={1}>
///       <Text dimColor>Esc to cancel · Tab to amend</Text>
///     </Box>
///   </Box>
pub fn render(
    frame: &mut Frame,
    area: Rect,
    tool_name: &str,
    description: &str,
    selected_option: usize,
) {
    let content_width = area.width.min(70);
    let x = (area.width.saturating_sub(content_width)) / 2;
    let start_y = (area.height.saturating_sub(12)) / 2;
    let content_x = x + 2;
    let inner_width = content_width.saturating_sub(4);

    // Clear background
    let bg = Block::default().style(Style::default().bg(Color::Black));
    frame.render_widget(bg, Rect::new(x, start_y, content_width, 12));

    let mut current_y = start_y + 1;

    // 1. Question text (default color, no styling)
    let q_area = Rect::new(content_x, current_y, inner_width, 1);
    frame.render_widget(
        Paragraph::new(Line::from(Span::raw("Do you want to proceed?"))),
        q_area,
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
    // Format: {indicator} {label}  {description}
    let options = [
        ("Yes, proceed", "allow this operation"),
        ("Yes, always allow", "skip future prompts for this tool"),
        ("No", "deny this operation"),
    ];

    for (i, (label, desc)) in options.iter().enumerate() {
        let is_focused = i == selected_option;
        let opt_area = Rect::new(content_x, current_y, inner_width, 1);

        let mut spans = Vec::new();

        // Indicator
        if is_focused {
            spans.push(Span::styled(POINTER, Style::default().fg(Color::Cyan)));
        } else {
            spans.push(Span::raw(" "));
        }

        // Gap
        spans.push(Span::raw(" "));

        // Label (suggestion color when focused)
        if is_focused {
            spans.push(Span::styled(label.to_string(), Style::default().fg(Color::Cyan)));
        } else {
            spans.push(Span::raw(label.to_string()));
        }

        // Inline description (dimColor, after space)
        spans.push(Span::raw("  "));
        spans.push(Span::styled(desc.to_string(), Style::default().fg(Color::DarkGray)));

        frame.render_widget(Paragraph::new(Line::from(spans)), opt_area);
        current_y += 1;
    }

    current_y += 1; // marginTop=1

    // 4. Footer: "Esc to cancel · Tab to amend" (dimColor)
    let hint_area = Rect::new(content_x, current_y, inner_width, 1);
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            format!("Esc to cancel {} Tab to amend", MIDDOT),
            Style::default().fg(Color::DarkGray),
        ))),
        hint_area,
    );
}
