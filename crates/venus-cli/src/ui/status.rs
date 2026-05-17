use ratatui::{
    prelude::*,
    widgets::*,
};

use crate::app::App;

/// Render the status bar at the top of the screen.
/// Claude Code uses dimColor for the entire status line.
pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let mut parts = Vec::new();

    // Model name
    parts.push(app.model().to_string());

    // Cost
    if !app.cost.is_empty() && app.cost != "$0.00" {
        parts.push(app.cost.clone());
    }

    // Context percentage
    if app.context_pct > 0 {
        parts.push(format!("{}% ctx", app.context_pct));
    }

    // Permission mode (only if not default)
    let perm_mode = app.engine.settings.permission_mode.as_deref().unwrap_or("default");
    if perm_mode != "default" {
        parts.push(perm_mode.to_string());
    }

    // Thinking mode (only if enabled)
    let thinking_mode = app.engine.settings.thinking.as_ref()
        .and_then(|t| t.mode.as_deref())
        .unwrap_or("disabled");
    if thinking_mode != "disabled" {
        parts.push("think".to_string());
    }

    // Session name
    if let Some(ref name) = app.engine.session_name {
        parts.push(name.clone());
    }

    // Git branch
    if let Some(ref branch) = app.branch {
        parts.push(format!("({})", branch));
    }

    let status_text = parts.join(" │ ");

    // Claude Code uses dimColor for the entire status line
    let status_line = Line::from(Span::styled(
        status_text,
        Style::default().fg(Color::DarkGray),
    ));

    let paragraph = Paragraph::new(status_line).block(
        Block::default()
            .borders(Borders::BOTTOM)
            .border_style(Style::default().fg(Color::DarkGray)),
    );

    frame.render_widget(paragraph, area);
}
