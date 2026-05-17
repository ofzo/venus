use ratatui::{
    prelude::*,
    widgets::*,
};

use crate::app::{App, PickerSource};

/// Unicode figures matching Claude Code's figures.ts
const POINTER: &str = "❯";     // U+276F - focused item
const TICK: &str = "✓";        // U+2713 - selected item
const ARROW_UP: &str = "↑";    // U+2191 - scroll up indicator
const ARROW_DOWN: &str = "↓";  // U+2193 - scroll down indicator
const DIVIDER: &str = "─";     // U+2500 - horizontal line

/// Render a picker overlay matching Claude Code's Select component exactly.
/// NO borders. Pane layout with colored divider. Index numbers. ❯ indicator.
pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let picker = match &app.picker {
        Some(p) => p,
        None => return,
    };

    let is_model_picker = matches!(picker.source, PickerSource::Model);
    let has_tabs = picker.tab_state.is_some();

    // Calculate max index width (e.g., 9 options = 1 digit)
    let max_index_width = if picker.items.len() >= 10 { 2 } else { 1 };

    // Calculate max label width for two-column alignment
    let max_label_width = picker.items.iter()
        .map(|i| i.label.chars().count() as u16)
        .max()
        .unwrap_or(10)
        .min(30);

    // Layout: Pane style (matching Claude Code's Pane component)
    // paddingTop=1, Divider, paddingX=2, content, hint at bottom
    let pane_padding_top = 1u16;
    let divider_height = 1u16;
    let hint_height = if is_model_picker { 2 } else { 1 }; // effort display + key hints
    let tab_height = if has_tabs { 1 } else { 0 };
    let visible_count = picker.visible_count.min(picker.items.len()) as u16;

    let content_height = visible_count + pane_padding_top + divider_height + hint_height + tab_height;
    let popup_height = content_height.min(area.height.saturating_sub(2));
    let popup_width = area.width.saturating_sub(4); // paddingX=2 on each side

    let x = 2; // paddingX=2
    let y = (area.height.saturating_sub(popup_height)) / 2;
    let popup_area = Rect::new(x, y, popup_width, popup_height);

    // Clear background
    let bg = Block::default().style(Style::default().bg(Color::Black));
    frame.render_widget(bg, popup_area);

    let mut current_y = popup_area.y;

    // 1. Top padding (1 line)
    current_y += pane_padding_top;

    // 2. Divider line (colored based on source)
    let divider_color = match picker.source {
        PickerSource::Help => Color::Blue,      // "professionalBlue"
        PickerSource::Model => Color::Yellow,   // "remember"
        _ => Color::Yellow,                     // "permission"
    };
    let divider_area = Rect::new(popup_area.x, current_y, popup_width, 1);
    let divider_line = if has_tabs {
        // Tabs in divider
        let tab_state = picker.tab_state.as_ref().unwrap();
        let mut spans = vec![];
        for (i, tab) in tab_state.tabs.iter().enumerate() {
            if i > 0 {
                spans.push(Span::raw(" "));
            }
            let is_current = i == tab_state.selected_tab;
            let style = if is_current {
                Style::default().fg(Color::White).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            spans.push(Span::styled(format!(" {} ", tab), style));
        }
        Line::from(spans)
    } else {
        Line::from(Span::styled(
            DIVIDER.repeat(popup_width as usize),
            Style::default().fg(divider_color),
        ))
    };
    frame.render_widget(Paragraph::new(divider_line), divider_area);
    current_y += 1;

    // 3. Content area with paddingX=2
    let content_x = popup_area.x + 2;
    let content_width = popup_width.saturating_sub(4);

    // Render each visible item
    for (vis_idx, item_idx) in (picker.scroll_offset..picker.scroll_offset + picker.visible_count).enumerate() {
        if item_idx >= picker.items.len() {
            break;
        }
        let item = &picker.items[item_idx];
        let is_focused = item_idx == picker.selected;
        let is_separator = item.value.is_empty() && item.description.is_empty();

        let item_y = current_y + vis_idx as u16;
        let item_area = Rect::new(content_x, item_y, content_width, 1);

        if is_separator {
            // Separator line (dimmed)
            let sep = Paragraph::new(Line::from(Span::styled(
                item.label.clone(),
                Style::default().fg(Color::DarkGray).add_modifier(Modifier::DIM),
            )));
            frame.render_widget(sep, item_area);
            continue;
        }

        // Build item spans
        let mut spans = Vec::new();

        // Left indicator: ❯ for focused, ↑/↓ for scroll edges, space otherwise
        if is_focused {
            spans.push(Span::styled(
                POINTER,
                Style::default().fg(Color::Cyan), // "suggestion" color
            ));
        } else if vis_idx == 0 && picker.scroll_offset > 0 {
            // First visible item, more above
            spans.push(Span::styled(
                ARROW_UP,
                Style::default().fg(Color::DarkGray),
            ));
        } else if vis_idx == picker.visible_count - 1 && item_idx + 1 < picker.items.len() {
            // Last visible item, more below
            spans.push(Span::styled(
                ARROW_DOWN,
                Style::default().fg(Color::DarkGray),
            ));
        } else {
            spans.push(Span::raw(" "));
        }

        // Gap
        spans.push(Span::raw(" "));

        // Index number (compact layout)
        let index_str = format!("{:>width$}.", item_idx + 1, width = max_index_width as usize);
        spans.push(Span::styled(
            index_str,
            Style::default().fg(Color::DarkGray),
        ));

        // Gap
        spans.push(Span::raw(" "));

        // Label text
        let label_style = if is_focused {
            Style::default().fg(Color::Cyan) // "suggestion" color
        } else {
            Style::default()
        };
        spans.push(Span::styled(item.label.clone(), label_style));

        // Right checkmark for selected items (not applicable for single-select pickers)
        // Skip for now as our pickers are single-select

        // Description (right-aligned in two-column layout)
        if !item.description.is_empty() {
            let label_pad = max_label_width.saturating_sub(item.label.chars().count() as u16);
            let padding = " ".repeat((label_pad + 2).min(20) as usize);
            spans.push(Span::raw(padding));
            spans.push(Span::styled(
                item.description.clone(),
                Style::default().fg(Color::DarkGray), // "inactive" color
            ));
        }

        let line = Line::from(spans);
        frame.render_widget(Paragraph::new(line), item_area);
    }

    current_y += visible_count;

    // 4. Effort display for model picker
    if is_model_picker {
        let effort = app.get_effort_label();
        let effort_indicator = match effort {
            "low" => "○",
            "medium" => "◐",
            "high" => "●",
            "max" => "◉",
            _ => "○",
        };
        let effort_area = Rect::new(content_x, current_y, content_width, 1);
        let effort_line = Line::from(vec![
            Span::styled(
                format!("{} {} effort", effort_indicator, effort),
                Style::default().fg(Color::DarkGray),
            ),
            Span::raw("  "),
            Span::styled(
                "← → to adjust",
                Style::default().fg(Color::DarkGray).add_modifier(Modifier::DIM),
            ),
        ]);
        frame.render_widget(Paragraph::new(effort_line), effort_area);
        current_y += 1;
    }

    // 5. Key hints at bottom (matching Claude Code's KeyboardShortcutHint style)
    let hint_area = Rect::new(content_x, current_y, content_width, 1);
    let hint_line = Line::from(vec![
        Span::styled("Enter", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(" to confirm  "),
        Span::styled("Esc", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(" to cancel"),
    ]);
    frame.render_widget(
        Paragraph::new(hint_line).style(Style::default().fg(Color::DarkGray)),
        hint_area,
    );
}
