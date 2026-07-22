pub mod input;
pub mod messages;
pub mod modal;
pub mod picker;
pub mod spinner;
pub mod status;

use ratatui::prelude::*;

use crate::app::{App, InputMode};

/// Brand accent colour reused across the UI (welcome banner, list
/// markers, spinner, and the input box border lines).
pub(crate) const THEME_COLOR: Color = Color::Rgb(0xFF, 0x4D, 0x6B); // rose (Venus / love), lifted from oklch(65% 0.22 17.585)
/// Full-width panel background for user-message cards: a light
/// (near-white, faintly cool) slate so the user's turn reads as a clear
/// "sticky-note" card against the black canvas. Pair with the dark
/// `USER_MSG_FG` text below for readable contrast.
pub(crate) const USER_MSG_BG: Color = Color::Rgb(0xEC, 0xEC, 0xF2);
/// Foreground for user-message body text on the light card: near-black so
/// bold white text never becomes illegible on a light background.
pub(crate) const USER_MSG_FG: Color = Color::Rgb(0x22, 0x22, 0x2A);
/// Muted caption colour for the `#N` ordinal on the light card.
pub(crate) const USER_MSG_CAPTION_FG: Color = Color::Rgb(0x6F, 0x6F, 0x7A);
/// Muted gray for system-meta divider lines (same RGB value as the
/// user-card caption, exposed under a domain-appropriate name).
pub(crate) const SYS_META_FG: Color = Color::Rgb(0x6F, 0x6F, 0x7A);

/// Diff preview palette (Write/Edit tool calls). Dark, saturated backgrounds
/// so the `+`/`-` bands read clearly against the black canvas while a bright
/// foreground sign pops on top of them.
pub(crate) const DIFF_ADD_BG: Color = Color::Rgb(0x14, 0x3A, 0x22); // dark moss
pub(crate) const DIFF_ADD_FG: Color = Color::Rgb(0xC8, 0xE8, 0xB8); // pale green text
pub(crate) const DIFF_ADD_SIGN: Color = Color::Rgb(0x5B, 0xCC, 0x7A); // bright + sign
pub(crate) const DIFF_REMOVE_BG: Color = Color::Rgb(0x3A, 0x14, 0x1A); // dark wine
pub(crate) const DIFF_REMOVE_FG: Color = Color::Rgb(0xE8, 0xC8, 0xCE); // pale red text
pub(crate) const DIFF_REMOVE_SIGN: Color = Color::Rgb(0xE0, 0x6A, 0x7A); // bright - sign
pub(crate) const DIFF_HUNK_BG: Color = Color::Rgb(0x1C, 0x1B, 0x2E); // dark slate
pub(crate) const DIFF_HUNK_FG: Color = Color::Rgb(0x9A, 0x8A, 0xD6); // muted purple @@ text
pub(crate) const DIFF_LN_FG: Color = Color::Rgb(0x5F, 0x5F, 0x6E); // line-number gutter

/// Render the full TUI layout.
pub fn render(frame: &mut Frame, app: &App) {
    let area = frame.area();

    // Layout per .layout.md:
    // - Messages (fills the top, includes the welcome banner when empty)
    // - Input (3 rows: top border + content + bottom border)
    // - Status bar (1 row, pinned to the bottom)
    let [messages_area, input_area, status_area] = ratatui::layout::Layout::default()
        .direction(ratatui::layout::Direction::Vertical)
        .constraints([
            ratatui::layout::Constraint::Min(3),    // messages (top)
            ratatui::layout::Constraint::Length(3), // input box
            ratatui::layout::Constraint::Length(1), // status bar (bottom)
        ])
        .areas(area);

    // Render messages
    messages::render(frame, messages_area, app);

    // Render spinner (overlaid at bottom of messages area if active)
    if app.spinner.active {
        spinner::render(frame, messages_area, app);
    }

    // Render input area
    input::render(frame, input_area, app);

    // Render status bar at the bottom
    status::render(frame, status_area, app);

    // Render overlays based on input mode
    match app.input_mode {
        InputMode::PermissionPrompt => {
            if let Some(ref pending) = app.pending_permission {
                modal::render(
                    frame,
                    area,
                    &pending.tool_name,
                    &pending.description,
                    pending.selected_option,
                );
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
