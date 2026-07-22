use ratatui::{
    layout::{Constraint, Layout},
    prelude::*,
    widgets::*,
};

use crate::app::App;
use crate::ui::messages::shorten_home;
use crate::ui::THEME_COLOR;

/// Bottom status bar (per `.layout.md`):
///   `<model> <effort> · <dir> · <branch> · <git_changes> · <mode> · used <ctx>%`
/// Each field is tinted a distinct colour and the items are separated by a
/// middle dot. A right-aligned hint reads `Tip: /compact to compact context`.
pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let dir = shorten_home(&app.engine.working_dir);
    let branch = app.branch.clone().unwrap_or_default();
    let mode = app
        .engine
        .settings
        .permission_mode
        .as_deref()
        .unwrap_or("default");
    let effort = app.get_effort_label().to_string();

    // Each status field gets its own colour; items are separated by a
    // middle dot (" \u{00B7} ") instead of a pipe.
    let sep = Span::styled(" \u{00B7} ", Style::default().fg(Color::DarkGray));
    let spans = vec![
        Span::styled(
            format!("{} {}", app.model(), effort),
            Style::default().fg(THEME_COLOR),
        ),
        sep.clone(),
        Span::styled(dir, Style::default().fg(Color::Blue)),
        sep.clone(),
        Span::styled(branch, Style::default().fg(Color::Magenta)),
        sep.clone(),
        Span::styled(app.git_changes.clone(), Style::default().fg(Color::Yellow)),
        sep.clone(),
        Span::styled(mode.to_string(), Style::default().fg(Color::Green)),
        sep.clone(),
        Span::styled(
            format!("used {}%", app.context_pct),
            Style::default().fg(Color::Gray),
        ),
    ];
    let left_line = Line::from(spans);

    // Right-aligned, single-cell hint.
    let tip = "Tip: /compact to compact context";
    let tip_line = Line::from(Span::styled(
        tip.to_string(),
        Style::default().fg(Color::DarkGray),
    ));

    let tip_w = tip.len() as u16;
    let [left_area, right_area] =
        Layout::horizontal([Constraint::Min(0), Constraint::Length(tip_w)])
            .spacing(2)
            .areas(area);

    frame.render_widget(Paragraph::new(left_line), left_area);
    frame.render_widget(
        Paragraph::new(tip_line).alignment(ratatui::layout::Alignment::Right),
        right_area,
    );
}
