pub mod input;
pub mod messages;
pub mod modal;
pub mod picker;
pub mod spinner;
pub mod status;

use ratatui::prelude::*;

use crate::app::{App, InputMode};

/// Render the full TUI layout.
pub fn render(frame: &mut Frame, app: &App) {
    let area = frame.area();

    // Layout: status bar (top, 1 row) + messages (middle, fills) + input (bottom, 3 rows)
    let [status_area, messages_area, input_area] = ratatui::layout::Layout::default()
        .direction(ratatui::layout::Direction::Vertical)
        .constraints([
            ratatui::layout::Constraint::Length(1),
            ratatui::layout::Constraint::Min(3),
            ratatui::layout::Constraint::Length(3),
        ])
        .areas(area);

    // Render status bar
    status::render(frame, status_area, app);

    // Render messages
    messages::render(frame, messages_area, app);

    // Render spinner (overlaid at bottom of messages area if active)
    if app.spinner.active {
        spinner::render(frame, messages_area, app);
    }

    // Render input area
    input::render(frame, input_area, app);

    // Render overlays based on input mode
    match app.input_mode {
        InputMode::PermissionPrompt => {
            if let Some(ref pending) = app.pending_permission {
                modal::render(frame, area, &pending.tool_name, &pending.description, pending.selected_option);
            }
        }
        InputMode::Picker => {
            picker::render(frame, area, app);
        }
        InputMode::HistorySearch => {
            render_history_search(frame, input_area, app);
        }
        _ => {}
    }
}

/// Render history search results below the input area.
fn render_history_search(frame: &mut Frame, area: Rect, app: &App) {
    let matches = app.history_search_matches();
    if matches.is_empty() {
        return;
    }

    let popup_height = (matches.len() as u16 + 1).min(6);
    let popup_width = area.width.saturating_sub(2);
    let popup_y = area.y.saturating_sub(popup_height);
    let popup_area = Rect::new(area.x + 1, popup_y, popup_width, popup_height);

    let items: Vec<ratatui::widgets::ListItem> = matches
        .iter()
        .map(|entry| {
            let display = if entry.len() > 60 {
                format!("{}...", &entry[..57])
            } else {
                entry.to_string()
            };
            ratatui::widgets::ListItem::new(ratatui::text::Line::from(vec![
                ratatui::text::Span::styled("  ", ratatui::style::Style::default()),
                ratatui::text::Span::raw(display),
            ]))
        })
        .collect();

    let list = ratatui::widgets::List::new(items).block(
        ratatui::widgets::Block::default()
            .borders(ratatui::widgets::Borders::ALL)
            .border_style(ratatui::style::Style::default().fg(ratatui::style::Color::Yellow))
            .title(ratatui::text::Span::styled(
                " History Search ",
                ratatui::style::Style::default()
                    .fg(ratatui::style::Color::Yellow)
                    .add_modifier(ratatui::style::Modifier::BOLD),
            )),
    );

    frame.render_widget(list, popup_area);
}
