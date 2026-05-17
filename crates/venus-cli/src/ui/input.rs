use ratatui::{
    prelude::*,
    widgets::*,
};

use crate::app::{App, InputMode};

/// Unicode figures
const POINTER: &str = "\u{276F}"; // ❯

/// Render the input area matching Claude Code's PromptInput exactly.
///
/// Claude Code layout:
///   Box marginTop={1}
///   Box borderStyle="round" borderLeft=false borderRight=false borderBottom
///     ❯ user input text
///   PromptInputFooter
///
/// Key: ONLY bottom border is shown (not top, not left, not right).
pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let border_color = match app.input_mode {
        InputMode::Streaming => Color::Yellow,
        InputMode::Normal => Color::DarkGray,
        InputMode::PermissionPrompt => Color::Yellow,
        InputMode::Picker => Color::DarkGray,
        InputMode::HistorySearch => Color::Yellow,
    };

    // Build input content with prompt character
    let prompt_char = match app.input_mode {
        InputMode::Streaming => Span::styled(
            format!("{} ", POINTER),
            Style::default().fg(Color::DarkGray), // dimColor when loading
        ),
        _ => Span::styled(
            format!("{} ", POINTER),
            Style::default().fg(border_color),
        ),
    };

    let input_text = if app.input.buffer.is_empty() && app.input_mode == InputMode::Normal {
        Line::from(vec![
            prompt_char,
            Span::styled("Type a message\u{2026}", Style::default().fg(Color::DarkGray)),
        ])
    } else if let Some(ref ghost) = app.input.ghost_text {
        Line::from(vec![
            prompt_char,
            Span::raw(app.input.buffer.clone()),
            Span::styled(ghost.clone(), Style::default().fg(Color::DarkGray)),
        ])
    } else {
        Line::from(vec![
            prompt_char,
            Span::raw(app.input.buffer.clone()),
        ])
    };

    // Claude Code: borderStyle="round", borderLeft=false, borderRight=false, borderBottom=true
    // Only BOTTOM border is shown
    let block = Block::default()
        .borders(Borders::BOTTOM) // ONLY bottom border
        .border_style(Style::default().fg(border_color))
        .border_type(BorderType::Rounded);

    let paragraph = Paragraph::new(input_text).block(block);
    frame.render_widget(paragraph, area);

    // Show cursor in normal mode only
    if app.input_mode == InputMode::Normal {
        let cursor_x = area.x + 2 + (app.input.cursor_pos as u16).min(area.width.saturating_sub(3));
        let cursor_y = area.y;
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
        let popup_area = Rect::new(area.x + 2, popup_y, popup_width, popup_height);

        let items: Vec<ListItem> = app
            .input
            .completion_matches
            .iter()
            .enumerate()
            .map(|(i, cmd)| {
                let style = if i == app.input.completion_index {
                    Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::White)
                };
                ListItem::new(Line::from(Span::styled(cmd.clone(), style)))
            })
            .collect();

        let list = List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::DarkGray)),
        );

        frame.render_widget(list, popup_area);
    }
}
