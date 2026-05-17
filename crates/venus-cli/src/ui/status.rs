use ratatui::{
    prelude::*,
    widgets::*,
};

use crate::app::App;

/// Render the status bar at the top of the screen.
pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let mut spans = vec![
        Span::styled(
            " Venus ",
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        ),
        Span::raw("│ "),
        Span::styled(&*app.model(), Style::default().fg(Color::Yellow)),
    ];

    if !app.cost.is_empty() && app.cost != "$0.00" {
        spans.push(Span::raw(" │ "));
        spans.push(Span::styled(
            app.cost.clone(),
            Style::default().fg(Color::Green),
        ));
    }

    if app.context_pct > 0 {
        spans.push(Span::raw(" │ "));
        let ctx_color = if app.context_pct > 80 {
            Color::Red
        } else if app.context_pct > 60 {
            Color::Yellow
        } else {
            Color::Green
        };
        spans.push(Span::styled(
            format!("{}% ctx", app.context_pct),
            Style::default().fg(ctx_color),
        ));
    }

    // Permission mode indicator
    let perm_mode = app.engine.settings.permission_mode.as_deref().unwrap_or("default");
    if perm_mode != "default" {
        spans.push(Span::raw(" │ "));
        let (perm_label, perm_color) = match perm_mode {
            "auto" => ("auto", Color::Yellow),
            "bypass" => ("bypass", Color::Red),
            _ => (perm_mode, Color::DarkGray),
        };
        spans.push(Span::styled(
            perm_label,
            Style::default().fg(perm_color).add_modifier(Modifier::BOLD),
        ));
    }

    // Thinking mode indicator
    let thinking_mode = app.engine.settings.thinking.as_ref()
        .and_then(|t| t.mode.as_deref())
        .unwrap_or("disabled");
    if thinking_mode != "disabled" {
        spans.push(Span::raw(" │ "));
        spans.push(Span::styled(
            "think",
            Style::default().fg(Color::Magenta).add_modifier(Modifier::DIM),
        ));
    }

    // Session name if set
    if let Some(ref name) = app.engine.session_name {
        spans.push(Span::raw(" │ "));
        spans.push(Span::styled(
            name.clone(),
            Style::default().fg(Color::DarkGray),
        ));
    }

    if let Some(ref branch) = app.branch {
        spans.push(Span::raw(" │ "));
        spans.push(Span::styled(
            format!("({})", branch),
            Style::default().fg(Color::DarkGray),
        ));
    }

    if app.spinner.active {
        let elapsed = app.spinner.elapsed_secs();
        if elapsed > 0 {
            spans.push(Span::raw(" │ "));
            spans.push(Span::styled(
                format_elapsed(elapsed),
                Style::default().fg(Color::DarkGray),
            ));
        }
    }

    let status_line = Line::from(spans);
    let paragraph = Paragraph::new(status_line).block(
        Block::default()
            .borders(Borders::BOTTOM)
            .border_style(Style::default().fg(Color::DarkGray)),
    );

    frame.render_widget(paragraph, area);
}

fn format_elapsed(secs: u64) -> String {
    if secs < 60 {
        format!("{}s", secs)
    } else {
        format!("{}m{}s", secs / 60, secs % 60)
    }
}
