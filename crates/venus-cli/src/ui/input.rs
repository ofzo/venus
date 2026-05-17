use ratatui::{
    prelude::*,
    widgets::*,
};

use crate::app::{App, InputMode};

/// Render the input area at the bottom of the screen.
pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let border_style = match app.input_mode {
        InputMode::Streaming => Style::default().fg(Color::Yellow),
        InputMode::Normal => Style::default().fg(Color::Cyan),
        InputMode::PermissionPrompt => Style::default().fg(Color::Yellow),
    };

    let title = match app.input_mode {
        InputMode::Streaming => " streaming... (Ctrl+C to cancel) ",
        InputMode::Normal => " enter message (Alt+Enter for newline) ",
        InputMode::PermissionPrompt => " permission required ",
    };

    let input_text = if app.input.buffer.is_empty() && app.input_mode == InputMode::Normal {
        // Show placeholder
        Span::styled(
            "Type a message...",
            Style::default().fg(Color::DarkGray),
        )
    } else {
        Span::raw(app.input.buffer.clone())
    };

    let paragraph = Paragraph::new(input_text).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(border_style)
            .title(Span::styled(
                title,
                Style::default().fg(Color::DarkGray),
            )),
    );

    frame.render_widget(paragraph, area);

    // Show cursor in normal mode only
    if app.input_mode == InputMode::Normal {
        let cursor_x = area.x + 1 + (app.input.cursor_pos as u16).min(area.width.saturating_sub(3));
        let cursor_y = area.y + 1;
        frame.set_cursor_position((cursor_x, cursor_y));
    }


    // Show completion popup if active
    if !app.input.completion_matches.is_empty() {
        let popup_height = (app.input.completion_matches.len() as u16).min(8);
        let popup_width = app
            .input
            .completion_matches
            .iter()
            .map(|s| s.len() as u16)
            .max()
            .unwrap_or(10)
            + 2;
        let popup_y = area.y.saturating_sub(popup_height);
        let popup_area = Rect::new(area.x + 1, popup_y, popup_width, popup_height);

        let items: Vec<ListItem> = app
            .input
            .completion_matches
            .iter()
            .enumerate()
            .map(|(i, cmd)| {
                let style = if i == app.input.completion_index {
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::White)
                };
                ListItem::new(Line::from(Span::styled(cmd.clone(), style)))
            })
            .collect();

        let list = List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan)),
        );

        frame.render_widget(list, popup_area);
    }
}
