use ratatui::{
    prelude::*,
    widgets::*,
};

use crate::app::{App, InputMode};

/// Unicode figures
const POINTER: &str = "\u{276F}"; // ❯ (Claude Code's figures.pointer)

/// Render the input area matching Claude Code's PromptInput exactly.
///
/// Layout:
///   marginTop=1 (blank line above)
///   ─── top border (round style) ───
///   ❯ user input text here
///   ─── bottom border (round style) ───
///
/// Claude Code uses borderStyle="round" with borderLeft=false, borderRight=false
/// So only top and bottom borders are shown.
pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    // marginTop=1 matching PromptInput's outer Box
    let input_area = Rect {
        x: area.x,
        y: area.y,
        width: area.width,
        height: area.height,
    };

    let border_color = match app.input_mode {
        InputMode::Streaming => Color::Yellow,
        InputMode::Normal => Color::DarkGray, // 'promptBorder' theme color
        InputMode::PermissionPrompt => Color::Yellow,
        InputMode::Picker => Color::DarkGray,
        InputMode::HistorySearch => Color::Yellow,
    };

    let title = match app.input_mode {
        InputMode::Streaming => " streaming\u{2026} (Ctrl+C to cancel) ",
        InputMode::Normal => " ",
        InputMode::PermissionPrompt => " permission required ",
        InputMode::Picker => " pick an option ",
        InputMode::HistorySearch => " history search ",
    };

    // Build input content with prompt character
    let prompt_char = match app.input_mode {
        InputMode::Streaming => Span::styled(
            format!("{} ", POINTER),
            Style::default().fg(Color::DarkGray),
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

    // Claude Code: borderStyle="round", borderLeft=false, borderRight=false
    // This means only top and bottom borders are shown
    let block = Block::default()
        .borders(Borders::TOP | Borders::BOTTOM)
        .border_style(Style::default().fg(border_color))
        .title(Span::styled(
            title,
            Style::default().fg(Color::DarkGray),
        ));

    let paragraph = Paragraph::new(input_text).block(block);
    frame.render_widget(paragraph, input_area);

    // Show cursor in normal mode only
    if app.input_mode == InputMode::Normal {
        // Cursor position: prompt char (2 chars) + cursor_pos
        let cursor_x = input_area.x + 2 + (app.input.cursor_pos as u16).min(input_area.width.saturating_sub(4));
        let cursor_y = input_area.y + 1; // inside the border
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
        let popup_y = input_area.y.saturating_sub(popup_height);
        let popup_area = Rect::new(input_area.x + 2, popup_y, popup_width, popup_height);

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
