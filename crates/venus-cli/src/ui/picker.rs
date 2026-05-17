use ratatui::{
    prelude::*,
    widgets::*,
};

use crate::app::App;

/// Render a picker/list selection overlay centered on the screen.
pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let picker = match &app.picker {
        Some(p) => p,
        None => return,
    };

    // Calculate popup dimensions
    let max_label_width = picker.items.iter()
        .map(|i| i.label.len() as u16)
        .max()
        .unwrap_or(10);
    let popup_width = (max_label_width + 12).min(area.width.saturating_sub(4)).max(30);
    let visible_count = picker.visible_count.min(picker.items.len()) as u16;
    let popup_height = (visible_count + 3).min(area.height.saturating_sub(2)); // +3 for title, border, hint

    let x = (area.width.saturating_sub(popup_width)) / 2;
    let y = (area.height.saturating_sub(popup_height)) / 2;
    let popup_area = Rect::new(x, y, popup_width, popup_height);

    // Clear background
    let bg = Block::default().style(Style::default().bg(Color::Black));
    frame.render_widget(bg, popup_area);

    // Build list items
    let items: Vec<ListItem> = picker.items
        .iter()
        .enumerate()
        .skip(picker.scroll_offset)
        .take(picker.visible_count)
        .map(|(i, item)| {
            let is_selected = i == picker.selected;
            let style = if is_selected {
                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };

            let prefix = if is_selected { "▸ " } else { "  " };
            let desc_style = if is_selected {
                Style::default().fg(Color::DarkGray).add_modifier(Modifier::ITALIC)
            } else {
                Style::default().fg(Color::DarkGray)
            };

            let line = Line::from(vec![
                Span::styled(prefix, style),
                Span::styled(item.label.clone(), style),
                if item.description.is_empty() {
                    Span::raw(String::new())
                } else {
                    Span::styled(format!(" — {}", item.description), desc_style)
                },
            ]);
            ListItem::new(line)
        })
        .collect();

    // Scroll indicator
    let total = picker.items.len();
    let scroll_info = if total > picker.visible_count {
        let start = picker.scroll_offset + 1;
        let end = (picker.scroll_offset + picker.visible_count).min(total);
        format!(" {}-{}/{} ", start, end, total)
    } else {
        String::new()
    };

    let title = format!(" {}{} ", picker.title, scroll_info);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(Span::styled(
            title,
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        ));

    let list = List::new(items).block(block);
    frame.render_widget(list, popup_area);
}
