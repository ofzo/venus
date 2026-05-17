use ratatui::{
    prelude::*,
    widgets::*,
};

/// Render a permission prompt modal overlay.
pub fn render(
    frame: &mut Frame,
    area: Rect,
    tool_name: &str,
    description: &str,
) {
    let popup_width = (area.width * 60 / 100).min(60);
    let popup_height = 8;
    let x = (area.width.saturating_sub(popup_width)) / 2;
    let y = (area.height.saturating_sub(popup_height)) / 2;
    let popup_area = Rect::new(x, y, popup_width, popup_height);

    // Background
    let bg = Block::default().style(Style::default().bg(Color::Black));
    frame.render_widget(bg, popup_area);

    let content = vec![
        Line::from(vec![
            Span::styled("  ⏺ ", Style::default().fg(Color::Yellow)),
            Span::styled(
                tool_name.to_string(),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            format!("  {}", description),
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled(
                "  y",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" allow  "),
            Span::styled(
                "n",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" deny  "),
            Span::styled(
                "a",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" always allow  "),
            Span::styled(
                "d",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" never"),
        ]),
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
