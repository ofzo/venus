use ratatui::{
    prelude::*,
    widgets::*,
};

use crate::app::App;

/// Render the status bar at the top of the screen.
pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let mut spans = vec![
        Span::styled(
            " Venus ",
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        ),
        Span::raw("│ "),
        Span::styled(&*app.model(), Style::default().fg(Color::Yellow)),
    ];

    if !app.cost.is_empty() && app.cost != "$0.00" {
        spans.push(Span::raw(" │ "));
        spans.push(Span::styled(
            app.cost.clone(),
            Style::default().fg(Color::Green),
        ));
    }

    if let Some(ref branch) = app.branch {
        spans.push(Span::raw(" │ "));
        spans.push(Span::styled(
            format!("({})", branch),
            Style::default().fg(Color::DarkGray),
        ));
    }

    let status_line = Line::from(spans);
    let paragraph = Paragraph::new(status_line).block(
        Block::default()
            .borders(Borders::BOTTOM)
            .border_style(Style::default().fg(Color::DarkGray)),
    );

    frame.render_widget(paragraph, area);
}
