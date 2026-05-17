use ratatui::{
    prelude::*,
    widgets::*,
};

use crate::app::App;

/// Render the spinner at the bottom of the message area.
pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let spinner_area = Rect {
        x: area.x,
        y: area.y + area.height.saturating_sub(1),
        width: area.width,
        height: 1,
    };

    let glyph = app.spinner_glyph();
    let elapsed = app.spinner.elapsed_secs();
    let time_str = if elapsed > 0 {
        if elapsed < 60 {
            format!(" ({}s)", elapsed)
        } else {
            format!(" ({}m{}s)", elapsed / 60, elapsed % 60)
        }
    } else {
        String::new()
    };

    let line = Line::from(vec![
        Span::styled(
            format!("  {} ", glyph),
            Style::default().fg(Color::Cyan),
        ),
        Span::styled(
            format!("{}{}", app.spinner.message, time_str),
            Style::default().fg(Color::DarkGray),
        ),
    ]);

    let paragraph = Paragraph::new(line);
    frame.render_widget(paragraph, spinner_area);
}
