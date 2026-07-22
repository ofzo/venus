use ratatui::{prelude::*, widgets::*};

use crate::app::App;
use crate::ui::THEME_COLOR;

/// Render the spinner matching Claude Code's SpinnerAnimationRow exactly.
///
/// Layout:
///   marginTop=1 (blank line above)
///   [2-char glyph][message…] (status)
///
/// Status format: (elapsed · tokens · thinking)
/// All dimColor
pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    // marginTop=1 matching SpinnerAnimationRow
    let spinner_area = Rect {
        x: area.x,
        y: area.y + area.height.saturating_sub(2),
        width: area.width,
        height: 1,
    };

    let glyph = app.spinner_glyph();
    let verb = crate::app::SPINNER_VERBS[app.spinner.verb_index % crate::app::SPINNER_VERBS.len()];
    let elapsed = app.spinner.elapsed_secs();

    // Build status parts (matching Claude Code's parenthesized format)
    let mut status_parts = Vec::new();

    // Timer (shows after some time)
    if elapsed > 5 {
        status_parts.push(format!("{}s", elapsed));
    }

    // Token count (if available)
    let cost_tracker = app.engine.cost_tracker.lock().unwrap();
    let total_tokens = cost_tracker.total_usage().input_tokens
        + cost_tracker.total_usage().cache_read_tokens
        + cost_tracker.total_usage().output_tokens;
    drop(cost_tracker);

    if total_tokens > 0 {
        status_parts.push(format!(
            "\u{2193} {} tokens",
            format_token_count(total_tokens)
        )); // ↓
    }

    // Build the full line
    let mut spans = vec![
        // Glyph (2 chars wide)
        Span::styled(format!(" {}", glyph), Style::default().fg(THEME_COLOR)),
        // Message verb + ellipsis
        Span::styled(
            format!("{}\u{2026}", verb),
            Style::default().fg(Color::DarkGray),
        ),
    ];

    // Status in parentheses (if any parts exist)
    if !status_parts.is_empty() {
        spans.push(Span::styled(
            format!(" ({})", status_parts.join(" \u{00B7} ")), // · separator
            Style::default().fg(Color::DarkGray),
        ));
    }

    let line = Line::from(spans);
    frame.render_widget(Paragraph::new(line), spinner_area);
}

fn format_token_count(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}K", n as f64 / 1_000.0)
    } else {
        format!("{}", n)
    }
}
