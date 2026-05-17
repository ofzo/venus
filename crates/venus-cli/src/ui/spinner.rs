use ratatui::{
    prelude::*,
    widgets::*,
};

use crate::app::App;

/// Render the spinner matching Claude Code's SpinnerAnimationRow exactly.
///
/// Layout:
///   [2-char glyph][message...] (no gap between glyph and message)
///   marginTop=1 (1 row above spinner)
pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    // marginTop=1 matching SpinnerAnimationRow
    let spinner_area = Rect {
        x: area.x,
        y: area.y + area.height.saturating_sub(2), // 2 rows from bottom (1 for margin + 1 for content)
        width: area.width,
        height: 1,
    };

    let glyph = app.spinner_glyph();
    let message = app.spinner.display_message();

    // Claude Code: glyph is 2 chars wide, then message immediately after (no gap)
    let line = Line::from(vec![
        Span::styled(
            format!(" {} ", glyph), // 2 chars wide with padding
            Style::default().fg(Color::Cyan),
        ),
        Span::styled(
            message,
            Style::default().fg(Color::DarkGray),
        ),
    ]);

    let paragraph = Paragraph::new(line);
    frame.render_widget(paragraph, spinner_area);
}
