use ratatui::{
    prelude::*,
    widgets::*,
};

use crate::app::{App, PickerSource};

/// Render a picker/list selection overlay centered on the screen.
pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let picker = match &app.picker {
        Some(p) => p,
        None => return,
    };

    // Calculate popup dimensions
    let max_label_width = picker.items.iter()
        .map(|i| {
            let label_w = i.label.chars().count() as u16;
            let desc_w = if i.description.is_empty() { 0 } else { i.description.chars().count() as u16 + 3 };
            label_w + desc_w
        })
        .max()
        .unwrap_or(20)
        .max(picker.title.chars().count() as u16 + 4);
    let popup_width = (max_label_width + 6).min(area.width.saturating_sub(4)).max(30);
    let visible_count = picker.visible_count.min(picker.items.len()) as u16;

    // Extra lines for model picker (effort display) and tabs
    let extra_lines = if matches!(picker.source, PickerSource::Model) { 2 } else { 0 };
    let tab_lines = if picker.tab_state.is_some() { 1 } else { 0 };

    // +2 for borders, +1 for hint footer, +extra for model picker, +tabs
    let popup_height = (visible_count + 3 + extra_lines + tab_lines).min(area.height.saturating_sub(2));

    let x = (area.width.saturating_sub(popup_width)) / 2;
    let y = (area.height.saturating_sub(popup_height)) / 2;
    let popup_area = Rect::new(x, y, popup_width, popup_height);

    // Dim background behind popup
    let bg = Block::default().style(Style::default().bg(Color::Black));
    frame.render_widget(bg, popup_area);

    // Render tabs if present
    let mut current_y = popup_area.y;
    if let Some(ref tab_state) = picker.tab_state {
        let tab_area = Rect::new(popup_area.x, current_y, popup_width, 1);
        let mut tab_spans = vec![Span::styled(
            format!("  {}: ", picker.title),
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        )];
        for (i, tab) in tab_state.tabs.iter().enumerate() {
            let is_selected = i == tab_state.selected_tab;
            let style = if is_selected {
                Style::default().fg(Color::White).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            tab_spans.push(Span::styled(format!(" {} ", tab), style));
            if i < tab_state.tabs.len() - 1 {
                tab_spans.push(Span::styled(" ", Style::default()));
            }
        }
        let tab_line = Line::from(tab_spans);
        let tab_paragraph = Paragraph::new(tab_line).style(Style::default().bg(Color::Black));
        frame.render_widget(tab_paragraph, tab_area);
        current_y += 1;
    }

    // Build list items
    let items: Vec<ListItem> = picker.items
        .iter()
        .enumerate()
        .skip(picker.scroll_offset)
        .take(picker.visible_count)
        .map(|(i, item)| {
            let is_selected = i == picker.selected;
            let is_separator = item.value.is_empty() && item.description.is_empty();

            if is_separator {
                return ListItem::new(Line::from(Span::styled(
                    format!("  {}", item.label),
                    Style::default().fg(Color::DarkGray).add_modifier(Modifier::DIM),
                )));
            }

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

            let mut spans = vec![
                Span::styled(prefix, style),
                Span::styled(item.label.clone(), style),
            ];
            if !item.description.is_empty() {
                spans.push(Span::styled(format!(" — {}", item.description), desc_style));
            }
            ListItem::new(Line::from(spans))
        })
        .collect();

    // Scroll indicator in title
    let total = picker.items.len();
    let scroll_info = if total > picker.visible_count {
        let end = (picker.scroll_offset + picker.visible_count).min(total);
        format!(" {}/{} ", end, total)
    } else {
        String::new()
    };

    let title = if picker.tab_state.is_some() {
        "".to_string() // Tabs already shown
    } else {
        format!(" {}{} ", picker.title, scroll_info)
    };

    // Split popup into list area, effort area (for model picker), and hint area
    let list_height = popup_height.saturating_sub(1 + extra_lines + tab_lines);
    let list_area = Rect::new(popup_area.x, current_y, popup_width, list_height);
    let hint_area = Rect::new(popup_area.x, popup_area.y + popup_height - 1, popup_width, 1);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(Span::styled(
            title,
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        ));

    let list = List::new(items).block(block);
    frame.render_widget(list, list_area);

    // Render effort display for model picker (matching Claude Code's ModelPicker)
    if matches!(picker.source, PickerSource::Model) && extra_lines > 0 {
        let effort_area = Rect::new(popup_area.x, current_y + list_height, popup_width, extra_lines);
        let effort = app.get_effort_label();
        let effort_indicator = match effort {
            "low" => "○",
            "medium" => "◐",
            "high" => "●",
            "max" => "◉",
            _ => "○",
        };
        let effort_text = format!("  {} {} effort  ← → to adjust", effort_indicator, effort);
        let effort_paragraph = Paragraph::new(Line::from(vec![
            Span::styled(effort_text, Style::default().fg(Color::DarkGray)),
        ]))
        .style(Style::default().bg(Color::Black));
        frame.render_widget(effort_paragraph, effort_area);
    }

    // Render keyboard hint at bottom
    let hint = Paragraph::new(Line::from(vec![
        Span::styled(" ↑↓", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        Span::styled(" navigate ", Style::default().fg(Color::DarkGray)),
        Span::styled("Enter", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        Span::styled(" select ", Style::default().fg(Color::DarkGray)),
        Span::styled("Esc", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        Span::styled(" cancel", Style::default().fg(Color::DarkGray)),
    ]))
    .alignment(Alignment::Center)
    .style(Style::default().bg(Color::Black));
    frame.render_widget(hint, hint_area);
}
