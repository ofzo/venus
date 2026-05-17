use ratatui::{
    prelude::*,
    widgets::*,
};

/// Unicode figures matching Claude Code
const POINTER: &str = "❯";

/// Render a permission prompt matching Claude Code's Select component exactly.
/// NO borders. Just a list with ❯ indicator.
pub fn render(
    frame: &mut Frame,
    area: Rect,
    tool_name: &str,
    description: &str,
    selected_option: usize,
) {
    // Claude Code's PermissionPrompt layout:
    // <Box flexDirection="column">
    //   <Text>{questionText}</Text>
    //   <Select options={options} .../>
    //   <Box marginTop={1}>
    //     <Text dimColor>Esc to cancel</Text>
    //   </Box>
    // </Box>

    let content_width = (area.width * 60 / 100).min(60);
    let x = (area.width.saturating_sub(content_width)) / 2;
    let y = (area.height.saturating_sub(10)) / 2;

    // Clear background
    let bg = Block::default().style(Style::default().bg(Color::Black));
    frame.render_widget(bg, Rect::new(x, y, content_width, 10));

    let mut current_y = y;

    // Question text (matching Claude Code's "Do you want to proceed?")
    let question_area = Rect::new(x + 2, current_y, content_width.saturating_sub(4), 1);
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "Do you want to proceed?",
            Style::default().add_modifier(Modifier::BOLD),
        ))),
        question_area,
    );
    current_y += 1;

    // Tool info
    let tool_area = Rect::new(x + 2, current_y, content_width.saturating_sub(4), 1);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                tool_name,
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled(
                description,
                Style::default().fg(Color::DarkGray),
            ),
        ])),
        tool_area,
    );
    current_y += 2;

    // Select options (matching Claude Code's Select component style)
    let options = [
        "Yes — allow this operation",
        "Yes, and always allow",
        "No",
    ];

    for (i, option) in options.iter().enumerate() {
        let is_focused = i == selected_option;
        let opt_area = Rect::new(x + 2, current_y, content_width.saturating_sub(4), 1);

        let mut spans = Vec::new();

        // Left indicator: ❯ for focused
        if is_focused {
            spans.push(Span::styled(
                POINTER,
                Style::default().fg(Color::Cyan), // "suggestion" color
            ));
        } else {
            spans.push(Span::raw(" "));
        }

        // Gap
        spans.push(Span::raw(" "));

        // Option text
        let text_style = if is_focused {
            Style::default().fg(Color::Cyan) // "suggestion" color
        } else {
            Style::default()
        };
        spans.push(Span::styled(option.to_string(), text_style));

        frame.render_widget(Paragraph::new(Line::from(spans)), opt_area);
        current_y += 1;
    }

    current_y += 1;

    // Footer hint (matching Claude Code's dimColor style)
    let hint_area = Rect::new(x + 2, current_y, content_width.saturating_sub(4), 1);
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "Esc to cancel",
            Style::default().fg(Color::DarkGray),
        ))),
        hint_area,
    );
}
