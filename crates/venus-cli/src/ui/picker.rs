use ratatui::{
    prelude::*,
    widgets::*,
};

use crate::app::{App, PickerSource};

/// Unicode figures matching Claude Code's figures.ts exactly
const POINTER: &str = "\u{276F}";     // ❯
const TICK: &str = "\u{2714}";        // ✔
const ARROW_UP: &str = "\u{2191}";    // ↑
const ARROW_DOWN: &str = "\u{2193}";  // ↓
const DIVIDER_CHAR: char = '\u{2500}'; // ─
const MIDDOT: &str = "\u{00B7}";      // ·

/// Render picker matching Claude Code's Select/Pane/ModelPicker exactly.
pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let picker = match &app.picker {
        Some(p) => p,
        None => return,
    };

    let is_model_picker = matches!(picker.source, PickerSource::Model);
    let has_tabs = picker.tab_state.is_some();

    // Max index width
    let total_items = picker.items.len();
    let max_index_width = if total_items >= 10 { 2 } else { 1 };
    let index_pad = max_index_width + 2; // "1. " = 3 chars for single digit

    // Pane layout
    let pane_padding_top = 1u16;
    let divider_height = 1u16;
    let tab_line_height = if has_tabs { 1u16 } else { 0 };
    let visible_count = picker.visible_count.min(picker.items.len()) as u16;
    // ModelPicker has: marginBottom=1(header) + select + marginBottom=1(effort) + footer
    // Others have: divider + select + footer
    let header_lines = if is_model_picker { 3 } else { 0 }; // title + desc + blank
    let footer_lines = if is_model_picker { 2 } else { 1 };

    let total_height = pane_padding_top + divider_height + tab_line_height
        + header_lines + visible_count + footer_lines;
    let popup_height = total_height.min(area.height.saturating_sub(2));
    let popup_width = area.width;

    let content_x = 2u16; // paddingX=2
    let content_width = popup_width.saturating_sub(4);

    let mut current_y = (area.height.saturating_sub(popup_height)) / 2;

    // 1. paddingTop=1
    current_y += pane_padding_top;

    // 2. Divider or Tabs
    if has_tabs {
        let tab_state = picker.tab_state.as_ref().unwrap();
        let tab_area = Rect::new(content_x, current_y, content_width, 1);
        let title_color = match picker.source {
            PickerSource::Help => Color::Blue,
            _ => Color::Yellow,
        };
        let mut spans = vec![
            Span::styled(picker.title.clone(), Style::default().fg(title_color).add_modifier(Modifier::BOLD)),
            Span::raw("  "),
        ];
        for (i, tab) in tab_state.tabs.iter().enumerate() {
            if i > 0 { spans.push(Span::raw(" ")); }
            if i == tab_state.selected_tab {
                spans.push(Span::styled(format!(" {} ", tab), Style::default().add_modifier(Modifier::BOLD)));
            } else {
                spans.push(Span::styled(format!(" {} ", tab), Style::default().fg(Color::DarkGray)));
            }
        }
        frame.render_widget(Paragraph::new(Line::from(spans)), tab_area);
        current_y += 1;
    } else {
        let divider_color = match picker.source {
            PickerSource::Help => Color::Blue,
            _ => Color::Yellow,
        };
        let divider_area = Rect::new(0, current_y, popup_width, 1);
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                DIVIDER_CHAR.to_string().repeat(popup_width as usize),
                Style::default().fg(divider_color),
            ))),
            divider_area,
        );
        current_y += 1;
    }

    // 3. ModelPicker header (Select model + description)
    if is_model_picker {
        // "Select model" in "remember" color, bold
        let title_area = Rect::new(content_x, current_y, content_width, 1);
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "Select model",
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD), // "remember" ≈ yellow
            ))),
            title_area,
        );
        current_y += 1;

        // Description in dimColor
        let desc_area = Rect::new(content_x, current_y, content_width, 1);
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "Switch between Claude models. For other model names, specify with --model.",
                Style::default().fg(Color::DarkGray),
            ))),
            desc_area,
        );
        current_y += 1;

        // Blank line (marginBottom=1)
        current_y += 1;
    }

    // 4. Select options (compact layout with two-column)
    // Check if we should use two-column layout (when descriptions exist)
    let has_descriptions = picker.items.iter().any(|i| !i.description.is_empty() && !i.value.is_empty());

    for (vis_idx, item_idx) in (picker.scroll_offset..picker.scroll_offset + picker.visible_count).enumerate() {
        if item_idx >= picker.items.len() { break; }
        let item = &picker.items[item_idx];
        let is_focused = item_idx == picker.selected;
        let is_separator = item.value.is_empty() && item.description.is_empty();

        let item_y = current_y + vis_idx as u16;
        let item_area = Rect::new(content_x, item_y, content_width, 1);

        if is_separator {
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    item.label.clone(),
                    Style::default().fg(Color::DarkGray).add_modifier(Modifier::DIM),
                ))),
                item_area,
            );
            continue;
        }

        let mut spans = Vec::new();

        if has_descriptions && !item.description.is_empty() {
            // Two-column layout (matching TwoColumnRow exactly)
            // Indicator
            if is_focused {
                spans.push(Span::styled(POINTER, Style::default().fg(Color::Cyan)));
            } else if vis_idx == 0 && picker.scroll_offset > 0 {
                spans.push(Span::styled(ARROW_DOWN, Style::default().fg(Color::DarkGray)));
            } else if vis_idx == picker.visible_count - 1 && item_idx + 1 < picker.items.len() {
                spans.push(Span::styled(ARROW_UP, Style::default().fg(Color::DarkGray)));
            } else {
                spans.push(Span::raw(" "));
            }

            // Space
            spans.push(Span::raw(" "));

            // Index prefix (dimColor) + Label (colored)
            let index_str = format!("{:>width$}.", item_idx + 1, width = max_index_width);
            let padded = format!("{:<width$}", index_str, width = index_pad);
            spans.push(Span::styled(padded, Style::default().fg(Color::DarkGray)));

            // Label
            let label_style = if is_focused {
                Style::default().fg(Color::Cyan)
            } else {
                Style::default()
            };
            spans.push(Span::styled(item.label.clone(), label_style));

            // Padding to align descriptions
            let used_width: usize = 1 + 1 + index_pad + item.label.chars().count();
            let label_col_width: usize = 30;
            let padding = label_col_width.saturating_sub(used_width);
            if padding > 0 {
                spans.push(Span::raw(" ".repeat(padding)));
            }

            // Description (marginLeft=2, dimColor)
            spans.push(Span::raw("  "));
            spans.push(Span::styled(
                item.description.clone(),
                Style::default().fg(Color::DarkGray),
            ));
        } else {
            // Compact layout (no descriptions or inline)
            // The index is INSIDE the Text with dimColor, label follows
            if is_focused {
                spans.push(Span::styled(POINTER, Style::default().fg(Color::Cyan)));
            } else if vis_idx == 0 && picker.scroll_offset > 0 {
                spans.push(Span::styled(ARROW_DOWN, Style::default().fg(Color::DarkGray)));
            } else if vis_idx == picker.visible_count - 1 && item_idx + 1 < picker.items.len() {
                spans.push(Span::styled(ARROW_UP, Style::default().fg(Color::DarkGray)));
            } else {
                spans.push(Span::raw(" "));
            }

            // Space
            spans.push(Span::raw(" "));

            // Index prefix (dimColor)
            let index_str = format!("{:>width$}.", item_idx + 1, width = max_index_width);
            let padded = format!("{:<width$}", index_str, width = index_pad);
            spans.push(Span::styled(padded, Style::default().fg(Color::DarkGray)));

            // Label (colored)
            let label_style = if is_focused {
                Style::default().fg(Color::Cyan)
            } else {
                Style::default()
            };
            spans.push(Span::styled(item.label.clone(), label_style));
        }

        frame.render_widget(Paragraph::new(Line::from(spans)), item_area);
    }

    current_y += visible_count;

    // 5. ModelPicker effort indicator
    if is_model_picker {
        // Blank line (marginBottom=1 before effort)
        current_y += 1;

        let effort = app.get_effort_label();
        let effort_symbol = match effort {
            "low" => "\u{25CB}",
            "medium" => "\u{25D0}",
            "high" => "\u{25CF}",
            "max" => "\u{25C9}",
            _ => "\u{25CB}",
        };
        let capitalized = format!("{}{}", &effort[..1].to_uppercase(), &effort[1..]);
        let effort_area = Rect::new(content_x, current_y, content_width, 1);
        // Matching: <Text dimColor><EffortLevelIndicator effort={displayEffort}/> {capitalize} effort <Text color="subtle">← → to adjust</Text></Text>
        let effort_line = Line::from(vec![
            Span::styled(effort_symbol, Style::default().fg(Color::DarkGray)), // "claude" ≈ dim
            Span::raw(" "),
            Span::styled(format!("{} effort", capitalized), Style::default().fg(Color::DarkGray)),
            Span::raw("  "),
            Span::styled("\u{2190} \u{2192} to adjust", Style::default().fg(Color::DarkGray)), // "subtle" ≈ dim
        ]);
        frame.render_widget(Paragraph::new(effort_line), effort_area);
        current_y += 1;
    }

    // 6. Footer
    let hint_area = Rect::new(content_x, current_y, content_width, 1);
    let footer_text = if is_model_picker {
        // italic, dimColor: "Enter to confirm · Esc to exit"
        format!("{} to confirm {} {} to exit", "Enter", MIDDOT, "Esc")
    } else {
        format!("{} to confirm {} {} to cancel", "Enter", MIDDOT, "Esc")
    };
    let footer_style = Style::default().fg(Color::DarkGray);
    let footer_style = if is_model_picker {
        footer_style.add_modifier(Modifier::ITALIC)
    } else {
        footer_style
    };
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(footer_text, footer_style))),
        hint_area,
    );
}
