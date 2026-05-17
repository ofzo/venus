pub mod input;
pub mod messages;
pub mod modal;
pub mod spinner;
pub mod status;

use ratatui::Frame;

use crate::app::App;

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

    // Render permission modal overlay (if active)
    if app.input_mode == crate::app::InputMode::PermissionPrompt {
        if let Some(ref pending) = app.pending_permission {
            modal::render(frame, area, &pending.tool_name, &pending.description);
        }
    }
}
