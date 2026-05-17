use ratatui::{
    prelude::*,
    widgets::*,
};

use crate::app::{App, PickerSource};

/// Unicode figures matching Claude Code's figures.ts exactly
const POINTER: &str = "\u{276F}";     // ❯ heavy right-pointing angle bracket
const TICK: &str = "\u{2714}";        // ✔ heavy check mark
const ARROW_UP: &str = "\u{2191}";    // ↑
const ARROW_DOWN: &str = "\u{2193}";  // ↓
const DIVIDER_CHAR: char = '\u{2500}'; // ─ box drawing horizontal
const MIDDOT: &str = "\u{00B7}";      // · middle dot

/// Render a picker overlay matching Claude Code's Select/Pane exactly.
///
/// Pane layout:
///   paddingTop=1 (blank line)
///   ──── divider line ────
///   paddingX=2 (2 spaces each side)
///   content
///   footer
pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let picker = match &app.picker {
        Some(p) => p,
        None => return,
    };

    let is_model_picker = matches!(picker.source, PickerSource::Model);
    let has_tabs = picker.tab_state.is_some();

    // Max index width for compact layout
    let total_items = picker.items.len();
    let max_index_width = if total_items >= 10 { 2 } else { 1 };
    // Index string format: "1. " padded to max_index_width + 2
    let index_pad = max_index_width + 2;

    // Calculate max label width for two-column alignment
    let max_label_width = picker.items.iter()
        .map(|i| {
            let idx_w = index_pad;
            let label_w = i.label.chars().count();
            idx_w + label_w
        })
        .max()
        .unwrap_or(20)
        .min(40) as u16;

    // Pane layout dimensions
    let pane_padding_top = 1u16;
    let divider_height = 1u16;
    let tab_line_height = if has_tabs { 1u16 } else { 0 };
    let visible_count = picker.visible_count.min(picker.items.len()) as u16;
    let footer_lines = if is_model_picker { 2 } else { 1 }; // effort + key hints

    let total_height = pane_padding_top + divider_height + tab_line_height
        + visible_count + footer_lines;
    let popup_height = total_height.min(area.height.saturating_sub(2));
    let popup_width = area.width;

    // Pane starts at left edge with paddingX=2
    let pane_x = 0u16;
    let pane_y = (area.height.saturating_sub(popup_height)) / 2;
    let pane_area = Rect::new(pane_x, pane_y, popup_width, popup_height);

    // Content paddingX=2
    let content_x = pane_x + 2;
    let content_width = popup_width.saturating_sub(4);

    let mut current_y = pane_y;

    // 1. paddingTop=1 (blank line)
    current_y += pane_padding_top;

    // 2. Divider line or Tab header
    if has_tabs {
        let tab_state = picker.tab_state.as_ref().unwrap();
        let tab_area = Rect::new(content_x, current_y, content_width, 1);
        let mut tab_spans = vec![];
        // Title
        tab_spans.push(Span::styled(
            picker.title.clone(),
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD), // "professionalBlue" / "permission"
        ));
        tab_spans.push(Span::raw("  "));
        // Tabs
        for (i, tab) in tab_state.tabs.iter().enumerate() {
            if i > 0 {
                tab_spans.push(Span::raw(" "));
            }
            let is_current = i == tab_state.selected_tab;
            if is_current {
                tab_spans.push(Span::styled(
                    format!(" {} ", tab),
                    Style::default().add_modifier(Modifier::BOLD),
                ));
            } else {
                tab_spans.push(Span::styled(
                    format!(" {} ", tab),
                    Style::default().fg(Color::DarkGray),
                ));
            }
        }
        frame.render_widget(Paragraph::new(Line::from(tab_spans)), tab_area);
        current_y += 1;
    } else {
        // Colored divider line
        let divider_color = match picker.source {
            PickerSource::Help => Color::Blue,
            _ => Color::Yellow, // "permission"
        };
        let divider_area = Rect::new(pane_x, current_y, popup_width, 1);
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                DIVIDER_CHAR.to_string().repeat(popup_width as usize),
                Style::default().fg(divider_color),
            ))),
            divider_area,
        );
        current_y += 1;
    }

    // 3. Render each visible item in compact layout
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
            let sep = Paragraph::new(Line::from(Span::styled(
                item.label.clone(),
                Style::default().fg(Color::DarkGray).add_modifier(Modifier::DIM),
            )));
            frame.render_widget(sep, item_area);
            continue;
        }

        // Build item spans matching Claude Code's TwoColumnRow exactly
        let mut spans = Vec::new();

        // Indicator: ❯ for focused, ↑/↓ for scroll edges, space otherwise
        if is_focused {
            spans.push(Span::styled(POINTER, Style::default().fg(Color::Cyan)));
        } else if vis_idx == 0 && picker.scroll_offset > 0 {
            spans.push(Span::styled(ARROW_UP, Style::default().fg(Color::DarkGray)));
        } else if vis_idx == picker.visible_count - 1 && item_idx + 1 < picker.items.len() {
            spans.push(Span::styled(ARROW_DOWN, Style::default().fg(Color::DarkGray)));
        } else {
            spans.push(Span::raw(" "));
        }

        // Gap (1 space)
        spans.push(Span::raw(" "));

        // Index prefix: "1. " padded to max_index_width + 2
        let index_str = format!("{:>width$}.", item_idx + 1, width = max_index_width);
        let padded_index = format!("{:<width$}", index_str, width = index_pad);
        spans.push(Span::styled(padded_index, Style::default().fg(Color::DarkGray)));

        // Label text
        let label_style = if is_focused {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default()
        };
        let current_label_width = index_pad + item.label.chars().count();
        spans.push(Span::styled(item.label.clone(), label_style));

        // Padding to align descriptions in two-column layout
        if !item.description.is_empty() {
            let padding_needed = (max_label_width as usize).saturating_sub(current_label_width);
            if padding_needed > 0 {
                spans.push(Span::raw(" ".repeat(padding_needed)));
            }
            // Description in "inactive" color (dimColor)
            spans.push(Span::styled(
                item.description.clone(),
                Style::default().fg(Color::DarkGray),
            ));
        }

        let line = Line::from(spans);
        frame.render_widget(Paragraph::new(line), item_area);
    }

    current_y += visible_count;

    // 4. Effort display for model picker
    if is_model_picker {
        let effort = app.get_effort_label();
        let effort_symbol = match effort {
            "low" => "\u{25CB}",     // ○
            "medium" => "\u{25D0}",  // ◐
            "high" => "\u{25CF}",    // ●
            "max" => "\u{25C9}",     // ◉
            _ => "\u{25CB}",
        };
        let effort_area = Rect::new(content_x, current_y, content_width, 1);
        let capitalized = format!("{}{}", &effort[..1].to_uppercase(), &effort[1..]);
        let effort_line = Line::from(vec![
            Span::raw(" "),
            Span::styled(effort_symbol, Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!(" {} effort", capitalized),
                Style::default().fg(Color::DarkGray),
            ),
            Span::raw("  "),
            Span::styled(
                "\u{2190} \u{2192} to adjust", // ← →
                Style::default().fg(Color::DarkGray),
            ),
        ]);
        frame.render_widget(Paragraph::new(effort_line), effort_area);
        current_y += 1;
    }

    // 5. Footer (matching Claude Code's KeyboardShortcutHint + Byline)
    let hint_area = Rect::new(content_x, current_y, content_width, 1);
    let hint_line = if is_model_picker {
        // ModelPicker footer: "Press Enter to confirm · Esc to exit"
        Line::from(vec![
            Span::styled(
                format!("Press {} to confirm {} {} to exit", "Enter", MIDDOT, "Esc"),
                Style::default().fg(Color::DarkGray).add_modifier(Modifier::ITALIC),
            ),
        ])
    } else {
        // Generic footer: "Enter to confirm · Esc to cancel"
        Line::from(vec![
            Span::styled(
                format!("{} to confirm {} {} to cancel", "Enter", MIDDOT, "Esc"),
                Style::default().fg(Color::DarkGray),
            ),
        ])
    };
    frame.render_widget(Paragraph::new(hint_line), hint_area);
}
