use ratatui::{
    prelude::*,
    widgets::*,
};

/// Render a permission prompt overlay matching Claude Code's Select component.
/// Claude Code shows "Do you want to proceed?" with a selectable list of options.
pub fn render(
    frame: &mut Frame,
    area: Rect,
    tool_name: &str,
    description: &str,
) {
    let popup_width = (area.width * 60 / 100).min(60);
    let popup_height = 10;
    let x = (area.width.saturating_sub(popup_width)) / 2;
    let y = (area.height.saturating_sub(popup_height)) / 2;
    let popup_area = Rect::new(x, y, popup_width, popup_height);

    // Background
    let bg = Block::default().style(Style::default().bg(Color::Black));
    frame.render_widget(bg, popup_area);

    let content = vec![
        // Question text (matching Claude Code's "Do you want to proceed?")
        Line::from(Span::styled(
            "Do you want to proceed?",
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        // Tool info
        Line::from(vec![
            Span::styled(
                format!("  {} ", tool_name),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                description,
                Style::default().fg(Color::DarkGray),
            ),
        ]),
        Line::from(""),
        // Select options (matching Claude Code's Select component style)
        Line::from(vec![
            Span::styled(
                "  ▸ ",
                Style::default().fg(Color::Green),
            ),
            Span::styled(
                "Yes",
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                " — allow this operation",
                Style::default().fg(Color::DarkGray),
            ),
        ]),
        Line::from(vec![
            Span::styled(
                "    ",
                Style::default(),
            ),
            Span::styled(
                "Yes, and always allow",
                Style::default().fg(Color::White),
            ),
        ]),
        Line::from(vec![
            Span::styled(
                "    ",
                Style::default(),
            ),
            Span::styled(
                "No",
                Style::default().fg(Color::White),
            ),
        ]),
        Line::from(""),
        // Footer hint (matching Claude Code's dimColor style)
        Line::from(Span::styled(
            "↑↓ to select · Enter to confirm · Esc to cancel",
            Style::default().fg(Color::DarkGray),
        )),
    ];

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow))
        .title(Span::styled(
            " Permission Required ",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ));

    let paragraph = Paragraph::new(content).block(block);
    frame.render_widget(paragraph, popup_area);
}
